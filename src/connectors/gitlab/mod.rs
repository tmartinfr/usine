//! GitLab connector (REST API v4).

mod api;
mod client;
mod config;
#[cfg(test)]
mod tests;

pub use client::GitlabClient;
pub use config::GitlabConfig;
