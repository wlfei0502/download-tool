use std::time::{SystemTime, UNIX_EPOCH};

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
    /// Unix seconds when the task was created (local clock).
    pub created_at: u64,
}

/// Download URLs must use `http://` or `https://` and must not carry userinfo
/// (`user:password@host`). Scheme comparison is case-insensitive.
pub fn require_http_url(url: &str) -> Result<(), String> {
    if url.is_empty() {
        return Err("url required".into());
    }
    let rest = if url.len() >= 8 && url[..8].eq_ignore_ascii_case("https://") {
        &url[8..]
    } else if url.len() >= 7 && url[..7].eq_ignore_ascii_case("http://") {
        &url[7..]
    } else {
        return Err("only http:// and https:// URLs are allowed".into());
    };
    if authority_has_userinfo(rest) {
        return Err("url must not include user info".into());
    }
    Ok(())
}

/// `user:pass@host` before the path, query, or fragment.
fn authority_has_userinfo(after_scheme: &str) -> bool {
    let authority = after_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or(after_scheme);
    authority.contains('@')
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
            created_at: now_unix_secs(),
        }
    }
}

fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
