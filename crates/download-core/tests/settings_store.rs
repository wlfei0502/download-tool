use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use download_core::settings::{default_save_dir, Settings};

#[test]
fn save_and_load_roundtrip_in_temp_dir() {
    let dir = TempDir::new();
    let path = Settings::file_in(&dir.path);
    let settings = Settings {
        save_dir: dir.path.join("downloads").to_string_lossy().into_owned(),
        global_speed_limit_bps: Some(1_048_576),
        max_connections_per_server: 4,
        autostart: true,
    };

    settings.save(&path).expect("save");
    assert!(path.is_file(), "settings file was not created");

    let loaded = Settings::load(&path).expect("load");
    assert_eq!(loaded, settings);
}

#[test]
fn file_path_is_download_tool_settings_json_under_the_config_dir() {
    let path = Settings::file_in(Path::new("/cfg"));
    assert_eq!(
        path,
        Path::new("/cfg")
            .join("download-tool")
            .join("settings.json")
    );

    let user = Settings::config_file().expect("os config dir");
    assert!(
        user.ends_with(Path::new("download-tool").join("settings.json")),
        "{user:?}"
    );
}

#[test]
fn default_save_dir_prefers_the_os_download_directory() {
    let resolved = default_save_dir();
    if let Some(downloads) =
        directories::UserDirs::new().and_then(|dirs| dirs.download_dir().map(Path::to_path_buf))
    {
        assert_eq!(resolved, downloads);
        return;
    }
    assert_eq!(resolved, std::env::current_dir().expect("cwd"));
}

#[test]
fn missing_file_loads_defaults() {
    let dir = TempDir::new();
    let path = Settings::file_in(&dir.path);
    let settings = Settings::load(&path).expect("missing file");
    assert_eq!(settings, Settings::default());
    assert_eq!(settings.max_connections_per_server, 8);
    assert_eq!(settings.global_speed_limit_bps, None);
    assert!(settings.autostart);
    assert!(settings.save_dir.is_empty());
}

#[test]
fn missing_fields_use_defaults_and_limits_are_normalized() {
    let dir = TempDir::new();
    let path = Settings::file_in(&dir.path);
    std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    std::fs::write(
        &path,
        r#"{"save_dir":"/data","global_speed_limit_bps":0,"max_connections_per_server":99}"#,
    )
    .expect("write");

    let settings = Settings::load(&path).expect("partial");
    assert_eq!(settings.save_dir, "/data");
    assert_eq!(settings.global_speed_limit_bps, None);
    assert_eq!(settings.max_connections_per_server, 16);
    assert!(settings.autostart);

    std::fs::write(&path, r#"{"max_connections_per_server":0}"#).expect("write zero");
    let clamped = Settings::load(&path).expect("zero connections");
    assert_eq!(clamped.max_connections_per_server, 1);
}

#[test]
fn invalid_json_is_an_error() {
    let dir = TempDir::new();
    let path = Settings::file_in(&dir.path);
    std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    std::fs::write(&path, "not json").expect("write");
    let err = Settings::load(&path).expect_err("invalid");
    assert!(err.contains("parse"), "{err}");
}

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "download-tool-settings-{}-{id}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).expect("temp dir");
        Self { path }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}
