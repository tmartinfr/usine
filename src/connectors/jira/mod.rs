//! Jira Cloud connector (REST API v3).

mod api;
mod client;
mod config;
#[cfg(test)]
mod tests;

pub use client::JiraClient;
pub use config::JiraConfig;

/// Unresolved issues assigned to the authenticated user, most recently updated first.
pub const DEFAULT_JQL: &str =
    "assignee = currentUser() AND statusCategory != Done ORDER BY updated DESC";
