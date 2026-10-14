//! HTTP-level tests against a mock Jira server.

use serde_json::{Value, json};
use wiremock::matchers::{header, method, path, query_param, query_param_is_missing};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::{JiraClient, JiraConfig};

fn client(server: &MockServer) -> JiraClient {
    JiraClient::new(&JiraConfig {
        url: format!("{}/", server.uri()),
        email: "me@example.com".into(),
        api_token: "secret".into(),
        jql: None,
    })
    .unwrap()
}

fn raw_issue(key: &str) -> Value {
    json!({
        "key": key,
        "fields": {
            "summary": format!("Summary of {key}"),
            "status": { "name": "To Do" },
            "priority": { "name": "Medium" },
            "project": { "key": "PAY" },
            "issuetype": { "name": "Task" },
            "updated": "2026-10-10T12:00:00.000+0000"
        }
    })
}

// base64("me@example.com:secret")
const AUTH: &str = "Basic bWVAZXhhbXBsZS5jb206c2VjcmV0";

#[tokio::test]
async fn follows_pagination_with_auth_and_jql() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/api/3/search/jql"))
        .and(header("authorization", AUTH))
        .and(query_param("jql", "project = PAY"))
        .and(query_param_is_missing("nextPageToken"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "issues": [raw_issue("PAY-1"), raw_issue("PAY-2")],
            "nextPageToken": "page2",
            "isLast": false
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/rest/api/3/search/jql"))
        .and(header("authorization", AUTH))
        .and(query_param("nextPageToken", "page2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "issues": [raw_issue("PAY-3")],
            "isLast": true
        })))
        .expect(1)
        .mount(&server)
        .await;

    let issues = client(&server).search("project = PAY", 50).await.unwrap();

    let keys: Vec<_> = issues.iter().map(|i| i.key.as_str()).collect();
    assert_eq!(keys, ["PAY-1", "PAY-2", "PAY-3"]);
    assert_eq!(issues[0].url, format!("{}/browse/PAY-1", server.uri()));
}

#[tokio::test]
async fn stops_at_limit() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/api/3/search/jql"))
        .and(query_param("maxResults", "2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "issues": [raw_issue("PAY-1"), raw_issue("PAY-2")],
            "nextPageToken": "page2",
            "isLast": false
        })))
        .expect(1)
        .mount(&server)
        .await;

    let issues = client(&server).search("project = PAY", 2).await.unwrap();

    assert_eq!(issues.len(), 2);
}

#[tokio::test]
async fn surfaces_jira_error_messages() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/api/3/search/jql"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "errorMessages": ["Error in the JQL Query: bad token"],
            "errors": {}
        })))
        .mount(&server)
        .await;

    let err = client(&server).search("bad jql", 10).await.unwrap_err();

    let message = err.to_string();
    assert!(message.contains("400"), "{message}");
    assert!(message.contains("Error in the JQL Query: bad token"), "{message}");
}

#[tokio::test]
async fn surfaces_raw_body_when_not_json() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/api/3/search/jql"))
        .respond_with(ResponseTemplate::new(401).set_body_string("Client must be authenticated"))
        .mount(&server)
        .await;

    let err = client(&server).search("project = PAY", 10).await.unwrap_err();

    let message = err.to_string();
    assert!(message.contains("401"), "{message}");
    assert!(message.contains("Client must be authenticated"), "{message}");
}
