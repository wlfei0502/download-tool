use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use download_core::store::{prepare_loaded_task, TaskStore};
use download_core::task::{DownloadTask, NewTaskRequest, TaskStatus};

#[test]
fn sync_snapshot_roundtrips_and_deletes_removed_rows() {
    let dir = TempDir::new();
    let path = TaskStore::file_in(&dir.path);
    let store = TaskStore::open(&path).expect("open");

    let mut a = DownloadTask::from_request(NewTaskRequest {
        url: "http://example.com/a.zip".into(),
        file_name: "a.zip".into(),
        size: Some(10),
        referrer: None,
        user_agent: None,
    });
    a.status = TaskStatus::Complete;
    a.completed_length = 10;

    let b = DownloadTask::from_request(NewTaskRequest {
        url: "https://example.com/b.bin".into(),
        file_name: "b.bin".into(),
        size: None,
        referrer: Some("https://example.com/".into()),
        user_agent: Some("ua".into()),
    });

    store.sync_snapshot(&[a.clone(), b.clone()]).expect("sync");
    assert_eq!(store.count().expect("count"), 2);

    let loaded = store.load_all().expect("load");
    assert_eq!(loaded.len(), 2);
    let loaded_a = loaded.iter().find(|t| t.id == a.id).expect("a");
    assert_eq!(loaded_a.file_name, "a.zip");
    assert_eq!(loaded_a.status, TaskStatus::Complete);
    assert_eq!(loaded_a.completed_length, 10);
    assert_eq!(loaded_a.size, Some(10));

    store.sync_snapshot(&[a.clone()]).expect("sync one");
    assert_eq!(store.count().expect("count"), 1);
    assert_eq!(store.load_all().expect("load")[0].id, a.id);
}

#[test]
fn prepare_loaded_task_resets_unfinished_work_for_restart() {
    let mut active = DownloadTask::from_request(NewTaskRequest {
        url: "http://example.com/x".into(),
        file_name: "x.bin".into(),
        size: None,
        referrer: None,
        user_agent: None,
    });
    active.status = TaskStatus::Active;
    active.gid = Some("gid-1".into());
    active.download_speed = 99;
    active.completed_length = 40;

    let prepared = prepare_loaded_task(active);
    assert_eq!(prepared.status, TaskStatus::Queued);
    assert_eq!(prepared.gid, None);
    assert_eq!(prepared.download_speed, 0);
    assert_eq!(prepared.completed_length, 40);

    let mut done = DownloadTask::from_request(NewTaskRequest {
        url: "http://example.com/y".into(),
        file_name: "y.bin".into(),
        size: None,
        referrer: None,
        user_agent: None,
    });
    done.status = TaskStatus::Complete;
    done.gid = Some("gid-2".into());
    let prepared = prepare_loaded_task(done);
    assert_eq!(prepared.status, TaskStatus::Complete);
    assert_eq!(prepared.gid, None);
}

#[test]
fn file_path_is_under_download_tool() {
    let path = TaskStore::file_in(std::path::Path::new("/cfg"));
    assert_eq!(
        path,
        PathBuf::from("/cfg").join("download-tool").join("tasks.db")
    );
}

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "download-tool-tasks-{}-{id}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).expect("temp dir");
        Self { path }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}
