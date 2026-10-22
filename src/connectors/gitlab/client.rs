use anyhow::{Context, Result, bail};
use tokio::task::JoinSet;

use super::GitlabConfig;
use crate::api_log::ApiLog;
use super::api::{ErrorResponse, RawMergeRequest, RawMergeRequestDetail, RawUser};
use crate::model::{MergeRequest, Role};

/// GitLab caps `per_page` at 100.
const MAX_PAGE_SIZE: usize = 100;

#[derive(Clone)]
pub struct GitlabClient {
    http: reqwest::Client,
    base_url: String,
    token: String,
    log: ApiLog,
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
            log: ApiLog::default(),
        })
    }

    /// Records every call in `log`.
    pub fn with_log(mut self, log: ApiLog) -> Self {
        self.log = log;
        self
    }

    /// Returns at most `limit` open MRs authored by the token owner, most recently updated first.
    pub async fn my_open_merge_requests(&self, limit: usize) -> Result<Vec<MergeRequest>> {
        let raw = self.open_merge_requests(&[("scope", "created_by_me")], limit).await?;
        self.with_pipelines(raw, Role::Author).await
    }

    /// Returns at most `limit` open MRs the token owner is a reviewer of and has
    /// not approved, most recently updated first.
    pub async fn merge_requests_to_review(&self, limit: usize) -> Result<Vec<MergeRequest>> {
        let username = self.current_username().await?;
        let to_review = [("scope", "reviews_for_me")];
        // The API ignores `not[approved_by_usernames]`: approved MRs are listed
        // separately and removed.
        let approved = [to_review[0], ("approved_by_usernames[]", &username)];
        let (raw, approved) = tokio::try_join!(
            self.open_merge_requests(&to_review, usize::MAX),
            self.open_merge_requests(&approved, usize::MAX),
        )?;
        let mut raw: Vec<RawMergeRequest> = raw
            .into_iter()
            .filter(|mr| !approved.iter().any(|a| (a.project_id, a.iid) == (mr.project_id, mr.iid)))
            .collect();
        raw.truncate(limit);
        self.with_pipelines(raw, Role::Reviewer).await
    }

    async fn current_username(&self) -> Result<String> {
        let url = format!("{}/api/v4/user", self.base_url);
        let user: RawUser = self
            .get(&url, &[], "user")
            .await?
            .json()
            .await
            .context("cannot decode GitLab user response")?;
        Ok(user.username)
    }

    /// At most `limit` open MRs matching `filters`, most recently updated first.
    async fn open_merge_requests(
        &self,
        filters: &[(&str, &str)],
        limit: usize,
    ) -> Result<Vec<RawMergeRequest>> {
        let url = format!("{}/api/v4/merge_requests", self.base_url);
        let per_page = limit.clamp(1, MAX_PAGE_SIZE).to_string();
        let mut raw = Vec::new();
        let mut page = "1".to_string();

        while raw.len() < limit {
            let mut query = vec![
                ("state", "opened"),
                ("order_by", "updated_at"),
                ("sort", "desc"),
                ("per_page", &per_page),
                ("page", &page),
            ];
            query.extend_from_slice(filters);
            let response = self.get(&url, &query, "merge requests").await?;

            // Empty or missing on the last page.
            let next_page = response
                .headers()
                .get("x-next-page")
                .and_then(|v| v.to_str().ok())
                .filter(|v| !v.is_empty())
                .map(str::to_string);

            let mrs: Vec<RawMergeRequest> = response
                .json()
                .await
                .context("cannot decode GitLab merge requests response")?;
            raw.extend(mrs);

            match next_page {
                Some(next) => page = next,
                None => break,
            }
        }

        raw.truncate(limit);
        Ok(raw)
    }

    async fn with_pipelines(&self, raw: Vec<RawMergeRequest>, role: Role) -> Result<Vec<MergeRequest>> {
        let pipelines = self.head_pipelines(&raw).await?;
        Ok(raw
            .into_iter()
            .zip(pipelines)
            .map(|(mr, pipeline)| mr.into_merge_request(pipeline, role))
            .collect())
    }

    /// Status of the latest pipeline of each MR, fetched concurrently: the
    /// list endpoint does not include it.
    async fn head_pipelines(&self, mrs: &[RawMergeRequest]) -> Result<Vec<Option<String>>> {
        let mut tasks = JoinSet::new();
        for (index, mr) in mrs.iter().enumerate() {
            let client = self.clone();
            let url = format!(
                "{}/api/v4/projects/{}/merge_requests/{}",
                self.base_url, mr.project_id, mr.iid
            );
            tasks.spawn(async move {
                let response = client.get(&url, &[], "pipeline").await?;
                let detail: RawMergeRequestDetail = response
                    .json()
                    .await
                    .context("cannot decode GitLab merge request response")?;
                anyhow::Ok((index, detail.head_pipeline.map(|p| p.status)))
            });
        }
        let mut pipelines = vec![None; mrs.len()];
        while let Some(result) = tasks.join_next().await {
            let (index, pipeline) = result.context("pipeline lookup aborted")??;
            pipelines[index] = pipeline;
        }
        Ok(pipelines)
    }

    /// Sends an authenticated GET; non-2xx responses become errors with GitLab's message.
    async fn get(&self, url: &str, query: &[(&str, &str)], what: &str) -> Result<reqwest::Response> {
        let request = self.http.get(url).header("PRIVATE-TOKEN", &self.token).query(query);
        let response = self
            .log
            .send(request)
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
            bail!("GitLab {what} request failed ({status}): {detail}");
        }
        Ok(response)
    }
}
