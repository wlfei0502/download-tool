use std::path::{Path, PathBuf};
use std::process::{Child, Command};

use super::rpc::Aria2Client;

const ENV_ARIA2C: &str = "DOWNLOAD_TOOL_ARIA2C";

/// Locate `aria2c`.
///
/// Order: `DOWNLOAD_TOOL_ARIA2C` → `override_path` →
/// `vendor/aria2/<target-triple>/aria2c` (`.exe` on Windows), walking up from
/// the current directory and from the running executable.
pub fn resolve_aria2c_path(override_path: Option<&Path>) -> Result<PathBuf, String> {
    if let Some(from_env) = env_path() {
        return Ok(from_env);
    }
    if let Some(path) = override_path {
        if !path.as_os_str().is_empty() {
            return Ok(path.to_path_buf());
        }
    }
    find_vendored_binary()
}

fn env_path() -> Option<PathBuf> {
    let value = std::env::var(ENV_ARIA2C).ok()?;
    if value.is_empty() {
        None
    } else {
        Some(PathBuf::from(value))
    }
}

fn binary_name() -> &'static str {
    if cfg!(windows) {
        "aria2c.exe"
    } else {
        "aria2c"
    }
}

fn vendor_relative() -> PathBuf {
    PathBuf::from("vendor")
        .join("aria2")
        .join(env!("DOWNLOAD_CORE_TARGET"))
        .join(binary_name())
}

fn find_vendored_binary() -> Result<PathBuf, String> {
    let rel = vendor_relative();
    let mut roots = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        roots.push(cwd);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            roots.push(dir.to_path_buf());
        }
    }

    let mut checked = Vec::new();
    for root in roots {
        let mut current = Some(root.as_path());
        while let Some(dir) = current {
            let candidate = dir.join(&rel);
            if candidate.is_file() {
                return Ok(candidate);
            }
            checked.push(candidate);
            current = dir.parent();
        }
    }

    Err(format!(
        "aria2c not found; set {ENV_ARIA2C} or place the binary at {}. searched: {}",
        rel.display(),
        checked
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

/// Owns nothing itself. [`Aria2Process::start`] returns the child so the caller
/// can kill and wait on it.
pub struct Aria2Process;

impl Aria2Process {
    /// Spawn `aria2c` with a localhost JSON-RPC listener and a client for that endpoint.
    ///
    /// `max_connections_per_server` is clamped to 1..=16.
    pub fn start(
        rpc_port: u16,
        secret: &str,
        dir: &Path,
        max_connections_per_server: u8,
    ) -> Result<(Child, Aria2Client), String> {
        let bin = resolve_aria2c_path(None)?;
        let mut command = Command::new(&bin);
        command.args(launch_args(
            rpc_port,
            secret,
            dir,
            max_connections_per_server,
        ));
        let child = command
            .spawn()
            .map_err(|err| format!("failed to spawn {}: {err}", bin.display()))?;
        let client = Aria2Client::new(
            format!("http://127.0.0.1:{rpc_port}/jsonrpc"),
            Some(secret.to_string()),
        );
        Ok((child, client))
    }
}

/// Arguments passed to `aria2c` besides the binary path.
pub fn launch_args(
    rpc_port: u16,
    secret: &str,
    dir: &Path,
    max_connections_per_server: u8,
) -> Vec<String> {
    let connections = crate::settings::clamp_connections(max_connections_per_server);
    vec![
        "--enable-rpc".into(),
        format!("--rpc-listen-port={rpc_port}"),
        format!("--rpc-secret={secret}"),
        "--rpc-listen-all=false".into(),
        format!("--dir={}", dir.display()),
        "--continue=true".into(),
        "--max-concurrent-downloads=5".into(),
        format!("--max-connection-per-server={connections}"),
    ]
}
