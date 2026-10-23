//! Interactive terminal UI: Jira issues, GitLab MRs and their Claude reviews.

mod app;
mod sort;
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
use usine::api_log::ApiLog;
use usine::config::Config;
use usine::connectors::claude::{ClaudeConfig, Tmux};
use usine::connectors::git::GitConfig;
use usine::connectors::gitlab::{GitlabClient, GitlabConfig};
use usine::connectors::jira::JiraClient;
use usine::model::MergeRequest;
use usine::workflows::review;

use app::{Action, App, Msg, Notice};

const LIMIT: usize = 100;
const AUTO_REFRESH: Duration = Duration::from_secs(60);
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
    fn new(config: &Config, log: &ApiLog) -> Result<Self> {
        let jira = match &config.jira {
            Some(jira) => {
                let client = JiraClient::new(jira)?.with_log(log.clone());
                Some((client, jira.effective_jql().to_string()))
            }
            None => None,
        };
        let gitlab = match &config.gitlab {
            Some(gitlab) => Some((GitlabClient::new(gitlab)?.with_log(log.clone()), gitlab.clone())),
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
                let result = tokio::try_join!(
                    client.my_open_merge_requests(LIMIT),
                    client.merge_requests_to_review(LIMIT),
                )
                .map(|(authored, to_review)| merge_lists(authored, to_review, LIMIT))
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

/// MRs I authored and MRs I review, most recently updated first; an MR in both
/// lists is kept as authored.
fn merge_lists(
    authored: Vec<MergeRequest>,
    to_review: Vec<MergeRequest>,
    limit: usize,
) -> Vec<MergeRequest> {
    let mut mrs = authored;
    for mr in to_review {
        if !mrs.iter().any(|m| m.reference == mr.reference) {
            mrs.push(mr);
        }
    }
    mrs.sort_by_key(|m| std::cmp::Reverse(m.updated));
    mrs.truncate(limit);
    mrs
}

/// Must be called from within the tokio runtime.
pub fn run(config: Config, config_path: &Path) -> Result<()> {
    let api_log = ApiLog::default();
    let sources = Sources::new(&config, &api_log)?;
    let mut app = App::new(
        sources.jira.is_some(),
        sources.gitlab.is_some(),
        config_path.display().to_string(),
    );
    app.api_log = api_log;
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
        if !app.notifications.is_empty() {
            // Best effort: no notification daemon must not break the UI.
            let _ = notify(&app.notifications.join("\n"));
            app.notifications.clear();
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
                Action::StartReview(mr) => sources.start_review(*mr, tx),
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
    spawn_quiet(Command::new(opener).arg(url))
}

/// Shows `body` as a desktop notification.
fn notify(body: &str) -> std::io::Result<()> {
    let mut command = if cfg!(target_os = "macos") {
        // Passed as an argument, not interpolated, so quotes in titles are safe.
        let mut command = Command::new("osascript");
        command.args([
            "-e",
            "on run argv",
            "-e",
            "display notification (item 1 of argv) with title \"usine\"",
            "-e",
            "end run",
            body,
        ]);
        command
    } else {
        let mut command = Command::new("notify-send");
        command.args(["--app-name=usine", "usine", body]);
        command
    };
    spawn_quiet(&mut command)
}

/// Runs `command` in the background, detached from the terminal.
fn spawn_quiet(command: &mut Command) -> std::io::Result<()> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    // Reap the process so it does not linger as a zombie.
    std::thread::spawn(move || child.wait());
    Ok(())
}

#[cfg(test)]
mod tests {
    use chrono::DateTime;
    use usine::model::Role;

    use super::*;
    use crate::tui::app::tests::mr;

    fn updated(mut mr: MergeRequest, day: u32, role: Role) -> MergeRequest {
        mr.updated = DateTime::parse_from_rfc3339(&format!("2026-10-{day:02}T12:00:00Z")).unwrap();
        mr.role = role;
        mr
    }

    #[test]
    fn merges_authored_and_review_lists_by_date() {
        let authored = vec![updated(mr(1), 20, Role::Author), updated(mr(2), 10, Role::Author)];
        let to_review = vec![updated(mr(3), 15, Role::Reviewer), updated(mr(2), 10, Role::Reviewer)];

        let mrs = merge_lists(authored, to_review, 100);

        let listed: Vec<(u64, Role)> = mrs.iter().map(|m| (m.iid, m.role)).collect();
        assert_eq!(listed, [(1, Role::Author), (3, Role::Reviewer), (2, Role::Author)]);
        assert_eq!(merge_lists(mrs, vec![], 2).len(), 2);
    }
}
