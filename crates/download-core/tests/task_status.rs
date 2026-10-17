use download_core::task::{DownloadTask, NewTaskRequest, TaskStatus};

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
