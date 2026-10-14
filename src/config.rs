//! Loads `config.toml`, which holds one section per connector.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::connectors::jira::JiraConfig;

#[derive(Debug, Deserialize)]
pub struct Config {
    pub jira: Option<JiraConfig>,
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
