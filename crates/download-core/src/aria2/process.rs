use std::path::{Path, PathBuf};
use std::process::{Child, Command};

use super::rpc::Aria2Client;

const ENV_ARIA2C: &str = "DOWNLOAD_TOOL_ARIA2C";
const ENV_CA_FILE: &str = "DOWNLOAD_TOOL_CA_FILE";

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
    // App bundles / installers often ship aria2c next to this executable.
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let sibling = dir.join(binary_name());
            if sibling.is_file() {
                return Ok(sibling);
            }
        }
    }

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
        let args = launch_args(
            rpc_port,
            secret,
            dir,
            max_connections_per_server,
        );
        if let Some(ca) = args.iter().find_map(|arg| arg.strip_prefix("--ca-certificate=")) {
            eprintln!("aria2 CA certificate: {ca}");
        } else {
            eprintln!(
                "aria2 CA certificate: none found (HTTPS may fail); set {ENV_CA_FILE} or SSL_CERT_FILE"
            );
        }
        let mut command = Command::new(&bin);
        command.args(&args);
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

/// Locate a PEM/CRT CA bundle for HTTPS verification.
///
/// Order: `DOWNLOAD_TOOL_CA_FILE` → `SSL_CERT_FILE` → `CURL_CA_BUNDLE` →
/// files next to `aria2c` / this app → OS-specific defaults.
///
/// Windows note: stock `aria2c` (OpenSSL builds) does **not** use the Windows
/// certificate store. Ship `curl-ca-bundle.crt` beside `aria2c.exe`, or set
/// `DOWNLOAD_TOOL_CA_FILE` to a Mozilla/curl CA bundle path.
pub fn resolve_ca_certificate() -> Option<PathBuf> {
    for key in [ENV_CA_FILE, "SSL_CERT_FILE", "CURL_CA_BUNDLE"] {
        if let Ok(value) = std::env::var(key) {
            let path = PathBuf::from(value);
            if !path.as_os_str().is_empty() && path.is_file() {
                return Some(path);
            }
        }
    }
    for candidate in ca_certificate_candidates() {
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn push_bundle_names(paths: &mut Vec<PathBuf>, dir: &Path) {
    for name in [
        "curl-ca-bundle.crt",
        "ca-bundle.crt",
        "cacert.pem",
        "cert.pem",
    ] {
        paths.push(dir.join(name));
    }
}

fn ca_certificate_candidates() -> Vec<PathBuf> {
    let mut paths = Vec::new();

    // Prefer a bundle next to aria2c (typical Windows packaging layout).
    if let Ok(aria2) = resolve_aria2c_path(None) {
        if let Some(dir) = aria2.parent() {
            push_bundle_names(&mut paths, dir);
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            push_bundle_names(&mut paths, dir);
        }
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        paths.extend([
            PathBuf::from("/etc/ssl/cert.pem"),
            PathBuf::from("/etc/ssl/certs/ca-certificates.crt"),
            PathBuf::from("/etc/pki/tls/certs/ca-bundle.crt"),
            PathBuf::from("/etc/ssl/ca-bundle.pem"),
        ]);
    }
    #[cfg(target_os = "macos")]
    {
        paths.push(PathBuf::from("/opt/homebrew/etc/ca-certificates/cert.pem"));
        paths.push(PathBuf::from("/usr/local/etc/ca-certificates/cert.pem"));
        paths.push(PathBuf::from("/opt/homebrew/etc/openssl@3/cert.pem"));
    }
    #[cfg(target_os = "windows")]
    {
        // Git for Windows ships a curl CA bundle many users already have.
        if let Ok(program_files) = std::env::var("ProgramFiles") {
            let git = PathBuf::from(program_files).join("Git");
            paths.push(git.join("mingw64/ssl/certs/ca-bundle.crt"));
            paths.push(git.join("usr/ssl/certs/ca-bundle.crt"));
        }
        if let Ok(program_files_x86) = std::env::var("ProgramFiles(x86)") {
            let git = PathBuf::from(program_files_x86).join("Git");
            paths.push(git.join("mingw64/ssl/certs/ca-bundle.crt"));
        }
        // Optional Mozilla CA extract under LOCALAPPDATA.
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            let base = PathBuf::from(local).join("download-tool");
            push_bundle_names(&mut paths, &base);
        }
    }

    paths
}

/// Arguments passed to `aria2c` besides the binary path.
pub fn launch_args(
    rpc_port: u16,
    secret: &str,
    dir: &Path,
    max_connections_per_server: u8,
) -> Vec<String> {
    let connections = crate::settings::clamp_connections(max_connections_per_server);
    let mut args = vec![
        "--enable-rpc".into(),
        format!("--rpc-listen-port={rpc_port}"),
        format!("--rpc-secret={secret}"),
        "--rpc-listen-all=false".into(),
        format!("--dir={}", dir.display()),
        "--continue=true".into(),
        "--max-concurrent-downloads=5".into(),
        format!("--max-connection-per-server={connections}"),
        "--check-certificate=true".into(),
    ];
    if let Some(ca) = resolve_ca_certificate() {
        args.push(format!("--ca-certificate={}", ca.display()));
    }
    args
}
