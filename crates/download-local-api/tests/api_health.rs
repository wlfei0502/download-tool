use axum::body::Body;
use axum::http::{Request, StatusCode};
use download_core::manager::TaskManager;
use download_local_api::{app, AppState, DEFAULT_PORT};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tokio::sync::Mutex;
use tower::ServiceExt;

fn state() -> AppState {
    state_with_aria2(true)
}

fn state_with_aria2(ready: bool) -> AppState {
    AppState {
        tasks: Arc::new(Mutex::new(TaskManager::new())),
        aria2_ready: Arc::new(AtomicBool::new(ready)),
    }
}

fn header<'a>(res: &'a axum::response::Response, name: &str) -> &'a str {
    res.headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
}

async fn json_body(res: axum::response::Response) -> Value {
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

#[test]
fn default_port_is_29128() {
    assert_eq!(DEFAULT_PORT, 29128);
}

#[tokio::test]
async fn health_ok() {
    let app = app(state());
    let res = app
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        json_body(res).await,
        json!({"ok": true, "aria2": true, "version": "0.1.0"})
    );
}

#[tokio::test]
async fn health_is_unavailable_when_aria2_is_not_ready() {
    let app = app(state_with_aria2(false));
    let res = app
        .oneshot(
            Request::builder()
                .uri("/health")
                .header("origin", "http://site.example")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        header(&res, "access-control-allow-origin"),
        "http://site.example"
    );
    assert_eq!(header(&res, "access-control-allow-private-network"), "true");
    assert_eq!(
        json_body(res).await,
        json!({"ok": false, "aria2": false, "version": "0.1.0"})
    );
}

#[tokio::test]
async fn cors_preflight_allows_page_origin_and_private_network() {
    let app = app(state());
    for uri in ["/health", "/v1/tasks"] {
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("OPTIONS")
                    .uri(uri)
                    .header("origin", "http://site.example")
                    .header("access-control-request-method", "POST")
                    .header("access-control-request-private-network", "true")
                    .header("access-control-request-headers", "content-type")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::NO_CONTENT, "{uri}");
        assert_eq!(
            header(&res, "access-control-allow-origin"),
            "http://site.example"
        );
        assert_eq!(header(&res, "access-control-allow-private-network"), "true");
        assert!(
            header(&res, "access-control-allow-methods").contains("POST"),
            "{uri}"
        );
        assert!(
            header(&res, "access-control-allow-headers").contains("content-type"),
            "{uri}"
        );
    }

    let open = app
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(header(&open, "access-control-allow-origin"), "*");
    assert_eq!(
        header(&open, "access-control-allow-private-network"),
        "true"
    );
}

#[tokio::test]
async fn post_task_returns_task_id() {
    let app = app(state());
    let body = json!({
        "url": "http://example.com/d/x",
        "fileName": "x.bin",
        "size": 10
    });
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/tasks")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let created = json_body(res).await;
    let task_id = created["taskId"].as_str().expect("taskId string");
    assert!(!task_id.is_empty());

    let listed = app
        .oneshot(
            Request::builder()
                .uri("/v1/tasks")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(listed.status(), StatusCode::OK);
    let tasks = json_body(listed).await;
    let task = tasks
        .as_array()
        .expect("task list")
        .iter()
        .find(|task| task["url"] == "http://example.com/d/x")
        .expect("created task");
    assert_eq!(task["id"], task_id);
    assert_eq!(task["fileName"], "x.bin");
    assert_eq!(task["size"], 10);
    assert_eq!(task["status"], "queued");
}

#[tokio::test]
async fn post_task_maps_camel_case_fields() {
    let app = app(state());
    let body = json!({
        "url": "http://example.com/d/y",
        "fileName": "y.bin",
        "referrer": "http://example.com/files",
        "userAgent": "Mozilla/5.0"
    });
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/tasks")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let listed = app
        .oneshot(
            Request::builder()
                .uri("/v1/tasks")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let tasks = json_body(listed).await;
    let task = tasks
        .as_array()
        .unwrap()
        .iter()
        .find(|task| task["fileName"] == "y.bin")
        .expect("created task");
    assert_eq!(task["referrer"], "http://example.com/files");
    assert_eq!(task["userAgent"], "Mozilla/5.0");
}

#[tokio::test]
async fn post_same_url_returns_existing_task_id() {
    let app = app(state());
    let body = json!({
        "url": "http://example.com/d/same",
        "fileName": "same.bin"
    });
    let first = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/tasks")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    let second = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/tasks")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    let first_id = json_body(first).await["taskId"].clone();
    let second_id = json_body(second).await["taskId"].clone();
    assert_eq!(first_id, second_id);
    assert!(!first_id.as_str().unwrap().is_empty());
}

#[tokio::test]
async fn post_task_rejects_non_http_urls() {
    let app = app(state());
    for url in [
        "https://example.com/d/x",
        "HTTPS://example.com/d/x",
        "ftp://example.com/d/x",
        "FTP://example.com/d/x",
    ] {
        let body = json!({
            "url": url,
            "fileName": "x.bin"
        });
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/tasks")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            json_body(res).await["error"],
            "only http:// URLs are allowed"
        );
    }

    let listed = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/tasks")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(json_body(listed).await.as_array().unwrap().len(), 0);

    let body = json!({
        "url": "HTTP://example.com/d/x",
        "fileName": "x.bin"
    });
    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/tasks")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
}

#[tokio::test]
async fn post_task_rejects_url_userinfo() {
    let app = app(state());
    for url in [
        "http://user:secret@example.com/d/x",
        "HTTP://user@example.com/d/x",
    ] {
        let body = json!({
            "url": url,
            "fileName": "x.bin"
        });
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/tasks")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            json_body(res).await["error"],
            "url must not include user info"
        );
    }

    let listed = app
        .oneshot(
            Request::builder()
                .uri("/v1/tasks")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(json_body(listed).await.as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn post_task_sanitizes_unsafe_file_names() {
    let app = app(state());
    let body = json!({
        "url": "http://example.com/d/report.bin",
        "fileName": "../../secret"
    });
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/tasks")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let listed = app
        .oneshot(
            Request::builder()
                .uri("/v1/tasks")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let tasks = json_body(listed).await;
    let task = tasks
        .as_array()
        .unwrap()
        .iter()
        .find(|task| task["url"] == "http://example.com/d/report.bin")
        .expect("created task");
    assert_eq!(task["fileName"], "report.bin");
}

#[tokio::test]
async fn post_task_rejects_empty_url() {
    let app = app(state());
    let body = json!({
        "url": "",
        "fileName": "x.bin"
    });
    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/tasks")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    assert_eq!(json_body(res).await["error"], "url required");
}
