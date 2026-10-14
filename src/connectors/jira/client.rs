use anyhow::{Context, Result, bail};

use super::JiraConfig;
use super::api::{ErrorResponse, FIELDS, SearchResponse};
use crate::model::Issue;

/// Jira Cloud caps `maxResults` at 100 for `/search/jql`.
const MAX_PAGE_SIZE: usize = 100;

pub struct JiraClient {
    http: reqwest::Client,
    base_url: String,
    email: String,
    api_token: String,
}

impl JiraClient {
    pub fn new(config: &JiraConfig) -> Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent(concat!("usine/", env!("CARGO_PKG_VERSION")))
            .build()
            .context("cannot build HTTP client")?;
        Ok(Self {
            http,
            base_url: config.url.trim_end_matches('/').to_string(),
            email: config.email.clone(),
            api_token: config.api_token.clone(),
        })
    }

    /// Returns at most `limit` issues matching `jql`, following pagination.
    pub async fn search(&self, jql: &str, limit: usize) -> Result<Vec<Issue>> {
        let url = format!("{}/rest/api/3/search/jql", self.base_url);
        let mut issues = Vec::new();
        let mut next_page_token: Option<String> = None;

        while issues.len() < limit {
            let page_size = (limit - issues.len()).min(MAX_PAGE_SIZE).to_string();
            let mut query = vec![("jql", jql), ("fields", FIELDS), ("maxResults", &page_size)];
            if let Some(token) = &next_page_token {
                query.push(("nextPageToken", token));
            }

            let response = self
                .http
                .get(&url)
                .basic_auth(&self.email, Some(&self.api_token))
                .query(&query)
                .send()
                .await
                .with_context(|| format!("request to {url} failed"))?;

            let status = response.status();
            if !status.is_success() {
                let body = response.text().await.unwrap_or_default();
                let detail = serde_json::from_str::<ErrorResponse>(&body)
                    .ok()
                    .filter(|e| !e.error_messages.is_empty())
                    .map(|e| e.error_messages.join("; "))
                    .unwrap_or(body);
                bail!("Jira search failed ({status}): {detail}");
            }

            let page: SearchResponse = response
                .json()
                .await
                .context("cannot decode Jira search response")?;
            for raw in page.issues {
                issues.push(raw.into_issue(&self.base_url)?);
            }

            match page.next_page_token {
                Some(token) if !page.is_last => next_page_token = Some(token),
                _ => break,
            }
        }

        issues.truncate(limit);
        Ok(issues)
    }
}
