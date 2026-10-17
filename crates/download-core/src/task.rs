use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TaskId(pub String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Queued,
    Active,
    Paused,
    Complete,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewTaskRequest {
    pub url: String,
    pub file_name: String,
    pub size: Option<u64>,
    pub referrer: Option<String>,
    pub user_agent: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadTask {
    pub id: TaskId,
    pub url: String,
    pub file_name: String,
    pub size: Option<u64>,
    pub referrer: Option<String>,
    pub user_agent: Option<String>,
    pub status: TaskStatus,
    pub gid: Option<String>,
    pub error: Option<String>,
    pub completed_length: u64,
    pub download_speed: u64,
}

/// Download URLs must use the `http://` scheme. Comparison is case-insensitive.
pub fn require_http_url(url: &str) -> Result<(), String> {
    if url.is_empty() {
        return Err("url required".into());
    }
    let is_http = url
        .get(.."http://".len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("http://"));
    if !is_http {
        return Err("only http:// URLs are allowed".into());
    }
    Ok(())
}

impl DownloadTask {
    pub fn from_request(req: NewTaskRequest) -> Self {
        Self {
            id: TaskId(Uuid::new_v4().to_string()),
            url: req.url,
            file_name: req.file_name,
            size: req.size,
            referrer: req.referrer,
            user_agent: req.user_agent,
            status: TaskStatus::Queued,
            gid: None,
            error: None,
            completed_length: 0,
            download_speed: 0,
        }
    }
}
