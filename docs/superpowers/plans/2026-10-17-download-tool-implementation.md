# Download Tool Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship a Rust+egui desktop downloader (aria2), a JS SDK that probes/invokes it, and integration docs/mock so websites can hand off large HTTP downloads.

**Architecture:** Workspace crates: `download-core` (tasks + aria2 RPC), `download-local-api` (127.0.0.1:29128), `download-app` (eframe/egui, tray, single-instance, `dltool://`). JS SDK lives in `sdk/`. Mock ticket server in `mock-backend/` for M4联调.

**Tech Stack:** Rust 2021, tokio, axum, serde, eframe/egui, tray-icon, single-instance, reqwest (aria2 JSON-RPC), embedded `aria2c` binaries under `vendor/aria2/<os-arch>/`, TypeScript SDK (or plain JS), Node mock server.

## Global Constraints

- Platforms: Windows + macOS + Linux
- Download transport to file server: **HTTP only** (not HTTPS, not FTP)
- Local API: bind **`127.0.0.1:29128` only**; no dynamic port in v1
- Custom protocol scheme: **`dltool`**
- Never accept or store server account passwords; only short-lived `downloadUrl`
- File identity on website side: opaque **`fileId`** (backend spec / mock only)
- Default size threshold for SDK: **100MB** (`104857600`), configurable
- Spec: `docs/superpowers/specs/2026-10-17-download-tool-design.md`

## Scope note

Spec has three deliverables (app, SDK, docs/mock). This plan sequences them M1→M5. After Task 10 the desktop path is usable; Tasks 11–13 are SDK/mock/packaging and can be a follow-up plan if you want a smaller PR.

## File structure (create as tasks proceed)

```
download-tool/
  Cargo.toml                          # workspace
  crates/
    download-core/
      Cargo.toml
      src/lib.rs
      src/task.rs                     # TaskId, TaskStatus, DownloadTask
      src/manager.rs                  # queue, dedupe, pause/resume/cancel
      src/aria2/mod.rs
      src/aria2/rpc.rs                # JSON-RPC client
      src/aria2/process.rs            # spawn/kill aria2c
      src/protocol.rs                 # parse dltool:// URLs
      src/settings.rs
      tests/...
    download-local-api/
      Cargo.toml
      src/lib.rs                      # axum router: /health, /v1/tasks
      tests/...
    download-app/
      Cargo.toml
      src/main.rs
      src/ui.rs                       # egui task list
      src/tray.rs
      src/single_instance.rs
      src/platform/mod.rs             # protocol registration helpers
  vendor/aria2/README.md              # drop aria2c binaries here
  sdk/
    package.json
    src/index.ts
    src/types.ts
    test/download-tool.test.ts
  mock-backend/
    package.json
    src/server.mjs
  examples/install-page/index.html
  docs/integration/backend-api.md     # copy/adapt from design §5
```

---

### Task 1: Cargo workspace + core task types

**Files:**
- Create: `Cargo.toml`
- Create: `crates/download-core/Cargo.toml`
- Create: `crates/download-core/src/lib.rs`
- Create: `crates/download-core/src/task.rs`
- Test: `crates/download-core/tests/task_status.rs`

**Interfaces:**
- Consumes: (none)
- Produces: `TaskId`, `TaskStatus`, `DownloadTask`, `NewTaskRequest`

- [ ] **Step 1: Write the failing test**

Create `crates/download-core/tests/task_status.rs`:

```rust
use download_core::task::{DownloadTask, NewTaskRequest, TaskStatus};

#[test]
fn new_task_starts_queued() {
    let req = NewTaskRequest {
        url: "http://example.com/d/abc".into(),
        file_name: "data.zip".into(),
        size: Some(1000),
        referrer: None,
        user_agent: None,
    };
    let task = DownloadTask::from_request(req);
    assert_eq!(task.status, TaskStatus::Queued);
    assert!(!task.id.0.is_empty());
    assert_eq!(task.file_name, "data.zip");
}
```

- [ ] **Step 2: Create workspace manifests (minimal so test can compile after impl)**

Root `Cargo.toml`:

```toml
[workspace]
resolver = "2"
members = ["crates/download-core"]
```

`crates/download-core/Cargo.toml`:

```toml
[package]
name = "download-core"
version = "0.1.0"
edition = "2021"

[dependencies]
serde = { version = "1", features = ["derive"] }
uuid = { version = "1", features = ["v4", "serde"] }
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p download-core --test task_status`

Expected: FAIL (crate/modules missing or `DownloadTask` not found)

- [ ] **Step 4: Implement types**

`crates/download-core/src/task.rs`:

```rust
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
```

`crates/download-core/src/lib.rs`:

```rust
pub mod task;
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p download-core --test task_status`

Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml crates/download-core
git commit -m "feat(core): add workspace and download task types"
```

---

### Task 2: TaskManager — queue, dedupe by URL

**Files:**
- Create: `crates/download-core/src/manager.rs`
- Modify: `crates/download-core/src/lib.rs`
- Test: `crates/download-core/tests/manager_dedupe.rs`

**Interfaces:**
- Consumes: `DownloadTask`, `NewTaskRequest`, `TaskId`, `TaskStatus`
- Produces: `TaskManager` with `add`, `list`, `get`, `set_status`, `find_by_url`

- [ ] **Step 1: Write the failing test**

```rust
use download_core::manager::TaskManager;
use download_core::task::NewTaskRequest;

#[test]
fn dedupes_same_url() {
    let mut mgr = TaskManager::new();
    let req = NewTaskRequest {
        url: "http://example.com/d/1".into(),
        file_name: "a.zip".into(),
        size: None,
        referrer: None,
        user_agent: None,
    };
    let a = mgr.add(req.clone()).expect("first");
    let b = mgr.add(req).expect("second");
    assert_eq!(a, b);
    assert_eq!(mgr.list().len(), 1);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p download-core --test manager_dedupe`

Expected: FAIL (`TaskManager` missing)

- [ ] **Step 3: Implement TaskManager**

```rust
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
        let t = self.tasks.get_mut(id).ok_or_else(|| "task not found".to_string())?;
        t.status = status;
        Ok(())
    }

    pub fn find_by_url(&self, url: &str) -> Option<&DownloadTask> {
        self.by_url.get(url).and_then(|id| self.tasks.get(id))
    }
}
```

Export in `lib.rs`: `pub mod manager;`

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p download-core --test manager_dedupe`

Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/download-core
git commit -m "feat(core): add TaskManager with URL dedupe"
```

---

### Task 3: Aria2 JSON-RPC client (test with wiremock / tiny mock)

**Files:**
- Create: `crates/download-core/src/aria2/mod.rs`
- Create: `crates/download-core/src/aria2/rpc.rs`
- Modify: `crates/download-core/Cargo.toml` (add `reqwest`, `serde_json`, `tokio`)
- Modify: `crates/download-core/src/lib.rs`
- Test: `crates/download-core/tests/aria2_rpc.rs`

**Interfaces:**
- Consumes: task URL / options
- Produces: `Aria2Client { endpoint }` with `add_uri`, `pause`, `unpause`, `remove`, `tell_status`

- [ ] **Step 1: Add dependencies**

```toml
reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls"] }
serde_json = "1"
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
```

Dev-deps for test:

```toml
[dev-dependencies]
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
wiremock = "0.6"
```

- [ ] **Step 2: Write the failing test**

```rust
use download_core::aria2::rpc::Aria2Client;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn add_uri_returns_gid() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/jsonrpc"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc": "2.0",
            "id": "1",
            "result": "2089b05ecca3d829"
        })))
        .mount(&server)
        .await;

    let client = Aria2Client::new(format!("{}/jsonrpc", server.uri()), None);
    let gid = client
        .add_uri("http://example.com/file.bin", "/tmp", "file.bin")
        .await
        .unwrap();
    assert_eq!(gid, "2089b05ecca3d829");
}
```

- [ ] **Step 3: Run test — expect FAIL**

Run: `cargo test -p download-core --test aria2_rpc`

- [ ] **Step 4: Implement RPC client**

`rpc.rs` (minimal):

```rust
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU64, Ordering};

pub struct Aria2Client {
    endpoint: String,
    secret: Option<String>,
    id_seq: AtomicU64,
    http: reqwest::Client,
}

impl Aria2Client {
    pub fn new(endpoint: String, secret: Option<String>) -> Self {
        Self {
            endpoint,
            secret,
            id_seq: AtomicU64::new(1),
            http: reqwest::Client::new(),
        }
    }

    async fn call(&self, method: &str, params: Vec<Value>) -> Result<Value, String> {
        let id = self.id_seq.fetch_add(1, Ordering::Relaxed).to_string();
        let mut full_params = Vec::new();
        if let Some(secret) = &self.secret {
            full_params.push(Value::String(format!("token:{}", secret)));
        }
        full_params.extend(params);
        let body = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": full_params
        });
        let resp = self
            .http
            .post(&self.endpoint)
            .json(&body)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        let v: Value = resp.json().await.map_err(|e| e.to_string())?;
        if let Some(err) = v.get("error") {
            return Err(err.to_string());
        }
        v.get("result").cloned().ok_or_else(|| "missing result".into())
    }

    pub async fn add_uri(
        &self,
        url: &str,
        dir: &str,
        out: &str,
    ) -> Result<String, String> {
        let options = json!({ "dir": dir, "out": out });
        let result = self
            .call("aria2.addUri", vec![json!([url]), options])
            .await?;
        result
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| "gid not string".into())
    }

    pub async fn pause(&self, gid: &str) -> Result<(), String> {
        self.call("aria2.pause", vec![json!(gid)]).await.map(|_| ())
    }

    pub async fn unpause(&self, gid: &str) -> Result<(), String> {
        self.call("aria2.unpause", vec![json!(gid)]).await.map(|_| ())
    }

    pub async fn remove(&self, gid: &str) -> Result<(), String> {
        self.call("aria2.remove", vec![json!(gid)]).await.map(|_| ())
    }

    pub async fn tell_status(&self, gid: &str) -> Result<Value, String> {
        self.call(
            "aria2.tellStatus",
            vec![
                json!(gid),
                json!(["status", "totalLength", "completedLength", "downloadSpeed"]),
            ],
        )
        .await
    }
}
```

`aria2/mod.rs`: `pub mod rpc;`  
`lib.rs`: `pub mod aria2;`

- [ ] **Step 5: Run test — expect PASS**

Run: `cargo test -p download-core --test aria2_rpc`

- [ ] **Step 6: Commit**

```bash
git add crates/download-core
git commit -m "feat(core): add aria2 JSON-RPC client"
```

---

### Task 4: Aria2 process host + binary path resolution

**Files:**
- Create: `crates/download-core/src/aria2/process.rs`
- Create: `vendor/aria2/README.md`
- Modify: `crates/download-core/src/aria2/mod.rs`
- Test: `crates/download-core/tests/aria2_binary_path.rs`

**Interfaces:**
- Consumes: path to `aria2c`, RPC secret, listen port for aria2 RPC
- Produces: `Aria2Process::start` → `(Child, Aria2Client)`, `resolve_aria2c_path()`

- [ ] **Step 1: Write failing test for path helper**

```rust
use download_core::aria2::process::resolve_aria2c_path;
use std::path::PathBuf;

#[test]
fn resolve_prefers_explicit_env() {
    std::env::set_var("DOWNLOAD_TOOL_ARIA2C", "/tmp/fake-aria2c");
    let p = resolve_aria2c_path(None).unwrap();
    assert_eq!(p, PathBuf::from("/tmp/fake-aria2c"));
    std::env::remove_var("DOWNLOAD_TOOL_ARIA2C");
}
```

- [ ] **Step 2: Implement `resolve_aria2c_path` and `Aria2Process`**

Resolution order: `DOWNLOAD_TOOL_ARIA2C` env → optional override arg → `vendor/aria2/<target-triple>/aria2c` (+ `.exe` on Windows) relative to executable/cwd.

`Aria2Process::start(rpc_port: u16, secret: &str, dir: &Path)` spawns:

```text
aria2c --enable-rpc --rpc-listen-port={port} --rpc-secret={secret}
  --rpc-listen-all=false --dir={dir} --continue=true
  --max-concurrent-downloads=5 --max-connection-per-server=8
```

Return client pointed at `http://127.0.0.1:{port}/jsonrpc`.

Document in `vendor/aria2/README.md` how to place official aria2 binaries (do not commit large binaries unless team agrees; CI can download).

- [ ] **Step 3: Unit-test path resolution; mark live spawn test `#[ignore]`**

```rust
#[tokio::test]
#[ignore = "requires vendor aria2c"]
async fn spawn_aria2_health() { /* start, tellGlobalStat or getVersion, kill */ }
```

Run: `cargo test -p download-core --test aria2_binary_path`

Expected: PASS for env test

- [ ] **Step 4: Commit**

```bash
git add crates/download-core vendor/aria2/README.md
git commit -m "feat(core): resolve and spawn aria2c process"
```

---

### Task 5: Local HTTP API — `/health` and `POST /v1/tasks`

**Files:**
- Create: `crates/download-local-api/Cargo.toml`
- Create: `crates/download-local-api/src/lib.rs`
- Modify: root `Cargo.toml` members
- Test: `crates/download-local-api/tests/api_health.rs`

**Interfaces:**
- Consumes: `Arc<Mutex<TaskManager>>` (or `tokio::sync::Mutex`)
- Produces: `router(state) -> axum::Router`, constant `DEFAULT_PORT: u16 = 29128`

- [ ] **Step 1: Write failing tests**

```rust
use axum::body::Body;
use axum::http::{Request, StatusCode};
use download_core::manager::TaskManager;
use download_local_api::{app, AppState};
use std::sync::Arc;
use tokio::sync::Mutex;
use tower::ServiceExt;

#[tokio::test]
async fn health_ok() {
    let state = AppState {
        tasks: Arc::new(Mutex::new(TaskManager::new())),
    };
    let app = app(state);
    let res = app
        .oneshot(Request::builder().uri("/health").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
}

#[tokio::test]
async fn post_task_returns_task_id() {
    let state = AppState {
        tasks: Arc::new(Mutex::new(TaskManager::new())),
    };
    let app = app(state);
    let body = serde_json::json!({
        "url": "http://example.com/d/x",
        "fileName": "x.bin",
        "size": 10
    });
    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/tasks")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
}
```

Note: API JSON uses **camelCase** `fileName` per design; map with serde `rename`.

- [ ] **Step 2: Implement axum app**

- `GET /health` → `{"ok":true,"version":"0.1.0"}`
- `POST /v1/tasks` → parse body → `TaskManager::add` → `{"taskId":"..."}`
- `GET /v1/tasks` → list tasks
- Bind helper later in app crate: `TcpListener::bind("127.0.0.1:29128")` only

CamelCase DTO:

```rust
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateTaskBody {
    url: String,
    file_name: String,
    size: Option<u64>,
    referrer: Option<String>,
    user_agent: Option<String>,
}
```

- [ ] **Step 3: Run tests — PASS**

Run: `cargo test -p download-local-api`

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml crates/download-local-api
git commit -m "feat(api): add local health and task endpoints"
```

---

### Task 6: Wire manager ↔ aria2 (start download on add)

**Files:**
- Create: `crates/download-core/src/engine.rs`
- Modify: `crates/download-core/src/lib.rs`
- Test: `crates/download-core/tests/engine_queue.rs` (mock Aria2 with a trait)

**Interfaces:**
- Consumes: `TaskManager`, `Aria2Client` (behind `trait Aria2Api`)
- Produces: `DownloadEngine::submit`, `poll_once`, `pause_task`, `resume_task`, `cancel_task`

- [ ] **Step 1: Introduce trait for testability**

```rust
#[async_trait::async_trait]
pub trait Aria2Api: Send + Sync {
    async fn add_uri(&self, url: &str, dir: &str, out: &str) -> Result<String, String>;
    async fn pause(&self, gid: &str) -> Result<(), String>;
    async fn unpause(&self, gid: &str) -> Result<(), String>;
    async fn remove(&self, gid: &str) -> Result<(), String>;
    async fn tell_status(&self, gid: &str) -> Result<serde_json::Value, String>;
}
```

Implement for `Aria2Client`. In tests, use a `FakeAria2` recording calls.

- [ ] **Step 2: `DownloadEngine` on submit**

- Look up save dir from settings
- `add_uri` → store `gid`, set status `Active`
- `poll_once`: for each active task, `tell_status`, update `completed_length` / `download_speed` / terminal states

- [ ] **Step 3: Tests with FakeAria2 — PASS**

- [ ] **Step 4: Commit**

```bash
git add crates/download-core
git commit -m "feat(core): wire TaskManager to aria2 via DownloadEngine"
```

---

### Task 7: egui app shell — list tasks + manual URL add

**Files:**
- Create: `crates/download-app/Cargo.toml`
- Create: `crates/download-app/src/main.rs`
- Create: `crates/download-app/src/ui.rs`
- Modify: workspace members

**Interfaces:**
- Consumes: `DownloadEngine`, `TaskManager`, local API in background tokio runtime
- Produces: runnable `download-tool` binary

- [ ] **Step 1: Scaffold eframe app**

Dependencies: `eframe`, `egui`, `download-core`, `download-local-api`, `tokio`, `anyhow`.

`main.rs`: spawn tokio runtime; start aria2 (or skip if missing and show banner); bind API on `127.0.0.1:29128` (on `AddrInUse`, show error dialog/text and exit); run egui loop.

- [ ] **Step 2: UI**

- Table/list: file name, status, progress, speed
- Buttons: Pause / Resume / Cancel
- Input: URL + file name + "Add" (manual path for M1)
- Settings field: download directory

- [ ] **Step 3: Manual verification**

Run: `DOWNLOAD_TOOL_ARIA2C=$(which aria2c) cargo run -p download-app`

Expected: window opens; adding an HTTP URL starts download when aria2 present

- [ ] **Step 4: Commit**

```bash
git add crates/download-app Cargo.toml
git commit -m "feat(app): egui shell with task list and manual add"
```

---

### Task 8: System tray + close-to-tray

**Files:**
- Create: `crates/download-app/src/tray.rs`
- Modify: `crates/download-app/src/main.rs`, `ui.rs`

- [ ] **Step 1: Add `tray-icon` (and platform deps as required by crate docs)**

- [ ] **Step 2: Implement tray menu: Show / Pause all / Quit**

Closing window hides visibility; Quit exits and stops aria2 child.

- [ ] **Step 3: Manual check on current OS — commit**

```bash
git add crates/download-app
git commit -m "feat(app): system tray and close-to-tray"
```

---

### Task 9: Single instance

**Files:**
- Create: `crates/download-app/src/single_instance.rs`
- Modify: `main.rs`

- [ ] **Step 1: Use `single-instance` crate with name `download-tool-dltool`**

If second process starts: forward CLI/`dltool://` args via localhost `POST /v1/tasks` (or a small `/v1/invoke` that accepts protocol string), then exit 0.

- [ ] **Step 2: Test manually — two launches → one window**

- [ ] **Step 3: Commit**

```bash
git add crates/download-app
git commit -m "feat(app): enforce single instance and forward tasks"
```

---

### Task 10: Parse `dltool://` and register protocol (per OS)

**Files:**
- Create: `crates/download-core/src/protocol.rs`
- Create: `crates/download-app/src/platform/mod.rs`
- Create: `crates/download-app/src/platform/windows.rs` (cfg)
- Create: `crates/download-app/src/platform/macos.rs` (cfg)
- Create: `crates/download-app/src/platform/linux.rs` (cfg)
- Test: `crates/download-core/tests/protocol_parse.rs`

**Interfaces:**
- Produces: `parse_dltool_url(s: &str) -> Result<NewTaskRequest, String>`
- Produces: `register_protocol(exe: &Path) -> Result<(), String>` (best-effort at first run / install)

- [ ] **Step 1: Failing parse tests**

```rust
use download_core::protocol::parse_dltool_url;

#[test]
fn parses_add() {
    let req = parse_dltool_url(
        "dltool://add?url=http%3A%2F%2Fhost%2Fd%2Ft&fileName=a.zip&size=9",
    )
    .unwrap();
    assert_eq!(req.url, "http://host/d/t");
    assert_eq!(req.file_name, "a.zip");
    assert_eq!(req.size, Some(9));
}
```

- [ ] **Step 2: Implement parser with `url` crate; reject non-`dltool` / non-`add`**

- [ ] **Step 3: Platform registration**

- Windows: registry `HKCU\Software\Classes\dltool\shell\open\command`
- macOS: `CFBundleURLTypes` in `Info.plist` (document bundling step; for `cargo run` use optional `lsregister` notes in README)
- Linux: `~/.local/share/applications/dltool.desktop` with `MimeType`/`x-scheme-handler/dltool` + `xdg-mime default`

- [ ] **Step 4: `main` accepts argv URL and enqueues**

- [ ] **Step 5: Commit**

```bash
git add crates/download-core crates/download-app
git commit -m "feat: parse dltool:// and register protocol handlers"
```

---

### Task 11: Settings — save dir, global speed limit, autostart flag

**Files:**
- Create: `crates/download-core/src/settings.rs`
- Modify: engine + UI
- Test: settings load/save roundtrip in temp dir

Persist JSON under OS config dir (`directories` crate): `download-tool/settings.json`.

Fields: `save_dir`, `global_speed_limit_bps: Option<u64>`, `max_connections_per_server: u8`, `autostart: bool`.

Apply limit via aria2 `aria2.changeGlobalOption` / `max-overall-download-limit`.

- [ ] Implement + test + commit: `feat(core): persistent settings and speed limit`

---

### Task 12: JS SDK

**Files:**
- Create: `sdk/package.json`
- Create: `sdk/src/types.ts`
- Create: `sdk/src/index.ts`
- Create: `sdk/test/download-tool.test.ts`
- Create: `sdk/README.md`

**Interfaces:**
- Produces: `download(options: DownloadOptions): Promise<DownloadResult>`

```ts
export type DownloadOptions = {
  fileId: string;
  size: number;
  apiBase: string; // website backend origin
  config?: {
    sizeThreshold?: number; // default 104857600
    installUrl?: string;
    localPort?: number; // default 29128
    protocol?: string; // default "dltool"
  };
  onNeedInstall?: () => void;
};

export type DownloadResult =
  | { mode: "browser" }
  | { mode: "local"; taskId: string }
  | { mode: "protocol" }
  | { mode: "install" };
```

Logic (exact order from spec):

1. If `size < sizeThreshold`: `POST ${apiBase}/api/download-ticket` → browser navigate/`<a download>` to `downloadUrl` → `{ mode: "browser" }`
2. Else: ticket → `fetch(http://127.0.0.1:${port}/health)` with short timeout (e.g. 400ms)
3. Health OK → `POST /v1/tasks` with camelCase body → `{ mode: "local", taskId }`
4. Else → `window.location.href = dltool://add?...` → `{ mode: "protocol" }`
5. Optional: after timeout without health, call `onNeedInstall` / `{ mode: "install" }`

- [ ] **Step 1: Unit tests with mocked `fetch` and `window`**

- [ ] **Step 2: Implement `src/index.ts`**

- [ ] **Step 3: `npm test` PASS**

- [ ] **Step 4: Commit**

```bash
git add sdk
git commit -m "feat(sdk): probe local API and invoke dltool protocol"
```

---

### Task 13: Mock backend + install page example + integration doc

**Files:**
- Create: `mock-backend/package.json`
- Create: `mock-backend/src/server.mjs`
- Create: `examples/install-page/index.html`
- Create: `docs/integration/backend-api.md`

**Mock behavior:**

- `GET /api/files` → sample tree with `fileId`s (no absolute paths)
- `POST /api/download-ticket` → `{ downloadUrl: "http://127.0.0.1:9090/d/"+token, ... }`
- `GET /d/:token` → stream a generated large file (or fixture), honor `Range`

Sign token with HMAC + `expiresAt`; reject expired.

- [ ] **Step 1: Implement mock; document run: `node mock-backend/src/server.mjs`**

- [ ] **Step 2: Static install page with links placeholders for Win/macOS/Linux**

- [ ] **Step 3: Write `docs/integration/backend-api.md` from design §5 (incl. X-Accel-Redirect + Range requirements)**

- [ ] **Step 4: Manual联调 checklist in that doc**

- [ ] **Step 5: Commit**

```bash
git add mock-backend examples/install-page docs/integration
git commit -m "docs: mock backend, install page example, integration API"
```

---

### Task 14: Packaging smoke scripts + README

**Files:**
- Create: `README.md` (root): build, vendor aria2, run, SDK usage
- Create: `scripts/smoke-local.sh` (health curl + POST task against running app)

- [ ] Document M5: produce platform installers later (`cargo-packager` / `cargo-bundle` — pick one in README as recommendation; first smoke = `cargo run` + protocol register)

- [ ] Commit: `docs: root README and local smoke script`

---

## Spec coverage checklist (self-review)

| Spec item | Task(s) |
|-----------|---------|
| Rust + egui + aria2 | 3–4, 7 |
| Tray, queue, resume, speed limit | 6, 8, 11 |
| 127.0.0.1:29128 health + tasks | 5 |
| dltool:// | 10 |
| Single instance | 9 |
| HTTP download (not FTP) | 3–4, 6 |
| No server passwords | 5, 10, 12 (only URL) |
| fileId + ticket (backend) | 13 |
| JS SDK threshold + probe + install | 12 |
| Large file Range / X-Accel notes | 13 |
| Win/macOS/Linux | 10, 14 |
| Default 100MB threshold | 12 |

## Placeholder / consistency notes

- JSON field names: local API + SDK use **camelCase** (`fileName`, `taskId`); Rust internals use snake_case.
- Aria2 RPC secret: generate per-run UUID; do not expose on local task API.
- Skip committing large `aria2c` binaries unless explicitly requested; use env/`vendor` path.

---

## Execution handoff

Plan complete and saved to `docs/superpowers/plans/2026-10-17-download-tool-implementation.md`.

**Two execution options:**

1. **Subagent-Driven (recommended)** — fresh subagent per task, review between tasks  
2. **Inline Execution** — execute tasks in this session with executing-plans checkpoints  

Which approach?
