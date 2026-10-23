//! TUI state and key handling, independent from the terminal.

use std::cmp::Ordering;

use chrono::{DateTime, Local};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::widgets::TableState;
use usine::api_log::ApiLog;
use usine::connectors::claude::Session;
use usine::model::{Issue, MergeRequest};
use usine::workflows::links::link;

use super::sort::{
    ISSUE_COLUMNS, ISSUE_DEFAULT_SORT, MR_COLUMNS, MR_DEFAULT_SORT, ReviewState, Sort,
    compare_issues, compare_merge_requests,
};

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
    /// None: the order of the source.
    pub sort: Option<Sort>,
}

impl<T: Item> Pane<T> {
    fn new(configured: bool, sort: Sort) -> Self {
        Self {
            content: if configured { Content::Pending } else { Content::NotConfigured },
            loading: false,
            table: TableState::default(),
            sort: Some(sort),
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

    /// Sorts on `self.sort` with `compare`, keeping the selected item selected.
    fn sort_items(&mut self, compare: impl Fn(&T, &T, usize) -> Ordering) {
        let Some(sort) = self.sort else { return };
        let selected = self.selected().map(|item| item.url().to_string());
        let Content::Loaded(items) = &mut self.content else { return };
        items.sort_by(|a, b| sort.apply(compare(a, b, sort.column)));
        if let Some(url) = selected {
            self.table.select(items.iter().position(|item| item.url() == url));
        }
    }

    fn select(&mut self, target: impl FnOnce(usize, usize) -> isize) {
        let len = self.items().len();
        move_selection(&mut self.table, len, target);
    }
}

/// Moves the selection of a table of `len` rows to `target(current, len)`, within bounds.
fn move_selection(table: &mut TableState, len: usize, target: impl FnOnce(usize, usize) -> isize) {
    if len == 0 {
        return;
    }
    let current = table.selected().unwrap_or(0);
    let index = target(current, len).clamp(0, len as isize - 1);
    table.select(Some(index as usize));
}

/// Selects `found`, or keeps the current row within the `len` rows.
fn select_or_clamp(table: &mut TableState, found: Option<usize>, len: usize) {
    let index = match len {
        0 => None,
        _ => Some(found.unwrap_or(table.selected().unwrap_or(0)).min(len - 1)),
    };
    table.select(index);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    /// Jira issues above GitLab MRs.
    Stacked,
    /// Jira issues next to their MRs, then the MRs of no listed issue.
    Split,
    /// Jira issues with their MRs indented below them, in one pane, then the
    /// MRs of no listed issue.
    Tree,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Jira,
    /// MRs next to the issues (split view only).
    LinkedMrs,
    /// MRs pane (stacked view only).
    Gitlab,
    /// Issues and their MRs (tree view only).
    Tree,
}

/// A row of the split or tree view: indexes of an issue and of an MR in the
/// items of their pane. In the tree view, `(Some(issue), None)` is an issue
/// row and `(None, None)` the row above the MRs of no listed issue.
pub type LinkedRow = (Option<usize>, Option<usize>);

/// URLs of the issue and MR of the selected row of the split or tree view.
type LinkedSelection = Option<(Option<String>, Option<String>)>;

/// Review state of `mr`, given the review sessions and the reviews being set up.
pub fn review_state(reviews: &[Session], starting: &[String], mr: &MergeRequest) -> ReviewState {
    if starting.contains(&mr.reference) {
        return ReviewState::Starting;
    }
    match reviews.iter().find(|s| s.subject == mr.reference) {
        Some(review) if review.finished => ReviewState::Finished,
        Some(_) => ReviewState::Running,
        None => ReviewState::Absent,
    }
}

#[derive(Debug, PartialEq)]
pub enum Action {
    None,
    Quit,
    Refresh,
    Open(String),
    StartReview(Box<MergeRequest>),
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
    /// Column being chosen to sort the focused pane on.
    pub sorting: Option<usize>,
    /// `q` was pressed, waiting for confirmation.
    pub confirm_quit: bool,
    /// Calls made by the connectors, shown in the API calls pane.
    pub api_log: ApiLog,
    pub show_calls: bool,
    pub view: View,
    /// Split or tree view rows: the issues with their MRs, then the MRs of no listed issue.
    pub linked_rows: Vec<LinkedRow>,
    /// Selected row of `linked_rows`, shared by the issues and the MRs.
    pub linked_table: TableState,
    /// Selected row and scroll of the tree view, whose table has more rows
    /// than `linked_rows`: the names of the MR columns.
    pub tree_table: TableState,
}

impl App {
    pub fn new(jira_configured: bool, gitlab_configured: bool, config_path: String) -> Self {
        Self {
            jira: Pane::new(jira_configured, ISSUE_DEFAULT_SORT),
            gitlab: Pane::new(gitlab_configured, MR_DEFAULT_SORT),
            focus: Focus::Jira,
            config_path,
            last_update: None,
            message: None,
            reviews: Vec::new(),
            starting: Vec::new(),
            preview: None,
            sorting: None,
            confirm_quit: false,
            api_log: ApiLog::default(),
            show_calls: false,
            view: View::Stacked,
            linked_rows: Vec::new(),
            linked_table: TableState::default(),
            tree_table: TableState::default(),
        }
    }

    pub fn start_refresh(&mut self) {
        self.jira.start_loading();
        self.gitlab.start_loading();
    }

    pub fn review_of(&self, mr_reference: &str) -> Option<&Session> {
        self.reviews.iter().find(|s| s.subject == mr_reference)
    }

    /// Issue and MR of the selected row of the split or tree view.
    pub fn selected_link(&self) -> Option<(Option<&Issue>, Option<&MergeRequest>)> {
        let (issue, mr) = *self.linked_rows.get(self.linked_table.selected()?)?;
        let issue = issue.and_then(|i| self.jira.items().get(i));
        Some((issue, mr.and_then(|m| self.gitlab.items().get(m))))
    }

    /// The MR the review keys and the Review pane act on.
    pub fn selected_mr(&self) -> Option<&MergeRequest> {
        match self.view {
            View::Stacked => self.gitlab.selected(),
            View::Split | View::Tree => self.selected_link()?.1,
        }
    }

    /// Review session of the selected MR.
    pub fn selected_review(&self) -> Option<&Session> {
        self.review_of(&self.selected_mr()?.reference)
    }

    /// Captured screen of `session`, if any.
    pub fn preview_of(&self, session: &str) -> Option<&str> {
        match &self.preview {
            Some((name, text)) if name == session => Some(text),
            _ => None,
        }
    }

    pub fn on_msg(&mut self, msg: Msg) {
        let selection = self.linked_selection();
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
        // New items, or review states for the R column.
        self.sort_panes();
        self.relink(selection);
    }

    pub fn on_key(&mut self, key: KeyEvent) -> Action {
        self.message = None;
        if self.confirm_quit {
            self.confirm_quit = false;
            return match key.code {
                KeyCode::Char('y' | 'q') => Action::Quit,
                _ => Action::None,
            };
        }
        if let Some(column) = self.sorting {
            return self.on_sort_key(key, column);
        }
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => {
                self.confirm_quit = true;
                self.message = Some(Notice::warning("Quit usine? y: yes, any other key: no"));
            }
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Action::Quit;
            }
            KeyCode::Char('r') => return Action::Refresh,
            KeyCode::Char('l') => self.show_calls = !self.show_calls,
            KeyCode::Char('w') => {
                let selection = self.linked_selection();
                (self.view, self.focus) = match (self.view, self.focus) {
                    (View::Stacked, Focus::Gitlab) => (View::Split, Focus::LinkedMrs),
                    (View::Stacked, focus) => (View::Split, focus),
                    (View::Split, _) => (View::Tree, Focus::Tree),
                    (View::Tree, _) => (View::Stacked, Focus::Jira),
                };
                // The split and tree views have different rows.
                self.relink(selection);
            }
            KeyCode::Tab => self.cycle_focus(true),
            KeyCode::BackTab => self.cycle_focus(false),
            KeyCode::Enter => {
                let url = match (self.view, self.focus) {
                    (View::Stacked, Focus::Jira) => self.jira.selected().map(Item::url),
                    (View::Split, Focus::Jira) => {
                        self.selected_link().and_then(|(issue, _)| issue).map(Item::url)
                    }
                    // The MR of an MR row, the issue of an issue row.
                    (View::Tree, _) => self.selected_link().and_then(|(issue, mr)| {
                        mr.map(Item::url).or(issue.map(Item::url))
                    }),
                    _ => self.selected_mr().map(Item::url),
                };
                let url = url.map(str::to_string);
                return url.map_or(Action::None, Action::Open);
            }
            KeyCode::Char('j') | KeyCode::Down => self.select(|current, _| current as isize + 1),
            KeyCode::Char('k') | KeyCode::Up => self.select(|current, _| current as isize - 1),
            KeyCode::Char('g') | KeyCode::Home => self.select(|_, _| 0),
            KeyCode::Char('G') | KeyCode::End => self.select(|_, len| len as isize - 1),
            KeyCode::Char('s') if self.focus == Focus::LinkedMrs => {
                self.message = Some(Notice::warning(
                    "MRs follow their issue: sort the issues, or the MRs in the stacked view",
                ));
            }
            KeyCode::Char('s') => {
                self.sorting = Some(self.focused_sort().map_or(0, |sort| sort.column));
            }
            // Review keys act on the MR panes.
            KeyCode::Char('v') if self.focus != Focus::Jira => return self.start_review(),
            KeyCode::Char('a') if self.focus != Focus::Jira => {
                return match self.selected_review() {
                    Some(review) => Action::Attach(review.name.clone()),
                    None => {
                        self.message =
                            Some(Notice::warning("No review for this MR, press v to start one"));
                        Action::None
                    }
                };
            }
            KeyCode::Char(c @ ('x' | 'X')) if self.focus != Focus::Jira => {
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
        let Some(mr) = self.selected_mr().cloned() else {
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
        self.resort();
        Action::StartReview(Box::new(mr))
    }

    /// Keys while choosing the sort column: Enter sorts on it, or reverses the
    /// order when the pane is already sorted on it.
    fn on_sort_key(&mut self, key: KeyEvent, column: usize) -> Action {
        let last = match self.focus {
            Focus::Jira | Focus::Tree => ISSUE_COLUMNS.len() - 1,
            _ => MR_COLUMNS.len() - 1,
        };
        match key.code {
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Action::Quit;
            }
            KeyCode::Char('h') | KeyCode::Left => self.sorting = Some(column.saturating_sub(1)),
            KeyCode::Char('l') | KeyCode::Right => self.sorting = Some((column + 1).min(last)),
            KeyCode::Enter => {
                let sort = match self.focus {
                    Focus::Jira | Focus::Tree => &mut self.jira.sort,
                    _ => &mut self.gitlab.sort,
                };
                let descending = matches!(sort, Some(s) if s.column == column && !s.descending);
                *sort = Some(Sort { column, descending });
                self.sorting = None;
                self.resort();
            }
            KeyCode::Char('s' | 'q') | KeyCode::Esc => self.sorting = None,
            _ => {}
        }
        Action::None
    }

    pub fn focused_sort(&self) -> Option<Sort> {
        match self.focus {
            Focus::Jira | Focus::Tree => self.jira.sort,
            _ => self.gitlab.sort,
        }
    }

    fn cycle_focus(&mut self, forward: bool) {
        let order: &[Focus] = match self.view {
            View::Stacked => &[Focus::Jira, Focus::Gitlab],
            View::Split => &[Focus::Jira, Focus::LinkedMrs],
            View::Tree => &[Focus::Tree],
        };
        let len = order.len();
        let current = order.iter().position(|f| *f == self.focus).unwrap_or(0);
        self.focus = order[if forward { (current + 1) % len } else { (current + len - 1) % len }];
    }

    /// Sorts the panes, keeping the selection of the split or tree view.
    fn resort(&mut self) {
        let selection = self.linked_selection();
        self.sort_panes();
        self.relink(selection);
    }

    fn sort_panes(&mut self) {
        self.jira.sort_items(compare_issues);
        let (reviews, starting) = (&self.reviews, &self.starting);
        self.gitlab.sort_items(|a, b, column| {
            compare_merge_requests(a, b, column, |mr| review_state(reviews, starting, mr))
        });
    }

    fn linked_selection(&self) -> LinkedSelection {
        let (issue, mr) = self.selected_link()?;
        Some((issue.map(|i| i.url.clone()), mr.map(|m| m.url.clone())))
    }

    /// Recomputes the split or tree view rows from the panes, keeping
    /// `selection` selected when it is still listed, or else its issue.
    fn relink(&mut self, selection: LinkedSelection) {
        let (issues, mrs) = (self.jira.items(), self.gitlab.items());
        let links = link(issues, mrs);
        self.linked_rows = match self.view {
            View::Tree => {
                let mut rows = Vec::new();
                for (n, &(i, m)) in links.rows.iter().enumerate() {
                    // Each issue row is followed by its MR rows.
                    if n == 0 || links.rows[n - 1].0 != i {
                        rows.push((Some(i), None));
                    }
                    if m.is_some() {
                        rows.push((Some(i), m));
                    }
                }
                if !links.others.is_empty() {
                    rows.push((None, None));
                }
                rows.extend(links.others.into_iter().map(|m| (None, Some(m))));
                rows
            }
            _ => {
                let others = links.others.into_iter().map(|m| (None, Some(m)));
                links.rows.into_iter().map(|(i, m)| (Some(i), m)).chain(others).collect()
            }
        };
        let issue_url = |i: Option<usize>| i.map(|i| issues[i].url.as_str());
        let mr_url = |m: Option<usize>| m.map(|m| mrs[m].url.as_str());
        let rows = &self.linked_rows;
        let found = selection.and_then(|(issue, mr)| {
            rows.iter()
                .position(|&(i, m)| issue_url(i) == issue.as_deref() && mr_url(m) == mr.as_deref())
                .or_else(|| rows.iter().position(|&(i, _)| issue_url(i) == issue.as_deref()))
        });
        select_or_clamp(&mut self.linked_table, found, self.linked_rows.len());
    }

    fn select(&mut self, target: impl FnOnce(usize, usize) -> isize) {
        match (self.view, self.focus) {
            (View::Stacked, Focus::Jira) => self.jira.select(target),
            (View::Stacked, _) => self.gitlab.select(target),
            (View::Split | View::Tree, _) => {
                move_selection(&mut self.linked_table, self.linked_rows.len(), target)
            }
        }
    }
}

#[cfg(test)]
pub mod tests {
    use chrono::DateTime;
    use usine::model::{Role, Source, StatusCategory};

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
            description: String::new(),
            draft: false,
            merge_status: "mergeable".into(),
            pipeline: None,
            role: Role::Author,
            source_branch: "feat".into(),
            target_branch: "main".into(),
            author: "me".into(),
            updated: DateTime::parse_from_rfc3339("2026-10-10T12:00:00Z").unwrap(),
            url: format!("https://gitlab.com/group/project/-/merge_requests/{iid}"),
        }
    }

    /// An MR whose branch refers to the issue `key`.
    pub fn linked_mr(iid: u32, key: &str) -> MergeRequest {
        MergeRequest { source_branch: format!("{key}-work"), ..mr(iid) }
    }

    /// Split view of PAY-1 (MRs 1 and 3), PAY-2 (no MR), PAY-3 (MR 4); MR 2 is linked to no issue.
    pub fn split_app() -> App {
        let mut app = App::new(true, true, "config.toml".into());
        app.on_msg(Msg::Jira(Ok(vec![issue("PAY-1"), issue("PAY-2"), issue("PAY-3")])));
        app.on_msg(Msg::Gitlab(Ok(vec![
            linked_mr(1, "PAY-1"),
            mr(2),
            linked_mr(3, "PAY-1"),
            linked_mr(4, "PAY-3"),
        ])));
        app.on_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE));
        app
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
        assert_eq!(app.on_key(key(KeyCode::Char('q'))), Action::None);
        assert!(app.message.as_ref().unwrap().text.starts_with("Quit usine?"));
        assert_eq!(app.on_key(key(KeyCode::Char('y'))), Action::Quit);
        app.on_key(key(KeyCode::Esc));
        assert_eq!(app.on_key(key(KeyCode::Char('q'))), Action::Quit);
        assert_eq!(
            app.on_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Action::Quit
        );
        assert_eq!(app.on_key(key(KeyCode::Char('r'))), Action::Refresh);
    }

    #[test]
    fn any_other_key_cancels_quit() {
        let mut app = loaded_app();
        app.on_key(key(KeyCode::Char('q')));
        assert_eq!(app.on_key(key(KeyCode::Char('n'))), Action::None);
        assert!(!app.confirm_quit);
        assert_eq!(app.message, None);
        // The cancelling key is not acted upon.
        app.on_key(key(KeyCode::Char('q')));
        assert_eq!(app.on_key(key(KeyCode::Char('j'))), Action::None);
        assert_eq!(app.jira.table.selected(), Some(0));
        app.on_key(key(KeyCode::Esc));
        assert_eq!(app.on_key(key(KeyCode::Esc)), Action::None);
        assert!(!app.confirm_quit);
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

        assert_eq!(app.on_key(key(KeyCode::Char('v'))), Action::StartReview(Box::new(mr(2))));
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
    fn s_chooses_the_sort_column_of_the_focused_pane() {
        let mut app = loaded_app();
        app.on_key(key(KeyCode::Char('j')));
        assert_eq!(app.jira.selected().map(|i| i.key.as_str()), Some("PAY-2"));

        // Starts on the current sort column.
        app.on_key(key(KeyCode::Char('s')));
        assert_eq!(app.sorting, Some(1));
        // Navigation keys move the column, not the row.
        app.on_key(key(KeyCode::Left));
        app.on_key(key(KeyCode::Left));
        assert_eq!(app.sorting, Some(0));
        app.on_key(key(KeyCode::Char('l')));
        app.on_key(key(KeyCode::Char('l')));
        app.on_key(key(KeyCode::Char('h')));
        assert_eq!(app.on_key(key(KeyCode::Char('j'))), Action::None);
        assert_eq!(app.sorting, Some(1));
        assert_eq!(app.jira.table.selected(), Some(1));

        app.on_key(key(KeyCode::Char('h')));
        assert_eq!(app.on_key(key(KeyCode::Enter)), Action::None);
        assert_eq!(app.sorting, None);
        assert_eq!(app.jira.sort, Some(Sort { column: 0, descending: false }));
        assert_eq!(app.gitlab.sort, Some(MR_DEFAULT_SORT));

        // Same column again: reversed, and the selected issue stays selected.
        app.on_key(key(KeyCode::Char('s')));
        assert_eq!(app.sorting, Some(0));
        app.on_key(key(KeyCode::Enter));
        assert_eq!(app.jira.sort, Some(Sort { column: 0, descending: true }));
        let keys: Vec<&str> = app.jira.items().iter().map(|i| i.key.as_str()).collect();
        assert_eq!(keys, ["PAY-3", "PAY-2", "PAY-1"]);
        assert_eq!(app.jira.selected().map(|i| i.key.as_str()), Some("PAY-2"));
    }

    #[test]
    fn panes_are_sorted_by_status_and_role_by_default() {
        let mut app = App::new(true, true, "config.toml".into());
        let with_status = |key: &str, category: StatusCategory| Issue {
            status_category: category,
            ..issue(key)
        };
        app.on_msg(Msg::Jira(Ok(vec![
            with_status("PAY-1", StatusCategory::Done),
            with_status("PAY-2", StatusCategory::Todo),
            with_status("PAY-3", StatusCategory::InProgress),
            with_status("PAY-4", StatusCategory::Todo),
        ])));
        let keys: Vec<&str> = app.jira.items().iter().map(|i| i.key.as_str()).collect();
        // Same status: the source order is kept.
        assert_eq!(keys, ["PAY-2", "PAY-4", "PAY-3", "PAY-1"]);

        let reviewer = |iid| MergeRequest { role: Role::Reviewer, ..mr(iid) };
        app.on_msg(Msg::Gitlab(Ok(vec![reviewer(1), mr(2), reviewer(3), mr(4)])));
        let iids: Vec<u64> = app.gitlab.items().iter().map(|m| m.iid).collect();
        assert_eq!(iids, [2, 4, 1, 3]);
    }

    #[test]
    fn l_toggles_the_api_calls_pane() {
        let mut app = loaded_app();
        assert!(!app.show_calls);
        assert_eq!(app.on_key(key(KeyCode::Char('l'))), Action::None);
        assert!(app.show_calls);
        app.on_key(key(KeyCode::Char('l')));
        assert!(!app.show_calls);
    }

    #[test]
    fn escape_cancels_sort_without_quitting() {
        let mut app = loaded_app();
        app.on_key(key(KeyCode::Char('s')));
        app.on_key(key(KeyCode::Char('l')));
        assert_eq!(app.on_key(key(KeyCode::Esc)), Action::None);
        assert_eq!(app.sorting, None);
        assert_eq!(app.jira.sort, Some(ISSUE_DEFAULT_SORT));
    }

    #[test]
    fn sort_is_kept_across_refreshes_and_review_changes() {
        let mut app = loaded_app();
        app.on_key(key(KeyCode::Tab));
        app.on_key(key(KeyCode::Char('s')));
        app.on_key(key(KeyCode::Char('h')));
        app.on_key(key(KeyCode::Enter));
        assert_eq!(app.gitlab.sort, Some(Sort { column: 0, descending: false }));

        app.on_msg(Msg::Gitlab(Ok(vec![mr(1), mr(2), mr(3)])));
        app.on_msg(Msg::Reviews(vec![session(3, false)]));
        let iids = |app: &App| app.gitlab.items().iter().map(|m| m.iid).collect::<Vec<_>>();
        assert_eq!(iids(&app), [3, 1, 2]);

        app.on_msg(Msg::Reviews(vec![session(3, true), session(2, false)]));
        assert_eq!(iids(&app), [2, 3, 1]);
    }

    #[test]
    fn w_toggles_the_split_view_and_tab_cycles_its_panes() {
        let mut app = split_app();
        assert_eq!(app.view, View::Split);
        // The MR of no listed issue comes last.
        assert_eq!(
            app.linked_rows,
            [(Some(0), Some(0)), (Some(0), Some(2)), (Some(1), None), (Some(2), Some(3)), (None, Some(1))]
        );

        app.on_key(key(KeyCode::Tab));
        assert_eq!(app.focus, Focus::LinkedMrs);
        app.on_key(key(KeyCode::Tab));
        assert_eq!(app.focus, Focus::Jira);
        app.on_key(key(KeyCode::BackTab));
        assert_eq!(app.focus, Focus::LinkedMrs);

        // w cycles the split, tree and stacked views; each has its own MRs pane.
        let w = |app: &mut App| {
            app.on_key(key(KeyCode::Char('w')));
            (app.view, app.focus)
        };
        assert_eq!(w(&mut app), (View::Tree, Focus::Tree));
        app.on_key(key(KeyCode::Tab));
        assert_eq!(app.focus, Focus::Tree);
        assert_eq!(w(&mut app), (View::Stacked, Focus::Jira));
        app.on_key(key(KeyCode::Tab));
        assert_eq!(w(&mut app), (View::Split, Focus::LinkedMrs));
    }

    /// Tree view of the split view fixture.
    pub fn tree_app() -> App {
        let mut app = split_app();
        app.on_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE));
        app
    }

    #[test]
    fn tree_rows_have_the_issues_then_their_mrs() {
        let app = tree_app();
        assert_eq!(app.view, View::Tree);
        // The MR of no listed issue comes last, below a row of its own.
        assert_eq!(
            app.linked_rows,
            [
                (Some(0), None),
                (Some(0), Some(0)),
                (Some(0), Some(2)),
                (Some(1), None),
                (Some(2), None),
                (Some(2), Some(3)),
                (None, None),
                (None, Some(1)),
            ]
        );
    }

    #[test]
    fn tree_rows_open_and_review_their_issue_or_mr() {
        let mut app = tree_app();
        // The selected MR of the split view stays selected.
        assert_eq!(app.linked_table.selected(), Some(1));
        app.on_key(key(KeyCode::Char('g')));
        let open = |app: &mut App| app.on_key(key(KeyCode::Enter));
        assert_eq!(open(&mut app), Action::Open(issue("PAY-1").url));
        assert_eq!(app.selected_mr(), None);
        assert_eq!(app.on_key(key(KeyCode::Char('v'))), Action::None);

        app.on_key(key(KeyCode::Char('j')));
        assert_eq!(open(&mut app), Action::Open(mr(1).url));
        let action = app.on_key(key(KeyCode::Char('v')));
        assert_eq!(action, Action::StartReview(Box::new(linked_mr(1, "PAY-1"))));

        // The row above the MRs of no listed issue opens nothing.
        app.on_key(key(KeyCode::Char('G')));
        app.on_key(key(KeyCode::Char('k')));
        assert_eq!(open(&mut app), Action::None);
        assert_eq!(app.selected_mr(), None);
        app.on_key(key(KeyCode::Char('j')));
        assert_eq!(open(&mut app), Action::Open(mr(2).url));
    }

    #[test]
    fn s_sorts_the_issues_of_the_tree() {
        let mut app = tree_app();
        app.on_key(key(KeyCode::Char('s')));
        assert_eq!(app.sorting, Some(ISSUE_DEFAULT_SORT.column));
        app.on_key(key(KeyCode::Char('h')));
        app.on_key(key(KeyCode::Enter));
        assert_eq!(app.jira.sort, Some(Sort { column: 0, descending: false }));
        assert_eq!(app.gitlab.sort, Some(MR_DEFAULT_SORT));
    }

    #[test]
    fn selection_is_kept_across_split_and_tree_views() {
        let mut app = split_app();
        // PAY-3 and its MR.
        for _ in 0..3 {
            app.on_key(key(KeyCode::Char('j')));
        }
        app.on_key(key(KeyCode::Char('w')));
        assert_eq!(app.linked_table.selected(), Some(5));

        // The PAY-1 row of the tree selects the first row of PAY-1 in the split view.
        app.on_key(key(KeyCode::Char('g')));
        for _ in 0..2 {
            app.on_key(key(KeyCode::Char('w')));
        }
        assert_eq!(app.view, View::Split);
        assert_eq!(app.linked_table.selected(), Some(0));
    }

    #[test]
    fn split_rows_are_shared_and_enter_opens_the_focused_side() {
        let mut app = split_app();
        app.on_key(key(KeyCode::Char('j')));
        assert_eq!(app.linked_table.selected(), Some(1));
        let open = |app: &mut App| app.on_key(key(KeyCode::Enter));
        assert_eq!(open(&mut app), Action::Open(issue("PAY-1").url));

        app.on_key(key(KeyCode::Tab));
        assert_eq!(open(&mut app), Action::Open(mr(3).url));
        assert_eq!(app.selected_mr().map(|m| m.iid), Some(3));
        // An issue without MR has nothing to open on the right.
        app.on_key(key(KeyCode::Char('j')));
        assert_eq!(open(&mut app), Action::None);
        assert_eq!(app.selected_mr(), None);

        // An MR of no listed issue has nothing to open on the left.
        app.on_key(key(KeyCode::Char('G')));
        assert_eq!(open(&mut app), Action::Open(mr(2).url));
        app.on_key(key(KeyCode::Tab));
        assert_eq!(open(&mut app), Action::None);
        assert_eq!(app.selected_mr().map(|m| m.iid), Some(2));
    }

    #[test]
    fn review_keys_act_on_the_linked_mrs() {
        let mut app = split_app();
        assert_eq!(app.on_key(key(KeyCode::Char('v'))), Action::None);

        app.on_key(key(KeyCode::Tab));
        let action = app.on_key(key(KeyCode::Char('v')));
        assert_eq!(action, Action::StartReview(Box::new(linked_mr(1, "PAY-1"))));

        app.on_key(key(KeyCode::Char('s')));
        assert_eq!(app.sorting, None);
        assert!(app.message.as_ref().unwrap().text.contains("MRs follow their issue"));
    }

    #[test]
    fn split_selection_is_kept_across_refreshes() {
        let mut app = split_app();
        for _ in 0..3 {
            app.on_key(key(KeyCode::Char('j')));
        }
        let selected_key = |app: &App| app.selected_link().and_then(|(i, _)| i).map(|i| i.key.clone());
        assert_eq!(selected_key(&app).as_deref(), Some("PAY-3"));

        let issues = vec![issue("PAY-9"), issue("PAY-1"), issue("PAY-2"), issue("PAY-3")];
        app.on_msg(Msg::Jira(Ok(issues)));
        assert_eq!(app.linked_table.selected(), Some(4));
        assert_eq!(selected_key(&app).as_deref(), Some("PAY-3"));

        // PAY-3 is gone: its MR joins the MRs of no listed issue.
        app.on_msg(Msg::Jira(Ok(vec![issue("PAY-1")])));
        assert_eq!(app.linked_rows.len(), 4);
        assert_eq!(app.linked_table.selected(), Some(3));
    }

    #[test]
    fn preview_is_matched_by_session() {
        let mut app = loaded_app();
        app.on_msg(Msg::Preview { session: "a".into(), text: "screen".into() });
        assert_eq!(app.preview_of("a"), Some("screen"));
        assert_eq!(app.preview_of("b"), None);
    }
}
