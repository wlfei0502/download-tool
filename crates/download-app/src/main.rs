mod ui;

use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use download_core::aria2::process::Aria2Process;
use download_core::aria2::rpc::Aria2Client;
use download_core::engine::DownloadEngine;
use download_core::manager::TaskManager;
use download_core::task::{NewTaskRequest, TaskId, TaskStatus};
use download_local_api::{app, AppState, DEFAULT_PORT};
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
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("start tokio runtime")?;
    match runtime.block_on(prepare()) {
        Ok(app) => ui::run(app).map_err(|err| anyhow::anyhow!("open window: {err}"))?,
        Err(message) => {
            eprintln!("{message}");
            if let Err(err) = ui::run_error(&message) {
                eprintln!("failed to open error window: {err}");
            }
            std::process::exit(1);
        }
    }
    Ok(())
}

async fn prepare() -> Result<DownloadUi, String> {
    let save_dir = default_save_dir();
    let save_dir_text = save_dir.to_string_lossy().to_string();
    let tasks = Arc::new(Mutex::new(TaskManager::new()));
    let (child, engine, banner) = start_backend(&save_dir, Arc::clone(&tasks)).await;
    if let Some(banner) = &banner {
        eprintln!("{banner}");
    }

    // Dropping this guard kills aria2 if binding the API fails.
    let mut aria2 = StopChild(child);
    let listener = bind_api().await?;
    eprintln!("listening on http://127.0.0.1:{DEFAULT_PORT}");

    let router = app(AppState {
        tasks: Arc::clone(&tasks),
    });
    let server = tokio::spawn(async move {
        if let Err(err) = axum::serve(listener, router).await {
            eprintln!("local API stopped: {err}");
        }
    });

    let (commands_tx, commands_rx) = mpsc::unbounded_channel();
    let model = SharedModel::new();
    let engine_dir = save_dir_text.clone();
    tokio::spawn(run_engine(
        engine,
        tasks,
        commands_rx,
        model.clone(),
        engine_dir,
    ));

    Ok(DownloadUi::new(
        save_dir_text,
        banner,
        model,
        commands_tx,
        server,
        aria2.0.take(),
    ))
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
    save_dir: &Path,
    tasks: Arc<Mutex<TaskManager>>,
) -> (
    Option<Child>,
    Option<DownloadEngine<Aria2Client>>,
    Option<String>,
) {
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
    let (mut child, client) = match Aria2Process::start(port, &secret, save_dir) {
        Ok(started) => started,
        Err(err) => {
            return (
                None,
                None,
                Some(format!(
                    "aria2c is not available ({err}). Tasks can still be queued; downloads will not start."
                )),
            )
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
    let engine = DownloadEngine::shared(tasks, client, save_dir.to_string_lossy());
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
    mut save_dir: String,
) {
    let mut ticker = tokio::time::interval(Duration::from_millis(500));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            command = commands.recv() => {
                let Some(command) = command else {
                    break;
                };
                handle_command(&mut engine, &tasks, &mut save_dir, command, &model).await;
                publish(&tasks, &model).await;
            }
            _ = ticker.tick() => {
                if let Some(engine) = engine.as_mut() {
                    start_queued(engine, &save_dir, &model).await;
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
    save_dir: &mut String,
    command: Command,
    model: &SharedModel,
) {
    match command {
        Command::SetSaveDir(dir) => {
            apply_save_dir(engine, save_dir, &dir, model);
        }
        Command::Add {
            url,
            file_name,
            save_dir: dir,
        } => {
            add_task(engine, tasks, save_dir, dir, url, file_name, model).await;
        }
        Command::Pause(id) => control(engine, tasks, model, &id, Control::Pause).await,
        Command::Resume(id) => control(engine, tasks, model, &id, Control::Resume).await,
        Command::Cancel(id) => control(engine, tasks, model, &id, Control::Cancel).await,
    }
}

fn apply_save_dir(
    engine: &mut Option<DownloadEngine<Aria2Client>>,
    save_dir: &mut String,
    dir: &str,
    model: &SharedModel,
) {
    if dir.is_empty() {
        model.set_notice(Some("save directory required".into()));
        return;
    }
    if let Err(err) = std::fs::create_dir_all(dir) {
        model.set_notice(Some(format!("cannot create save directory: {err}")));
        return;
    }
    *save_dir = dir.to_string();
    if let Some(engine) = engine.as_mut() {
        engine.set_save_dir(dir);
    }
}

async fn add_task(
    engine: &mut Option<DownloadEngine<Aria2Client>>,
    tasks: &Arc<Mutex<TaskManager>>,
    save_dir: &mut String,
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
    let Some(engine) = engine.as_mut() else {
        *save_dir = dir;
        match tasks.lock().await.add(request) {
            Ok(_) => model.set_notice(Some(
                "Queued. aria2c is not available, so the download has not started.".into(),
            )),
            Err(err) => model.set_notice(Some(err)),
        }
        return;
    };
    if let Err(err) = std::fs::create_dir_all(&dir) {
        model.set_notice(Some(format!("cannot create save directory: {err}")));
        return;
    }
    *save_dir = dir.clone();
    engine.set_save_dir(&dir);
    match engine.submit(request).await {
        Ok(_) => model.set_notice(None),
        Err(err) => model.set_notice(Some(err)),
    }
}

enum Control {
    Pause,
    Resume,
    Cancel,
}

async fn control(
    engine: &mut Option<DownloadEngine<Aria2Client>>,
    tasks: &Arc<Mutex<TaskManager>>,
    model: &SharedModel,
    id: &TaskId,
    action: Control,
) {
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

    use super::{api_listen_addr, bind_api, retry_until_rpc_ready};
    use download_local_api::DEFAULT_PORT;

    #[test]
    fn api_listens_on_loopback_default_port_only() {
        let addr = api_listen_addr();
        assert_eq!(addr.ip().to_string(), "127.0.0.1");
        assert!(addr.ip().is_loopback());
        assert_eq!(addr.port(), DEFAULT_PORT);
        assert_eq!(DEFAULT_PORT, 29128);
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
}
