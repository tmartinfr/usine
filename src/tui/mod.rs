//! Interactive terminal UI: Jira issues and GitLab MRs in two stacked panes.

mod app;
mod ui;

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::Result;
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event, KeyEventKind};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use usine::config::Config;
use usine::connectors::gitlab::GitlabClient;
use usine::connectors::jira::JiraClient;

use app::{Action, App, Msg};

const LIMIT: usize = 100;
const AUTO_REFRESH: Duration = Duration::from_secs(5 * 60);
const TICK: Duration = Duration::from_millis(200);

/// Clients of the configured connectors.
struct Sources {
    jira: Option<(JiraClient, String)>,
    gitlab: Option<GitlabClient>,
}

impl Sources {
    fn new(config: &Config) -> Result<Self> {
        let jira = match &config.jira {
            Some(jira) => Some((JiraClient::new(jira)?, jira.effective_jql().to_string())),
            None => None,
        };
        let gitlab = config.gitlab.as_ref().map(GitlabClient::new).transpose()?;
        Ok(Self { jira, gitlab })
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
        if let Some(client) = &self.gitlab {
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
    app.start_refresh();
    sources.refresh(tx);

    loop {
        while let Ok(msg) = rx.try_recv() {
            app.on_msg(msg);
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
                        app.message = Some(format!("cannot open {url}: {err}"));
                    }
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
