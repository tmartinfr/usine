//! Serde mirrors of Jira REST responses, and their mapping to domain types.

use anyhow::{Context, Result};
use chrono::DateTime;
use serde::Deserialize;
use serde_json::Value;

use crate::model::{Issue, Source, StatusCategory};

/// Fields requested from `/rest/api/3/search/jql`; must match `Fields` below.
pub const FIELDS: &str = "summary,status,priority,project,issuetype,description,updated";

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
    /// Atlassian Document Format, null when empty.
    pub description: Option<Value>,
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
            description: self.fields.description.as_ref().map(document_text).unwrap_or_default(),
            updated,
        })
    }
}

/// Plain text of an Atlassian Document Format document: one paragraph per
/// block, separated by blank lines, `- ` before list items.
// shortcut: nested lists lose their indentation and tables are flattened to
// one cell per line, render them if descriptions need it.
fn document_text(doc: &Value) -> String {
    let blocks = doc["content"].as_array().map(Vec::as_slice).unwrap_or_default();
    let blocks: Vec<String> = blocks
        .iter()
        .map(|block| {
            let mut text = String::new();
            node_text(block, &mut text);
            text.trim_end().to_string()
        })
        .filter(|text| !text.is_empty())
        .collect();
    blocks.join("\n\n")
}

fn node_text(node: &Value, out: &mut String) {
    let attr = |name: &str| node["attrs"][name].as_str().unwrap_or_default();
    match node["type"].as_str().unwrap_or_default() {
        "text" => out.push_str(node["text"].as_str().unwrap_or_default()),
        "hardBreak" => out.push('\n'),
        "mention" | "emoji" | "status" => out.push_str(attr("text")),
        "inlineCard" | "blockCard" | "embedCard" => out.push_str(attr("url")),
        "rule" => out.push_str("---"),
        "listItem" => out.push_str("- "),
        "heading" => {
            let level = node["attrs"]["level"].as_u64().unwrap_or(1) as usize;
            out.push_str(&format!("{} ", "#".repeat(level)));
        }
        _ => {}
    }
    for child in node["content"].as_array().into_iter().flatten() {
        node_text(child, out);
    }
    let block = ["paragraph", "heading", "codeBlock", "listItem", "tableCell", "tableHeader"];
    if block.contains(&node["type"].as_str().unwrap_or_default()) && !out.ends_with('\n') {
        out.push('\n');
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
        assert_eq!(
            first.description,
            "# Context\n\nRetries never stop when the bank answers 500.\nSee @Jane Doe.\n\n- Stop after 3 retries\n- Log https://example.com/runbook"
        );
        assert_eq!(issues[1].description, "");

        assert_eq!(issues[1].priority, None);
        assert_eq!(issues[1].status_category, StatusCategory::Todo);
    }
}
