//! TUI state and key handling, independent from the terminal.

use chrono::{DateTime, Local};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::widgets::TableState;
use usine::connectors::claude::Session;
use usine::model::{Issue, MergeRequest};

/// Something a pane can list and open in a browser.
pub trait Item {
    fn url(&self) -> &str;
}

impl Item for Issue {
    fn url(&self) -> &str {
        &self.url
    }
}

impl Item for MergeRequest {
    fn url(&self) -> &str {
        &self.url
    }
}

pub enum Content<T> {
    /// The connector has no section in the config file.
    NotConfigured,
    /// First fetch not finished yet.
    Pending,
    Loaded(Vec<T>),
    Failed(String),
}

pub struct Pane<T> {
    pub content: Content<T>,
    /// A fetch is in flight (the previous content stays visible).
    pub loading: bool,
    pub table: TableState,
}

impl<T: Item> Pane<T> {
    fn new(configured: bool) -> Self {
        Self {
            content: if configured { Content::Pending } else { Content::NotConfigured },
            loading: false,
            table: TableState::default(),
        }
    }

    pub fn items(&self) -> &[T] {
        match &self.content {
            Content::Loaded(items) => items,
            _ => &[],
        }
    }

    pub fn selected(&self) -> Option<&T> {
        self.items().get(self.table.selected()?)
    }

    fn start_loading(&mut self) {
        if !matches!(self.content, Content::NotConfigured) {
            self.loading = true;
        }
    }

    fn set_result(&mut self, result: Result<Vec<T>, String>) {
        self.loading = false;
        self.content = match result {
            Ok(items) => Content::Loaded(items),
            Err(err) => Content::Failed(err),
        };
        let len = self.items().len();
        let selected = match len {
            0 => None,
            _ => Some(self.table.selected().unwrap_or(0).min(len - 1)),
        };
        self.table.select(selected);
    }

    fn select(&mut self, target: impl FnOnce(usize, usize) -> isize) {
        let len = self.items().len();
        if len == 0 {
            return;
        }
        let current = self.table.selected().unwrap_or(0);
        let index = target(current, len).clamp(0, len as isize - 1);
        self.table.select(Some(index as usize));
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Jira,
    Gitlab,
}

#[derive(Debug, PartialEq)]
pub enum Action {
    None,
    Quit,
    Refresh,
    Open(String),
    StartReview(MergeRequest),
    /// Attach the terminal to this tmux session.
    Attach(String),
    /// Kill this tmux session.
    CloseReview(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Info,
    Warning,
    Error,
}

/// Transient message shown in the footer until the next key press.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub level: Level,
    pub text: String,
}

impl Notice {
    pub fn info(text: impl Into<String>) -> Self {
        Self { level: Level::Info, text: text.into() }
    }

    pub fn warning(text: impl Into<String>) -> Self {
        Self { level: Level::Warning, text: text.into() }
    }

    pub fn error(text: impl Into<String>) -> Self {
        Self { level: Level::Error, text: text.into() }
    }
}

pub enum Msg {
    Jira(Result<Vec<Issue>, String>),
    Gitlab(Result<Vec<MergeRequest>, String>),
    Reviews(Vec<Session>),
    Preview { session: String, text: String },
    ReviewStarted { mr: String, result: Result<String, String> },
    Notice(Notice),
}

pub struct App {
    pub jira: Pane<Issue>,
    pub gitlab: Pane<MergeRequest>,
    pub focus: Focus,
    pub config_path: String,
    pub last_update: Option<DateTime<Local>>,
    pub message: Option<Notice>,
    /// Review sessions, running or finished; `subject` is the MR reference.
    pub reviews: Vec<Session>,
    /// MR references whose review is being set up.
    pub starting: Vec<String>,
    /// Last captured screen of a review session: (session name, text).
    pub preview: Option<(String, String)>,
}

impl App {
    pub fn new(jira_configured: bool, gitlab_configured: bool, config_path: String) -> Self {
        Self {
            jira: Pane::new(jira_configured),
            gitlab: Pane::new(gitlab_configured),
            focus: Focus::Jira,
            config_path,
            last_update: None,
            message: None,
            reviews: Vec::new(),
            starting: Vec::new(),
            preview: None,
        }
    }

    pub fn start_refresh(&mut self) {
        self.jira.start_loading();
        self.gitlab.start_loading();
    }

    pub fn review_of(&self, mr_reference: &str) -> Option<&Session> {
        self.reviews.iter().find(|s| s.subject == mr_reference)
    }

    /// Review session of the MR selected in the GitLab pane.
    pub fn selected_review(&self) -> Option<&Session> {
        self.review_of(&self.gitlab.selected()?.reference)
    }

    /// Captured screen of `session`, if any.
    pub fn preview_of(&self, session: &str) -> Option<&str> {
        match &self.preview {
            Some((name, text)) if name == session => Some(text),
            _ => None,
        }
    }

    pub fn on_msg(&mut self, msg: Msg) {
        match msg {
            Msg::Jira(result) => {
                if result.is_ok() {
                    self.last_update = Some(Local::now());
                }
                self.jira.set_result(result);
            }
            Msg::Gitlab(result) => {
                if result.is_ok() {
                    self.last_update = Some(Local::now());
                }
                self.gitlab.set_result(result);
            }
            Msg::Reviews(reviews) => self.reviews = reviews,
            Msg::Preview { session, text } => self.preview = Some((session, text)),
            Msg::ReviewStarted { mr, result } => {
                self.starting.retain(|r| *r != mr);
                match result {
                    Ok(name) => {
                        self.message =
                            Some(Notice::info(format!("Review of {mr} started, press a to attach")));
                        if !self.reviews.iter().any(|s| s.name == name) {
                            self.reviews.push(Session { name, subject: mr, finished: false });
                        }
                    }
                    Err(err) => {
                        self.message = Some(Notice::error(format!("Cannot review {mr}: {err}")));
                    }
                }
            }
            Msg::Notice(notice) => self.message = Some(notice),
        }
    }

    pub fn on_key(&mut self, key: KeyEvent) -> Action {
        self.message = None;
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => return Action::Quit,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Action::Quit;
            }
            KeyCode::Char('r') => return Action::Refresh,
            KeyCode::Tab | KeyCode::BackTab => {
                self.focus = match self.focus {
                    Focus::Jira => Focus::Gitlab,
                    Focus::Gitlab => Focus::Jira,
                };
            }
            KeyCode::Enter => {
                let url = match self.focus {
                    Focus::Jira => self.jira.selected().map(|i| i.url().to_string()),
                    Focus::Gitlab => self.gitlab.selected().map(|m| m.url().to_string()),
                };
                return url.map_or(Action::None, Action::Open);
            }
            KeyCode::Char('j') | KeyCode::Down => self.select(|current, _| current as isize + 1),
            KeyCode::Char('k') | KeyCode::Up => self.select(|current, _| current as isize - 1),
            KeyCode::Char('g') | KeyCode::Home => self.select(|_, _| 0),
            KeyCode::Char('G') | KeyCode::End => self.select(|_, len| len as isize - 1),
            KeyCode::Char('v') if self.focus == Focus::Gitlab => return self.start_review(),
            KeyCode::Char('a') if self.focus == Focus::Gitlab => {
                return match self.selected_review() {
                    Some(review) => Action::Attach(review.name.clone()),
                    None => {
                        self.message =
                            Some(Notice::warning("No review for this MR, press v to start one"));
                        Action::None
                    }
                };
            }
            KeyCode::Char(c @ ('x' | 'X')) if self.focus == Focus::Gitlab => {
                return match self.selected_review() {
                    Some(review) if review.finished || c == 'X' => {
                        Action::CloseReview(review.name.clone())
                    }
                    Some(_) => {
                        self.message =
                            Some(Notice::warning("Review still running: press X to close it anyway"));
                        Action::None
                    }
                    None => Action::None,
                };
            }
            _ => {}
        }
        Action::None
    }

    fn start_review(&mut self) -> Action {
        let Some(mr) = self.gitlab.selected().cloned() else {
            return Action::None;
        };
        if self.review_of(&mr.reference).is_some() {
            self.message = Some(Notice::warning("This MR already has a review, press a to attach"));
            return Action::None;
        }
        if self.starting.contains(&mr.reference) {
            self.message = Some(Notice::warning("Review already starting"));
            return Action::None;
        }
        self.starting.push(mr.reference.clone());
        Action::StartReview(mr)
    }

    fn select(&mut self, target: impl FnOnce(usize, usize) -> isize) {
        match self.focus {
            Focus::Jira => self.jira.select(target),
            Focus::Gitlab => self.gitlab.select(target),
        }
    }
}

#[cfg(test)]
pub mod tests {
    use chrono::DateTime;
    use usine::model::Source;

    use super::*;

    pub fn issue(key: &str) -> Issue {
        Issue {
            source: Source::Jira,
            key: key.into(),
            title: format!("Summary of {key}"),
            status_category: usine::model::StatusCategory::Todo,
            status: "To Do".into(),
            priority: None,
            project: "PAY".into(),
            kind: "Task".into(),
            updated: DateTime::parse_from_rfc3339("2026-10-10T12:00:00+02:00").unwrap(),
            url: format!("https://example.atlassian.net/browse/{key}"),
        }
    }

    pub fn mr(iid: u32) -> MergeRequest {
        MergeRequest {
            source: Source::Gitlab,
            reference: format!("group/project!{iid}"),
            project: "group/project".into(),
            iid: iid.into(),
            title: format!("MR {iid}"),
            draft: false,
            merge_status: "mergeable".into(),
            source_branch: "feat".into(),
            target_branch: "main".into(),
            author: "me".into(),
            updated: DateTime::parse_from_rfc3339("2026-10-10T12:00:00Z").unwrap(),
            url: format!("https://gitlab.com/group/project/-/merge_requests/{iid}"),
        }
    }

    pub fn session(iid: u32, finished: bool) -> Session {
        Session {
            name: format!("usine-review-group-project-{iid}"),
            subject: format!("group/project!{iid}"),
            finished,
        }
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn loaded_app() -> App {
        let mut app = App::new(true, true, "config.toml".into());
        app.on_msg(Msg::Jira(Ok(vec![issue("PAY-1"), issue("PAY-2"), issue("PAY-3")])));
        app.on_msg(Msg::Gitlab(Ok(vec![mr(1), mr(2)])));
        app
    }

    #[test]
    fn panes_start_pending_or_not_configured() {
        let app = App::new(true, false, "config.toml".into());
        assert!(matches!(app.jira.content, Content::Pending));
        assert!(matches!(app.gitlab.content, Content::NotConfigured));
    }

    #[test]
    fn first_load_selects_first_row() {
        let app = loaded_app();
        assert_eq!(app.jira.table.selected(), Some(0));
        assert_eq!(app.gitlab.table.selected(), Some(0));
        assert!(app.last_update.is_some());
    }

    #[test]
    fn moves_selection_within_bounds() {
        let mut app = loaded_app();
        app.on_key(key(KeyCode::Up));
        assert_eq!(app.jira.table.selected(), Some(0));
        app.on_key(key(KeyCode::Char('j')));
        app.on_key(key(KeyCode::Down));
        app.on_key(key(KeyCode::Down));
        assert_eq!(app.jira.table.selected(), Some(2));
        app.on_key(key(KeyCode::Char('g')));
        assert_eq!(app.jira.table.selected(), Some(0));
        app.on_key(key(KeyCode::Char('G')));
        assert_eq!(app.jira.table.selected(), Some(2));
    }

    #[test]
    fn tab_switches_focus_and_moves_only_focused_pane() {
        let mut app = loaded_app();
        assert_eq!(app.on_key(key(KeyCode::Tab)), Action::None);
        assert_eq!(app.focus, Focus::Gitlab);
        app.on_key(key(KeyCode::Char('j')));
        assert_eq!(app.gitlab.table.selected(), Some(1));
        assert_eq!(app.jira.table.selected(), Some(0));
        app.on_key(key(KeyCode::BackTab));
        assert_eq!(app.focus, Focus::Jira);
    }

    #[test]
    fn enter_opens_selected_item_of_focused_pane() {
        let mut app = loaded_app();
        app.on_key(key(KeyCode::Char('j')));
        assert_eq!(
            app.on_key(key(KeyCode::Enter)),
            Action::Open("https://example.atlassian.net/browse/PAY-2".into())
        );
        app.on_key(key(KeyCode::Tab));
        assert_eq!(
            app.on_key(key(KeyCode::Enter)),
            Action::Open("https://gitlab.com/group/project/-/merge_requests/1".into())
        );
    }

    #[test]
    fn enter_on_empty_pane_does_nothing() {
        let mut app = App::new(false, false, "config.toml".into());
        assert_eq!(app.on_key(key(KeyCode::Enter)), Action::None);
        app.on_key(key(KeyCode::Down));
        assert_eq!(app.jira.table.selected(), None);
    }

    #[test]
    fn quit_and_refresh_keys() {
        let mut app = loaded_app();
        assert_eq!(app.on_key(key(KeyCode::Char('q'))), Action::Quit);
        assert_eq!(app.on_key(key(KeyCode::Esc)), Action::Quit);
        assert_eq!(
            app.on_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Action::Quit
        );
        assert_eq!(app.on_key(key(KeyCode::Char('r'))), Action::Refresh);
    }

    #[test]
    fn refresh_keeps_content_and_clamps_selection() {
        let mut app = loaded_app();
        app.on_key(key(KeyCode::Char('G')));
        app.start_refresh();
        assert!(app.jira.loading);
        assert_eq!(app.jira.items().len(), 3);

        app.on_msg(Msg::Jira(Ok(vec![issue("PAY-1")])));
        assert!(!app.jira.loading);
        assert_eq!(app.jira.table.selected(), Some(0));

        app.on_msg(Msg::Jira(Err("boom".into())));
        assert!(matches!(app.jira.content, Content::Failed(_)));
        assert_eq!(app.jira.table.selected(), None);
    }

    #[test]
    fn refresh_skips_unconfigured_panes() {
        let mut app = App::new(true, false, "config.toml".into());
        app.start_refresh();
        assert!(app.jira.loading);
        assert!(!app.gitlab.loading);
    }

    #[test]
    fn review_keys_only_act_on_gitlab_pane() {
        let mut app = loaded_app();
        assert_eq!(app.on_key(key(KeyCode::Char('v'))), Action::None);
        assert_eq!(app.on_key(key(KeyCode::Char('a'))), Action::None);
        assert!(app.starting.is_empty());
    }

    #[test]
    fn v_starts_a_review_once() {
        let mut app = loaded_app();
        app.on_key(key(KeyCode::Tab));
        app.on_key(key(KeyCode::Char('j')));

        assert_eq!(app.on_key(key(KeyCode::Char('v'))), Action::StartReview(mr(2)));
        assert_eq!(app.starting, ["group/project!2"]);
        assert_eq!(app.on_key(key(KeyCode::Char('v'))), Action::None);
        assert_eq!(app.message.as_ref().map(|m| m.text.as_str()), Some("Review already starting"));

        app.on_msg(Msg::ReviewStarted {
            mr: "group/project!2".into(),
            result: Ok("usine-review-group-project-2".into()),
        });
        assert!(app.starting.is_empty());
        assert_eq!(app.selected_review(), Some(&session(2, false)));
        assert_eq!(app.on_key(key(KeyCode::Char('v'))), Action::None);
        assert!(app.message.as_ref().map(|m| m.text.as_str()).unwrap().contains("press a to attach"));
    }

    #[test]
    fn failed_start_is_reported() {
        let mut app = loaded_app();
        app.on_key(key(KeyCode::Tab));
        app.on_key(key(KeyCode::Char('v')));
        app.on_msg(Msg::ReviewStarted {
            mr: "group/project!1".into(),
            result: Err("no local clone".into()),
        });
        assert!(app.starting.is_empty());
        assert_eq!(app.selected_review(), None);
        assert_eq!(app.message.as_ref().map(|m| m.text.as_str()), Some("Cannot review group/project!1: no local clone"));
    }

    #[test]
    fn attach_and_close_selected_review() {
        let mut app = loaded_app();
        app.on_key(key(KeyCode::Tab));
        assert_eq!(app.on_key(key(KeyCode::Char('a'))), Action::None);
        assert!(app.message.is_some());

        app.on_msg(Msg::Reviews(vec![session(1, false)]));
        let name = session(1, false).name;
        assert_eq!(app.on_key(key(KeyCode::Char('a'))), Action::Attach(name.clone()));
        // A running review needs X to be closed.
        assert_eq!(app.on_key(key(KeyCode::Char('x'))), Action::None);
        assert_eq!(app.on_key(key(KeyCode::Char('X'))), Action::CloseReview(name.clone()));

        app.on_msg(Msg::Reviews(vec![session(1, true)]));
        assert_eq!(app.on_key(key(KeyCode::Char('x'))), Action::CloseReview(name));
    }

    #[test]
    fn preview_is_matched_by_session() {
        let mut app = loaded_app();
        app.on_msg(Msg::Preview { session: "a".into(), text: "screen".into() });
        assert_eq!(app.preview_of("a"), Some("screen"));
        assert_eq!(app.preview_of("b"), None);
    }
}
