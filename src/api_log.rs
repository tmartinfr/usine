//! Record of the HTTP calls made to the connectors' APIs.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::{DateTime, Local};

/// Number of calls kept; older ones are dropped.
const CAPACITY: usize = 500;

#[derive(Debug, Clone, PartialEq)]
pub struct ApiCall {
    pub at: DateTime<Local>,
    pub method: String,
    /// With its query string. Credentials are sent in headers, never here.
    pub url: String,
    /// HTTP status, or why no response was received.
    pub outcome: Result<u16, String>,
    pub duration: Duration,
}

/// The latest calls, oldest first. Clones share the same record.
#[derive(Debug, Clone, Default)]
pub struct ApiLog(Arc<Mutex<VecDeque<ApiCall>>>);

impl ApiLog {
    pub fn record(&self, call: ApiCall) {
        let mut calls = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if calls.len() == CAPACITY {
            calls.pop_front();
        }
        calls.push_back(call);
    }

    pub fn calls(&self) -> Vec<ApiCall> {
        let calls = self.0.lock().unwrap_or_else(|e| e.into_inner());
        calls.iter().cloned().collect()
    }

    /// Sends `request` and records it.
    pub async fn send(&self, request: reqwest::RequestBuilder) -> reqwest::Result<reqwest::Response> {
        let (client, request) = request.build_split();
        let request = request?;
        let (method, url) = (request.method().to_string(), request.url().to_string());
        let (at, start) = (Local::now(), Instant::now());
        let result = client.execute(request).await;
        let outcome = match &result {
            Ok(response) => Ok(response.status().as_u16()),
            Err(err) => Err(err.to_string()),
        };
        self.record(ApiCall { at, method, url, outcome, duration: start.elapsed() });
        result
    }
}

#[cfg(test)]
mod tests {
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    fn call(url: &str) -> ApiCall {
        ApiCall {
            at: Local::now(),
            method: "GET".into(),
            url: url.into(),
            outcome: Ok(200),
            duration: Duration::ZERO,
        }
    }

    #[test]
    fn keeps_the_latest_calls() {
        let log = ApiLog::default();
        for i in 0..CAPACITY + 2 {
            log.clone().record(call(&i.to_string()));
        }
        let calls = log.calls();
        assert_eq!(calls.len(), CAPACITY);
        assert_eq!(calls[0].url, "2");
        assert_eq!(calls[CAPACITY - 1].url, (CAPACITY + 1).to_string());
    }

    #[tokio::test]
    async fn records_sent_requests_and_failures() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        let log = ApiLog::default();
        let http = reqwest::Client::new();

        let request = http.get(format!("{}/api", server.uri())).query(&[("q", "a b")]);
        let response = log.send(request).await.unwrap();
        assert_eq!(response.status(), 404);
        // Nothing listens on port 1.
        assert!(log.send(http.get("http://127.0.0.1:1/down")).await.is_err());

        let calls = log.calls();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].method, "GET");
        assert_eq!(calls[0].url, format!("{}/api?q=a+b", server.uri()));
        assert_eq!(calls[0].outcome, Ok(404));
        assert_eq!(calls[1].url, "http://127.0.0.1:1/down");
        assert!(calls[1].outcome.is_err());
    }
}
