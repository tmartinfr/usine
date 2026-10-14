//! Serde mirrors of GitLab REST responses, and their mapping to domain types.

use chrono::{DateTime, FixedOffset};
use serde::Deserialize;

use crate::model::{MergeRequest, Source};

#[derive(Debug, Deserialize)]
pub struct RawMergeRequest {
    pub title: String,
    pub web_url: String,
    pub draft: bool,
    pub detailed_merge_status: String,
    pub source_branch: String,
    pub target_branch: String,
    pub updated_at: DateTime<FixedOffset>,
    pub author: Author,
    pub references: References,
}

#[derive(Debug, Deserialize)]
pub struct Author {
    pub username: String,
}

#[derive(Debug, Deserialize)]
pub struct References {
    /// `group/project!123`
    pub full: String,
}

#[derive(Debug, Deserialize)]
pub struct ErrorResponse {
    pub message: Option<serde_json::Value>,
    pub error: Option<String>,
}

impl RawMergeRequest {
    pub fn into_merge_request(self) -> MergeRequest {
        let project = match self.references.full.rsplit_once('!') {
            Some((project, _)) => project.to_string(),
            None => self.references.full.clone(),
        };
        MergeRequest {
            source: Source::Gitlab,
            reference: self.references.full,
            project,
            title: self.title,
            draft: self.draft,
            merge_status: self.detailed_merge_status,
            source_branch: self.source_branch,
            target_branch: self.target_branch,
            author: self.author.username,
            updated: self.updated_at,
            url: self.web_url,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("fixtures/merge_requests.json");

    #[test]
    fn maps_fixture_to_merge_requests() {
        let raw: Vec<RawMergeRequest> = serde_json::from_str(FIXTURE).unwrap();
        let mrs: Vec<MergeRequest> = raw.into_iter().map(RawMergeRequest::into_merge_request).collect();

        assert_eq!(mrs.len(), 2);
        let first = &mrs[0];
        assert_eq!(first.source, Source::Gitlab);
        assert_eq!(first.reference, "paylead/backend/api!42");
        assert_eq!(first.project, "paylead/backend/api");
        assert_eq!(first.title, "Add retry on payment webhook");
        assert!(!first.draft);
        assert_eq!(first.merge_status, "not_approved");
        assert_eq!(first.source_branch, "feat/webhook-retry");
        assert_eq!(first.target_branch, "main");
        assert_eq!(first.author, "thomas");
        assert_eq!(first.updated.to_rfc3339(), "2026-10-10T12:34:56.789+00:00");
        assert_eq!(first.url, "https://gitlab.com/paylead/backend/api/-/merge_requests/42");

        assert!(mrs[1].draft);
    }
}
