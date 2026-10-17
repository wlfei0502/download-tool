use crate::manager::TaskManager;
use crate::task::{DownloadTask, NewTaskRequest, TaskId, TaskStatus};
use async_trait::async_trait;
use serde_json::Value;

#[async_trait]
pub trait Aria2Api: Send + Sync {
    async fn add_uri(&self, url: &str, dir: &str, out: &str) -> Result<String, String>;
    async fn pause(&self, gid: &str) -> Result<(), String>;
    async fn unpause(&self, gid: &str) -> Result<(), String>;
    async fn remove(&self, gid: &str) -> Result<(), String>;
    async fn tell_status(&self, gid: &str) -> Result<Value, String>;
}

/// Owns the task list and drives aria2 for start, poll, pause, resume, and cancel.
///
/// `save_dir` is the directory passed to `add_uri`. Persistence of that setting
/// belongs to a later settings task.
pub struct DownloadEngine<A> {
    manager: TaskManager,
    aria2: A,
    save_dir: String,
}

impl<A: Aria2Api> DownloadEngine<A> {
    pub fn new(aria2: A, save_dir: impl Into<String>) -> Self {
        Self {
            manager: TaskManager::new(),
            aria2,
            save_dir: save_dir.into(),
        }
    }

    pub fn manager(&self) -> &TaskManager {
        &self.manager
    }

    pub fn set_save_dir(&mut self, dir: impl Into<String>) {
        self.save_dir = dir.into();
    }

    /// Registers the task and starts it in aria2. The same URL returns the existing id.
    ///
    /// A previous attempt that never received a gid is started again. A task that
    /// already has a gid is left as-is.
    pub async fn submit(&mut self, req: NewTaskRequest) -> Result<TaskId, String> {
        let id = self.manager.add(req)?;
        if self
            .manager
            .get(&id)
            .map(|task| task.gid.is_some())
            .unwrap_or(false)
        {
            return Ok(id);
        }

        let (url, file_name) = {
            let task = self
                .manager
                .get(&id)
                .ok_or_else(|| "task not found".to_string())?;
            (task.url.clone(), task.file_name.clone())
        };
        match self.aria2.add_uri(&url, &self.save_dir, &file_name).await {
            Ok(gid) => {
                let task = self
                    .manager
                    .get_mut(&id)
                    .ok_or_else(|| "task not found".to_string())?;
                task.gid = Some(gid);
                task.status = TaskStatus::Active;
                task.error = None;
                Ok(id)
            }
            Err(err) => {
                if let Some(task) = self.manager.get_mut(&id) {
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
        let active: Vec<(TaskId, String)> = self
            .manager
            .list()
            .into_iter()
            .filter(|task| task.status == TaskStatus::Active)
            .filter_map(|task| task.gid.map(|gid| (task.id, gid)))
            .collect();

        for (id, gid) in active {
            match self.aria2.tell_status(&gid).await {
                Ok(value) => {
                    if let Some(task) = self.manager.get_mut(&id) {
                        apply_aria2_status(task, &value);
                    }
                }
                Err(err) => {
                    if let Some(task) = self.manager.get_mut(&id) {
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
        let gid = self.gid_of(id)?;
        self.aria2.pause(&gid).await?;
        self.manager.set_status(id, TaskStatus::Paused)?;
        Ok(())
    }

    pub async fn resume_task(&mut self, id: &TaskId) -> Result<(), String> {
        let gid = self.gid_of(id)?;
        self.aria2.unpause(&gid).await?;
        self.manager.set_status(id, TaskStatus::Active)?;
        Ok(())
    }

    pub async fn cancel_task(&mut self, id: &TaskId) -> Result<(), String> {
        let gid = self.gid_of(id)?;
        self.aria2.remove(&gid).await?;
        let task = self
            .manager
            .get_mut(id)
            .ok_or_else(|| "task not found".to_string())?;
        task.status = TaskStatus::Error;
        task.error = Some("cancelled".to_string());
        task.download_speed = 0;
        Ok(())
    }

    fn gid_of(&self, id: &TaskId) -> Result<String, String> {
        let task = self
            .manager
            .get(id)
            .ok_or_else(|| "task not found".to_string())?;
        task.gid
            .clone()
            .ok_or_else(|| "task has no gid".to_string())
    }
}

fn apply_aria2_status(task: &mut DownloadTask, value: &Value) {
    task.completed_length = json_u64(value, "completedLength");
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
