//! Rendering of the App state with ratatui widgets.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Row, Table, Wrap};
use usine::model::{Issue, MergeRequest};

use super::app::{App, Content, Focus, Item, Pane};

pub fn render(frame: &mut Frame, app: &mut App) {
    let review_height = match app.selected_review() {
        Some(_) => Constraint::Fill(2),
        None => Constraint::Length(3),
    };
    let [top, middle, bottom, footer] = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Fill(1),
        review_height,
        Constraint::Length(1),
    ])
    .areas(frame.area());

    render_pane(
        frame,
        top,
        PaneView {
            title: "Jira issues",
            section: "[jira]",
            header: &["KEY", "STATUS", "UPDATED", "SUMMARY"],
            row: &issue_row,
            focused: app.focus == Focus::Jira,
            config_path: &app.config_path,
        },
        &mut app.jira,
    );
    let (reviews, starting) = (&app.reviews, &app.starting);
    let merge_request_row = |mr: &MergeRequest| {
        let marker = if starting.contains(&mr.reference) {
            "~"
        } else if reviews.iter().any(|s| s.subject == mr.reference) {
            "*"
        } else {
            " "
        };
        vec![
            marker.to_string(),
            mr.reference.clone(),
            mr.merge_status.clone(),
            mr.updated.format("%Y-%m-%d").to_string(),
            mr.title.clone(),
        ]
    };
    render_pane(
        frame,
        middle,
        PaneView {
            title: "GitLab MRs",
            section: "[gitlab]",
            header: &["R", "MR", "STATUS", "UPDATED", "TITLE"],
            row: &merge_request_row,
            focused: app.focus == Focus::Gitlab,
            config_path: &app.config_path,
        },
        &mut app.gitlab,
    );
    render_review(frame, bottom, app);
    render_footer(frame, footer, app);
}

struct PaneView<'a, T> {
    title: &'a str,
    section: &'a str,
    header: &'a [&'a str],
    row: &'a dyn Fn(&T) -> Vec<String>,
    focused: bool,
    config_path: &'a str,
}

fn issue_row(issue: &Issue) -> Vec<String> {
    vec![
        issue.key.clone(),
        issue.status.clone(),
        issue.updated.format("%Y-%m-%d").to_string(),
        issue.title.clone(),
    ]
}

fn render_pane<T: Item>(frame: &mut Frame, area: Rect, view: PaneView<T>, pane: &mut Pane<T>) {
    let mut title = match &pane.content {
        Content::Loaded(items) => format!(" {} ({}) ", view.title, items.len()),
        _ => format!(" {} ", view.title),
    };
    if pane.loading {
        title.push_str("(refreshing) ");
    }
    let block = Block::bordered().title(title).border_style(border_style(view.focused));

    let items = match &pane.content {
        Content::Loaded(items) if !items.is_empty() => items,
        content => {
            let text = match content {
                Content::NotConfigured => {
                    Line::from(format!("No {} section in {}", view.section, view.config_path))
                }
                Content::Pending => Line::from("Loading..."),
                Content::Failed(err) => Line::from(err.as_str()).red(),
                Content::Loaded(_) => Line::from("Nothing to show."),
            };
            let paragraph = Paragraph::new(text).wrap(Wrap { trim: true }).block(block);
            frame.render_widget(paragraph, area);
            return;
        }
    };

    let rows: Vec<Vec<String>> = items.iter().map(view.row).collect();
    let mut widths: Vec<usize> = view.header.iter().map(|h| h.chars().count()).collect();
    for row in &rows {
        for (width, cell) in widths.iter_mut().zip(row) {
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
        Style::new().add_modifier(Modifier::REVERSED)
    } else {
        Style::new()
    };
    let table = Table::new(rows.into_iter().map(Row::new), constraints)
        .header(Row::new(view.header.iter().copied()).bold())
        .column_spacing(2)
        .row_highlight_style(highlight)
        .block(block);
    frame.render_stateful_widget(table, area, &mut pane.table);
}

fn border_style(focused: bool) -> Style {
    if focused {
        Style::new().fg(Color::Cyan)
    } else {
        Style::new().fg(Color::DarkGray)
    }
}

/// Live view of the review session of the selected MR.
fn render_review(frame: &mut Frame, area: Rect, app: &App) {
    let block = Block::bordered().border_style(border_style(false));
    let Some(mr) = app.gitlab.selected() else {
        frame.render_widget(Paragraph::new("No merge request selected.").block(block.title(" Review ")), area);
        return;
    };
    let Some(review) = app.selected_review() else {
        let text = if app.starting.contains(&mr.reference) {
            "Preparing the worktree and starting Claude..."
        } else {
            "No review for this MR. Press v to start one."
        };
        let title = format!(" Review {} ", mr.reference);
        frame.render_widget(Paragraph::new(text).block(block.title(title)), area);
        return;
    };

    let state = if review.finished { "finished" } else { "running" };
    let title = format!(" Review {} [{state}]  a: attach ", mr.reference);
    let inner_height = area.height.saturating_sub(2) as usize;
    let text = app.preview_of(&review.name).unwrap_or("");
    // Show the end of the session screen, like a terminal would.
    let lines: Vec<Line> = text
        .lines()
        .skip(text.lines().count().saturating_sub(inner_height))
        .map(Line::from)
        .collect();
    frame.render_widget(Paragraph::new(lines).block(block.title(title)), area);
}

/// Key help, replaced by the last message until the next key press.
fn render_footer(frame: &mut Frame, area: Rect, app: &App) {
    let line = match &app.message {
        Some(message) => Line::from(format!(" {message}")).red(),
        None => {
            let mut spans = vec![Span::raw(
                " Tab: switch  j/k: move  Enter: open  r: refresh  v: review  a: attach  x: close  q: quit",
            )];
            if let Some(at) = app.last_update {
                spans.push(Span::raw(format!("  updated {}", at.format("%H:%M"))).dark_gray());
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

    use super::*;
    use crate::tui::app::Msg;
    use crate::tui::app::tests::{issue, mr, session};

    fn draw(app: &mut App) -> String {
        let mut terminal = Terminal::new(TestBackend::new(110, 24)).unwrap();
        terminal.draw(|frame| render(frame, app)).unwrap();
        let buffer = terminal.backend().buffer();
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
        assert!(screen.contains("KEY    STATUS  UPDATED     SUMMARY"), "{screen}");
        assert!(screen.contains("PAY-2  To Do   2026-10-10  Summary of PAY-2"), "{screen}");
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
        assert!(screen.contains("*  group/project!1"), "{screen}");
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
        assert!(screen.contains("~  group/project!1"), "{screen}");
        assert!(screen.contains("Preparing the worktree"), "{screen}");
    }
}
