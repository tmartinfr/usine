use std::collections::HashMap;

use serde::Deserialize;

/// `[jira]` section of `config.toml`.
#[derive(Debug, Clone, Deserialize)]
pub struct JiraConfig {
    /// Site URL, e.g. `https://example.atlassian.net`.
    pub url: String,
    pub email: String,
    pub api_token: String,
    /// Overrides `DEFAULT_JQL` when set.
    pub jql: Option<String>,
    /// Color of a status in the terminal interface, by status name, e.g.
    /// `"Dev Done" = "green"`. Other statuses are colored by category.
    #[serde(default)]
    pub status_colors: HashMap<String, String>,
}

impl JiraConfig {
    /// The configured `jql`, or `DEFAULT_JQL`.
    pub fn effective_jql(&self) -> &str {
        self.jql.as_deref().unwrap_or(super::DEFAULT_JQL)
    }
}

#[cfg(test)]
mod tests {
    use crate::config::Config;

    #[test]
    fn parses_jira_section() {
        let config = Config::parse(
            r#"
            [jira]
            url = "https://example.atlassian.net"
            email = "me@example.com"
            api_token = "secret"
            "#,
        )
        .unwrap();
        let jira = config.jira.unwrap();
        assert_eq!(jira.url, "https://example.atlassian.net");
        assert_eq!(jira.email, "me@example.com");
        assert_eq!(jira.api_token, "secret");
        assert!(jira.jql.is_none());
        assert!(jira.status_colors.is_empty());
        assert_eq!(jira.effective_jql(), crate::connectors::jira::DEFAULT_JQL);
    }

    #[test]
    fn configured_jql_overrides_default() {
        let config = Config::parse(
            r#"
            [jira]
            url = "https://example.atlassian.net"
            email = "me@example.com"
            api_token = "secret"
            jql = "project = PAY"
            status_colors = { "Dev Done" = "green" }
            "#,
        )
        .unwrap();
        let jira = config.jira.unwrap();
        assert_eq!(jira.effective_jql(), "project = PAY");
        assert_eq!(jira.status_colors["Dev Done"], "green");
    }

    #[test]
    fn rejects_incomplete_jira_section() {
        let err = Config::parse("[jira]\nurl = \"https://example.atlassian.net\"\n").unwrap_err();
        assert!(err.to_string().contains("email"));
    }
}
