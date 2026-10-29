//! Rendering of the App state with ratatui widgets.

use std::collections::HashMap;

use ansi_to_tui::IntoText;
use chrono::{Duration, Local};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Cell, Clear, Paragraph, Row, Table, TableState, Wrap};
use usine::config::display_path;
use usine::connectors::claude::Session;
use usine::model::{Issue, MergeRequest, Role, StatusCategory};

use super::app::{
    App, Content, Focus, Item, Level, MENU_ENTRIES, Menu, Pane, View, claude_state, review_state,
};
use super::sort::{
    ISSUE_COLUMNS, MR_COLUMNS, ReviewState, Sort, merge_readiness, pipeline_rank, priority_rank,
};

/// Color of focused borders, titles and key hints.
const ACCENT: Color = Color::Cyan;
/// Color of the Claude review pane.
const CLAUDE: Color = Color::Magenta;
const MUTED: Color = Color::DarkGray;
/// Color of the names of the MR columns in the tree view: orange, used nowhere else.
const MR_HEADER: Color = Color::Indexed(208);

/// A table cell: text and style.
type Styled = (String, Style);

/// Columns of the MRs in the split view, as indexes into `MR_COLUMNS`.
const SPLIT_MR_CELLS: [usize; 8] = [0, 1, 2, 3, 4, 5, 7, 8];
/// Row of the tree view above the MRs of no listed issue.
const OTHER_MRS: &str = "MRs of no listed issue";
/// Width of the role of the MRs in the tree view, so that their titles line up.
const TREE_ROLE_WIDTH: usize = 10;

pub fn render(frame: &mut Frame, app: &mut App) {
    // The bottom panes show the sessions of the selected MR, if any.
    let starting = app.selected_mr().is_some_and(|mr| app.starting.contains(&mr.reference));
    let review = starting || app.selected_review().is_some();
    let review_height = match (app.selected_review(), app.selected_claude()) {
        (None, None) if starting => Constraint::Length(3),
        (None, None) => Constraint::Length(0),
        _ => Constraint::Fill(2),
    };
    let calls_height = if app.show_calls { Constraint::Fill(1) } else { Constraint::Length(0) };
    let notifications_height =
        if app.show_notifications { Constraint::Fill(1) } else { Constraint::Length(0) };
    // The split and tree views use the height of both stacked panes.
    let middle_height = match app.view {
        View::Stacked => Constraint::Fill(1),
        View::Split | View::Tree => Constraint::Length(0),
    };
    let [top, middle, bottom, notifications, calls, footer] = Layout::vertical([
        Constraint::Fill(1),
        middle_height,
        review_height,
        notifications_height,
        calls_height,
        Constraint::Length(1),
    ])
    .areas(frame.area());

    match app.view {
        View::Stacked => render_stacked(frame, top, middle, app),
        View::Split => render_split(frame, top, app),
        View::Tree => render_tree(frame, top, app),
    }
    match (review, app.selected_claude()) {
        (true, Some(session)) => {
            let [left, right] =
                Layout::horizontal([Constraint::Fill(1), Constraint::Fill(1)]).areas(bottom);
            render_review(frame, left, app);
            render_claude(frame, right, app, session);
        }
        (true, None) => render_review(frame, bottom, app),
        (false, Some(session)) => render_claude(frame, bottom, app, session),
        (false, None) => {}
    }
    if app.show_notifications {
        render_notifications(frame, notifications, app);
    }
    if app.show_calls {
        render_calls(frame, calls, app);
    }
    render_footer(frame, footer, app);
    if let Some(menu) = &mut app.menu {
        render_menu(frame, frame.area(), menu);
    }
}

/// The menu of `c`, centered over the panes.
fn render_menu(frame: &mut Frame, area: Rect, menu: &mut Menu) {
    let (title, rows): (String, Vec<Row>) = match &menu.conversations {
        None => (
            menu.mr.reference.clone(),
            MENU_ENTRIES
                .iter()
                .enumerate()
                .map(|(n, entry)| {
                    Row::new(vec![Cell::from((n + 1).to_string()).fg(ACCENT), Cell::from(*entry)])
                })
                .collect(),
        ),
        Some(conversations) => (
            format!("Resume in {}", display_path(menu.mr.worktree.as_deref())),
            conversations
                .iter()
                .map(|c| {
                    let updated = c.updated.format("%Y-%m-%d %H:%M").to_string();
                    Row::new(vec![Cell::from(updated).fg(MUTED), Cell::from(c.title.clone())])
                })
                .collect(),
        ),
    };
    let first_width = if menu.conversations.is_some() { 16 } else { 1 };
    let width = area.width.saturating_sub(4).min(100);
    let height = (rows.len() as u16 + 2).min(area.height.saturating_sub(2));
    let popup = Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    };
    let block = pane_block(vec![Span::styled(format!(" {title} "), Style::new().fg(ACCENT).bold())], ACCENT);
    let table = Table::new(rows, [Constraint::Length(first_width), Constraint::Fill(1)])
        .column_spacing(2)
        .block(block)
        .row_highlight_style(Style::new().add_modifier(Modifier::REVERSED | Modifier::BOLD));
    frame.render_widget(Clear, popup);
    frame.render_stateful_widget(table, popup, &mut menu.table);
}

fn render_stacked(frame: &mut Frame, top: Rect, middle: Rect, app: &mut App) {
    let status_colors = &app.status_colors;
    let issue_row = |issue: &Issue| issue_row(issue, status_colors);
    render_pane(
        frame,
        top,
        PaneView {
            title: "Jira issues",
            section: "[jira]",
            header: &ISSUE_COLUMNS,
            row: &issue_row,
            focused: app.focus == Focus::Jira,
            choosing: app.sorting.filter(|_| app.focus == Focus::Jira),
            config_path: &app.config_path,
        },
        &mut app.jira,
    );
    let (reviews, starting, claude) = (&app.reviews, &app.starting, &app.claude_sessions);
    let merge_request_row = |mr: &MergeRequest| merge_request_row(mr, reviews, starting, claude);
    render_pane(
        frame,
        middle,
        PaneView {
            title: "GitLab MRs",
            section: "[gitlab]",
            header: &MR_COLUMNS,
            row: &merge_request_row,
            focused: app.focus == Focus::Gitlab,
            choosing: app.sorting.filter(|_| app.focus == Focus::Gitlab),
            config_path: &app.config_path,
        },
        &mut app.gitlab,
    );
}

/// Issues on the left, their MRs on the right, then the MRs of no listed issue.
fn render_split(frame: &mut Frame, area: Rect, app: &mut App) {
    let [left, right] = Layout::horizontal([Constraint::Fill(1), Constraint::Fill(1)]).areas(area);
    let (issues, mrs) = (app.jira.items(), app.gitlab.items());
    let blank = |count: usize| vec![(String::new(), Style::new()); count];
    let (mut issue_rows, mut mr_rows) = (Vec::new(), Vec::new());
    for (n, &(issue, mr)) in app.linked_rows.iter().enumerate() {
        // An issue with several MRs is shown on its first row only.
        let first = n == 0 || app.linked_rows[n - 1].0 != issue;
        issue_rows.push(match issue {
            Some(issue) if first => issue_row(&issues[issue], &app.status_colors),
            _ => blank(ISSUE_COLUMNS.len()),
        });
        mr_rows.push(match mr {
            Some(mr) => {
                let cells =
                    merge_request_row(&mrs[mr], &app.reviews, &app.starting, &app.claude_sessions);
                SPLIT_MR_CELLS.iter().map(|&c| cells[c].clone()).collect()
            }
            None => blank(SPLIT_MR_CELLS.len()),
        });
    }
    let jira_count = matches!(app.jira.content, Content::Loaded(_)).then_some(issues.len());
    let gitlab_count = matches!(app.gitlab.content, Content::Loaded(_)).then_some(mrs.len());

    // The focused side is highlighted, the other side of the same row is bold.
    let focus = app.focus;
    let highlight = |side: Focus| match focus {
        f if f == side => Style::new().add_modifier(Modifier::REVERSED | Modifier::BOLD),
        _ => Style::new().bold(),
    };

    // Once loaded, a side shows its rows even when it has no item of its own.
    let block = titled_block("Jira issues", jira_count, app.jira.loading, focus == Focus::Jira);
    let text = placeholder(&app.jira.content, "[jira]", &app.config_path);
    match text.filter(|_| jira_count.is_none()) {
        Some(text) => render_message(frame, left, block, text),
        None => {
            let header = header_labels(&ISSUE_COLUMNS, app.jira.sort);
            let choosing = app.sorting.filter(|_| focus == Focus::Jira);
            let style = highlight(Focus::Jira);
            let table = &mut app.linked_table;
            let rows = issue_rows.into_iter().map(lines).collect();
            render_table(frame, left, block, header, choosing, rows, style, table);
        }
    }

    // Same rows and selection as the issues: both sides scroll together.
    let mut linked_table = app.linked_table;
    let focused = focus == Focus::LinkedMrs;
    let block = titled_block("GitLab MRs", gitlab_count, app.gitlab.loading, focused);
    let text = placeholder(&app.gitlab.content, "[gitlab]", &app.config_path);
    match text.filter(|_| gitlab_count.is_none()) {
        Some(text) => render_message(frame, right, block, text),
        None => {
            let header = SPLIT_MR_CELLS.iter().map(|&c| MR_COLUMNS[c].to_string()).collect();
            let style = highlight(Focus::LinkedMrs);
            let rows = mr_rows.into_iter().map(lines).collect();
            render_table(frame, right, block, header, None, rows, style, &mut linked_table);
        }
    }
}

/// Issues with their MRs below them, then the MRs of no listed issue, in one
/// pane. MRs fill the issue columns: reference, merge status, pipeline, update
/// date, then role and title, under their own column names.
fn render_tree(frame: &mut Frame, area: Rect, app: &mut App) {
    let (issues, mrs) = (app.jira.items(), app.gitlab.items());
    let linked = &app.linked_rows;
    let mr_header = tree_mr_header(app.gitlab.sort);
    // Rows of the table: those of `linked`, each one above MRs followed by
    // the names of their columns, which cannot be selected.
    let mut rows = Vec::new();
    let mut selected = None;
    for (n, &(issue, mr)) in linked.iter().enumerate() {
        if app.linked_table.selected() == Some(n) {
            selected = Some(rows.len());
        }
        rows.push(match (issue, mr) {
            (_, Some(mr)) => {
                // The last MR below an issue closes its branch.
                let last = linked.get(n + 1).is_none_or(|&(i, m)| i != issue || m.is_none());
                tree_mr_row(&mrs[mr], last, &app.reviews, &app.starting, &app.claude_sessions)
            }
            (Some(issue), None) => lines(issue_row(&issues[issue], &app.status_colors)),
            (None, None) => vec![Line::styled(OTHER_MRS, Style::new().fg(MUTED).bold())],
        });
        if mr.is_none() && linked.get(n + 1).is_some_and(|&(_, m)| m.is_some()) {
            rows.push(mr_header.clone());
        }
    }

    let jira_count = matches!(app.jira.content, Content::Loaded(_)).then_some(issues.len());
    let gitlab_count = matches!(app.gitlab.content, Content::Loaded(_)).then_some(mrs.len());
    let title_style = Style::new().fg(ACCENT).bold();
    let mut title = title_spans("Jira issues", jira_count, app.jira.loading, title_style);
    title.extend(title_spans("GitLab MRs", gitlab_count, app.gitlab.loading, title_style));
    let block = pane_block(title, ACCENT);

    // A source not loaded tells why above the rows of the other one.
    let messages: Vec<Line> = [
        placeholder(&app.jira.content, "[jira]", &app.config_path).filter(|_| jira_count.is_none()),
        placeholder(&app.gitlab.content, "[gitlab]", &app.config_path)
            .filter(|_| gitlab_count.is_none()),
    ]
    .into_iter()
    .flatten()
    .collect();
    if rows.is_empty() {
        let text = match messages.is_empty() {
            true => vec![Line::from("Nothing to show.").fg(MUTED)],
            false => messages,
        };
        frame.render_widget(Paragraph::new(text).wrap(Wrap { trim: true }).block(block), area);
        return;
    }
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let [top, rest] =
        Layout::vertical([Constraint::Length(messages.len() as u16), Constraint::Fill(1)])
            .areas(inner);
    frame.render_widget(Paragraph::new(messages), top);
    let header = header_labels(&ISSUE_COLUMNS, app.jira.sort);
    let choosing = app.sorting.filter(|_| app.focus == Focus::Tree);
    let highlight = Style::new().add_modifier(Modifier::REVERSED | Modifier::BOLD);
    app.tree_table.select(selected);
    let table = &mut app.tree_table;
    render_table(frame, rest, Block::new(), header, choosing, rows, highlight, table);
}

/// Cells of an MR of the tree view, in the issue columns.
fn tree_mr_row(
    mr: &MergeRequest,
    last: bool,
    reviews: &[Session],
    starting: &[String],
    claude: &[Session],
) -> Vec<Line<'static>> {
    let [marker, claude, role, reference, status, pipeline, updated, worktree, title] =
        merge_request_row(mr, reviews, starting, claude).try_into().expect("a cell per MR column");
    let span = |(text, style): Styled| Span::styled(text, style);
    let branch = if last { "└─ " } else { "├─ " };
    let role = (format!("{:<TREE_ROLE_WIDTH$}", role.0), role.1);
    vec![
        Line::from(vec![
            Span::styled(branch, Style::new().fg(MUTED)),
            span(marker),
            Span::raw(" "),
            span(claude),
            Span::raw(" "),
            span(reference),
        ]),
        Line::from(span(status)),
        Line::from(span(pipeline)),
        Line::from(span(updated)),
        // The issue columns have no room for the worktree: it follows the title.
        Line::from(vec![span(role), span(title), Span::raw("  "), span(worktree)]),
    ]
}

/// Names of the MR columns of the tree view, laid out as `tree_mr_row`, with
/// the sort of the MRs.
fn tree_mr_header(sort: Option<Sort>) -> Vec<Line<'static>> {
    let [marker, claude, role, reference, status, pipeline, updated, worktree, title] =
        header_labels(&MR_COLUMNS, sort).try_into().expect("a label per MR column");
    let label = |text: String| Span::styled(text, Style::new().fg(MR_HEADER).underlined());
    // The padding of the role is not underlined.
    let padding = " ".repeat(TREE_ROLE_WIDTH.saturating_sub(role.chars().count()));
    vec![
        Line::from(vec![
            Span::raw("   "),
            label(marker),
            Span::raw(" "),
            label(claude),
            Span::raw(" "),
            label(reference),
        ]),
        Line::from(label(status)),
        Line::from(label(pipeline)),
        Line::from(label(updated)),
        Line::from(vec![
            label(role),
            Span::raw(padding),
            label(title),
            Span::raw("  "),
            label(worktree),
        ]),
    ]
}

/// Table cells of `row`, one span each.
fn lines(row: Vec<Styled>) -> Vec<Line<'static>> {
    row.into_iter().map(|(text, style)| Line::styled(text, style)).collect()
}

struct PaneView<'a, T> {
    title: &'a str,
    section: &'a str,
    header: &'a [&'a str],
    row: &'a dyn Fn(&T) -> Vec<Styled>,
    focused: bool,
    /// Column highlighted while choosing the sort column.
    choosing: Option<usize>,
    config_path: &'a str,
}

/// Cells of `mr`, one per column of `MR_COLUMNS`. `claude`: sessions running
/// Claude in a worktree.
fn merge_request_row(
    mr: &MergeRequest,
    reviews: &[Session],
    starting: &[String],
    claude: &[Session],
) -> Vec<Styled> {
    let marker = |state| match state {
        ReviewState::Starting => ("~".to_string(), Style::new().fg(Color::Yellow).bold()),
        ReviewState::Finished => ("*".to_string(), Style::new().fg(Color::Blue).bold()),
        ReviewState::Running => ("*".to_string(), Style::new().fg(Color::Green).bold()),
        ReviewState::Absent => (" ".to_string(), Style::new()),
    };
    let title_style = if mr.draft { Style::new().fg(MUTED).italic() } else { Style::new() };
    let role = match mr.role {
        Role::Author => ("author".to_string(), Style::new().fg(MUTED)),
        Role::Reviewer => ("reviewer".to_string(), Style::new().fg(Color::LightBlue)),
    };
    vec![
        marker(review_state(reviews, starting, mr)),
        marker(claude_state(claude, mr)),
        role,
        (mr.reference.clone(), Style::new().bold()),
        (mr.merge_status.clone(), Style::new().fg(merge_status_color(&mr.merge_status))),
        pipeline_cell(mr.pipeline.as_deref()),
        updated_cell(mr.updated),
        (display_path(mr.worktree.as_deref()), Style::new().fg(MUTED)),
        (mr.title.clone(), title_style),
    ]
}

fn issue_row(issue: &Issue, status_colors: &HashMap<String, Color>) -> Vec<Styled> {
    let status_color = match issue.status_category {
        StatusCategory::Todo => Color::Blue,
        StatusCategory::InProgress => Color::Yellow,
        StatusCategory::Done => Color::Green,
    };
    let status_color = status_colors.get(&issue.status.to_lowercase()).copied().unwrap_or(status_color);
    let priority_color = priority_color(issue.priority.as_deref());
    let priority = issue.priority.clone().unwrap_or_else(|| "-".into());
    vec![
        (issue.key.clone(), Style::new().bold()),
        (issue.status.clone(), Style::new().fg(status_color)),
        (priority, Style::new().fg(priority_color)),
        updated_cell(issue.updated),
        (issue.title.clone(), Style::new()),
    ]
}

fn priority_color(priority: Option<&str>) -> Color {
    match priority_rank(priority) {
        0 => Color::LightRed,
        1 => Color::Red,
        2 => Color::Yellow,
        3 => Color::Green,
        4 => Color::Blue,
        _ => MUTED,
    }
}

fn merge_status_color(status: &str) -> Color {
    match merge_readiness(status) {
        0 => Color::Green,
        1 => Color::Yellow,
        2 => Color::Red,
        _ => MUTED,
    }
}

fn pipeline_cell(pipeline: Option<&str>) -> Styled {
    let color = match pipeline_rank(pipeline) {
        0 => Color::Green,
        1 => Color::Yellow,
        2 => Color::Red,
        _ => MUTED,
    };
    (pipeline.unwrap_or("-").to_string(), Style::new().fg(color))
}

/// Dates of the last 24 hours stand out, older ones are muted.
fn updated_cell(updated: chrono::DateTime<chrono::FixedOffset>) -> Styled {
    let recent = Local::now().fixed_offset() - updated < Duration::hours(24);
    let style = if recent { Style::new().fg(Color::White) } else { Style::new().fg(MUTED) };
    (updated.format("%Y-%m-%d").to_string(), style)
}

fn pane_block<'a>(title: Vec<Span<'a>>, border: Color) -> Block<'a> {
    Block::bordered()
        .title(Line::from(title))
        .border_style(Style::new().fg(border))
}

/// Bordered block titled `name (count)`, highlighted when focused.
fn titled_block(name: &str, count: Option<usize>, loading: bool, focused: bool) -> Block<'static> {
    let (border, title_style) = if focused {
        (ACCENT, Style::new().fg(ACCENT).bold())
    } else {
        (MUTED, Style::new().bold())
    };
    pane_block(title_spans(name, count, loading, title_style), border)
}

/// `name (count)`, then `(refreshing)` while loading.
fn title_spans(name: &str, count: Option<usize>, loading: bool, style: Style) -> Vec<Span<'static>> {
    let mut title = vec![Span::styled(format!(" {name} "), style)];
    if let Some(count) = count {
        title.push(Span::styled(format!("({count}) "), Style::new().fg(MUTED)));
    }
    if loading {
        title.push(Span::styled("(refreshing) ", Style::new().fg(Color::Yellow)));
    }
    title
}

/// What to show instead of the table when `content` has no item.
fn placeholder<T>(content: &Content<T>, section: &str, config_path: &str) -> Option<Line<'static>> {
    Some(match content {
        Content::Loaded(items) if !items.is_empty() => return None,
        Content::NotConfigured => {
            Line::from(format!("No {section} section in {config_path}")).fg(Color::Yellow)
        }
        Content::Pending => Line::from("Loading...").fg(MUTED).italic(),
        Content::Failed(err) => Line::from(err.clone()).fg(Color::Red),
        Content::Loaded(_) => Line::from("Nothing to show.").fg(MUTED),
    })
}

fn render_message(frame: &mut Frame, area: Rect, block: Block, text: Line) {
    frame.render_widget(Paragraph::new(text).wrap(Wrap { trim: true }).block(block), area);
}

fn render_pane<T: Item>(frame: &mut Frame, area: Rect, view: PaneView<T>, pane: &mut Pane<T>) {
    let count = match &pane.content {
        Content::Loaded(items) => Some(items.len()),
        _ => None,
    };
    let block = titled_block(view.title, count, pane.loading, view.focused);
    if let Some(text) = placeholder(&pane.content, view.section, view.config_path) {
        render_message(frame, area, block, text);
        return;
    }
    let rows = pane.items().iter().map(|item| lines((view.row)(item))).collect();
    let header = header_labels(view.header, pane.sort);
    let highlight = if view.focused {
        Style::new().add_modifier(Modifier::REVERSED | Modifier::BOLD)
    } else {
        Style::new()
    };
    render_table(frame, area, block, header, view.choosing, rows, highlight, &mut pane.table);
}

/// Table with a highlighted header cell while choosing the sort column.
#[allow(clippy::too_many_arguments)]
fn render_table(
    frame: &mut Frame,
    area: Rect,
    block: Block,
    header: Vec<String>,
    choosing: Option<usize>,
    rows: Vec<Vec<Line<'static>>>,
    highlight: Style,
    state: &mut TableState,
) {
    let mut widths: Vec<usize> = header.iter().map(|h| h.chars().count()).collect();
    for row in &rows {
        for (width, cell) in widths.iter_mut().zip(row) {
            *width = (*width).max(cell.width());
        }
    }
    // Every column fits its content, except the last one which takes the rest.
    let last = widths.len() - 1;
    let constraints = widths.iter().enumerate().map(|(i, &w)| match i {
        i if i == last => Constraint::Fill(1),
        _ => Constraint::Length(w as u16),
    });
    let rows = rows.into_iter().map(Row::new);
    let header = Row::new(header.into_iter().enumerate().map(|(i, label)| {
        let cell = Cell::from(label);
        if choosing == Some(i) { cell.reversed() } else { cell }
    }))
    .style(Style::new().fg(Color::Yellow).bold().underlined());
    let table = Table::new(rows, constraints)
        .header(header)
        .column_spacing(2)
        .row_highlight_style(highlight)
        .block(block);
    frame.render_stateful_widget(table, area, state);
}

/// Column names, with `^` (ascending) or `v` (descending) on the sort column.
fn header_labels(columns: &[&str], sort: Option<Sort>) -> Vec<String> {
    columns
        .iter()
        .enumerate()
        .map(|(i, name)| match sort {
            Some(sort) if sort.column == i => {
                format!("{name} {}", if sort.descending { "v" } else { "^" })
            }
            _ => name.to_string(),
        })
        .collect()
}

/// Live view of the review session of the selected MR.
/// Review of the selected MR, running, finished or starting.
fn render_review(frame: &mut Frame, area: Rect, app: &App) {
    let title_style = Style::new().fg(CLAUDE).bold();
    let Some(mr) = app.selected_mr() else { return };
    let mut title = vec![
        Span::styled(" Review ", title_style),
        Span::styled(format!("{} ", mr.reference), Style::new().bold()),
    ];
    let Some(review) = app.selected_review() else {
        title.push(Span::styled("[starting] ", Style::new().fg(Color::Yellow)));
        let text = Line::from("Preparing the worktree and starting Claude...").fg(Color::Yellow);
        frame.render_widget(Paragraph::new(text).block(pane_block(title, MUTED)), area);
        return;
    };

    title.push(session_state(review));
    title.extend([
        Span::styled(" a", Style::new().fg(ACCENT).bold()),
        Span::styled(": attach ", Style::new().fg(MUTED)),
    ]);
    render_preview(frame, area, pane_block(title, CLAUDE), app.preview_of(&review.name));
}

/// Live screen of the Claude session started with `c` on the selected MR.
fn render_claude(frame: &mut Frame, area: Rect, app: &App, session: &Session) {
    let mut title = vec![Span::styled(" Claude ", Style::new().fg(CLAUDE).bold())];
    title.push(session_state(session));
    // Once Claude exited, `c` opens the menu again.
    let action = if session.finished { ": menu " } else { ": attach " };
    title.extend([
        Span::styled(" c", Style::new().fg(ACCENT).bold()),
        Span::styled(action, Style::new().fg(MUTED)),
    ]);
    render_preview(frame, area, pane_block(title, CLAUDE), app.preview_of(&session.name));
}

fn session_state(session: &Session) -> Span<'static> {
    match session.finished {
        true => Span::styled("[finished] ", Style::new().fg(Color::Blue).bold()),
        false => Span::styled("[running] ", Style::new().fg(Color::Green).bold()),
    }
}

/// End of a captured session screen, like a terminal would show it.
fn render_preview(frame: &mut Frame, area: Rect, block: Block, raw: Option<&str>) {
    let raw = raw.unwrap_or("");
    let inner_height = area.height.saturating_sub(2) as usize;
    // The preview keeps Claude's colors; unparsable output falls back to plain text.
    let text = raw.into_text().unwrap_or_else(|_| Text::raw(raw));
    let skip = text.lines.len().saturating_sub(inner_height);
    let lines: Vec<Line> = text.lines.into_iter().skip(skip).collect();
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

/// Calls made by the connectors, latest at the bottom.
fn render_calls(frame: &mut Frame, area: Rect, app: &App) {
    let calls = app.api_log.calls();
    let title = vec![
        Span::styled(" API calls ", Style::new().bold()),
        Span::styled(format!("({}) ", calls.len()), Style::new().fg(MUTED)),
        Span::styled(" l", Style::new().fg(ACCENT).bold()),
        Span::styled(": hide ", Style::new().fg(MUTED)),
    ];
    let block = pane_block(title, MUTED);
    if calls.is_empty() {
        let text = Line::from("No call yet.").fg(MUTED);
        frame.render_widget(Paragraph::new(text).block(block), area);
        return;
    }
    let inner_height = area.height.saturating_sub(2) as usize;
    let skip = calls.len().saturating_sub(inner_height);
    let lines: Vec<Line> = calls[skip..]
        .iter()
        .map(|call| {
            let (outcome, color) = match &call.outcome {
                Ok(status) if (200..300).contains(status) => (status.to_string(), Color::Green),
                Ok(status) => (status.to_string(), Color::Red),
                Err(err) => (err.clone(), Color::Red),
            };
            let duration = format!("{}ms", call.duration.as_millis());
            Line::from(vec![
                Span::styled(format!("{}  ", call.at.format("%H:%M:%S")), Style::new().fg(MUTED)),
                Span::raw(format!("{:<4}  ", call.method)),
                Span::styled(format!("{outcome:<3}  "), Style::new().fg(color)),
                Span::styled(format!("{duration:>6}  "), Style::new().fg(MUTED)),
                Span::raw(call.url.clone()),
            ])
        })
        .collect();
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

/// Changes notified so far, latest at the bottom.
fn render_notifications(frame: &mut Frame, area: Rect, app: &mut App) {
    let focused = app.focus == Focus::Notifications;
    let title_style = if focused { Style::new().fg(ACCENT).bold() } else { Style::new().bold() };
    let title = vec![
        Span::styled(" Notifications ", title_style),
        Span::styled(format!("({}) ", app.notified.len()), Style::new().fg(MUTED)),
        Span::styled(" n", Style::new().fg(ACCENT).bold()),
        Span::styled(": hide ", Style::new().fg(MUTED)),
    ];
    let block = pane_block(title, if focused { ACCENT } else { MUTED });
    if app.notified.is_empty() {
        let text = Line::from("No notification yet.").fg(MUTED);
        frame.render_widget(Paragraph::new(text).block(block), area);
        return;
    }
    let rows = app.notified.iter().map(|(at, change)| {
        Row::new(vec![
            Cell::from(at.format("%H:%M:%S").to_string()).fg(MUTED),
            Cell::from(change.text.clone()),
        ])
    });
    let highlight = if focused {
        Style::new().add_modifier(Modifier::REVERSED | Modifier::BOLD)
    } else {
        Style::new()
    };
    let table = Table::new(rows, [Constraint::Length(8), Constraint::Fill(1)])
        .column_spacing(2)
        .block(block)
        .row_highlight_style(highlight);
    frame.render_stateful_widget(table, area, &mut app.notifications_table);
}

const SORT_KEYS: [(&str, &str); 3] = [
    ("h/l", "choose column"),
    ("Enter", "sort (again: reverse)"),
    ("Esc", "cancel"),
];

const MENU_KEYS: [(&str, &str); 3] = [("j/k", "move"), ("Enter", "choose"), ("Esc", "back")];

const KEYS: [(&str, &str); 12] = [
    ("Tab", "switch"),
    ("j/k", "move"),
    ("Enter", "open"),
    ("s", "sort"),
    ("r", "refresh"),
    ("a", "attach"),
    ("x", "close"),
    ("c", "claude"),
    ("w", "view"),
    ("n", "notifs"),
    ("l", "calls"),
    ("q", "quit"),
];

/// Key help, replaced by the last message until the next key press.
fn render_footer(frame: &mut Frame, area: Rect, app: &App) {
    let line = match &app.message {
        Some(notice) => {
            let color = match notice.level {
                Level::Info => Color::Green,
                Level::Warning => Color::Yellow,
                Level::Error => Color::Red,
            };
            Line::from(format!(" {}", notice.text)).fg(color)
        }
        None => {
            let (mut spans, keys) = match (app.sorting, &app.menu) {
                (Some(_), _) => {
                    let label = Span::styled(" Sort by: ", Style::new().fg(ACCENT).bold());
                    (vec![label], &SORT_KEYS[..])
                }
                (None, Some(_)) => (vec![Span::raw(" ")], &MENU_KEYS[..]),
                (None, None) => (vec![Span::raw(" ")], &KEYS[..]),
            };
            for (key, action) in keys {
                spans.push(Span::styled(*key, Style::new().fg(ACCENT).bold()));
                spans.push(Span::styled(format!(": {action}  "), Style::new().fg(MUTED)));
            }
            if let Some(at) = app.last_update.filter(|_| app.sorting.is_none() && app.menu.is_none()) {
                spans.push(Span::styled(format!("updated {}", at.format("%H:%M")), Style::new().fg(MUTED)));
            }
            Line::from(spans)
        }
    };
    frame.render_widget(line, area);
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use super::*;
    use usine::connectors::claude::Conversation;

    use crate::tui::app::{Msg, Notice};
    use crate::tui::app::tests::{issue, mr, session, split_app, tree_app};

    fn draw_buffer(app: &mut App) -> Buffer {
        let mut terminal = Terminal::new(TestBackend::new(130, 24)).unwrap();
        terminal.draw(|frame| render(frame, app)).unwrap();
        terminal.backend().buffer().clone()
    }

    /// Style of the first cell of the first occurrence of `text` on screen.
    fn style_of(buffer: &Buffer, text: &str) -> Style {
        let width = buffer.area.width as usize;
        for (y, line) in buffer.content().chunks(width).enumerate() {
            let symbols: Vec<&str> = line.iter().map(|cell| cell.symbol()).collect();
            let row = symbols.concat();
            if let Some(byte) = row.find(text) {
                // Map the byte offset back to a cell index.
                let mut offset = 0;
                for (x, symbol) in symbols.iter().enumerate() {
                    if offset == byte {
                        return buffer[(x as u16, y as u16)].style();
                    }
                    offset += symbol.len();
                }
            }
        }
        panic!("{text:?} not on screen");
    }

    fn draw(app: &mut App) -> String {
        let buffer = draw_buffer(app);
        buffer
            .content()
            .chunks(buffer.area.width as usize)
            .map(|line| line.iter().map(|cell| cell.symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn renders_both_panes_with_data() {
        let mut app = App::new(true, true, "config.toml".into());
        app.on_msg(Msg::Jira(Ok(vec![issue("PAY-1"), issue("PAY-2")])));
        let worktree = Some("/srv/api-feat".into());
        app.on_msg(Msg::Gitlab(Ok(vec![MergeRequest { worktree, ..mr(42) }])));

        let screen = draw(&mut app);

        assert!(screen.contains("Jira issues (2)"), "{screen}");
        assert!(screen.contains("R  C  ROLE ^  MR"), "{screen}");
        assert!(screen.contains("WORKTREE       TITLE"), "{screen}");
        assert!(screen.contains("/srv/api-feat  MR 42"), "{screen}");

        let claude = |finished| Session { name: "c".into(), subject: "group/project!42".into(), finished };
        app.on_msg(Msg::ClaudeSessions(vec![claude(false)]));
        // The C column marks the Claude session.
        let buffer = draw_buffer(&mut app);
        assert_eq!(style_of(&buffer, "*  author  group/project!42").fg, Some(Color::Green));
        app.on_msg(Msg::ClaudeSessions(vec![claude(true)]));
        assert_eq!(style_of(&draw_buffer(&mut app), "*  author  group/project!42").fg, Some(Color::Blue));
        assert!(screen.contains("GitLab MRs (1)"), "{screen}");
        assert!(screen.contains("KEY    STATUS ^  PRIORITY  UPDATED     SUMMARY"), "{screen}");
        assert!(screen.contains("PAY-2  To Do     -         2026-10-10  Summary of PAY-2"), "{screen}");
        assert!(screen.contains("group/project!42"), "{screen}");
        assert!(screen.contains("q: quit"), "{screen}");
    }

    #[test]
    fn renders_placeholder_states() {
        let mut app = App::new(true, false, "/home/me/.config/usine/config.toml".into());

        let screen = draw(&mut app);
        assert!(screen.contains("Loading..."), "{screen}");
        assert!(
            screen.contains("No [gitlab] section in /home/me/.config/usine/config.toml"),
            "{screen}"
        );

        app.on_msg(Msg::Jira(Err("Jira search failed (401 Unauthorized)".into())));
        let screen = draw(&mut app);
        assert!(screen.contains("Jira search failed (401 Unauthorized)"), "{screen}");
    }

    #[test]
    fn shows_refreshing_in_title() {
        let mut app = App::new(true, true, "config.toml".into());
        app.on_msg(Msg::Jira(Ok(vec![issue("PAY-1")])));
        app.start_refresh();

        let screen = draw(&mut app);
        assert!(screen.contains("Jira issues (1) (refreshing)"), "{screen}");
    }

    #[test]
    fn renders_review_preview_of_selected_mr() {
        let mut app = App::new(false, true, "config.toml".into());
        app.on_msg(Msg::Gitlab(Ok(vec![mr(1), mr(2)])));
        app.on_msg(Msg::Reviews(vec![session(1, false)]));
        let lines: Vec<String> = (1..=40).map(|i| format!("claude line {i}")).collect();
        app.on_msg(Msg::Preview { session: session(1, false).name, text: lines.join("\n") });

        let screen = draw(&mut app);
        assert!(screen.contains("Review group/project!1 [running]"), "{screen}");
        assert!(screen.contains("*     author  group/project!1"), "{screen}");
        // The end of the session screen is shown.
        assert!(screen.contains("claude line 40"), "{screen}");
        assert!(!screen.contains("claude line 1 "), "{screen}");
    }

    #[test]
    fn claude_pane_is_next_to_the_review_while_the_mr_has_a_session() {
        let mut app = App::new(false, true, "config.toml".into());
        app.on_msg(Msg::Gitlab(Ok(vec![mr(1), mr(2)])));
        assert!(!draw(&mut app).contains(" Claude "));

        let claude = |finished| Session { name: "usine-claude-1".into(), subject: mr(1).reference, finished };
        app.on_msg(Msg::ClaudeSessions(vec![claude(false)]));
        app.on_msg(Msg::Preview { session: "usine-claude-1".into(), text: "working on it".into() });
        let screen = draw(&mut app);
        assert!(!screen.contains("Review"), "{screen}");
        assert!(screen.contains("Claude [running]  c: attach"), "{screen}");
        assert!(screen.contains("working on it"), "{screen}");

        app.on_msg(Msg::ClaudeSessions(vec![claude(true)]));
        assert!(draw(&mut app).contains("Claude [finished]  c: menu"));

        // Another MR: no session, no Claude pane.
        app.on_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        app.on_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
        assert!(!draw(&mut app).contains("working on it"));
    }

    #[test]
    fn renders_review_placeholders() {
        let mut app = App::new(false, true, "config.toml".into());
        app.on_msg(Msg::Gitlab(Ok(vec![mr(1)])));
        let screen = draw(&mut app);
        assert!(!screen.contains("Review"), "{screen}");

        app.starting.push("group/project!1".into());
        let screen = draw(&mut app);
        assert!(screen.contains("~     author  group/project!1"), "{screen}");
        assert!(screen.contains("Preparing the worktree"), "{screen}");
    }

    #[test]
    fn colors_statuses_priorities_and_markers() {
        let mut app = App::new(true, true, "config.toml".into());
        let mut urgent = issue("PAY-1");
        urgent.priority = Some("High".into());
        let mut done = issue("PAY-2");
        done.status = "Done".into();
        done.status_category = StatusCategory::Done;
        let mut dev_done = issue("PAY-3");
        dev_done.status = "Dev OK".into();
        dev_done.status_category = StatusCategory::InProgress;
        app.status_colors.insert("dev ok".into(), Color::Magenta);
        app.on_msg(Msg::Jira(Ok(vec![urgent, done, dev_done])));
        let mut blocked = mr(2);
        blocked.merge_status = "conflict".into();
        blocked.pipeline = Some("failed".into());
        let mut draft = mr(3);
        draft.draft = true;
        draft.title = "Draft: wip".into();
        draft.role = Role::Reviewer;
        app.on_msg(Msg::Gitlab(Ok(vec![mr(1), blocked, draft])));
        app.on_msg(Msg::Reviews(vec![session(1, false), session(2, true)]));

        let buffer = draw_buffer(&mut app);
        assert_eq!(style_of(&buffer, "To Do").fg, Some(Color::Blue));
        assert_eq!(style_of(&buffer, "Done").fg, Some(Color::Green));
        assert_eq!(style_of(&buffer, "Dev OK").fg, Some(Color::Magenta));
        assert_eq!(style_of(&buffer, "High").fg, Some(Color::Red));
        assert_eq!(style_of(&buffer, "mergeable").fg, Some(Color::Green));
        assert_eq!(style_of(&buffer, "conflict").fg, Some(Color::Red));
        assert_eq!(style_of(&buffer, "failed").fg, Some(Color::Red));
        assert_eq!(style_of(&buffer, "Draft: wip").fg, Some(MUTED));
        assert_eq!(style_of(&buffer, "reviewer").fg, Some(Color::LightBlue));
        assert_eq!(style_of(&buffer, "*     author    group/project!1").fg, Some(Color::Green));
        assert_eq!(style_of(&buffer, "*     author    group/project!2").fg, Some(Color::Blue));
        assert_eq!(style_of(&buffer, "KEY").fg, Some(Color::Yellow));
        // Focused pane title uses the accent color.
        assert_eq!(style_of(&buffer, "Jira issues").fg, Some(ACCENT));
    }

    #[test]
    fn review_preview_keeps_session_colors() {
        let mut app = App::new(false, true, "config.toml".into());
        app.on_msg(Msg::Gitlab(Ok(vec![mr(1)])));
        app.on_msg(Msg::Reviews(vec![session(1, false)]));
        app.on_msg(Msg::Preview {
            session: session(1, false).name,
            text: "plain line\n\x1b[32mgreen claude output\x1b[0m".into(),
        });

        let buffer = draw_buffer(&mut app);
        assert_eq!(style_of(&buffer, "green claude output").fg, Some(Color::Green));
        assert_eq!(style_of(&buffer, "[running]").fg, Some(Color::Green));
    }

    #[test]
    fn shows_sort_column_and_choice() {
        let mut app = App::new(true, true, "config.toml".into());
        app.on_msg(Msg::Jira(Ok(vec![issue("PAY-1"), issue("PAY-2")])));
        app.on_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE));
        app.on_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE));

        let buffer = draw_buffer(&mut app);
        let screen = draw(&mut app);
        assert!(screen.contains("Sort by: h/l: choose column"), "{screen}");
        assert!(style_of(&buffer, "PRIORITY").add_modifier.contains(Modifier::REVERSED));
        assert!(!style_of(&buffer, "STATUS").add_modifier.contains(Modifier::REVERSED));

        app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let screen = draw(&mut app);
        assert!(screen.contains("KEY    STATUS  PRIORITY ^  UPDATED"), "{screen}");
        assert!(screen.contains("s: sort"), "{screen}");
        app.on_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE));
        app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(draw(&mut app).contains("PRIORITY v"));
    }

    #[test]
    fn split_view_shows_issues_next_to_their_mrs() {
        let mut app = split_app();
        let screen = draw(&mut app);
        let line = |text: &str| screen.lines().find(|l| l.contains(text)).unwrap_or_default();

        assert!(screen.contains("Jira issues (3)"), "{screen}");
        assert!(screen.contains("GitLab MRs (4)"), "{screen}");
        assert!(!screen.contains("Other"), "{screen}");
        assert!(screen.contains("w: view"), "{screen}");
        assert!(line("PAY-1 ").contains("author  group/project!1 "), "{screen}");
        // Second MR of PAY-1: the issue cells are blank.
        assert!(!line("group/project!3").contains("PAY-"), "{screen}");
        assert!(!line("PAY-2 ").contains("group/project"), "{screen}");
        assert!(line("PAY-3 ").contains("group/project!4"), "{screen}");
        // The MR of no listed issue comes last, without issue.
        let lines: Vec<&str> = screen.lines().collect();
        let last_mr = lines.iter().position(|l| l.contains("group/project!2")).unwrap();
        let pay_3 = lines.iter().position(|l| l.contains("PAY-3 ")).unwrap();
        assert!(last_mr > pay_3, "{screen}");
        assert!(!lines[last_mr].contains("PAY-"), "{screen}");

        // Both sides of the selected row are highlighted, the focused one reversed.
        let buffer = draw_buffer(&mut app);
        assert!(style_of(&buffer, "PAY-1").add_modifier.contains(Modifier::REVERSED));
        let mr_style = style_of(&buffer, "group/project!1");
        assert!(mr_style.add_modifier.contains(Modifier::BOLD));
        assert!(!mr_style.add_modifier.contains(Modifier::REVERSED));
    }

    #[test]
    fn tree_view_shows_mrs_below_their_issue_in_its_columns() {
        let mut app = tree_app();
        app.on_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE));
        let screen = draw(&mut app);
        let lines: Vec<&str> = screen.lines().collect();
        let row = |text: &str| lines.iter().position(|l| l.contains(text)).unwrap();
        // Column of `text` on the row of `anchor`, in characters.
        let column = |anchor: &str, text: &str| {
            let line = lines[row(anchor)];
            line[..line.find(text).unwrap()].chars().count()
        };

        assert!(screen.contains("Jira issues (3)  GitLab MRs (4)"), "{screen}");
        assert!(lines[row("KEY ")].contains("STATUS ^"), "{screen}");
        // MRs follow their issue on branches; issues stay at the left edge.
        assert!(lines[row("PAY-1 ")].starts_with("│PAY-1 "), "{screen}");
        assert_eq!(row("group/project!1"), row("PAY-1 ") + 2, "{screen}");
        assert!(lines[row("group/project!1")].starts_with("│├─     group/project!1"), "{screen}");
        assert!(lines[row("group/project!3")].starts_with("│└─     group/project!3"), "{screen}");
        // MR cells line up with the issue columns.
        assert_eq!(column("group/project!1", "mergeable"), column("PAY-1 ", "To Do"), "{screen}");
        assert_eq!(column("group/project!1", "author"), column("PAY-1 ", "Summary"), "{screen}");
        // Each group of MRs has the names of their columns, issues without MR do not.
        let headers: Vec<usize> = (0..lines.len()).filter(|&n| lines[n].contains("PIPELINE")).collect();
        let expected = [row("PAY-1 "), row("PAY-3 "), row(OTHER_MRS)].map(|n| n + 1);
        assert_eq!(headers, expected, "{screen}");
        assert_eq!(column("PIPELINE", "STATUS"), column("group/project!1", "mergeable"), "{screen}");
        assert_eq!(column("PIPELINE", "MR"), column("group/project!1", "group"), "{screen}");
        assert_eq!(column("PIPELINE", "TITLE"), column("group/project!1", "MR 1"), "{screen}");
        assert_eq!(row("group/project!4"), row("PAY-3 ") + 2, "{screen}");
        // The MR of no listed issue comes last, below its own row.
        assert_eq!(row("group/project!2"), row(OTHER_MRS) + 2, "{screen}");
        assert!(row(OTHER_MRS) > row("group/project!4"), "{screen}");
        assert!(!screen.contains("Review group"), "{screen}");

        let buffer = draw_buffer(&mut app);
        assert!(style_of(&buffer, "PAY-1").add_modifier.contains(Modifier::REVERSED));
        assert!(!style_of(&buffer, "group/project!1").add_modifier.contains(Modifier::REVERSED));
        assert_eq!(style_of(&buffer, "PIPELINE").fg, Some(MR_HEADER));
        // The names of the MR columns are skipped by the selection.
        assert!(!style_of(&buffer, "PIPELINE").add_modifier.contains(Modifier::REVERSED));
        app.on_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
        let buffer = draw_buffer(&mut app);
        assert!(style_of(&buffer, "group/project!1").add_modifier.contains(Modifier::REVERSED));
        assert!(!style_of(&buffer, "PIPELINE").add_modifier.contains(Modifier::REVERSED));
    }

    #[test]
    fn api_calls_pane_is_hidden_until_toggled() {
        use std::time::Duration;
        use usine::api_log::ApiCall;

        let mut app = App::new(false, false, "config.toml".into());
        let call = |url: &str, outcome| ApiCall {
            at: chrono::Local::now(),
            method: "GET".into(),
            url: url.into(),
            outcome,
            duration: Duration::from_millis(42),
        };
        app.api_log.record(call("https://gitlab.com/api/v4/user", Ok(200)));
        app.api_log.record(call("https://gitlab.com/api/v4/merge_requests", Ok(401)));
        app.api_log.record(call("https://example.atlassian.net/rest", Err("timed out".into())));
        assert!(!draw(&mut app).contains("API calls"));

        app.on_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE));
        let buffer = draw_buffer(&mut app);
        let screen = draw(&mut app);
        assert!(screen.contains("API calls (3)  l: hide"), "{screen}");
        assert!(screen.contains("GET   200    42ms  https://gitlab.com/api/v4/user"), "{screen}");
        assert_eq!(style_of(&buffer, "200").fg, Some(Color::Green));
        assert_eq!(style_of(&buffer, "401").fg, Some(Color::Red));
        assert_eq!(style_of(&buffer, "timed out").fg, Some(Color::Red));
    }

    #[test]
    fn notifications_pane_is_hidden_until_toggled() {
        let mut app = App::new(false, false, "config.toml".into());
        app.on_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE));
        assert!(draw(&mut app).contains("No notification yet."));

        let at = chrono::Local::now();
        let text = "PAY-1: To Do -> In Progress".into();
        app.notified.push((at, usine::workflows::changes::Change { text, url: None }));
        let screen = draw(&mut app);
        assert!(screen.contains("Notifications (1)  n: hide"), "{screen}");
        let line = format!("{}  PAY-1: To Do -> In Progress", at.format("%H:%M:%S"));
        assert!(screen.contains(&line), "{screen}");

        app.on_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE));
        assert!(!draw(&mut app).contains("Notifications"));
    }

    #[test]
    fn c_menu_is_drawn_over_the_panes() {
        let mut app = App::new(true, true, "config.toml".into());
        let worktree = Some("/srv/api-feat".into());
        app.on_msg(Msg::Gitlab(Ok(vec![MergeRequest { worktree, ..mr(42) }])));
        app.on_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        app.on_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
        let screen = draw(&mut app);
        assert!(screen.contains(" group/project!42 "), "{screen}");
        assert!(screen.contains("1  New Claude session"), "{screen}");
        assert!(screen.contains("3  Review with Claude"), "{screen}");
        assert!(screen.contains("Enter: choose"), "{screen}");

        let mr = Box::new(app.gitlab.items()[0].clone());
        let updated = chrono::Local::now();
        let conversation = Conversation { id: "a".into(), title: "Fix the retry loop".into(), updated };
        app.show_conversations(mr, vec![conversation]);
        let screen = draw(&mut app);
        assert!(screen.contains("Resume in /srv/api-feat"), "{screen}");
        let line = format!("{}  Fix the retry loop", updated.format("%Y-%m-%d %H:%M"));
        assert!(screen.contains(&line), "{screen}");
    }

    #[test]
    fn footer_colors_messages_by_level() {
        let mut app = App::new(false, false, "config.toml".into());
        assert_eq!(style_of(&draw_buffer(&mut app), "Tab").fg, Some(ACCENT));

        app.message = Some(Notice::error("Cannot review: no clone"));
        assert_eq!(style_of(&draw_buffer(&mut app), "Cannot review").fg, Some(Color::Red));
        app.message = Some(Notice::info("Review started"));
        assert_eq!(style_of(&draw_buffer(&mut app), "Review started").fg, Some(Color::Green));
    }
}
