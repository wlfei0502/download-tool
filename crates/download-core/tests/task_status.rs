use download_core::task::{require_http_url, DownloadTask, NewTaskRequest, TaskStatus};

#[test]
fn new_task_starts_queued() {
    let req = NewTaskRequest {
        url: "http://example.com/d/abc".into(),
        file_name: "data.zip".into(),
        size: Some(1000),
        referrer: None,
        user_agent: None,
    };
    let task = DownloadTask::from_request(req);
    assert_eq!(task.status, TaskStatus::Queued);
    assert!(!task.id.0.is_empty());
    assert_eq!(task.file_name, "data.zip");
}

#[test]
fn rejects_urls_with_userinfo() {
    assert_eq!(
        require_http_url("http://user:secret@example.com/d/x").unwrap_err(),
        "url must not include user info"
    );
    assert_eq!(
        require_http_url("HTTP://user@example.com/d/x").unwrap_err(),
        "url must not include user info"
    );
    assert_eq!(
        require_http_url("http://:secret@example.com/d/x").unwrap_err(),
        "url must not include user info"
    );
    assert_eq!(
        require_http_url("https://user:secret@example.com/d/x").unwrap_err(),
        "url must not include user info"
    );
    assert!(require_http_url("http://example.com/user@file").is_ok());
    assert!(require_http_url("http://example.com/d/x").is_ok());
    assert!(require_http_url("https://example.com/d/x").is_ok());
    assert!(require_http_url("HTTPS://example.com/d/x").is_ok());
    assert_eq!(
        require_http_url("ftp://example.com/d/x").unwrap_err(),
        "only http:// and https:// URLs are allowed"
    );
}
