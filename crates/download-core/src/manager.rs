use crate::task::{require_http_url, DownloadTask, NewTaskRequest, TaskId, TaskStatus};
use std::collections::HashMap;

#[derive(Default)]
pub struct TaskManager {
    tasks: HashMap<TaskId, DownloadTask>,
    by_url: HashMap<String, TaskId>,
}

impl TaskManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a new download task, or returns the existing id if the URL is already known.
    ///
    /// The URL string is indexed at insert time for deduplication; it must not be changed
    /// afterward (see [`Self::get_mut`]).
    pub fn add(&mut self, req: NewTaskRequest) -> Result<TaskId, String> {
        require_http_url(&req.url)?;
        if let Some(id) = self.by_url.get(&req.url) {
            return Ok(id.clone());
        }
        let task = DownloadTask::from_request(req);
        let id = task.id.clone();
        self.by_url.insert(task.url.clone(), id.clone());
        self.tasks.insert(id.clone(), task);
        Ok(id)
    }

    pub fn list(&self) -> Vec<DownloadTask> {
        self.tasks.values().cloned().collect()
    }

    pub fn get(&self, id: &TaskId) -> Option<&DownloadTask> {
        self.tasks.get(id)
    }

    /// Returns a mutable reference to the task with the given id.
    ///
    /// **Do not mutate [`DownloadTask::url`]** on the returned task. URL deduplication
    /// (`by_url`) is keyed on the URL at insert time; changing `url` without updating
    /// `TaskManager`'s internal index will desync lookups such as [`Self::find_by_url`]
    /// and duplicate adds for the new URL.
    pub fn get_mut(&mut self, id: &TaskId) -> Option<&mut DownloadTask> {
        self.tasks.get_mut(id)
    }

    pub fn set_status(&mut self, id: &TaskId, status: TaskStatus) -> Result<(), String> {
        let t = self
            .tasks
            .get_mut(id)
            .ok_or_else(|| "task not found".to_string())?;
        t.status = status;
        Ok(())
    }

    pub fn find_by_url(&self, url: &str) -> Option<&DownloadTask> {
        self.by_url.get(url).and_then(|id| self.tasks.get(id))
    }

    /// Drops the task and its URL index entry so the same URL can be added again.
    pub fn remove(&mut self, id: &TaskId) -> Result<(), String> {
        if self.tasks.remove(id).is_none() {
            return Err("task not found".into());
        }
        self.by_url.retain(|_, existing| existing != id);
        Ok(())
    }

    /// Inserts a previously persisted task (keeps its id). Skips duplicates.
    pub fn restore(&mut self, task: DownloadTask) -> Result<(), String> {
        if self.tasks.contains_key(&task.id) {
            return Ok(());
        }
        if self.by_url.contains_key(&task.url) {
            return Ok(());
        }
        require_http_url(&task.url)?;
        self.by_url.insert(task.url.clone(), task.id.clone());
        self.tasks.insert(task.id.clone(), task);
        Ok(())
    }
}
