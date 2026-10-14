use std::path::Path;

use anyhow::{Context, Result};
use clap::Subcommand;
use usine::config::Config;
use usine::connectors::jira::{DEFAULT_JQL, JiraClient};

#[derive(Debug, Subcommand)]
pub enum JiraCommand {
    /// List issues (default: unresolved issues assigned to me)
    Issues {
        /// JQL query, overrides `jira.jql` from the config file
        #[arg(long)]
        jql: Option<String>,
        /// Maximum number of issues to fetch
        #[arg(long, default_value_t = 50)]
        limit: usize,
        /// Print JSON instead of a table
        #[arg(long)]
        json: bool,
    },
}

pub async fn run(command: JiraCommand, config: &Config, config_path: &Path) -> Result<()> {
    let jira = config
        .jira
        .as_ref()
        .with_context(|| format!("no [jira] section in {}", config_path.display()))?;
    let client = JiraClient::new(jira)?;

    match command {
        JiraCommand::Issues { jql, limit, json } => {
            let jql = jql.as_deref().or(jira.jql.as_deref()).unwrap_or(DEFAULT_JQL);
            let issues = client.search(jql, limit).await?;
            super::print_issues(&issues, json)
        }
    }
}
