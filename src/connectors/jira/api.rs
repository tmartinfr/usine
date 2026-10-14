//! Serde mirrors of Jira REST responses, and their mapping to domain types.

use anyhow::{Context, Result};
use chrono::DateTime;
use serde::Deserialize;

use crate::model::{Issue, Source, StatusCategory};

/// Fields requested from `/rest/api/3/search/jql`; must match `Fields` below.
pub const FIELDS: &str = "summary,status,priority,project,issuetype,updated";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResponse {
    pub issues: Vec<RawIssue>,
    pub next_page_token: Option<String>,
    #[serde(default)]
    pub is_last: bool,
}

#[derive(Debug, Deserialize)]
pub struct RawIssue {
    pub key: String,
    pub fields: Fields,
}

#[derive(Debug, Deserialize)]
pub struct Fields {
    pub summary: String,
    pub status: Status,
    pub priority: Option<Named>,
    pub project: Project,
    pub issuetype: Named,
    pub updated: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub name: String,
    pub status_category: Option<CategoryKey>,
}

#[derive(Debug, Deserialize)]
pub struct CategoryKey {
    /// `new`, `indeterminate` or `done`.
    pub key: String,
}

#[derive(Debug, Deserialize)]
pub struct Named {
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct Project {
    pub key: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorResponse {
    #[serde(default)]
    pub error_messages: Vec<String>,
}

impl RawIssue {
    pub fn into_issue(self, base_url: &str) -> Result<Issue> {
        // Jira timestamps look like `2026-10-10T12:34:56.789+0200`.
        let updated = DateTime::parse_from_str(&self.fields.updated, "%Y-%m-%dT%H:%M:%S%.f%z")
            .with_context(|| format!("{}: invalid updated date {:?}", self.key, self.fields.updated))?;
        Ok(Issue {
            source: Source::Jira,
            url: format!("{base_url}/browse/{}", self.key),
            key: self.key,
            title: self.fields.summary,
            status_category: match self.fields.status.status_category.as_ref().map(|c| c.key.as_str()) {
                Some("indeterminate") => StatusCategory::InProgress,
                Some("done") => StatusCategory::Done,
                _ => StatusCategory::Todo,
            },
            status: self.fields.status.name,
            priority: self.fields.priority.map(|p| p.name),
            project: self.fields.project.key,
            kind: self.fields.issuetype.name,
            updated,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("fixtures/search.json");

    #[test]
    fn maps_fixture_to_issues() {
        let response: SearchResponse = serde_json::from_str(FIXTURE).unwrap();
        assert!(response.is_last);
        let issues: Vec<Issue> = response
            .issues
            .into_iter()
            .map(|raw| raw.into_issue("https://example.atlassian.net"))
            .collect::<Result<_>>()
            .unwrap();

        assert_eq!(issues.len(), 2);
        let first = &issues[0];
        assert_eq!(first.source, Source::Jira);
        assert_eq!(first.key, "PAY-123");
        assert_eq!(first.title, "Fix the payment retry loop");
        assert_eq!(first.status, "In Progress");
        assert_eq!(first.status_category, StatusCategory::InProgress);
        assert_eq!(first.priority.as_deref(), Some("High"));
        assert_eq!(first.project, "PAY");
        assert_eq!(first.kind, "Bug");
        assert_eq!(first.url, "https://example.atlassian.net/browse/PAY-123");
        assert_eq!(first.updated.to_rfc3339(), "2026-10-10T12:34:56.789+02:00");

        assert_eq!(issues[1].priority, None);
        assert_eq!(issues[1].status_category, StatusCategory::Todo);
    }
}
