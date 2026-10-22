//! Rendering of the App state with ratatui widgets.

use ansi_to_tui::IntoText;
use chrono::{Duration, Local};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Cell, Paragraph, Row, Table, Wrap};
use usine::model::{Issue, MergeRequest, Role, StatusCategory};

use super::app::{App, Content, Focus, Item, Level, Pane, review_state};
use super::sort::{
    ISSUE_COLUMNS, MR_COLUMNS, ReviewState, Sort, merge_readiness, pipeline_rank, priority_rank,
};

/// Color of focused borders, titles and key hints.
const ACCENT: Color = Color::Cyan;
/// Color of the Claude review pane.
const CLAUDE: Color = Color::Magenta;
const MUTED: Color = Color::DarkGray;

/// A table cell: text and style.
type Styled = (String, Style);

pub fn render(frame: &mut Frame, app: &mut App) {
    let review_height = match app.selected_review() {
        Some(_) => Constraint::Fill(2),
        None => Constraint::Length(3),
    };
    let calls_height = if app.show_calls { Constraint::Fill(1) } else { Constraint::Length(0) };
    let [top, middle, bottom, calls, footer] = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Fill(1),
        review_height,
        calls_height,
        Constraint::Length(1),
    ])
    .areas(frame.area());

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
    let (reviews, starting) = (&app.reviews, &app.starting);
    let merge_request_row = |mr: &MergeRequest| {
        let marker = match review_state(reviews, starting, mr) {
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
            marker,
            role,
            (mr.reference.clone(), Style::new().bold()),
            (mr.merge_status.clone(), Style::new().fg(merge_status_color(&mr.merge_status))),
            pipeline_cell(mr.pipeline.as_deref()),
            updated_cell(mr.updated),
            (mr.title.clone(), title_style),
        ]
    };
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
    render_review(frame, bottom, app);
    if app.show_calls {
        render_calls(frame, calls, app);
    }
    render_footer(frame, footer, app);
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

fn issue_row(issue: &Issue) -> Vec<Styled> {
    let status_color = match issue.status_category {
        StatusCategory::Todo => Color::Blue,
        StatusCategory::InProgress => Color::Yellow,
        StatusCategory::Done => Color::Green,
    };
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

fn render_pane<T: Item>(frame: &mut Frame, area: Rect, view: PaneView<T>, pane: &mut Pane<T>) {
    let (border, title_style) = if view.focused {
        (ACCENT, Style::new().fg(ACCENT).bold())
    } else {
        (MUTED, Style::new().bold())
    };
    let mut title = vec![Span::styled(format!(" {} ", view.title), title_style)];
    if let Content::Loaded(items) = &pane.content {
        title.push(Span::styled(format!("({}) ", items.len()), Style::new().fg(MUTED)));
    }
    if pane.loading {
        title.push(Span::styled("(refreshing) ", Style::new().fg(Color::Yellow)));
    }
    let block = pane_block(title, border);

    let items = match &pane.content {
        Content::Loaded(items) if !items.is_empty() => items,
        content => {
            let text = match content {
                Content::NotConfigured => Line::from(format!(
                    "No {} section in {}",
                    view.section, view.config_path
                ))
                .fg(Color::Yellow),
                Content::Pending => Line::from("Loading...").fg(MUTED).italic(),
                Content::Failed(err) => Line::from(err.as_str()).fg(Color::Red),
                Content::Loaded(_) => Line::from("Nothing to show.").fg(MUTED),
            };
            let paragraph = Paragraph::new(text).wrap(Wrap { trim: true }).block(block);
            frame.render_widget(paragraph, area);
            return;
        }
    };

    let rows: Vec<Vec<Styled>> = items.iter().map(view.row).collect();
    let header = header_labels(view.header, pane.sort);
    let mut widths: Vec<usize> = header.iter().map(|h| h.chars().count()).collect();
    for row in &rows {
        for (width, (cell, _)) in widths.iter_mut().zip(row) {
            *width = (*width).max(cell.chars().count());
        }
    }
    // Every column fits its content, except the last one which takes the rest.
    let last = widths.len() - 1;
    let constraints = widths.iter().enumerate().map(|(i, &w)| match i {
        i if i == last => Constraint::Fill(1),
        _ => Constraint::Length(w as u16),
    });
    let highlight = if view.focused {
        Style::new().add_modifier(Modifier::REVERSED | Modifier::BOLD)
    } else {
        Style::new()
    };
    let rows = rows.into_iter().map(|row| {
        Row::new(row.into_iter().map(|(text, style)| Cell::from(text).style(style)))
    });
    let header = Row::new(header.into_iter().enumerate().map(|(i, label)| {
        let cell = Cell::from(label);
        if view.choosing == Some(i) { cell.reversed() } else { cell }
    }))
    .style(Style::new().fg(Color::Yellow).bold().underlined());
    let table = Table::new(rows, constraints)
        .header(header)
        .column_spacing(2)
        .row_highlight_style(highlight)
        .block(block);
    frame.render_stateful_widget(table, area, &mut pane.table);
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
fn render_review(frame: &mut Frame, area: Rect, app: &App) {
    let title_style = Style::new().fg(CLAUDE).bold();
    let Some(mr) = app.gitlab.selected() else {
        let block = pane_block(vec![Span::styled(" Review ", title_style)], MUTED);
        let text = Line::from("No merge request selected.").fg(MUTED);
        frame.render_widget(Paragraph::new(text).block(block), area);
        return;
    };
    let mut title = vec![
        Span::styled(" Review ", title_style),
        Span::styled(format!("{} ", mr.reference), Style::new().bold()),
    ];
    let Some(review) = app.selected_review() else {
        let text = if app.starting.contains(&mr.reference) {
            title.push(Span::styled("[starting] ", Style::new().fg(Color::Yellow)));
            Line::from("Preparing the worktree and starting Claude...").fg(Color::Yellow)
        } else {
            Line::from(vec![
                Span::raw("No review for this MR. Press "),
                Span::styled("v", Style::new().fg(ACCENT).bold()),
                Span::raw(" to start one."),
            ])
            .fg(MUTED)
        };
        frame.render_widget(Paragraph::new(text).block(pane_block(title, MUTED)), area);
        return;
    };

    if review.finished {
        title.push(Span::styled("[finished] ", Style::new().fg(Color::Blue).bold()));
    } else {
        title.push(Span::styled("[running] ", Style::new().fg(Color::Green).bold()));
    }
    title.extend([
        Span::styled(" a", Style::new().fg(ACCENT).bold()),
        Span::styled(": attach ", Style::new().fg(MUTED)),
    ]);
    let block = pane_block(title, CLAUDE);
    let inner_height = area.height.saturating_sub(2) as usize;
    let raw = app.preview_of(&review.name).unwrap_or("");
    // The preview keeps Claude's colors; unparsable output falls back to plain text.
    let text = raw.into_text().unwrap_or_else(|_| Text::raw(raw));
    // Show the end of the session screen, like a terminal would.
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

const SORT_KEYS: [(&str, &str); 3] = [
    ("h/l", "choose column"),
    ("Enter", "sort (again: reverse)"),
    ("Esc", "cancel"),
];

const KEYS: [(&str, &str); 10] = [
    ("Tab", "switch"),
    ("j/k", "move"),
    ("Enter", "open"),
    ("s", "sort"),
    ("r", "refresh"),
    ("v", "review"),
    ("a", "attach"),
    ("x", "close"),
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
            let (mut spans, keys) = match app.sorting {
                Some(_) => {
                    let label = Span::styled(" Sort by: ", Style::new().fg(ACCENT).bold());
                    (vec![label], &SORT_KEYS[..])
                }
                None => (vec![Span::raw(" ")], &KEYS[..]),
            };
            for (key, action) in keys {
                spans.push(Span::styled(*key, Style::new().fg(ACCENT).bold()));
                spans.push(Span::styled(format!(": {action}  "), Style::new().fg(MUTED)));
            }
            if let Some(at) = app.last_update.filter(|_| app.sorting.is_none()) {
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
    use crate::tui::app::{Msg, Notice};
    use crate::tui::app::tests::{issue, mr, session};

    fn draw_buffer(app: &mut App) -> Buffer {
        let mut terminal = Terminal::new(TestBackend::new(110, 24)).unwrap();
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
        app.on_msg(Msg::Gitlab(Ok(vec![mr(42)])));

        let screen = draw(&mut app);

        assert!(screen.contains("Jira issues (2)"), "{screen}");
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
        assert!(screen.contains("*  author  group/project!1"), "{screen}");
        // The end of the session screen is shown.
        assert!(screen.contains("claude line 40"), "{screen}");
        assert!(!screen.contains("claude line 1 "), "{screen}");
    }

    #[test]
    fn renders_review_placeholders() {
        let mut app = App::new(false, true, "config.toml".into());
        app.on_msg(Msg::Gitlab(Ok(vec![mr(1)])));
        let screen = draw(&mut app);
        assert!(screen.contains("No review for this MR. Press v to start one."), "{screen}");

        app.starting.push("group/project!1".into());
        let screen = draw(&mut app);
        assert!(screen.contains("~  author  group/project!1"), "{screen}");
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
        app.on_msg(Msg::Jira(Ok(vec![urgent, done])));
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
        assert_eq!(style_of(&buffer, "High").fg, Some(Color::Red));
        assert_eq!(style_of(&buffer, "mergeable").fg, Some(Color::Green));
        assert_eq!(style_of(&buffer, "conflict").fg, Some(Color::Red));
        assert_eq!(style_of(&buffer, "failed").fg, Some(Color::Red));
        assert_eq!(style_of(&buffer, "Draft: wip").fg, Some(MUTED));
        assert_eq!(style_of(&buffer, "reviewer").fg, Some(Color::LightBlue));
        assert_eq!(style_of(&buffer, "*  author    group/project!1").fg, Some(Color::Green));
        assert_eq!(style_of(&buffer, "*  author    group/project!2").fg, Some(Color::Blue));
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
    fn footer_colors_messages_by_level() {
        let mut app = App::new(false, false, "config.toml".into());
        assert_eq!(style_of(&draw_buffer(&mut app), "Tab").fg, Some(ACCENT));

        app.message = Some(Notice::error("Cannot review: no clone"));
        assert_eq!(style_of(&draw_buffer(&mut app), "Cannot review").fg, Some(Color::Red));
        app.message = Some(Notice::info("Review started"));
        assert_eq!(style_of(&draw_buffer(&mut app), "Review started").fg, Some(Color::Green));
    }
}
