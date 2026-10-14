use anyhow::{Context, Result, bail};

use super::GitlabConfig;
use super::api::{ErrorResponse, RawMergeRequest};
use crate::model::MergeRequest;

/// GitLab caps `per_page` at 100.
const MAX_PAGE_SIZE: usize = 100;

#[derive(Clone)]
pub struct GitlabClient {
    http: reqwest::Client,
    base_url: String,
    token: String,
}

impl GitlabClient {
    pub fn new(config: &GitlabConfig) -> Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent(concat!("usine/", env!("CARGO_PKG_VERSION")))
            .build()
            .context("cannot build HTTP client")?;
        Ok(Self {
            http,
            base_url: config.url.trim_end_matches('/').to_string(),
            token: config.token.clone(),
        })
    }

    /// Returns at most `limit` open MRs authored by the token owner, most recently updated first.
    pub async fn my_open_merge_requests(&self, limit: usize) -> Result<Vec<MergeRequest>> {
        let url = format!("{}/api/v4/merge_requests", self.base_url);
        let per_page = limit.clamp(1, MAX_PAGE_SIZE).to_string();
        let mut mrs = Vec::new();
        let mut page = "1".to_string();

        while mrs.len() < limit {
            let response = self
                .http
                .get(&url)
                .header("PRIVATE-TOKEN", &self.token)
                .query(&[
                    ("state", "opened"),
                    ("scope", "created_by_me"),
                    ("order_by", "updated_at"),
                    ("sort", "desc"),
                    ("per_page", &per_page),
                    ("page", &page),
                ])
                .send()
                .await
                .with_context(|| format!("request to {url} failed"))?;

            let status = response.status();
            if !status.is_success() {
                let body = response.text().await.unwrap_or_default();
                let detail = serde_json::from_str::<ErrorResponse>(&body)
                    .ok()
                    .and_then(|e| {
                        e.error.or(e.message.map(|m| match m {
                            serde_json::Value::String(s) => s,
                            other => other.to_string(),
                        }))
                    })
                    .unwrap_or(body);
                bail!("GitLab merge requests request failed ({status}): {detail}");
            }

            // Empty or missing on the last page.
            let next_page = response
                .headers()
                .get("x-next-page")
                .and_then(|v| v.to_str().ok())
                .filter(|v| !v.is_empty())
                .map(str::to_string);

            let raw: Vec<RawMergeRequest> = response
                .json()
                .await
                .context("cannot decode GitLab merge requests response")?;
            mrs.extend(raw.into_iter().map(RawMergeRequest::into_merge_request));

            match next_page {
                Some(next) => page = next,
                None => break,
            }
        }

        mrs.truncate(limit);
        Ok(mrs)
    }
}
