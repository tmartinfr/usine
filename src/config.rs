//! Loads `config.toml`, which holds one section per connector.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::connectors::claude::ClaudeConfig;
use crate::connectors::git::GitConfig;
use crate::connectors::gitlab::GitlabConfig;
use crate::connectors::jira::JiraConfig;

#[derive(Debug, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub claude: ClaudeConfig,
    pub git: Option<GitConfig>,
    pub gitlab: Option<GitlabConfig>,
    pub jira: Option<JiraConfig>,
}

/// Expands a leading `~/` to the home directory.
pub fn expand_home(path: &str) -> PathBuf {
    match (path.strip_prefix("~/"), dirs::home_dir()) {
        (Some(rest), Some(home)) => home.join(rest),
        _ => PathBuf::from(path),
    }
}

/// `path` with a leading home directory shown as `~`, `-` for None.
pub fn display_path(path: Option<&Path>) -> String {
    let Some(path) = path else { return "-".into() };
    match dirs::home_dir().and_then(|home| path.strip_prefix(home).ok().map(Path::to_path_buf)) {
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

impl Config {
    /// `~/.config/usine/config.toml` on Linux.
    pub fn default_path() -> Result<PathBuf> {
        let dir = dirs::config_dir().context("cannot determine the user config directory")?;
        Ok(dir.join("usine").join("config.toml"))
    }

    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("cannot read config file {}", path.display()))?;
        Self::parse(&content).with_context(|| format!("invalid config file {}", path.display()))
    }

    pub fn parse(content: &str) -> Result<Self> {
        Ok(toml::from_str(content)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_config_has_no_jira_section() {
        let config = Config::parse("").unwrap();
        assert!(config.jira.is_none());
    }

    #[test]
    fn load_reports_missing_file_path() {
        let err = Config::load(Path::new("/nonexistent/usine.toml")).unwrap_err();
        assert!(format!("{err:#}").contains("/nonexistent/usine.toml"));
    }
}
