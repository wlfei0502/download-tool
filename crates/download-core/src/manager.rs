use crate::task::{DownloadTask, NewTaskRequest, TaskId, TaskStatus};
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

    pub fn add(&mut self, req: NewTaskRequest) -> Result<TaskId, String> {
        if req.url.is_empty() {
            return Err("url required".into());
        }
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
}
