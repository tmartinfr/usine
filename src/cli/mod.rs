//! Command-line front end: argument parsing and plain-text rendering.

mod jira;

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
use usine::config::Config;
use usine::model::Issue;

#[derive(Debug, Parser)]
#[command(version, about = "Software factory orchestrator")]
pub struct Cli {
    /// Config file [default: ~/.config/usine/config.toml]
    #[arg(long, global = true)]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Jira Cloud
    Jira {
        #[command(subcommand)]
        command: jira::JiraCommand,
    },
}

pub async fn run(cli: Cli) -> Result<()> {
    let path = match cli.config {
        Some(path) => path,
        None => Config::default_path()?,
    };
    let config = Config::load(&path)?;
    match cli.command {
        Command::Jira { command } => jira::run(command, &config, &path).await,
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
    let header = ["KEY", "TYPE", "STATUS", "PRIORITY", "UPDATED"];
    let rows: Vec<[String; 5]> = issues
        .iter()
        .map(|i| {
            [
                i.key.clone(),
                i.kind.clone(),
                i.status.clone(),
                i.priority.clone().unwrap_or_else(|| "-".into()),
                i.updated.format("%Y-%m-%d").to_string(),
            ]
        })
        .collect();

    let mut widths = header.map(|h| h.chars().count());
    for row in &rows {
        for (width, cell) in widths.iter_mut().zip(row) {
            *width = (*width).max(cell.chars().count());
        }
    }
    let summary_width = MAX_WIDTH
        .saturating_sub(widths.iter().map(|w| w + 2).sum())
        .max(20);

    let mut out = String::new();
    let mut push_line = |cells: [&str; 5], summary: &str| {
        for (cell, width) in cells.iter().zip(widths) {
            out.push_str(&format!("{cell:<width$}  "));
        }
        out.push_str(&truncate(summary, summary_width));
        out.push('\n');
    };
    push_line(header, "SUMMARY");
    for (row, issue) in rows.iter().zip(issues) {
        push_line(row.each_ref().map(String::as_str), &issue.title);
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
    use usine::model::Source;

    use super::*;

    fn issue(key: &str, priority: Option<&str>, title: &str) -> Issue {
        Issue {
            source: Source::Jira,
            key: key.into(),
            title: title.into(),
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
    fn truncates_long_summaries() {
        let long = "x".repeat(200);
        let table = issues_table(&[issue("PAY-1", None, &long)]);
        let line = table.lines().nth(1).unwrap();
        assert_eq!(line.chars().count(), MAX_WIDTH);
        assert!(line.ends_with("..."));
    }
}
