//! UI-agnostic domain types shared by all connectors.

use std::path::PathBuf;

use chrono::{DateTime, FixedOffset};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Jira,
    Gitlab,
}

/// Coarse progress of an issue, comparable across trackers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StatusCategory {
    Todo,
    InProgress,
    Done,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Issue {
    pub source: Source,
    pub key: String,
    pub title: String,
    pub status: String,
    pub status_category: StatusCategory,
    pub priority: Option<String>,
    pub project: String,
    pub kind: String,
    /// Plain text, empty when the issue has none.
    pub description: String,
    pub updated: DateTime<FixedOffset>,
    pub url: String,
}

/// Why a merge request is listed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Author,
    Reviewer,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MergeRequest {
    pub source: Source,
    /// `group/project!123`
    pub reference: String,
    pub project: String,
    /// Number of the MR within its project.
    pub iid: u64,
    pub title: String,
    /// Empty when the merge request has none.
    pub description: String,
    pub draft: bool,
    pub merge_status: String,
    /// Status of the latest pipeline (`success`, `failed`, `running`...), None without pipeline.
    pub pipeline: Option<String>,
    pub source_branch: String,
    pub target_branch: String,
    pub author: String,
    pub role: Role,
    pub updated: DateTime<FixedOffset>,
    pub url: String,
    /// Local worktree with the source branch checked out, None when not found
    /// or not looked up.
    pub worktree: Option<PathBuf>,
}
