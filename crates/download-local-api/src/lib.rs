use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::{header, HeaderName, HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::routing::get;
use axum::{Json, Router};
use download_core::manager::TaskManager;
use download_core::protocol::sanitize_file_name;
use download_core::task::{DownloadTask, NewTaskRequest, TaskStatus};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

/// Fixed loopback port. Binding is done by the app crate.
pub const DEFAULT_PORT: u16 = 29128;

#[derive(Clone)]
pub struct AppState {
    pub tasks: Arc<Mutex<TaskManager>>,
    /// Set when aria2 JSON-RPC is ready to accept downloads.
    pub aria2_ready: Arc<AtomicBool>,
}

/// Routes for `GET /health`, `POST /v1/tasks`, and `GET /v1/tasks`, without CORS.
pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health).options(preflight))
        .route(
            "/v1/tasks",
            get(list_tasks).post(create_task).options(preflight),
        )
        .with_state(state)
}

/// Same routes as [`routes`], with browser CORS and private-network preflight.
pub fn app(state: AppState) -> Router {
    with_cors(routes(state))
}

/// Allow a website origin to call this loopback API, including Chromium's
/// private-network preflight.
pub fn with_cors(router: Router) -> Router {
    router.layer(middleware::from_fn(cors_headers))
}

/// `204` for `OPTIONS` preflight. CORS headers are added by [`with_cors`].
pub async fn preflight() -> StatusCode {
    StatusCode::NO_CONTENT
}

async fn cors_headers(request: Request, next: Next) -> Response {
    let origin = request.headers().get(header::ORIGIN).cloned();
    let allow_headers = request
        .headers()
        .get(header::ACCESS_CONTROL_REQUEST_HEADERS)
        .cloned();
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    match origin {
        Some(origin) => {
            headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin);
            headers.insert(header::VARY, HeaderValue::from_static("Origin"));
        }
        None => {
            headers.insert(
                header::ACCESS_CONTROL_ALLOW_ORIGIN,
                HeaderValue::from_static("*"),
            );
        }
    }
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("GET, POST, OPTIONS"),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        allow_headers.unwrap_or_else(|| HeaderValue::from_static("content-type, accept")),
    );
    headers.insert(
        header::ACCESS_CONTROL_MAX_AGE,
        HeaderValue::from_static("600"),
    );
    headers.insert(
        HeaderName::from_static("access-control-allow-private-network"),
        HeaderValue::from_static("true"),
    );
    response
}

#[derive(Serialize)]
struct HealthBody {
    ok: bool,
    aria2: bool,
    version: &'static str,
}

async fn health(State(state): State<AppState>) -> (StatusCode, Json<HealthBody>) {
    let aria2 = state.aria2_ready.load(Ordering::Relaxed);
    let status = if aria2 {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (
        status,
        Json(HealthBody {
            ok: aria2,
            aria2,
            version: env!("CARGO_PKG_VERSION"),
        }),
    )
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
    created_at: u64,
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
            created_at: task.created_at,
        }
    }
}

async fn create_task(
    State(state): State<AppState>,
    Json(body): Json<CreateTaskBody>,
) -> Result<Json<CreateTaskResponse>, (StatusCode, Json<ErrorBody>)> {
    let file_name = sanitize_file_name(&body.file_name, &body.url);
    let req = NewTaskRequest {
        url: body.url,
        file_name,
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
