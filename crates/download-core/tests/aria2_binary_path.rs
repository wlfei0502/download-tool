use download_core::aria2::process::{resolve_aria2c_path, Aria2Process};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner())
}

struct EnvVarGuard {
    key: &'static str,
    previous: Option<String>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let previous = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self { key, previous }
    }

    fn remove(key: &'static str) -> Self {
        let previous = std::env::var(key).ok();
        std::env::remove_var(key);
        Self { key, previous }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match &self.previous {
            Some(value) => std::env::set_var(self.key, value),
            None => std::env::remove_var(self.key),
        }
    }
}

#[test]
fn resolve_prefers_explicit_env() {
    let _guard = env_lock();
    let _env = EnvVarGuard::set("DOWNLOAD_TOOL_ARIA2C", "/tmp/fake-aria2c");
    let p = resolve_aria2c_path(None).unwrap();
    assert_eq!(p, PathBuf::from("/tmp/fake-aria2c"));
}

#[test]
fn resolve_env_beats_override() {
    let _guard = env_lock();
    let _env = EnvVarGuard::set("DOWNLOAD_TOOL_ARIA2C", "/tmp/from-env");
    let p = resolve_aria2c_path(Some(Path::new("/tmp/from-arg"))).unwrap();
    assert_eq!(p, PathBuf::from("/tmp/from-env"));
}

#[test]
fn resolve_uses_override_when_env_unset() {
    let _guard = env_lock();
    let _env = EnvVarGuard::remove("DOWNLOAD_TOOL_ARIA2C");
    let p = resolve_aria2c_path(Some(Path::new("/tmp/override-aria2c"))).unwrap();
    assert_eq!(p, PathBuf::from("/tmp/override-aria2c"));
}

#[test]
fn resolve_finds_vendored_binary_from_cwd_and_parents() {
    let _guard = env_lock();
    let _env = EnvVarGuard::remove("DOWNLOAD_TOOL_ARIA2C");

    let triple = rustc_host_triple();
    let bin_name = if cfg!(windows) {
        "aria2c.exe"
    } else {
        "aria2c"
    };
    let tmp = std::env::temp_dir().join(format!(
        "dltool-aria2-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let bin = tmp
        .join("vendor")
        .join("aria2")
        .join(&triple)
        .join(bin_name);
    std::fs::create_dir_all(bin.parent().unwrap()).unwrap();
    std::fs::write(&bin, b"not-a-real-binary").unwrap();

    let prev = std::env::current_dir().unwrap();
    let nested = tmp.join("nested").join("deeper");
    std::fs::create_dir_all(&nested).unwrap();
    let _cwd = CwdGuard(prev);
    std::env::set_current_dir(&nested).unwrap();
    let result = resolve_aria2c_path(None);
    drop(_cwd);
    let resolved = result.unwrap().canonicalize().unwrap();
    let expected = bin.canonicalize().unwrap();
    let _ = std::fs::remove_dir_all(&tmp);

    assert_eq!(resolved, expected);
}

struct CwdGuard(PathBuf);

impl Drop for CwdGuard {
    fn drop(&mut self) {
        let _ = std::env::set_current_dir(&self.0);
    }
}

#[tokio::test]
#[ignore = "requires vendor aria2c"]
async fn spawn_aria2_health() {
    let port = ephemeral_port();
    let dir = std::env::temp_dir();
    let (child, client) = {
        let _guard = env_lock();
        Aria2Process::start(port, "test-secret", &dir).expect("spawn aria2c")
    };
    let child = KillChild(child);

    let mut healthy = false;
    for _ in 0..50 {
        if client.get_version().await.is_ok() {
            healthy = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    drop(child);
    assert!(healthy, "aria2 RPC did not answer getVersion");
}

struct KillChild(std::process::Child);

impl Drop for KillChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn ephemeral_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    listener.local_addr().expect("local addr").port()
}

fn rustc_host_triple() -> String {
    let output = std::process::Command::new("rustc")
        .arg("-vV")
        .output()
        .expect("rustc -vV");
    let text = String::from_utf8(output.stdout).expect("rustc output utf8");
    text.lines()
        .find_map(|line| line.strip_prefix("host: "))
        .expect("host triple")
        .trim()
        .to_string()
}
