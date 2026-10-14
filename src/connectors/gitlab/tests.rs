//! HTTP-level tests against a mock GitLab server.

use serde_json::{Value, json};
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::{GitlabClient, GitlabConfig};

fn client(server: &MockServer) -> GitlabClient {
    GitlabClient::new(&GitlabConfig {
        url: format!("{}/", server.uri()),
        token: "glpat-secret".into(),
    })
    .unwrap()
}

fn raw_mr(iid: u32) -> Value {
    json!({
        "iid": iid,
        "title": format!("MR {iid}"),
        "draft": false,
        "detailed_merge_status": "mergeable",
        "source_branch": "feat",
        "target_branch": "main",
        "updated_at": "2026-10-10T12:00:00.000Z",
        "author": { "username": "thomas" },
        "references": { "full": format!("group/project!{iid}") },
        "web_url": format!("https://gitlab.com/group/project/-/merge_requests/{iid}")
    })
}

#[tokio::test]
async fn follows_pagination_with_token_and_filters() {
    let server = MockServer::start().await;
    for (page, body, next) in [
        ("1", json!([raw_mr(1), raw_mr(2)]), "2"),
        ("2", json!([raw_mr(3)]), ""),
    ] {
        Mock::given(method("GET"))
            .and(path("/api/v4/merge_requests"))
            .and(header("private-token", "glpat-secret"))
            .and(query_param("state", "opened"))
            .and(query_param("scope", "created_by_me"))
            .and(query_param("page", page))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("x-next-page", next)
                    .set_body_json(body),
            )
            .expect(1)
            .mount(&server)
            .await;
    }

    let mrs = client(&server).my_open_merge_requests(50).await.unwrap();

    let refs: Vec<_> = mrs.iter().map(|m| m.reference.as_str()).collect();
    assert_eq!(refs, ["group/project!1", "group/project!2", "group/project!3"]);
}

#[tokio::test]
async fn stops_at_limit() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v4/merge_requests"))
        .and(query_param("per_page", "2"))
        .and(query_param("page", "1"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("x-next-page", "2")
                .set_body_json(json!([raw_mr(1), raw_mr(2)])),
        )
        .expect(1)
        .mount(&server)
        .await;

    let mrs = client(&server).my_open_merge_requests(2).await.unwrap();

    assert_eq!(mrs.len(), 2);
}

#[tokio::test]
async fn surfaces_gitlab_error_message() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v4/merge_requests"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({ "message": "401 Unauthorized" })))
        .mount(&server)
        .await;

    let err = client(&server).my_open_merge_requests(10).await.unwrap_err();

    let message = err.to_string();
    assert!(message.contains("401 Unauthorized"), "{message}");
}
