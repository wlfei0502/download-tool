//! One running Download Tool process.
//!
//! The first process holds a [`single_instance::SingleInstance`] named
//! `download-tool-dltool` until it exits. A later process posts any http
//! download, or a `dltool://add` URL, to `POST /v1/tasks`, asks the existing
//! window to come forward, and returns so the caller can exit 0.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use download_core::protocol::{parse_dltool_url, suggested_file_name};
use download_core::task::{require_http_url, NewTaskRequest};
use download_local_api::DEFAULT_PORT;

pub const INSTANCE_NAME: &str = "download-tool-dltool";

const HANDOFF_TIMEOUT: Duration = Duration::from_secs(20);
const HANDOFF_RETRY: Duration = Duration::from_millis(100);

/// Held by the primary process. Dropping it lets another process become primary.
#[must_use = "dropping the lock lets another process start"]
pub struct InstanceLock {
    _lock: single_instance::SingleInstance,
}

pub enum Launch {
    Primary(InstanceLock),
    Secondary,
}

/// Signal from `POST /v1/invoke` to the window loop.
#[derive(Clone)]
pub struct Activation {
    inner: Arc<ActivationInner>,
}

struct ActivationInner {
    show: AtomicBool,
    ctx: Mutex<Option<egui::Context>>,
}

impl Activation {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(ActivationInner {
                show: AtomicBool::new(false),
                ctx: Mutex::new(None),
            }),
        }
    }

    pub fn bind_context(&self, ctx: &egui::Context) {
        *self.lock_ctx() = Some(ctx.clone());
    }

    pub fn request_show(&self) {
        self.inner.show.store(true, Ordering::SeqCst);
        if let Some(ctx) = self.lock_ctx().as_ref() {
            ctx.request_repaint();
        }
    }

    /// Returns whether the window should be shown, and clears the request.
    pub fn take_show(&self) -> bool {
        self.inner.show.swap(false, Ordering::SeqCst)
    }

    fn lock_ctx(&self) -> std::sync::MutexGuard<'_, Option<egui::Context>> {
        self.inner.ctx.lock().unwrap_or_else(|err| err.into_inner())
    }
}

impl Default for Activation {
    fn default() -> Self {
        Self::new()
    }
}

/// Acquire the process lock. The primary must keep [`InstanceLock`] alive.
pub fn claim() -> Result<Launch, String> {
    claim_name(&lock_id())
}

/// Read process arguments and hand them to the already running instance.
pub fn forward_current_args() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    forward_args(DEFAULT_PORT, &args)
}

pub(crate) fn claim_name(name: &str) -> Result<Launch, String> {
    let lock = single_instance::SingleInstance::new(name)
        .map_err(|err| format!("single instance: {err}"))?;
    if lock.is_single() {
        Ok(Launch::Primary(InstanceLock { _lock: lock }))
    } else {
        Ok(Launch::Secondary)
    }
}

pub(crate) fn forward_args(port: u16, args: &[String]) -> Result<(), String> {
    forward_args_timeout(port, args, HANDOFF_TIMEOUT)
}

pub(crate) fn forward_args_timeout(
    port: u16,
    args: &[String],
    timeout: Duration,
) -> Result<(), String> {
    let tasks = requests_from_args(args);
    let deadline = Instant::now() + timeout;
    let mut last = format!("127.0.0.1:{port} is not accepting connections");
    loop {
        match forward_once(port, &tasks) {
            Ok(()) => {
                if !tasks.is_empty() {
                    eprintln!(
                        "forwarded {} download task(s) to the running Download Tool",
                        tasks.len()
                    );
                }
                return Ok(());
            }
            Err(ForwardError::Rejected(err)) => return Err(err),
            Err(ForwardError::Unreachable(err)) => {
                last = err;
                if Instant::now() >= deadline {
                    return Err(format!("could not reach 127.0.0.1:{port}: {last}"));
                }
                std::thread::sleep(HANDOFF_RETRY.min(timeout));
            }
        }
    }
}

/// http URLs and `dltool://add?...` arguments. Other argv is ignored.
pub(crate) fn requests_from_args(args: &[String]) -> Vec<NewTaskRequest> {
    let mut tasks = Vec::new();
    for arg in args {
        let arg = arg.trim().trim_matches('"');
        if arg.is_empty() || arg.starts_with('-') {
            continue;
        }
        if let Some(task) = task_from_arg(arg) {
            tasks.push(task);
        } else if looks_like_url(arg) {
            eprintln!("skipped argument that is not an http download: {arg}");
        }
    }
    tasks
}

fn lock_id() -> String {
    // On macOS the crate uses this string as a file path. A relative name would
    // follow the working directory, so a Finder launch and a terminal launch
    // would not lock each other. Linux and Windows use the name itself
    // (abstract socket, named mutex).
    #[cfg(target_os = "macos")]
    {
        std::env::temp_dir()
            .join(INSTANCE_NAME)
            .to_string_lossy()
            .into_owned()
    }
    #[cfg(not(target_os = "macos"))]
    {
        INSTANCE_NAME.to_string()
    }
}

fn task_from_arg(arg: &str) -> Option<NewTaskRequest> {
    if is_dltool(arg) {
        return match parse_dltool_url(arg) {
            Ok(request) => Some(request),
            Err(err) => {
                eprintln!("skipped dltool url: {err}");
                None
            }
        };
    }
    if require_http_url(arg).is_ok() {
        return Some(NewTaskRequest {
            file_name: suggested_file_name(arg),
            url: arg.to_string(),
            size: None,
            referrer: None,
            user_agent: None,
        });
    }
    None
}

fn is_dltool(arg: &str) -> bool {
    arg.len() >= "dltool:".len()
        && arg
            .get(.."dltool:".len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("dltool:"))
}

fn looks_like_url(arg: &str) -> bool {
    arg.contains("://")
}

enum ForwardError {
    Unreachable(String),
    Rejected(String),
}

fn forward_once(port: u16, tasks: &[NewTaskRequest]) -> Result<(), ForwardError> {
    let mut rejected = None;
    for task in tasks {
        if let Err(err) = post_json(port, "/v1/tasks", &task_json(task)) {
            match err {
                ForwardError::Unreachable(message) => {
                    return Err(ForwardError::Unreachable(message));
                }
                ForwardError::Rejected(message) => {
                    rejected.get_or_insert(message);
                }
            }
        }
    }
    post_json(port, "/v1/invoke", b"{}")?;
    if let Some(message) = rejected {
        return Err(ForwardError::Rejected(message));
    }
    Ok(())
}

fn task_json(task: &NewTaskRequest) -> Vec<u8> {
    let mut body = serde_json::json!({
        "url": task.url,
        "fileName": task.file_name,
    });
    if let Some(size) = task.size {
        body["size"] = serde_json::json!(size);
    }
    if let Some(referrer) = &task.referrer {
        body["referrer"] = serde_json::json!(referrer);
    }
    if let Some(user_agent) = &task.user_agent {
        body["userAgent"] = serde_json::json!(user_agent);
    }
    serde_json::to_vec(&body).expect("forwarded task is valid json")
}

fn post_json(port: u16, path: &str, body: &[u8]) -> Result<(), ForwardError> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_millis(400))
        .map_err(|err| ForwardError::Unreachable(format!("connect {path}: {err}")))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(|err| ForwardError::Rejected(format!("read timeout {path}: {err}")))?;
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .map_err(|err| ForwardError::Rejected(format!("write timeout {path}: {err}")))?;

    let header = format!(
        "POST {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream
        .write_all(header.as_bytes())
        .and_then(|_| stream.write_all(body))
        .map_err(|err| ForwardError::Unreachable(format!("write {path}: {err}")))?;

    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .map_err(|err| ForwardError::Unreachable(format!("read {path}: {err}")))?;
    let text = String::from_utf8_lossy(&response);
    let status = text.lines().next().unwrap_or("");
    let code = status.split_whitespace().nth(1).unwrap_or("");
    if code.starts_with('2') {
        Ok(())
    } else if code.is_empty() {
        Err(ForwardError::Unreachable(format!(
            "empty response from {path}"
        )))
    } else {
        Err(ForwardError::Rejected(format!("{path} returned {status}")))
    }
}

#[cfg(test)]
mod tests {
    use std::process::Command;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use axum::extract::State;
    use axum::routing::post;
    use axum::Router;
    use serde_json::json;

    use super::{
        claim_name, forward_args, forward_args_timeout, requests_from_args, Activation, Launch,
        INSTANCE_NAME,
    };

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn instance_name_is_download_tool_dltool() {
        assert_eq!(INSTANCE_NAME, "download-tool-dltool");
    }

    #[test]
    fn second_process_does_not_become_primary() {
        if std::env::var_os("DLTOOL_LOCK_CHILD").is_some() {
            let path = std::env::var("DLTOOL_LOCK_PATH").expect("lock path");
            let marker = std::env::var("DLTOOL_LOCK_MARKER").expect("marker");
            let (label, code) = match claim_name(&path) {
                Ok(Launch::Secondary) => ("secondary", 0),
                Ok(Launch::Primary(_)) => ("primary", 2),
                Err(err) => {
                    let _ = std::fs::write(&marker, err);
                    std::process::exit(3);
                }
            };
            std::fs::write(&marker, label).expect("write marker");
            std::process::exit(code);
        }

        let path = std::env::temp_dir().join(format!(
            "download-tool-dltool-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        let path = path.to_string_lossy().to_string();
        let marker = std::env::temp_dir().join(format!(
            "download-tool-dltool-marker-{}",
            std::process::id()
        ));
        let marker = marker.to_string_lossy().to_string();
        let parent = match claim_name(&path).expect("parent lock") {
            Launch::Primary(lock) => lock,
            Launch::Secondary => panic!("parent should be the first lock"),
        };

        let status = Command::new(std::env::current_exe().expect("test exe"))
            .env("DLTOOL_LOCK_CHILD", "1")
            .env("DLTOOL_LOCK_PATH", &path)
            .env("DLTOOL_LOCK_MARKER", &marker)
            .args([
                "--exact",
                "single_instance::tests::second_process_does_not_become_primary",
            ])
            .status()
            .expect("spawn lock child");
        let wrote = std::fs::read_to_string(&marker).unwrap_or_default();
        assert!(status.success(), "child status: {status}, marker: {wrote}");
        assert_eq!(wrote, "secondary");
        drop(parent);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(&marker);
    }

    #[test]
    fn collects_http_and_simple_dltool_args() {
        let tasks = requests_from_args(&args(&[
            "http://example.com/files/a.zip",
            "--flag",
            "dltool://add?url=http%3A%2F%2Fhost%2Fd%2Ft&fileName=a.zip&size=9",
            "https://example.com/nope",
            "notes.txt",
        ]));
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].url, "http://example.com/files/a.zip");
        assert_eq!(tasks[0].file_name, "a.zip");
        assert_eq!(tasks[0].size, None);
        assert_eq!(tasks[1].url, "http://host/d/t");
        assert_eq!(tasks[1].file_name, "a.zip");
        assert_eq!(tasks[1].size, Some(9));
    }

    #[test]
    fn derives_a_file_name_and_skips_a_dltool_url_without_http() {
        let bare = requests_from_args(&args(&["http://example.com/a%20b.zip?x=1"]));
        assert_eq!(bare[0].file_name, "a b.zip");

        let directory = requests_from_args(&args(&["http://example.com/"]));
        assert_eq!(directory[0].file_name, "download");

        let missing = requests_from_args(&args(&[
            "dltool://add?url=https%3A%2F%2Fhost%2Fa",
            "DLTOOL://add?url=http://example.com/from-scheme.bin",
        ]));
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].url, "http://example.com/from-scheme.bin");
        assert_eq!(missing[0].file_name, "from-scheme.bin");
    }

    #[test]
    fn dltool_args_keep_referrer_and_user_agent() {
        let tasks = requests_from_args(&args(&[
            "dltool://add?url=http%3A%2F%2Fhost%2Fa&fileName=a.bin&referrer=http%3A%2F%2Fhost%2Ffiles&userAgent=Mozilla%2F5.0",
        ]));
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].referrer.as_deref(), Some("http://host/files"));
        assert_eq!(tasks[0].user_agent.as_deref(), Some("Mozilla/5.0"));
    }

    #[test]
    fn show_request_is_taken_once() {
        let activation = Activation::new();
        assert!(!activation.take_show());
        activation.request_show();
        assert!(activation.take_show());
        assert!(!activation.take_show());
    }

    #[derive(Clone)]
    struct Hits(Arc<Mutex<Vec<(String, serde_json::Value)>>>);

    async fn record(
        State(hits): State<Hits>,
        request: axum::extract::Request,
    ) -> axum::http::StatusCode {
        let path = request.uri().path().to_string();
        let bytes = axum::body::to_bytes(request.into_body(), 64 * 1024)
            .await
            .expect("body");
        let json = serde_json::from_slice(&bytes).unwrap_or_else(|_| json!({}));
        hits.0.lock().expect("hits").push((path, json));
        axum::http::StatusCode::OK
    }

    #[tokio::test]
    async fn handoff_posts_tasks_then_asks_to_show_the_window() {
        let hits = Hits(Arc::new(Mutex::new(Vec::new())));
        let app = Router::new()
            .route("/v1/tasks", post(record))
            .route("/v1/invoke", post(record))
            .with_state(hits.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("handoff listener");
        let port = listener.local_addr().expect("addr").port();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("handoff server");
        });

        tokio::task::spawn_blocking(move || {
            forward_args(
                port,
                &args(&[
                    "http://example.com/from-cli.bin",
                    "dltool://add?url=http%3A%2F%2Fhost%2Fd%2Ft&fileName=a.zip&size=9",
                ]),
            )
        })
        .await
        .expect("blocking handoff")
        .expect("forward");

        let recorded = hits.0.lock().expect("hits").clone();
        assert_eq!(recorded.len(), 3);
        assert_eq!(recorded[0].0, "/v1/tasks");
        assert_eq!(recorded[0].1["url"], "http://example.com/from-cli.bin");
        assert_eq!(recorded[0].1["fileName"], "from-cli.bin");
        assert!(recorded[0].1.get("size").is_none());
        assert_eq!(recorded[1].0, "/v1/tasks");
        assert_eq!(recorded[1].1["url"], "http://host/d/t");
        assert_eq!(recorded[1].1["fileName"], "a.zip");
        assert_eq!(recorded[1].1["size"], 9);
        assert_eq!(recorded[2].0, "/v1/invoke");
        server.abort();
    }

    #[tokio::test]
    async fn handoff_with_no_download_args_only_asks_to_show_the_window() {
        let hits = Hits(Arc::new(Mutex::new(Vec::new())));
        let app = Router::new()
            .route("/v1/invoke", post(record))
            .with_state(hits.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("show listener");
        let port = listener.local_addr().expect("addr").port();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("show server");
        });

        tokio::task::spawn_blocking(move || forward_args(port, &args(&["--ignored", "readme"])))
            .await
            .expect("join")
            .expect("forward");

        let recorded = hits.0.lock().expect("hits").clone();
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].0, "/v1/invoke");
        server.abort();
    }

    #[test]
    fn handoff_reports_when_nothing_is_listening() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        drop(listener);
        let err = forward_args_timeout(port, &[], Duration::from_millis(250)).unwrap_err();
        assert!(err.contains("could not reach"), "{err}");
        assert!(err.contains(&port.to_string()), "{err}");
    }
}
