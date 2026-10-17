use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use download_core::manager::TaskManager;
use download_core::task::{DownloadTask, NewTaskRequest, TaskStatus};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

/// Fixed loopback port. Binding is done by the app crate.
pub const DEFAULT_PORT: u16 = 29128;

#[derive(Clone)]
pub struct AppState {
    pub tasks: Arc<Mutex<TaskManager>>,
}

/// Axum router for `GET /health`, `POST /v1/tasks`, and `GET /v1/tasks`.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/v1/tasks", get(list_tasks).post(create_task))
        .with_state(state)
}

pub use router as app;

#[derive(Serialize)]
struct HealthBody {
    ok: bool,
    version: &'static str,
}

async fn health() -> Json<HealthBody> {
    Json(HealthBody {
        ok: true,
        version: env!("CARGO_PKG_VERSION"),
    })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateTaskBody {
    url: String,
    file_name: String,
    size: Option<u64>,
    referrer: Option<String>,
    user_agent: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CreateTaskResponse {
    task_id: String,
}

#[derive(Serialize)]
struct ErrorBody {
    error: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TaskJson {
    id: String,
    url: String,
    file_name: String,
    size: Option<u64>,
    referrer: Option<String>,
    user_agent: Option<String>,
    status: TaskStatus,
    gid: Option<String>,
    error: Option<String>,
    completed_length: u64,
    download_speed: u64,
}

impl From<&DownloadTask> for TaskJson {
    fn from(task: &DownloadTask) -> Self {
        Self {
            id: task.id.0.clone(),
            url: task.url.clone(),
            file_name: task.file_name.clone(),
            size: task.size,
            referrer: task.referrer.clone(),
            user_agent: task.user_agent.clone(),
            status: task.status,
            gid: task.gid.clone(),
            error: task.error.clone(),
            completed_length: task.completed_length,
            download_speed: task.download_speed,
        }
    }
}

async fn create_task(
    State(state): State<AppState>,
    Json(body): Json<CreateTaskBody>,
) -> Result<Json<CreateTaskResponse>, (StatusCode, Json<ErrorBody>)> {
    let req = NewTaskRequest {
        url: body.url,
        file_name: body.file_name,
        size: body.size,
        referrer: body.referrer,
        user_agent: body.user_agent,
    };
    let mut tasks = state.tasks.lock().await;
    match tasks.add(req) {
        Ok(id) => Ok(Json(CreateTaskResponse { task_id: id.0 })),
        Err(error) => Err((StatusCode::BAD_REQUEST, Json(ErrorBody { error }))),
    }
}

async fn list_tasks(State(state): State<AppState>) -> Json<Vec<TaskJson>> {
    let tasks = state.tasks.lock().await;
    Json(tasks.list().iter().map(TaskJson::from).collect())
}
