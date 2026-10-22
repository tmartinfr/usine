//! Command-line front end: argument parsing and plain-text rendering.
//! Without a subcommand, starts the TUI.

mod gitlab;
mod jira;

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
use usine::config::Config;
use usine::model::{Issue, MergeRequest};

#[derive(Debug, Parser)]
#[command(version, about = "Software factory orchestrator")]
pub struct Cli {
    /// Config file [default: ~/.config/usine/config.toml]
    #[arg(long, global = true)]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// GitLab
    Gitlab {
        #[command(subcommand)]
        command: gitlab::GitlabCommand,
    },
    /// Jira Cloud
    Jira {
        #[command(subcommand)]
        command: jira::JiraCommand,
    },
    /// Interactive terminal UI (default when no command is given)
    Tui,
}

pub async fn run(cli: Cli) -> Result<()> {
    let path = match cli.config {
        Some(path) => path,
        None => Config::default_path()?,
    };
    let config = Config::load(&path)?;
    match cli.command {
        Some(Command::Gitlab { command }) => gitlab::run(command, &config, &path).await,
        Some(Command::Jira { command }) => jira::run(command, &config, &path).await,
        Some(Command::Tui) | None => crate::tui::run(config, &path),
    }
}

const MAX_WIDTH: usize = 120;

fn print_issues(issues: &[Issue], json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(issues)?);
    } else if issues.is_empty() {
        println!("No issues.");
    } else {
        print!("{}", issues_table(issues));
    }
    Ok(())
}

fn issues_table(issues: &[Issue]) -> String {
    let rows: Vec<Vec<String>> = issues
        .iter()
        .map(|i| {
            vec![
                i.key.clone(),
                i.kind.clone(),
                i.status.clone(),
                i.priority.clone().unwrap_or_else(|| "-".into()),
                i.updated.format("%Y-%m-%d").to_string(),
                i.title.clone(),
            ]
        })
        .collect();
    table(&["KEY", "TYPE", "STATUS", "PRIORITY", "UPDATED", "SUMMARY"], &rows)
}

fn print_merge_requests(mrs: &[MergeRequest], json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(mrs)?);
    } else if mrs.is_empty() {
        println!("No merge requests.");
    } else {
        print!("{}", merge_requests_table(mrs));
    }
    Ok(())
}

fn merge_requests_table(mrs: &[MergeRequest]) -> String {
    let rows: Vec<Vec<String>> = mrs
        .iter()
        .map(|m| {
            vec![
                m.reference.clone(),
                m.merge_status.clone(),
                m.pipeline.clone().unwrap_or_else(|| "-".into()),
                m.target_branch.clone(),
                m.updated.format("%Y-%m-%d").to_string(),
                m.title.clone(),
            ]
        })
        .collect();
    table(&["MR", "STATUS", "PIPELINE", "TARGET", "UPDATED", "TITLE"], &rows)
}

/// Left-aligned columns; the last one is truncated so lines fit in `MAX_WIDTH`.
fn table(header: &[&str], rows: &[Vec<String>]) -> String {
    let (fixed, _) = header.split_at(header.len() - 1);
    let mut widths: Vec<usize> = fixed.iter().map(|h| h.chars().count()).collect();
    for row in rows {
        for (width, cell) in widths.iter_mut().zip(row) {
            *width = (*width).max(cell.chars().count());
        }
    }
    let last_width = MAX_WIDTH
        .saturating_sub(widths.iter().map(|w| w + 2).sum())
        .max(20);

    let mut out = String::new();
    let header: Vec<String> = header.iter().map(|h| h.to_string()).collect();
    for row in std::iter::once(&header).chain(rows) {
        let (last, cells) = row.split_last().expect("rows are not empty");
        for (cell, width) in cells.iter().zip(&widths) {
            out.push_str(&format!("{cell:<width$}  "));
        }
        out.push_str(&truncate(last, last_width));
        out.push('\n');
    }
    out
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        let kept: String = text.chars().take(max.saturating_sub(3)).collect();
        format!("{kept}...")
    }
}

#[cfg(test)]
mod tests {
    use chrono::DateTime;
    use usine::model::{Role, Source};

    use super::*;

    fn issue(key: &str, priority: Option<&str>, title: &str) -> Issue {
        Issue {
            source: Source::Jira,
            key: key.into(),
            title: title.into(),
            status_category: usine::model::StatusCategory::InProgress,
            status: "In Progress".into(),
            priority: priority.map(Into::into),
            project: "PAY".into(),
            kind: "Bug".into(),
            updated: DateTime::parse_from_rfc3339("2026-10-10T12:00:00+02:00").unwrap(),
            url: format!("https://example.atlassian.net/browse/{key}"),
        }
    }

    #[test]
    fn renders_aligned_table() {
        let table = issues_table(&[
            issue("PAY-1", Some("High"), "Short"),
            issue("PAY-1234", None, "Other"),
        ]);
        assert_eq!(
            table,
            "KEY       TYPE  STATUS       PRIORITY  UPDATED     SUMMARY\n\
             PAY-1     Bug   In Progress  High      2026-10-10  Short\n\
             PAY-1234  Bug   In Progress  -         2026-10-10  Other\n"
        );
    }

    #[test]
    fn renders_merge_requests_table() {
        let mr = MergeRequest {
            source: Source::Gitlab,
            reference: "group/project!42".into(),
            project: "group/project".into(),
            iid: 42,
            title: "Add retry".into(),
            draft: false,
            merge_status: "mergeable".into(),
            pipeline: None,
            role: Role::Author,
            source_branch: "feat".into(),
            target_branch: "main".into(),
            author: "thomas".into(),
            updated: DateTime::parse_from_rfc3339("2026-10-10T12:00:00Z").unwrap(),
            url: "https://gitlab.com/group/project/-/merge_requests/42".into(),
        };
        assert_eq!(
            merge_requests_table(&[mr]),
            "MR                STATUS     PIPELINE  TARGET  UPDATED     TITLE\n\
             group/project!42  mergeable  -         main    2026-10-10  Add retry\n"
        );
    }

    #[test]
    fn truncates_long_summaries() {
        let long = "x".repeat(200);
        let table = issues_table(&[issue("PAY-1", None, &long)]);
        let line = table.lines().nth(1).unwrap();
        assert_eq!(line.chars().count(), MAX_WIDTH);
        assert!(line.ends_with("..."));
    }
}
