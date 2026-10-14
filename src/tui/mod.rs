//! Interactive terminal UI: Jira issues, GitLab MRs and their Claude reviews.

mod app;
mod ui;

use std::io::stdout;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use ratatui::backend::CrosstermBackend;
use ratatui::{DefaultTerminal, Terminal};
use ratatui::crossterm::event::{self, Event, KeyEventKind};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{Clear, ClearType, EnterAlternateScreen, enable_raw_mode};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use usine::config::Config;
use usine::connectors::claude::{ClaudeConfig, Tmux};
use usine::connectors::git::GitConfig;
use usine::connectors::gitlab::{GitlabClient, GitlabConfig};
use usine::connectors::jira::JiraClient;
use usine::model::MergeRequest;
use usine::workflows::review;

use app::{Action, App, Msg, Notice};

const LIMIT: usize = 100;
const AUTO_REFRESH: Duration = Duration::from_secs(5 * 60);
const REVIEW_POLL: Duration = Duration::from_secs(1);
const TICK: Duration = Duration::from_millis(200);

/// Clients of the configured connectors.
struct Sources {
    jira: Option<(JiraClient, String)>,
    gitlab: Option<(GitlabClient, GitlabConfig)>,
    git: Option<GitConfig>,
    claude: ClaudeConfig,
    tmux: Tmux,
}

impl Sources {
    fn new(config: &Config) -> Result<Self> {
        let jira = match &config.jira {
            Some(jira) => Some((JiraClient::new(jira)?, jira.effective_jql().to_string())),
            None => None,
        };
        let gitlab = match &config.gitlab {
            Some(gitlab) => Some((GitlabClient::new(gitlab)?, gitlab.clone())),
            None => None,
        };
        Ok(Self {
            jira,
            gitlab,
            git: config.git.clone(),
            claude: config.claude.clone(),
            tmux: Tmux::new(),
        })
    }

    /// Fetches every source in the background; results arrive on `tx`.
    fn refresh(&self, tx: &UnboundedSender<Msg>) {
        if let Some((client, jql)) = &self.jira {
            let (client, jql, tx) = (client.clone(), jql.clone(), tx.clone());
            tokio::spawn(async move {
                let result = client.search(&jql, LIMIT).await.map_err(|e| format!("{e:#}"));
                let _ = tx.send(Msg::Jira(result));
            });
        }
        if let Some((client, _)) = &self.gitlab {
            let (client, tx) = (client.clone(), tx.clone());
            tokio::spawn(async move {
                let result = client
                    .my_open_merge_requests(LIMIT)
                    .await
                    .map_err(|e| format!("{e:#}"));
                let _ = tx.send(Msg::Gitlab(result));
            });
        }
    }

    /// Lists review sessions and captures the screen of `selected`.
    fn poll_reviews(&self, selected: Option<String>, tx: &UnboundedSender<Msg>) {
        let (tmux, tx) = (self.tmux.clone(), tx.clone());
        tokio::task::spawn_blocking(move || {
            // tmux missing or failing: no review to show.
            let _ = tx.send(Msg::Reviews(review::list_reviews(&tmux).unwrap_or_default()));
            if let Some(session) = selected
                && let Ok(text) = tmux.capture(&session, true)
            {
                let _ = tx.send(Msg::Preview { session, text });
            }
        });
    }

    fn start_review(&self, mr: MergeRequest, tx: &UnboundedSender<Msg>) {
        let (gitlab, git) = match (&self.gitlab, &self.git) {
            (Some((_, gitlab)), Some(git)) => (gitlab.clone(), git.clone()),
            _ => {
                let result = Err("add a [git] section with the roots of your clones".into());
                let _ = tx.send(Msg::ReviewStarted { mr: mr.reference, result });
                return;
            }
        };
        let (claude, tmux, tx) = (self.claude.clone(), self.tmux.clone(), tx.clone());
        tokio::task::spawn_blocking(move || {
            let result = review::start_review(&gitlab, &git, &claude, &tmux, &mr)
                .map_err(|e| format!("{e:#}"));
            let _ = tx.send(Msg::ReviewStarted { mr: mr.reference, result });
        });
    }

    fn close_review(&self, session: String, tx: &UnboundedSender<Msg>) {
        let (tmux, tx) = (self.tmux.clone(), tx.clone());
        tokio::task::spawn_blocking(move || {
            if let Err(err) = tmux.kill(&session) {
                let _ = tx.send(Msg::Notice(Notice::error(format!("{err:#}"))));
            }
        });
    }
}

/// Must be called from within the tokio runtime.
pub fn run(config: Config, config_path: &Path) -> Result<()> {
    let sources = Sources::new(&config)?;
    let mut app = App::new(
        sources.jira.is_some(),
        sources.gitlab.is_some(),
        config_path.display().to_string(),
    );
    let (tx, rx) = unbounded_channel();

    // ratatui::init also installs a panic hook that restores the terminal.
    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal, &mut app, &sources, &tx, rx);
    ratatui::restore();
    result
}

fn event_loop(
    terminal: &mut DefaultTerminal,
    app: &mut App,
    sources: &Sources,
    tx: &UnboundedSender<Msg>,
    mut rx: UnboundedReceiver<Msg>,
) -> Result<()> {
    let mut last_refresh = Instant::now();
    let mut last_poll: Option<Instant> = None;
    app.start_refresh();
    sources.refresh(tx);

    loop {
        while let Ok(msg) = rx.try_recv() {
            app.on_msg(msg);
        }
        if sources.gitlab.is_some() && last_poll.is_none_or(|t| t.elapsed() >= REVIEW_POLL) {
            last_poll = Some(Instant::now());
            sources.poll_reviews(app.selected_review().map(|s| s.name.clone()), tx);
        }
        terminal.draw(|frame| ui::render(frame, app))?;

        let mut refresh = last_refresh.elapsed() >= AUTO_REFRESH;
        if event::poll(TICK)?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            match app.on_key(key) {
                Action::Quit => return Ok(()),
                Action::Refresh => refresh = true,
                Action::Open(url) => {
                    if let Err(err) = open_in_browser(&url) {
                        app.message = Some(Notice::error(format!("cannot open {url}: {err}")));
                    }
                }
                Action::StartReview(mr) => sources.start_review(mr, tx),
                Action::Attach(session) => {
                    if let Err(err) = attach(terminal, &sources.tmux, &session) {
                        app.message = Some(Notice::error(format!("{err:#}")));
                    }
                    last_poll = None;
                }
                Action::CloseReview(session) => {
                    sources.close_review(session, tx);
                    last_poll = None;
                }
                Action::None => {}
            }
        }
        if refresh {
            last_refresh = Instant::now();
            app.start_refresh();
            sources.refresh(tx);
        }
    }
}

/// Hands the terminal over to the tmux session until the user detaches.
fn attach(terminal: &mut DefaultTerminal, tmux: &Tmux, session: &str) -> Result<()> {
    let mut command = tmux.attach_command(session);
    if std::env::var_os("TMUX").is_some() {
        // Inside tmux: the client switches session, usine keeps running.
        let output = command.output()?;
        if !output.status.success() {
            bail!("tmux: {}", String::from_utf8_lossy(&output.stderr).trim());
        }
        return Ok(());
    }
    ratatui::restore();
    let status = command.status();
    enable_raw_mode()?;
    execute!(stdout(), EnterAlternateScreen, Clear(ClearType::All))?;
    // A new terminal has empty buffers, so the next draw repaints everything
    // (Terminal::clear would query the cursor position).
    *terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
    let status = status?;
    if !status.success() {
        bail!("tmux attach exited with {status}");
    }
    Ok(())
}

fn open_in_browser(url: &str) -> std::io::Result<()> {
    let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    let mut child = Command::new(opener)
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    // Reap the process so it does not linger as a zombie.
    std::thread::spawn(move || child.wait());
    Ok(())
}
