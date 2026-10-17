use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use download_core::engine::{Aria2Api, DownloadEngine};
use download_core::manager::TaskManager;
use download_core::task::{DownloadTask, NewTaskRequest, TaskId, TaskStatus};
use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq, Eq)]
enum Call {
    AddUri {
        url: String,
        dir: String,
        out: String,
    },
    Pause {
        gid: String,
    },
    Unpause {
        gid: String,
    },
    Remove {
        gid: String,
    },
    TellStatus {
        gid: String,
    },
}

#[derive(Clone, Default)]
struct FakeAria2 {
    inner: Arc<Mutex<FakeInner>>,
}

#[derive(Default)]
struct FakeInner {
    calls: Vec<Call>,
    next_gid: u64,
    statuses: HashMap<String, Value>,
    fail_add: Option<String>,
    fail_status: HashMap<String, String>,
}

impl FakeAria2 {
    fn calls(&self) -> Vec<Call> {
        self.inner.lock().expect("fake lock").calls.clone()
    }

    fn fail_next_add(&self, message: &str) {
        self.inner.lock().expect("fake lock").fail_add = Some(message.to_string());
    }

    fn set_status(&self, gid: &str, status: Value) {
        self.inner
            .lock()
            .expect("fake lock")
            .statuses
            .insert(gid.to_string(), status);
    }

    fn fail_status(&self, gid: &str, message: &str) {
        self.inner
            .lock()
            .expect("fake lock")
            .fail_status
            .insert(gid.to_string(), message.to_string());
    }
}

#[async_trait]
impl Aria2Api for FakeAria2 {
    async fn add_uri(&self, url: &str, dir: &str, out: &str) -> Result<String, String> {
        let mut inner = self.inner.lock().expect("fake lock");
        inner.calls.push(Call::AddUri {
            url: url.to_string(),
            dir: dir.to_string(),
            out: out.to_string(),
        });
        if let Some(message) = inner.fail_add.take() {
            return Err(message);
        }
        inner.next_gid += 1;
        let gid = format!("gid-{}", inner.next_gid);
        inner.statuses.insert(
            gid.clone(),
            json!({
                "status": "active",
                "completedLength": "0",
                "downloadSpeed": "0",
                "totalLength": "0"
            }),
        );
        Ok(gid)
    }

    async fn pause(&self, gid: &str) -> Result<(), String> {
        self.inner
            .lock()
            .expect("fake lock")
            .calls
            .push(Call::Pause {
                gid: gid.to_string(),
            });
        Ok(())
    }

    async fn unpause(&self, gid: &str) -> Result<(), String> {
        self.inner
            .lock()
            .expect("fake lock")
            .calls
            .push(Call::Unpause {
                gid: gid.to_string(),
            });
        Ok(())
    }

    async fn remove(&self, gid: &str) -> Result<(), String> {
        self.inner
            .lock()
            .expect("fake lock")
            .calls
            .push(Call::Remove {
                gid: gid.to_string(),
            });
        Ok(())
    }

    async fn tell_status(&self, gid: &str) -> Result<Value, String> {
        let mut inner = self.inner.lock().expect("fake lock");
        inner.calls.push(Call::TellStatus {
            gid: gid.to_string(),
        });
        if let Some(message) = inner.fail_status.remove(gid) {
            return Err(message);
        }
        inner
            .statuses
            .get(gid)
            .cloned()
            .ok_or_else(|| format!("unknown gid {gid}"))
    }
}

async fn task(engine: &DownloadEngine<FakeAria2>, id: &TaskId) -> DownloadTask {
    engine.manager().await.get(id).cloned().expect("task")
}

fn request(url: &str, file_name: &str) -> NewTaskRequest {
    NewTaskRequest {
        url: url.into(),
        file_name: file_name.into(),
        size: None,
        referrer: Some("http://example.com/files".into()),
        user_agent: Some("test-agent".into()),
    }
}

#[tokio::test]
async fn submit_adds_uri_stores_gid_and_marks_active() {
    let aria = FakeAria2::default();
    let mut engine = DownloadEngine::new(aria.clone(), "/downloads");

    let id = engine
        .submit(request("http://example.com/a.zip", "a.zip"))
        .await
        .expect("submit");

    assert_eq!(
        aria.calls(),
        vec![Call::AddUri {
            url: "http://example.com/a.zip".into(),
            dir: "/downloads".into(),
            out: "a.zip".into(),
        }]
    );
    let task = task(&engine, &id).await;
    assert_eq!(task.status, TaskStatus::Active);
    assert_eq!(task.gid.as_deref(), Some("gid-1"));
    assert_eq!(task.completed_length, 0);
}

#[tokio::test]
async fn submit_uses_updated_save_dir() {
    let aria = FakeAria2::default();
    let mut engine = DownloadEngine::new(aria.clone(), "/downloads");
    engine.set_save_dir("/other");

    engine
        .submit(request("http://example.com/b.bin", "b.bin"))
        .await
        .expect("submit");

    assert_eq!(
        aria.calls(),
        vec![Call::AddUri {
            url: "http://example.com/b.bin".into(),
            dir: "/other".into(),
            out: "b.bin".into(),
        }]
    );
}

#[tokio::test]
async fn submit_same_url_reuses_task_without_second_add() {
    let aria = FakeAria2::default();
    let mut engine = DownloadEngine::new(aria.clone(), "/downloads");
    let req = request("http://example.com/a.zip", "a.zip");

    let first = engine.submit(req.clone()).await.expect("first");
    let second = engine.submit(req).await.expect("second");

    assert_eq!(first, second);
    assert_eq!(engine.manager().await.list().len(), 1);
    assert_eq!(aria.calls().len(), 1);
}

#[tokio::test]
async fn submit_retries_add_uri_when_previous_attempt_has_no_gid() {
    let aria = FakeAria2::default();
    aria.fail_next_add("rpc down");
    let mut engine = DownloadEngine::new(aria.clone(), "/downloads");
    let req = request("http://example.com/a.zip", "a.zip");

    engine.submit(req.clone()).await.expect_err("first");
    let id = engine.submit(req).await.expect("retry");

    let task = task(&engine, &id).await;
    assert_eq!(task.status, TaskStatus::Active);
    assert_eq!(task.gid.as_deref(), Some("gid-1"));
    assert!(task.error.is_none());
    assert_eq!(aria.calls().len(), 2);
}

#[tokio::test]
async fn submit_marks_error_when_add_uri_fails() {
    let aria = FakeAria2::default();
    aria.fail_next_add("rpc down");
    let mut engine = DownloadEngine::new(aria.clone(), "/downloads");

    let err = engine
        .submit(request("http://example.com/a.zip", "a.zip"))
        .await
        .expect_err("add fails");

    assert_eq!(err, "rpc down");
    let task = engine.manager().await.list().pop().expect("task kept");
    assert_eq!(task.status, TaskStatus::Error);
    assert_eq!(task.error.as_deref(), Some("rpc down"));
    assert!(task.gid.is_none());
}

#[tokio::test]
async fn poll_once_updates_progress_for_active_tasks() {
    let aria = FakeAria2::default();
    let mut engine = DownloadEngine::new(aria.clone(), "/downloads");
    let id = engine
        .submit(request("http://example.com/a.zip", "a.zip"))
        .await
        .expect("submit");

    aria.set_status(
        "gid-1",
        json!({
            "status": "active",
            "completedLength": "40",
            "downloadSpeed": "10",
            "totalLength": "100"
        }),
    );

    engine.poll_once().await.expect("poll");

    let task = task(&engine, &id).await;
    assert_eq!(task.status, TaskStatus::Active);
    assert_eq!(task.completed_length, 40);
    assert_eq!(task.download_speed, 10);
    assert!(aria.calls().iter().any(|call| call
        == &Call::TellStatus {
            gid: "gid-1".into()
        }));
}

#[tokio::test]
async fn poll_once_applies_terminal_and_waiting_states() {
    let aria = FakeAria2::default();
    let mut engine = DownloadEngine::new(aria.clone(), "/downloads");
    let complete_id = engine
        .submit(request("http://example.com/done.zip", "done.zip"))
        .await
        .expect("done");
    let error_id = engine
        .submit(request("http://example.com/bad.zip", "bad.zip"))
        .await
        .expect("bad");
    let waiting_id = engine
        .submit(request("http://example.com/wait.zip", "wait.zip"))
        .await
        .expect("wait");

    aria.set_status(
        "gid-1",
        json!({
            "status": "complete",
            "completedLength": "100",
            "downloadSpeed": "5"
        }),
    );
    aria.set_status(
        "gid-2",
        json!({
            "status": "error",
            "completedLength": "3",
            "downloadSpeed": "1",
            "errorMessage": "not found"
        }),
    );
    aria.set_status(
        "gid-3",
        json!({
            "status": "waiting",
            "completedLength": "0",
            "downloadSpeed": "0"
        }),
    );

    engine.poll_once().await.expect("poll");

    let complete = task(&engine, &complete_id).await;
    assert_eq!(complete.status, TaskStatus::Complete);
    assert_eq!(complete.completed_length, 100);
    assert_eq!(complete.download_speed, 0);
    assert!(complete.error.is_none());

    let failed = task(&engine, &error_id).await;
    assert_eq!(failed.status, TaskStatus::Error);
    assert_eq!(failed.error.as_deref(), Some("not found"));
    assert_eq!(failed.completed_length, 3);
    assert_eq!(failed.download_speed, 0);

    let waiting = task(&engine, &waiting_id).await;
    assert_eq!(waiting.status, TaskStatus::Active);
    assert_eq!(waiting.download_speed, 0);

    let tells_before = aria
        .calls()
        .iter()
        .filter(|call| matches!(call, Call::TellStatus { .. }))
        .count();
    engine.poll_once().await.expect("second poll");
    let tells_after = aria
        .calls()
        .iter()
        .filter(|call| matches!(call, Call::TellStatus { .. }))
        .count();
    // Only the still-active (waiting) task is polled again.
    assert_eq!(tells_after - tells_before, 1);
}

#[tokio::test]
async fn poll_once_records_rpc_error_and_continues() {
    let aria = FakeAria2::default();
    let mut engine = DownloadEngine::new(aria.clone(), "/downloads");
    let broken = engine
        .submit(request("http://example.com/a.zip", "a.zip"))
        .await
        .expect("a");
    let healthy = engine
        .submit(request("http://example.com/b.zip", "b.zip"))
        .await
        .expect("b");
    aria.fail_status("gid-1", "tell failed");
    aria.set_status(
        "gid-2",
        json!({
            "status": "active",
            "completedLength": "8",
            "downloadSpeed": "2"
        }),
    );

    engine.poll_once().await.expect("poll continues");

    let broken_task = task(&engine, &broken).await;
    assert_eq!(broken_task.status, TaskStatus::Error);
    assert_eq!(broken_task.error.as_deref(), Some("tell failed"));

    let healthy_task = task(&engine, &healthy).await;
    assert_eq!(healthy_task.status, TaskStatus::Active);
    assert_eq!(healthy_task.completed_length, 8);
    assert_eq!(healthy_task.download_speed, 2);
}

#[tokio::test]
async fn pause_resume_and_cancel_call_aria2() {
    let aria = FakeAria2::default();
    let mut engine = DownloadEngine::new(aria.clone(), "/downloads");
    let id = engine
        .submit(request("http://example.com/a.zip", "a.zip"))
        .await
        .expect("submit");

    engine.pause_task(&id).await.expect("pause");
    assert_eq!(task(&engine, &id).await.status, TaskStatus::Paused);

    engine.poll_once().await.expect("poll while paused");
    assert!(!aria
        .calls()
        .iter()
        .any(|call| matches!(call, Call::TellStatus { .. })));

    engine.resume_task(&id).await.expect("resume");
    assert_eq!(task(&engine, &id).await.status, TaskStatus::Active);

    engine.cancel_task(&id).await.expect("cancel");
    let task = task(&engine, &id).await;
    assert_eq!(task.status, TaskStatus::Error);
    assert_eq!(task.error.as_deref(), Some("cancelled"));
    assert_eq!(task.download_speed, 0);

    assert_eq!(
        aria.calls(),
        vec![
            Call::AddUri {
                url: "http://example.com/a.zip".into(),
                dir: "/downloads".into(),
                out: "a.zip".into(),
            },
            Call::Pause {
                gid: "gid-1".into()
            },
            Call::Unpause {
                gid: "gid-1".into()
            },
            Call::Remove {
                gid: "gid-1".into()
            },
        ]
    );
}

#[tokio::test]
async fn control_methods_reject_unknown_task() {
    let aria = FakeAria2::default();
    let mut engine = DownloadEngine::new(aria, "/downloads");
    let missing = TaskId("missing".into());

    assert_eq!(
        engine.pause_task(&missing).await.unwrap_err(),
        "task not found"
    );
    assert_eq!(
        engine.resume_task(&missing).await.unwrap_err(),
        "task not found"
    );
    assert_eq!(
        engine.cancel_task(&missing).await.unwrap_err(),
        "task not found"
    );
}

#[tokio::test]
async fn shared_manager_submit_starts_task_added_elsewhere() {
    let tasks = Arc::new(tokio::sync::Mutex::new(TaskManager::new()));
    let id = tasks
        .lock()
        .await
        .add(request("http://example.com/a.zip", "a.zip"))
        .expect("queue");
    let aria = FakeAria2::default();
    let mut engine = DownloadEngine::shared(Arc::clone(&tasks), aria.clone(), "/downloads");

    let submitted = engine
        .submit(request("http://example.com/a.zip", "a.zip"))
        .await
        .expect("submit");

    assert_eq!(submitted, id);
    let stored = tasks.lock().await.get(&id).cloned().expect("shared task");
    assert_eq!(stored.status, TaskStatus::Active);
    assert_eq!(stored.gid.as_deref(), Some("gid-1"));
    assert_eq!(engine.tasks().lock().await.list().len(), 1);
    assert_eq!(aria.calls().len(), 1);
}
