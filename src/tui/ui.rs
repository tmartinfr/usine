//! Rendering of the App state with ratatui widgets.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Row, Table, Wrap};
use usine::model::{Issue, MergeRequest};

use super::app::{App, Content, Focus, Item, Pane};

pub fn render(frame: &mut Frame, app: &mut App) {
    let [top, bottom, footer] = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    render_pane(
        frame,
        top,
        PaneView {
            title: "Jira issues",
            section: "[jira]",
            header: ["KEY", "STATUS", "UPDATED", "SUMMARY"],
            row: issue_row,
            focused: app.focus == Focus::Jira,
            config_path: &app.config_path,
        },
        &mut app.jira,
    );
    render_pane(
        frame,
        bottom,
        PaneView {
            title: "GitLab MRs",
            section: "[gitlab]",
            header: ["MR", "STATUS", "UPDATED", "TITLE"],
            row: merge_request_row,
            focused: app.focus == Focus::Gitlab,
            config_path: &app.config_path,
        },
        &mut app.gitlab,
    );
    render_footer(frame, footer, app);
}

struct PaneView<'a, T> {
    title: &'a str,
    section: &'a str,
    header: [&'a str; 4],
    row: fn(&T) -> [String; 4],
    focused: bool,
    config_path: &'a str,
}

fn issue_row(issue: &Issue) -> [String; 4] {
    [
        issue.key.clone(),
        issue.status.clone(),
        issue.updated.format("%Y-%m-%d").to_string(),
        issue.title.clone(),
    ]
}

fn merge_request_row(mr: &MergeRequest) -> [String; 4] {
    [
        mr.reference.clone(),
        mr.merge_status.clone(),
        mr.updated.format("%Y-%m-%d").to_string(),
        mr.title.clone(),
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
    let border = if view.focused {
        Style::new().fg(Color::Cyan)
    } else {
        Style::new().fg(Color::DarkGray)
    };
    let block = Block::bordered().title(title).border_style(border);

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

    let rows: Vec<[String; 4]> = items.iter().map(view.row).collect();
    let mut widths = view.header.map(|h| h.chars().count());
    for row in &rows {
        for (width, cell) in widths.iter_mut().zip(row) {
            *width = (*width).max(cell.chars().count());
        }
    }
    let constraints = [
        Constraint::Length(widths[0] as u16),
        Constraint::Length(widths[1] as u16),
        Constraint::Length(widths[2] as u16),
        Constraint::Fill(1),
    ];
    let highlight = if view.focused {
        Style::new().add_modifier(Modifier::REVERSED)
    } else {
        Style::new()
    };
    let table = Table::new(rows.into_iter().map(Row::new), constraints)
        .header(Row::new(view.header).bold())
        .column_spacing(2)
        .row_highlight_style(highlight)
        .block(block);
    frame.render_stateful_widget(table, area, &mut pane.table);
}

fn render_footer(frame: &mut Frame, area: Rect, app: &App) {
    let mut spans = vec![Span::raw(
        " Tab: switch  j/k: move  g/G: first/last  Enter: open  r: refresh  q: quit",
    )];
    if let Some(message) = &app.message {
        spans.push(Span::raw("  "));
        spans.push(Span::raw(message.as_str()).red());
    } else if let Some(at) = app.last_update {
        spans.push(Span::raw(format!("  updated {}", at.format("%H:%M"))).dark_gray());
    }
    frame.render_widget(Line::from(spans), area);
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;
    use crate::tui::app::Msg;
    use crate::tui::app::tests::{issue, mr};

    fn draw(app: &mut App) -> String {
        let mut terminal = Terminal::new(TestBackend::new(100, 20)).unwrap();
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
}
