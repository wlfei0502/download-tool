use std::sync::Arc;

use std::path::Path;

use crate::manager::TaskManager;
use crate::protocol::sanitize_file_name;
use crate::settings::{clamp_connections, DEFAULT_MAX_CONNECTIONS_PER_SERVER};
use crate::task::{require_http_url, DownloadTask, NewTaskRequest, TaskId, TaskStatus};
use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::{Mutex, MutexGuard};

#[async_trait]
pub trait Aria2Api: Send + Sync {
    async fn add_uri(
        &self,
        url: &str,
        dir: &str,
        out: &str,
        max_connections: u8,
    ) -> Result<String, String>;
    async fn pause(&self, gid: &str) -> Result<(), String>;
    async fn unpause(&self, gid: &str) -> Result<(), String>;
    async fn remove(&self, gid: &str) -> Result<(), String>;
    /// Drop a finished download from aria2's result list (`aria2.removeDownloadResult`).
    async fn remove_download_result(&self, gid: &str) -> Result<(), String>;
    async fn tell_status(&self, gid: &str) -> Result<Value, String>;
    /// `aria2.changeGlobalOption` with `max-overall-download-limit`.
    /// `None` sends `"0"`, which aria2 treats as unlimited.
    async fn set_max_overall_download_limit(&self, limit_bps: Option<u64>) -> Result<(), String>;
}

/// Drives aria2 for start, poll, pause, resume, and cancel.
///
/// The task list is shared (`Arc<Mutex<TaskManager>>`) so the local API and this
/// engine update one queue. `save_dir` is the directory passed to `add_uri`.
/// Callers persist that directory, the global speed limit, and the connection
/// cap through `crate::settings::Settings`.
pub struct DownloadEngine<A> {
    manager: Arc<Mutex<TaskManager>>,
    aria2: A,
    save_dir: String,
    max_connections_per_server: u8,
}

impl<A: Aria2Api> DownloadEngine<A> {
    pub fn new(aria2: A, save_dir: impl Into<String>) -> Self {
        Self::shared(Arc::new(Mutex::new(TaskManager::new())), aria2, save_dir)
    }

    /// Uses `tasks` as the queue. Callers that also serve the local API should
    /// pass the same `Arc` they put in `AppState`.
    pub fn shared(tasks: Arc<Mutex<TaskManager>>, aria2: A, save_dir: impl Into<String>) -> Self {
        Self {
            manager: tasks,
            aria2,
            save_dir: save_dir.into(),
            max_connections_per_server: DEFAULT_MAX_CONNECTIONS_PER_SERVER,
        }
    }

    pub fn tasks(&self) -> Arc<Mutex<TaskManager>> {
        Arc::clone(&self.manager)
    }

    /// Locks the shared task list. Drop the guard before awaiting other work.
    pub async fn manager(&self) -> MutexGuard<'_, TaskManager> {
        self.manager.lock().await
    }

    pub fn set_save_dir(&mut self, dir: impl Into<String>) {
        self.save_dir = dir.into();
    }

    /// Clamps to 1..=16, matching aria2's `max-connection-per-server` range.
    pub fn set_max_connections_per_server(&mut self, value: u8) {
        self.max_connections_per_server = clamp_connections(value);
    }

    pub fn max_connections_per_server(&self) -> u8 {
        self.max_connections_per_server
    }

    /// Applies `max-overall-download-limit` for every download aria2 is serving.
    pub async fn set_global_speed_limit(&self, limit_bps: Option<u64>) -> Result<(), String> {
        self.aria2.set_max_overall_download_limit(limit_bps).await
    }

    /// Registers the task and starts it in aria2. The same URL returns the existing id.
    ///
    /// - Active / paused / error tasks with a known row are left or retried in place.
    /// - A **completed** URL may be submitted again as a new task (duplicate download);
    ///   existing files are kept (`auto-file-renaming` on `addUri`).
    /// - A previous attempt that never received a gid is started again.
    pub async fn submit(&mut self, mut req: NewTaskRequest) -> Result<TaskId, String> {
        require_http_url(&req.url)?;
        req.file_name = sanitize_file_name(&req.file_name, &req.url);
        let id = {
            let mut manager = self.manager.lock().await;
            manager.add(req.clone())?
        };

        enum Plan {
            Keep,
            Start,
            Restart { old_gid: Option<String> },
        }
        let plan = {
            let manager = self.manager.lock().await;
            match manager.get(&id) {
                Some(task)
                    if matches!(task.status, TaskStatus::Active | TaskStatus::Paused)
                        && task.gid.is_some() =>
                {
                    Plan::Keep
                }
                Some(task) if task.status == TaskStatus::Error => Plan::Restart {
                    old_gid: task.gid.clone(),
                },
                Some(task) if task.gid.is_some() => Plan::Keep,
                _ => Plan::Start,
            }
        };
        match plan {
            Plan::Keep => return Ok(id),
            Plan::Restart { old_gid } => {
                if let Some(gid) = old_gid.as_deref() {
                    ignore_missing_download(self.aria2.remove_download_result(gid).await)?;
                }
                let mut manager = self.manager.lock().await;
                if let Some(task) = manager.get_mut(&id) {
                    task.file_name = req.file_name.clone();
                    if req.size.is_some() {
                        task.size = req.size;
                    }
                    task.gid = None;
                    task.completed_length = 0;
                    task.download_speed = 0;
                    task.error = None;
                    task.status = TaskStatus::Queued;
                }
            }
            Plan::Start => {}
        }

        let (url, stored_name) = {
            let manager = self.manager.lock().await;
            let task = manager
                .get(&id)
                .ok_or_else(|| "task not found".to_string())?;
            (task.url.clone(), task.file_name.clone())
        };
        let file_name = sanitize_file_name(&stored_name, &url);
        match self
            .aria2
            .add_uri(
                &url,
                &self.save_dir,
                &file_name,
                self.max_connections_per_server,
            )
            .await
        {
            Ok(gid) => {
                let mut manager = self.manager.lock().await;
                let task = manager
                    .get_mut(&id)
                    .ok_or_else(|| "task not found".to_string())?;
                task.file_name = file_name;
                task.gid = Some(gid);
                task.status = TaskStatus::Active;
                task.error = None;
                Ok(id)
            }
            Err(err) => {
                let mut manager = self.manager.lock().await;
                if let Some(task) = manager.get_mut(&id) {
                    task.status = TaskStatus::Error;
                    task.error = Some(err.clone());
                }
                Err(err)
            }
        }
    }

    /// Refreshes every active task from `tell_status`.
    ///
    /// A failed status call is stored on that task and polling continues for the rest.
    pub async fn poll_once(&mut self) -> Result<(), String> {
        let active: Vec<(TaskId, String)> = {
            let manager = self.manager.lock().await;
            manager
                .list()
                .into_iter()
                .filter(|task| task.status == TaskStatus::Active)
                .filter_map(|task| task.gid.map(|gid| (task.id, gid)))
                .collect()
        };

        for (id, gid) in active {
            match self.aria2.tell_status(&gid).await {
                Ok(value) => {
                    let mut manager = self.manager.lock().await;
                    if let Some(task) = manager.get_mut(&id) {
                        apply_aria2_status(task, &value);
                    }
                }
                Err(err) => {
                    let mut manager = self.manager.lock().await;
                    if let Some(task) = manager.get_mut(&id) {
                        task.status = TaskStatus::Error;
                        task.error = Some(err);
                        task.download_speed = 0;
                    }
                }
            }
        }
        Ok(())
    }

    pub async fn pause_task(&mut self, id: &TaskId) -> Result<(), String> {
        let gid = self.gid_of(id).await?;
        self.aria2.pause(&gid).await?;
        let mut manager = self.manager.lock().await;
        manager.set_status(id, TaskStatus::Paused)?;
        Ok(())
    }

    pub async fn resume_task(&mut self, id: &TaskId) -> Result<(), String> {
        let gid = self.gid_of(id).await?;
        self.aria2.unpause(&gid).await?;
        let mut manager = self.manager.lock().await;
        manager.set_status(id, TaskStatus::Active)?;
        Ok(())
    }

    pub async fn cancel_task(&mut self, id: &TaskId) -> Result<(), String> {
        let (gid, status) = {
            let manager = self.manager.lock().await;
            let task = manager
                .get(id)
                .ok_or_else(|| "task not found".to_string())?;
            (task.gid.clone(), task.status)
        };
        let Some(gid) = gid else {
            let mut manager = self.manager.lock().await;
            manager.remove(id)?;
            return Ok(());
        };
        // Finished downloads are no longer "active"; aria2.remove fails with
        // "Active Download not found" and needs removeDownloadResult instead.
        match status {
            TaskStatus::Complete | TaskStatus::Error => {
                ignore_missing_download(self.aria2.remove_download_result(&gid).await)?;
            }
            _ => {
                if let Err(err) = self.aria2.remove(&gid).await {
                    if is_inactive_download_error(&err) {
                        ignore_missing_download(self.aria2.remove_download_result(&gid).await)?;
                    } else {
                        return Err(err);
                    }
                }
            }
        }
        let mut manager = self.manager.lock().await;
        manager.remove(id)?;
        Ok(())
    }

    async fn gid_of(&self, id: &TaskId) -> Result<String, String> {
        let manager = self.manager.lock().await;
        let task = manager
            .get(id)
            .ok_or_else(|| "task not found".to_string())?;
        task.gid
            .clone()
            .ok_or_else(|| "task has no gid".to_string())
    }
}

fn is_inactive_download_error(err: &str) -> bool {
    err.contains("Active Download not found")
}

fn ignore_missing_download(result: Result<(), String>) -> Result<(), String> {
    match result {
        Ok(()) => Ok(()),
        Err(err) if err.contains("not found") || err.contains("No such download") => Ok(()),
        Err(err) => Err(err),
    }
}

fn apply_aria2_status(task: &mut DownloadTask, value: &Value) {
    task.completed_length = json_u64(value, "completedLength");
    let total = json_u64(value, "totalLength");
    // Prefer aria2's measured size over the optional size from the site/SDK.
    if total > 0 {
        task.size = Some(total);
    }
    if let Some(name) = aria2_file_name(value) {
        task.file_name = name;
    }
    let speed = json_u64(value, "downloadSpeed");
    let status = value.get("status").and_then(Value::as_str).unwrap_or("");
    match status {
        "active" | "waiting" => {
            task.status = TaskStatus::Active;
            task.download_speed = speed;
            task.error = None;
        }
        "paused" => {
            task.status = TaskStatus::Paused;
            task.download_speed = 0;
        }
        "complete" => {
            task.status = TaskStatus::Complete;
            task.download_speed = 0;
            task.error = None;
            // Align declared size with bytes actually written so progress is 100%.
            if task.completed_length > 0 {
                task.size = Some(task.completed_length);
            }
        }
        "error" => {
            task.status = TaskStatus::Error;
            task.download_speed = 0;
            task.error = Some(aria2_error_message(value));
        }
        "removed" => {
            task.status = TaskStatus::Error;
            task.download_speed = 0;
            task.error = Some("removed".to_string());
        }
        other => {
            task.status = TaskStatus::Error;
            task.download_speed = 0;
            task.error = Some(format!("unknown aria2 status: {other}"));
        }
    }
}

/// Basename from aria2 `files[0].path` (reflects auto-renamed names like `a.1.zip`).
fn aria2_file_name(value: &Value) -> Option<String> {
    let path = value
        .get("files")?
        .as_array()?
        .first()?
        .get("path")?
        .as_str()
        .filter(|path| !path.is_empty())?;
    Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map(|name| name.to_string())
}

fn aria2_error_message(value: &Value) -> String {
    value
        .get("errorMessage")
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .unwrap_or("aria2 error")
        .to_string()
}

fn json_u64(value: &Value, key: &str) -> u64 {
    match value.get(key) {
        Some(Value::String(text)) => text.parse().unwrap_or(0),
        Some(Value::Number(number)) => number.as_u64().unwrap_or(0),
        _ => 0,
    }
}
