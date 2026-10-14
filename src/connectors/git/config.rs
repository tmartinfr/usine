use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::config::expand_home;

/// `[git]` section of `config.toml`.
#[derive(Debug, Clone, Deserialize)]
pub struct GitConfig {
    /// Directories scanned for local clones, e.g. `["~/git"]`.
    pub roots: Vec<String>,
    /// Where worktrees are created, defaults to `~/.local/share/usine/worktrees`.
    pub worktrees_dir: Option<String>,
}

impl GitConfig {
    pub fn root_paths(&self) -> Vec<PathBuf> {
        self.roots.iter().map(|root| expand_home(root)).collect()
    }

    pub fn worktrees_path(&self) -> Result<PathBuf> {
        match &self.worktrees_dir {
            Some(dir) => Ok(expand_home(dir)),
            None => Ok(dirs::data_dir()
                .context("cannot determine the user data directory")?
                .join("usine")
                .join("worktrees")),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::config::Config;

    #[test]
    fn parses_git_section_and_expands_home() {
        let config = Config::parse("[git]\nroots = [\"~/git\", \"/srv/src\"]\n").unwrap();
        let git = config.git.unwrap();
        let home = dirs::home_dir().unwrap();
        assert_eq!(git.root_paths(), [home.join("git"), "/srv/src".into()]);
        assert!(git.worktrees_path().unwrap().ends_with("usine/worktrees"));
    }

    #[test]
    fn custom_worktrees_dir() {
        let config =
            Config::parse("[git]\nroots = []\nworktrees_dir = \"/tmp/wt\"\n").unwrap();
        assert_eq!(config.git.unwrap().worktrees_path().unwrap(), std::path::Path::new("/tmp/wt"));
    }
}
