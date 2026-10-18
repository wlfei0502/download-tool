mod platform;
mod single_instance;
mod tray;
mod ui;

use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use download_core::aria2::process::Aria2Process;
use download_core::aria2::rpc::Aria2Client;
use download_core::engine::DownloadEngine;
use download_core::manager::TaskManager;
use download_core::settings::Settings;
use download_core::task::{NewTaskRequest, TaskId, TaskStatus};
use download_local_api::{app, AppState, DEFAULT_PORT};
use single_instance::Activation;
use tokio::net::TcpListener;
use tokio::sync::{mpsc, Mutex};
use ui::{Command, DownloadUi, SharedModel};

const RPC_READY_ATTEMPTS: u32 = 100;
const RPC_READY_INTERVAL: Duration = Duration::from_millis(100);

fn main() {
    let code = match try_main() {
        Ok(()) => 0,
        Err(err) => {
            eprintln!("{err:#}");
            1
        }
    };
    if code != 0 {
        std::process::exit(code);
    }
}

fn try_main() -> anyhow::Result<()> {
    let instance = match single_instance::claim().map_err(|err| anyhow::anyhow!(err))? {
        single_instance::Launch::Secondary => {
            // The running process owns the window. Hand off and exit 0 even
            // when the local API does not answer, so a second window is not opened.
            if let Err(err) = single_instance::forward_current_args() {
                eprintln!("could not forward to the running Download Tool: {err}");
            }
            return Ok(());
        }
        single_instance::Launch::Primary(instance) => instance,
    };

    match std::env::current_exe() {
        Ok(exe) => {
            if let Err(err) = platform::register_protocol(&exe) {
                eprintln!("dltool:// protocol was not registered: {err}");
            }
        }
        Err(err) => eprintln!("dltool:// protocol was not registered: {err}"),
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("start tokio runtime")?;
    let activation = Activation::new();
    match runtime.block_on(prepare(activation)) {
        Ok(mut app) => {
            app.hold_instance(instance);
            ui::run(app).map_err(|err| anyhow::anyhow!("open window: {err}"))?;
        }
        Err(message) => {
            eprintln!("{message}");
            if let Err(err) = ui::run_error(&message, instance) {
                eprintln!("failed to open error window: {err}");
            }
            std::process::exit(1);
        }
    }
    Ok(())
}

async fn prepare(activation: Activation) -> Result<DownloadUi, String> {
    let stored = load_settings();
    if let Err(err) = sync_autostart(stored.values.autostart) {
        eprintln!("autostart was not updated: {err}");
    }
    let tasks = Arc::new(Mutex::new(TaskManager::new()));
    let (child, mut engine, banner) = start_backend(&stored.values, Arc::clone(&tasks)).await;
    if let Some(banner) = &banner {
        eprintln!("{banner}");
    }

    // Dropping this guard kills aria2 if binding the API fails.
    let mut aria2 = StopChild(child);
    let listener = bind_api().await?;
    eprintln!("listening on http://127.0.0.1:{DEFAULT_PORT}");

    let router = api_router(Arc::clone(&tasks), activation.clone());
    let server = tokio::spawn(async move {
        if let Err(err) = axum::serve(listener, router).await {
            eprintln!("local API stopped: {err}");
        }
    });

    let (commands_tx, commands_rx) = mpsc::unbounded_channel();
    let model = SharedModel::new();
    let startup: Vec<String> = std::env::args().skip(1).collect();
    enqueue_requests(
        &mut engine,
        &tasks,
        single_instance::requests_from_args(&startup),
    )
    .await;
    let settings_for_ui = stored.values.clone();
    tokio::spawn(run_engine(
        engine,
        tasks,
        commands_rx,
        model.clone(),
        stored,
    ));

    Ok(DownloadUi::new(
        settings_for_ui,
        banner,
        model,
        commands_tx,
        server,
        aria2.0.take(),
        activation,
    ))
}

fn api_router(tasks: Arc<Mutex<TaskManager>>, activation: Activation) -> Router {
    app(AppState { tasks }).merge(
        Router::new()
            .route("/v1/invoke", post(invoke))
            .with_state(activation),
    )
}

async fn invoke(State(activation): State<Activation>) -> Json<serde_json::Value> {
    activation.request_show();
    Json(serde_json::json!({ "ok": true }))
}

fn api_listen_addr() -> std::net::SocketAddr {
    std::net::SocketAddr::from(([127, 0, 0, 1], DEFAULT_PORT))
}

async fn bind_api() -> Result<TcpListener, String> {
    let addr = api_listen_addr();
    match TcpListener::bind(addr).await {
        Ok(listener) => Ok(listener),
        Err(err) if err.kind() == std::io::ErrorKind::AddrInUse => Err(format!(
            "127.0.0.1:{DEFAULT_PORT} is already in use. \
             Quit the other Download Tool process and start again."
        )),
        Err(err) => Err(format!("failed to bind 127.0.0.1:{DEFAULT_PORT}: {err}")),
    }
}

struct StopChild(Option<Child>);

impl Drop for StopChild {
    fn drop(&mut self) {
        if let Some(child) = self.0.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

async fn start_backend(
    settings: &Settings,
    tasks: Arc<Mutex<TaskManager>>,
) -> (
    Option<Child>,
    Option<DownloadEngine<Aria2Client>>,
    Option<String>,
) {
    let save_dir = Path::new(&settings.save_dir);
    if let Err(err) = std::fs::create_dir_all(save_dir) {
        return (
            None,
            None,
            Some(format!(
                "aria2c skipped: cannot create save directory {}: {err}",
                save_dir.display()
            )),
        );
    }

    let port = match pick_rpc_port() {
        Ok(port) => port,
        Err(err) => return (None, None, Some(format!("aria2c skipped: {err}"))),
    };
    let secret = rpc_secret();
    let (mut child, client) = match Aria2Process::start(
        port,
        &secret,
        save_dir,
        settings.max_connections_per_server,
    ) {
        Ok(started) => started,
        Err(err) => {
            return (
                None,
                None,
                Some(format!(
                    "aria2c is not available ({err}). Tasks can still be queued; downloads will not start."
                )),
            );
        }
    };

    if let Err(err) = wait_for_rpc(&client).await {
        let _ = child.kill();
        let _ = child.wait();
        return (
            None,
            None,
            Some(format!(
                "aria2c started but JSON-RPC did not become ready ({err}). Downloads will not start."
            )),
        );
    }

    eprintln!("aria2 JSON-RPC ready on 127.0.0.1:{port}");
    let mut engine = DownloadEngine::shared(tasks, client, save_dir.to_string_lossy());
    engine.set_max_connections_per_server(settings.max_connections_per_server);
    if let Err(err) = engine
        .set_global_speed_limit(settings.global_speed_limit_bps)
        .await
    {
        return (
            Some(child),
            Some(engine),
            Some(format!(
                "global speed limit was not applied ({err}). Downloads will run without that limit."
            )),
        );
    }
    (Some(child), Some(engine), None)
}

fn pick_rpc_port() -> Result<u16, String> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")
        .map_err(|err| format!("pick aria2 RPC port: {err}"))?;
    listener
        .local_addr()
        .map(|addr| addr.port())
        .map_err(|err| format!("pick aria2 RPC port: {err}"))
}

fn rpc_secret() -> String {
    let tick = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    format!("dltool-{tick:x}-{}", std::process::id())
}

/// Retry `aria2.getVersion` until JSON-RPC accepts calls.
///
/// `Aria2Process::start` returns as soon as the process is spawned.
async fn wait_for_rpc(client: &Aria2Client) -> Result<(), String> {
    retry_until_rpc_ready(client, RPC_READY_ATTEMPTS, RPC_READY_INTERVAL).await
}

async fn retry_until_rpc_ready(
    client: &Aria2Client,
    attempts: u32,
    interval: Duration,
) -> Result<(), String> {
    let mut last_error = "no response".to_string();
    for attempt in 0..attempts {
        match client.get_version().await {
            Ok(_) => return Ok(()),
            Err(err) => {
                last_error = err;
                if attempt + 1 < attempts {
                    tokio::time::sleep(interval).await;
                }
            }
        }
    }
    Err(last_error)
}

struct StoredSettings {
    values: Settings,
    path: Option<PathBuf>,
}

impl StoredSettings {
    fn save(&self) -> Result<(), String> {
        match &self.path {
            Some(path) => self.values.save(path),
            None => Ok(()),
        }
    }
}

fn load_settings() -> StoredSettings {
    let path = match Settings::config_file() {
        Ok(path) => path,
        Err(err) => {
            eprintln!("could not resolve settings path: {err}");
            return StoredSettings {
                values: with_default_save_dir(Settings::default()),
                path: None,
            };
        }
    };
    let existed = path.is_file();
    match Settings::load(&path) {
        Ok(loaded) => {
            let stored = StoredSettings {
                values: with_default_save_dir(loaded.clone()),
                path: Some(path),
            };
            if !existed || stored.values != loaded {
                if let Err(err) = stored.save() {
                    eprintln!("could not save settings: {err}");
                }
            }
            stored
        }
        Err(err) => {
            eprintln!("could not load settings {}: {err}", path.display());
            // Keep the path so a later edit can replace the corrupt file.
            // Do not overwrite it during startup.
            StoredSettings {
                values: with_default_save_dir(Settings::default()),
                path: Some(path),
            }
        }
    }
}

fn with_default_save_dir(mut settings: Settings) -> Settings {
    if settings.save_dir.trim().is_empty() {
        settings.save_dir = default_save_dir().to_string_lossy().into_owned();
    }
    settings
}

fn sync_autostart(enabled: bool) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|err| format!("current executable: {err}"))?;
    platform::set_autostart(enabled, &exe)
}

fn default_save_dir() -> PathBuf {
    if let Some(home) = std::env::var_os("HOME") {
        let downloads = PathBuf::from(home).join("Downloads");
        if downloads.is_dir() {
            return downloads;
        }
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

async fn run_engine(
    mut engine: Option<DownloadEngine<Aria2Client>>,
    tasks: Arc<Mutex<TaskManager>>,
    mut commands: mpsc::UnboundedReceiver<Command>,
    model: SharedModel,
    mut stored: StoredSettings,
) {
    let mut ticker = tokio::time::interval(Duration::from_millis(500));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            command = commands.recv() => {
                let Some(command) = command else {
                    break;
                };
                handle_command(&mut engine, &tasks, &mut stored, command, &model).await;
                publish(&tasks, &model).await;
            }
            _ = ticker.tick() => {
                if let Some(engine) = engine.as_mut() {
                    start_queued(engine, &stored.values.save_dir, &model).await;
                    if let Err(err) = engine.poll_once().await {
                        model.set_notice(Some(err));
                    }
                }
                publish(&tasks, &model).await;
            }
        }
    }
}

async fn handle_command(
    engine: &mut Option<DownloadEngine<Aria2Client>>,
    tasks: &Arc<Mutex<TaskManager>>,
    stored: &mut StoredSettings,
    command: Command,
    model: &SharedModel,
) {
    match command {
        Command::UpdateSettings(next) => {
            apply_settings(engine, stored, next, model).await;
        }
        Command::Add {
            url,
            file_name,
            save_dir: dir,
        } => {
            add_task(engine, tasks, stored, dir, url, file_name, model).await;
        }
        Command::Pause(id) => control(engine, tasks, model, &id, Control::Pause).await,
        Command::Resume(id) => control(engine, tasks, model, &id, Control::Resume).await,
        Command::Cancel(id) => control(engine, tasks, model, &id, Control::Cancel).await,
        Command::PauseAll => pause_all(engine, tasks, model).await,
    }
}

async fn apply_settings(
    engine: &mut Option<DownloadEngine<Aria2Client>>,
    stored: &mut StoredSettings,
    next: Settings,
    model: &SharedModel,
) {
    let next = next.normalized();
    if next.save_dir.trim().is_empty() {
        model.set_notice(Some("save directory required".into()));
        return;
    }
    if let Err(err) = std::fs::create_dir_all(&next.save_dir) {
        model.set_notice(Some(format!("cannot create save directory: {err}")));
        return;
    }
    let autostart_changed = stored.values.autostart != next.autostart;
    stored.values = next;
    let mut notice = None;
    if let Err(err) = stored.save() {
        notice = Some(format!("could not save settings: {err}"));
    }
    if let Some(engine) = engine.as_mut() {
        engine.set_save_dir(&stored.values.save_dir);
        engine.set_max_connections_per_server(stored.values.max_connections_per_server);
        if let Err(err) = engine
            .set_global_speed_limit(stored.values.global_speed_limit_bps)
            .await
        {
            notice = Some(format!("speed limit was not applied: {err}"));
        }
    }
    if autostart_changed {
        if let Err(err) = sync_autostart(stored.values.autostart) {
            notice = Some(format!("autostart was not updated: {err}"));
        }
    }
    model.set_notice(notice);
}

async fn add_task(
    engine: &mut Option<DownloadEngine<Aria2Client>>,
    tasks: &Arc<Mutex<TaskManager>>,
    stored: &mut StoredSettings,
    dir: String,
    url: String,
    file_name: String,
    model: &SharedModel,
) {
    if let Err(err) = ui::validate_task_input(&url, &file_name) {
        model.set_notice(Some(err));
        return;
    }
    if dir.is_empty() {
        model.set_notice(Some("save directory required".into()));
        return;
    }
    let request = NewTaskRequest {
        url,
        file_name,
        size: None,
        referrer: None,
        user_agent: None,
    };
    if let Err(err) = std::fs::create_dir_all(&dir) {
        model.set_notice(Some(format!("cannot create save directory: {err}")));
        return;
    }
    stored.values.save_dir = dir.clone();
    let save_error = stored
        .save()
        .err()
        .map(|err| format!("could not save settings: {err}"));
    let Some(engine) = engine.as_mut() else {
        match tasks.lock().await.add(request) {
            Ok(_) => model.set_notice(Some(save_error.unwrap_or_else(|| {
                "Queued. aria2c is not available, so the download has not started.".into()
            }))),
            Err(err) => model.set_notice(Some(err)),
        }
        return;
    };
    engine.set_save_dir(&dir);
    match engine.submit(request).await {
        Ok(_) => model.set_notice(save_error),
        Err(err) => model.set_notice(Some(err)),
    }
}

enum Control {
    Pause,
    Resume,
    Cancel,
}

async fn pause_all(
    engine: &mut Option<DownloadEngine<Aria2Client>>,
    tasks: &Arc<Mutex<TaskManager>>,
    model: &SharedModel,
) {
    let targets = {
        let manager = tasks.lock().await;
        tray::pause_all_ids(&manager.list())
    };
    if targets.is_empty() {
        return;
    }
    let mut first_error = None;
    for id in targets {
        if let Err(err) = pause_one(engine, tasks, &id).await {
            first_error.get_or_insert(err);
        }
    }
    model.set_notice(first_error);
}

async fn pause_one(
    engine: &mut Option<DownloadEngine<Aria2Client>>,
    tasks: &Arc<Mutex<TaskManager>>,
    id: &TaskId,
) -> Result<(), String> {
    let gid = task_gid(tasks, id).await;
    if gid.is_some() {
        if let Some(engine) = engine.as_mut() {
            return engine.pause_task(id).await;
        }
    }
    tasks.lock().await.set_status(id, TaskStatus::Paused)
}

async fn task_gid(tasks: &Arc<Mutex<TaskManager>>, id: &TaskId) -> Option<String> {
    tasks.lock().await.get(id).and_then(|task| task.gid.clone())
}

async fn control(
    engine: &mut Option<DownloadEngine<Aria2Client>>,
    tasks: &Arc<Mutex<TaskManager>>,
    model: &SharedModel,
    id: &TaskId,
    action: Control,
) {
    // A paused task that never received a gid goes back to the queue. Resume
    // would otherwise fail with "task has no gid", so Pause all could not be undone.
    if matches!(action, Control::Resume) && task_gid(tasks, id).await.is_none() {
        match tasks.lock().await.set_status(id, TaskStatus::Queued) {
            Ok(()) => model.set_notice(None),
            Err(err) => model.set_notice(Some(err)),
        }
        return;
    }

    let Some(engine) = engine.as_mut() else {
        if let Control::Cancel = action {
            match tasks.lock().await.remove(id) {
                Ok(()) => model.set_notice(None),
                Err(err) => model.set_notice(Some(err)),
            }
            return;
        }
        model.set_notice(Some("aria2c is not available".into()));
        return;
    };
    let result = match action {
        Control::Pause => engine.pause_task(id).await,
        Control::Resume => engine.resume_task(id).await,
        Control::Cancel => engine.cancel_task(id).await,
    };
    match result {
        Ok(()) => model.set_notice(None),
        Err(err) => model.set_notice(Some(err)),
    }
}

async fn enqueue_requests(
    engine: &mut Option<DownloadEngine<Aria2Client>>,
    tasks: &Arc<Mutex<TaskManager>>,
    requests: Vec<NewTaskRequest>,
) {
    for request in requests {
        let label = request.file_name.clone();
        let result = if let Some(engine) = engine.as_mut() {
            engine.submit(request).await.map(|_| ())
        } else {
            tasks.lock().await.add(request).map(|_| ())
        };
        if let Err(err) = result {
            eprintln!("could not enqueue {label}: {err}");
        }
    }
}

async fn start_queued(
    engine: &mut DownloadEngine<Aria2Client>,
    save_dir: &str,
    model: &SharedModel,
) {
    let pending: Vec<NewTaskRequest> = {
        let manager = engine.manager().await;
        manager
            .list()
            .into_iter()
            .filter(|task| task.status == TaskStatus::Queued && task.gid.is_none())
            .map(|task| NewTaskRequest {
                url: task.url,
                file_name: task.file_name,
                size: task.size,
                referrer: task.referrer,
                user_agent: task.user_agent,
            })
            .collect()
    };
    if pending.is_empty() {
        return;
    }
    if let Err(err) = std::fs::create_dir_all(save_dir) {
        model.set_notice(Some(format!("cannot create save directory: {err}")));
        return;
    }
    for request in pending {
        if let Err(err) = engine.submit(request).await {
            model.set_notice(Some(err));
        }
    }
}

async fn publish(tasks: &Arc<Mutex<TaskManager>>, model: &SharedModel) {
    let listed = tasks.lock().await.list();
    model.set_tasks(listed);
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    use axum::extract::State;
    use axum::routing::post;
    use axum::Router;
    use download_core::aria2::rpc::Aria2Client;

    use super::{
        api_listen_addr, api_router, apply_settings, bind_api, control, enqueue_requests,
        pause_all, retry_until_rpc_ready, Control, StoredSettings,
    };
    use crate::single_instance::Activation;
    use download_core::manager::TaskManager;
    use download_core::task::{NewTaskRequest, TaskStatus};
    use download_local_api::DEFAULT_PORT;
    use tokio::sync::Mutex;

    #[test]
    fn api_listens_on_loopback_default_port_only() {
        let addr = api_listen_addr();
        assert_eq!(addr.ip().to_string(), "127.0.0.1");
        assert!(addr.ip().is_loopback());
        assert_eq!(addr.port(), DEFAULT_PORT);
        assert_eq!(DEFAULT_PORT, 29128);
    }

    #[tokio::test]
    async fn invoke_asks_the_existing_window_to_show() {
        let activation = Activation::new();
        let router = api_router(Arc::new(Mutex::new(TaskManager::new())), activation.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("invoke listener");
        let addr = listener.local_addr().expect("invoke addr");
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.expect("invoke server");
        });
        for _ in 0..50 {
            if tokio::net::TcpStream::connect(addr).await.is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        let response = tokio::task::spawn_blocking(move || {
            use std::io::{Read, Write};
            let mut client = std::net::TcpStream::connect(addr).expect("connect invoke");
            client
                .set_read_timeout(Some(Duration::from_secs(2)))
                .expect("read timeout");
            client
                .set_write_timeout(Some(Duration::from_secs(2)))
                .expect("write timeout");
            client
                .write_all(b"POST /v1/invoke HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}")
                .expect("write invoke");
            let mut response = String::new();
            client.read_to_string(&mut response).expect("read invoke");
            response
        })
        .await
        .expect("invoke client");
        assert!(response.contains("200"), "{response}");
        assert!(response.contains("\"ok\":true"), "{response}");
        assert!(activation.take_show());
        server.abort();
    }

    #[tokio::test]
    async fn bind_api_reports_when_port_is_taken() {
        let addr = api_listen_addr();
        let hold = match std::net::TcpListener::bind(addr) {
            Ok(listener) => listener,
            Err(err) if err.kind() == std::io::ErrorKind::AddrInUse => {
                let message = bind_api().await.expect_err("port already taken");
                assert!(message.contains("127.0.0.1:29128"));
                assert!(message.contains("already in use"));
                return;
            }
            Err(err) => panic!("bind loopback port: {err}"),
        };
        let message = bind_api().await.expect_err("second bind");
        assert!(message.contains("127.0.0.1:29128"));
        assert!(message.contains("already in use"));
        drop(hold);
    }

    #[derive(Clone)]
    struct RpcGate {
        hits: Arc<AtomicUsize>,
        fail_first: usize,
    }

    async fn rpc_gate(
        State(gate): State<RpcGate>,
    ) -> ([(axum::http::HeaderName, &'static str); 1], String) {
        let n = gate.hits.fetch_add(1, Ordering::SeqCst);
        let body = if n < gate.fail_first {
            r#"{"jsonrpc":"2.0","id":"1","error":{"code":1,"message":"not ready"}}"#
        } else {
            r#"{"jsonrpc":"2.0","id":"1","result":{"version":"1.37.0"}}"#
        };
        (
            [(axum::http::header::CONTENT_TYPE, "application/json")],
            body.to_string(),
        )
    }

    async fn spawn_rpc(
        fail_first: usize,
    ) -> (String, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
        let hits = Arc::new(AtomicUsize::new(0));
        let app = Router::new()
            .route("/jsonrpc", post(rpc_gate))
            .with_state(RpcGate {
                hits: Arc::clone(&hits),
                fail_first,
            });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("rpc test listener");
        let addr = listener.local_addr().expect("rpc test addr");
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("rpc test server");
        });
        let url = format!("http://{addr}/jsonrpc");
        for _ in 0..50 {
            if tokio::net::TcpStream::connect(addr).await.is_ok() {
                return (url, hits, server);
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("jsonrpc test server did not accept");
    }

    #[tokio::test]
    async fn rpc_ready_retries_until_get_version_succeeds() {
        let (url, hits, server) = spawn_rpc(2).await;
        let client = Aria2Client::new(url, Some("secret".into()));
        tokio::time::timeout(
            Duration::from_secs(5),
            retry_until_rpc_ready(&client, 10, Duration::from_millis(1)),
        )
        .await
        .expect("rpc ready timed out")
        .expect("rpc becomes ready");
        assert!(hits.load(Ordering::SeqCst) >= 3);
        server.abort();
    }

    #[tokio::test]
    async fn rpc_ready_returns_last_error_when_attempts_run_out() {
        let (url, hits, server) = spawn_rpc(usize::MAX).await;
        let client = Aria2Client::new(url, None);
        let err = tokio::time::timeout(
            Duration::from_secs(5),
            retry_until_rpc_ready(&client, 3, Duration::from_millis(1)),
        )
        .await
        .expect("rpc failure timed out")
        .expect_err("never ready");
        assert!(err.contains("not ready"), "{err}");
        assert_eq!(hits.load(Ordering::SeqCst), 3);
        server.abort();
    }

    #[tokio::test]
    async fn pause_all_without_engine_pauses_open_tasks_and_resume_requeues() {
        let tasks = Arc::new(Mutex::new(TaskManager::new()));
        let queued = tasks
            .lock()
            .await
            .add(request("http://example.com/queued.zip", "queued.zip"))
            .expect("queue");
        let active = tasks
            .lock()
            .await
            .add(request("http://example.com/active.zip", "active.zip"))
            .expect("active");
        {
            let mut manager = tasks.lock().await;
            manager
                .set_status(&active, TaskStatus::Active)
                .expect("mark active");
            let done = manager
                .add(request("http://example.com/done.zip", "done.zip"))
                .expect("done");
            manager
                .set_status(&done, TaskStatus::Complete)
                .expect("mark complete");
        }

        let model = crate::ui::SharedModel::new();
        let mut engine = None;
        pause_all(&mut engine, &tasks, &model).await;

        let manager = tasks.lock().await;
        assert_eq!(
            manager.get(&queued).expect("queued").status,
            TaskStatus::Paused
        );
        assert_eq!(
            manager.get(&active).expect("active").status,
            TaskStatus::Paused
        );
        assert!(manager
            .list()
            .iter()
            .any(|task| task.status == TaskStatus::Complete));
        drop(manager);

        control(&mut engine, &tasks, &model, &queued, Control::Resume).await;
        assert_eq!(
            tasks.lock().await.get(&queued).expect("requeued").status,
            TaskStatus::Queued
        );
    }

    #[tokio::test]
    async fn startup_urls_are_queued_when_aria2_is_absent() {
        let tasks = Arc::new(Mutex::new(TaskManager::new()));
        let requests = crate::single_instance::requests_from_args(&[
            "dltool://add?url=http%3A%2F%2Fhost%2Fd%2Ft&fileName=a.zip&size=9".into(),
            "http://example.com/plain.bin".into(),
            "https://example.com/nope".into(),
        ]);
        let mut engine = None;
        enqueue_requests(&mut engine, &tasks, requests).await;
        let mut listed = tasks.lock().await.list();
        listed.sort_by(|left, right| left.file_name.cmp(&right.file_name));
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].file_name, "a.zip");
        assert_eq!(listed[0].url, "http://host/d/t");
        assert_eq!(listed[0].size, Some(9));
        assert_eq!(listed[1].file_name, "plain.bin");
        assert_eq!(listed[1].url, "http://example.com/plain.bin");
    }

    #[tokio::test]
    async fn update_settings_persists_save_dir_and_speed_limit() {
        let dir =
            std::env::temp_dir().join(format!("download-tool-settings-app-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("settings.json");
        let save_dir = dir.join("out");
        let mut stored = StoredSettings {
            values: download_core::settings::Settings {
                save_dir: dir.join("in").to_string_lossy().into_owned(),
                global_speed_limit_bps: None,
                max_connections_per_server: 8,
                autostart: false,
            },
            path: Some(path.clone()),
        };
        let model = crate::ui::SharedModel::new();
        let mut engine = None;
        apply_settings(
            &mut engine,
            &mut stored,
            download_core::settings::Settings {
                save_dir: save_dir.to_string_lossy().into_owned(),
                global_speed_limit_bps: Some(2048),
                max_connections_per_server: 4,
                autostart: false,
            },
            &model,
        )
        .await;

        let loaded = download_core::settings::Settings::load(&path).expect("load settings");
        assert_eq!(loaded.save_dir, save_dir.to_string_lossy());
        assert_eq!(loaded.global_speed_limit_bps, Some(2048));
        assert_eq!(loaded.max_connections_per_server, 4);
        assert!(!loaded.autostart);
        assert!(save_dir.is_dir());
        let (_, notice) = model.view();
        assert_eq!(notice, None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn request(url: &str, file_name: &str) -> NewTaskRequest {
        NewTaskRequest {
            url: url.into(),
            file_name: file_name.into(),
            size: None,
            referrer: None,
            user_agent: None,
        }
    }
}
