use anyhow::{Context, Result};
use serde::Deserialize;

/// `[gitlab]` section of `config.toml`.
#[derive(Debug, Clone, Deserialize)]
pub struct GitlabConfig {
    /// Instance URL, defaults to `https://gitlab.com`.
    #[serde(default = "default_url")]
    pub url: String,
    /// Personal access token with `read_api` scope.
    pub token: String,
}

impl GitlabConfig {
    /// Host of the instance, as found in the remotes of local clones.
    pub fn host(&self) -> Result<String> {
        reqwest::Url::parse(&self.url)
            .ok()
            .and_then(|url| url.host_str().map(str::to_string))
            .with_context(|| format!("cannot read the host of GitLab url {:?}", self.url))
    }
}

fn default_url() -> String {
    "https://gitlab.com".into()
}

#[cfg(test)]
mod tests {
    use crate::config::Config;

    #[test]
    fn parses_gitlab_section_with_default_url() {
        let config = Config::parse("[gitlab]\ntoken = \"glpat-secret\"\n").unwrap();
        let gitlab = config.gitlab.unwrap();
        assert_eq!(gitlab.url, "https://gitlab.com");
        assert_eq!(gitlab.token, "glpat-secret");
    }

    #[test]
    fn rejects_missing_token() {
        let err = Config::parse("[gitlab]\nurl = \"https://gitlab.example.com\"\n").unwrap_err();
        assert!(err.to_string().contains("token"));
    }
}
