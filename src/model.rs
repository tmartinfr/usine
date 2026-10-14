//! UI-agnostic domain types shared by all connectors.

use chrono::{DateTime, FixedOffset};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Jira,
}

#[derive(Debug, Clone, Serialize)]
pub struct Issue {
    pub source: Source,
    pub key: String,
    pub title: String,
    pub status: String,
    pub priority: Option<String>,
    pub project: String,
    pub kind: String,
    pub updated: DateTime<FixedOffset>,
    pub url: String,
}
