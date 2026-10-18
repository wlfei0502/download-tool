//! Persistent app settings.
//!
//! Stored as `download-tool/settings.json` under the OS config directory
//! (`directories::BaseDirs::config_dir`).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const DEFAULT_MAX_CONNECTIONS_PER_SERVER: u8 = 8;
/// aria2 rejects values above 16 for `max-connection-per-server`.
pub const MAX_CONNECTIONS_PER_SERVER: u8 = 16;

fn default_max_connections() -> u8 {
    DEFAULT_MAX_CONNECTIONS_PER_SERVER
}

fn default_autostart() -> bool {
    true
}

/// User preferences for where files land and how fast aria2 may download.
/// Login autostart is always on (not user-configurable).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub save_dir: String,
    pub global_speed_limit_bps: Option<u64>,
    #[serde(default = "default_max_connections")]
    pub max_connections_per_server: u8,
    #[serde(default = "default_autostart")]
    pub autostart: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            save_dir: String::new(),
            global_speed_limit_bps: None,
            max_connections_per_server: DEFAULT_MAX_CONNECTIONS_PER_SERVER,
            autostart: true,
        }
    }
}

impl Settings {
    /// `{config_dir}/download-tool/settings.json`.
    pub fn file_in(config_dir: &Path) -> PathBuf {
        config_dir.join("download-tool").join("settings.json")
    }

    /// OS config path from the `directories` crate.
    pub fn config_file() -> Result<PathBuf, String> {
        let dirs = directories::BaseDirs::new()
            .ok_or_else(|| "could not resolve the OS config directory".to_string())?;
        Ok(Self::file_in(dirs.config_dir()))
    }

    pub fn load_user() -> Result<Self, String> {
        Self::load(&Self::config_file()?)
    }

    pub fn save_user(&self) -> Result<(), String> {
        self.save(&Self::config_file()?)
    }

    /// A missing file is the default settings. Invalid JSON is an error so a
    /// corrupt file is not overwritten by accident.
    pub fn load(path: &Path) -> Result<Self, String> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(path)
            .map_err(|err| format!("read settings {}: {err}", path.display()))?;
        let settings: Self = serde_json::from_str(&text)
            .map_err(|err| format!("parse settings {}: {err}", path.display()))?;
        Ok(settings.normalized())
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|err| format!("create settings dir {}: {err}", parent.display()))?;
        }
        let text = serde_json::to_string_pretty(&self.clone().normalized())
            .map_err(|err| format!("encode settings: {err}"))?;
        let body = format!("{text}\n");
        std::fs::write(path, body)
            .map_err(|err| format!("write settings {}: {err}", path.display()))
    }

    /// aria2 `max-overall-download-limit`. `"0"` means unlimited.
    pub fn overall_download_limit(&self) -> String {
        self.global_speed_limit_bps.unwrap_or(0).to_string()
    }

    pub fn normalized(mut self) -> Self {
        self.max_connections_per_server = clamp_connections(self.max_connections_per_server);
        if self.global_speed_limit_bps == Some(0) {
            self.global_speed_limit_bps = None;
        }
        self.autostart = true;
        self
    }
}

/// OS downloads directory from `directories::UserDirs`, then `$HOME/Downloads`
/// when that folder exists, then the current directory.
pub fn default_save_dir() -> PathBuf {
    if let Some(downloads) =
        directories::UserDirs::new().and_then(|dirs| dirs.download_dir().map(Path::to_path_buf))
    {
        return downloads;
    }
    if let Some(home) = std::env::var_os("HOME") {
        let downloads = PathBuf::from(home).join("Downloads");
        if downloads.is_dir() {
            return downloads;
        }
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

pub fn clamp_connections(value: u8) -> u8 {
    if value == 0 {
        1
    } else {
        value.min(MAX_CONNECTIONS_PER_SERVER)
    }
}
