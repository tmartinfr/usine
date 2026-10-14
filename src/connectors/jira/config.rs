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
    }

    #[test]
    fn rejects_incomplete_jira_section() {
        let err = Config::parse("[jira]\nurl = \"https://example.atlassian.net\"\n").unwrap_err();
        assert!(err.to_string().contains("email"));
    }
}
