//! HTTP-level tests against a mock GitLab server.

use serde_json::{Value, json};
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::{GitlabClient, GitlabConfig};
use crate::model::Role;

fn client(server: &MockServer) -> GitlabClient {
    GitlabClient::new(&GitlabConfig {
        url: format!("{}/", server.uri()),
        token: "glpat-secret".into(),
    })
    .unwrap()
}

fn raw_mr(iid: u32) -> Value {
    json!({
        "project_id": 100,
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

/// Answers the detail of MRs 1 to `count` of project 100, with a pipeline
/// for odd numbers only.
async fn mount_details(server: &MockServer, count: u32) {
    for iid in 1..=count {
        let pipeline = (iid % 2 == 1).then(|| json!({ "id": iid, "status": "success" }));
        Mock::given(method("GET"))
            .and(path(format!("/api/v4/projects/100/merge_requests/{iid}")))
            .and(header("private-token", "glpat-secret"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "iid": iid, "head_pipeline": pipeline })))
            .expect(1)
            .mount(server)
            .await;
    }
}

#[tokio::test]
async fn follows_pagination_with_token_and_filters() {
    let server = MockServer::start().await;
    mount_details(&server, 3).await;
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
    let pipelines: Vec<_> = mrs.iter().map(|m| m.pipeline.as_deref()).collect();
    assert_eq!(pipelines, [Some("success"), None, Some("success")]);
}

#[tokio::test]
async fn lists_merge_requests_to_review_except_approved_ones() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v4/user"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "id": 5, "username": "me" })))
        .expect(1)
        .mount(&server)
        .await;
    // Most specific first: wiremock uses the first mock that matches.
    Mock::given(method("GET"))
        .and(path("/api/v4/merge_requests"))
        .and(query_param("scope", "reviews_for_me"))
        .and(query_param("approved_by_usernames[]", "me"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([raw_mr(2)])))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v4/merge_requests"))
        .and(query_param("state", "opened"))
        .and(query_param("scope", "reviews_for_me"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([raw_mr(1), raw_mr(2), raw_mr(3)])))
        .expect(1)
        .mount(&server)
        .await;
    // No pipeline lookup for the approved MR.
    for iid in [1, 3] {
        Mock::given(method("GET"))
            .and(path(format!("/api/v4/projects/100/merge_requests/{iid}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "iid": iid, "head_pipeline": null })))
            .expect(1)
            .mount(&server)
            .await;
    }

    let mrs = client(&server).merge_requests_to_review(50).await.unwrap();

    let iids: Vec<u64> = mrs.iter().map(|m| m.iid).collect();
    assert_eq!(iids, [1, 3]);
    assert!(mrs.iter().all(|m| m.role == Role::Reviewer));
}

#[tokio::test]
async fn stops_at_limit() {
    let server = MockServer::start().await;
    mount_details(&server, 2).await;
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

#[tokio::test]
async fn fails_when_a_pipeline_lookup_fails() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v4/merge_requests"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([raw_mr(1)])))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v4/projects/100/merge_requests/1"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({ "message": "404 Not found" })))
        .mount(&server)
        .await;

    let err = client(&server).my_open_merge_requests(10).await.unwrap_err();

    let message = format!("{err:#}");
    assert!(message.contains("GitLab pipeline request failed (404 Not Found): 404 Not found"), "{message}");
}
