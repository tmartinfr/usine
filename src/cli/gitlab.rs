use std::path::Path;

use anyhow::{Context, Result};
use clap::Subcommand;
use usine::config::Config;
use usine::connectors::gitlab::GitlabClient;

#[derive(Debug, Subcommand)]
pub enum GitlabCommand {
    /// List open merge requests authored by me
    Mrs {
        /// Maximum number of merge requests to fetch
        #[arg(long, default_value_t = 50)]
        limit: usize,
        /// Print JSON instead of a table
        #[arg(long)]
        json: bool,
    },
}

pub async fn run(command: GitlabCommand, config: &Config, config_path: &Path) -> Result<()> {
    let gitlab = config
        .gitlab
        .as_ref()
        .with_context(|| format!("no [gitlab] section in {}", config_path.display()))?;
    let client = GitlabClient::new(gitlab)?;

    match command {
        GitlabCommand::Mrs { limit, json } => {
            let mrs = client.my_open_merge_requests(limit).await?;
            super::print_merge_requests(&mrs, json)
        }
    }
}
