//! Linux `dltool://` registration via a user desktop entry.
//!
//! Writes `~/.local/share/applications/dltool.desktop` (or `$XDG_DATA_HOME`)
//! and asks `xdg-mime` to claim `x-scheme-handler/dltool`.

#[cfg(any(target_os = "linux", test))]
use std::path::{Path, PathBuf};
#[cfg(target_os = "linux")]
use std::process::Command;

#[cfg(any(target_os = "linux", test))]
pub const DESKTOP_FILE_NAME: &str = "dltool.desktop";

#[cfg(any(target_os = "linux", test))]
pub fn desktop_entry(exe: &Path) -> String {
    format!(
        "[Desktop Entry]\n\
         Name=下载器\n\
         Comment=Receive dltool:// download links\n\
         Exec={exec} %u\n\
         Type=Application\n\
         Terminal=false\n\
         MimeType=x-scheme-handler/dltool;\n\
         NoDisplay=true\n",
        exec = desktop_exec(exe),
    )
}

#[cfg(any(target_os = "linux", test))]
pub fn applications_dir_from(
    xdg_data_home: Option<&Path>,
    home: Option<&Path>,
) -> Result<PathBuf, String> {
    if let Some(xdg) = xdg_data_home {
        if !xdg.as_os_str().is_empty() {
            return Ok(xdg.join("applications"));
        }
    }
    let home = home.ok_or_else(|| "HOME is not set".to_string())?;
    if home.as_os_str().is_empty() {
        return Err("HOME is not set".into());
    }
    Ok(home.join(".local/share/applications"))
}

#[cfg(any(target_os = "linux", test))]
fn desktop_exec(exe: &Path) -> String {
    let raw = exe.to_string_lossy();
    if raw.contains([' ', '\t', '"', '\\', '`', '$']) {
        let escaped = raw.replace('\\', "\\\\").replace('"', "\\\"");
        format!("\"{escaped}\"")
    } else {
        raw.into_owned()
    }
}

#[cfg(target_os = "linux")]
pub fn register(exe: &Path) -> Result<(), String> {
    let xdg = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from);
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let dir = applications_dir_from(xdg.as_deref(), home.as_deref())?;
    std::fs::create_dir_all(&dir).map_err(|err| format!("create {}: {err}", dir.display()))?;
    let path = dir.join(DESKTOP_FILE_NAME);
    std::fs::write(&path, desktop_entry(exe))
        .map_err(|err| format!("write {}: {err}", path.display()))?;

    let status = Command::new("xdg-mime")
        .args(["default", DESKTOP_FILE_NAME, "x-scheme-handler/dltool"])
        .status()
        .map_err(|err| format!("xdg-mime: {err}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "xdg-mime default {DESKTOP_FILE_NAME} x-scheme-handler/dltool exited with {status}"
        ))
    }
}

#[cfg(any(target_os = "linux", test))]
pub const AUTOSTART_FILE_NAME: &str = "download-tool.desktop";

#[cfg(any(target_os = "linux", test))]
pub fn autostart_dir_from(
    xdg_config_home: Option<&Path>,
    home: Option<&Path>,
) -> Result<PathBuf, String> {
    if let Some(xdg) = xdg_config_home {
        if !xdg.as_os_str().is_empty() {
            return Ok(xdg.join("autostart"));
        }
    }
    let home = home.ok_or_else(|| "HOME is not set".to_string())?;
    if home.as_os_str().is_empty() {
        return Err("HOME is not set".into());
    }
    Ok(home.join(".config/autostart"))
}

#[cfg(any(target_os = "linux", test))]
pub fn autostart_entry(exe: &Path) -> String {
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=下载器\n\
         Exec={exec}\n\
         X-GNOME-Autostart-enabled=true\n\
         Terminal=false\n",
        exec = desktop_exec(exe),
    )
}

#[cfg(target_os = "linux")]
pub fn set_autostart(enabled: bool, exe: &Path) -> Result<(), String> {
    let xdg = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let dir = autostart_dir_from(xdg.as_deref(), home.as_deref())?;
    let path = dir.join(AUTOSTART_FILE_NAME);
    if !enabled {
        if path.exists() {
            std::fs::remove_file(&path)
                .map_err(|err| format!("remove {}: {err}", path.display()))?;
        }
        return Ok(());
    }
    std::fs::create_dir_all(&dir).map_err(|err| format!("create {}: {err}", dir.display()))?;
    std::fs::write(&path, autostart_entry(exe))
        .map_err(|err| format!("write {}: {err}", path.display()))
}

#[cfg(target_os = "linux")]
pub fn reveal_in_folder(path: &Path) -> Result<(), String> {
    let dir = if path.is_file() {
        path.parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .ok_or_else(|| "无效的保存路径".to_string())?
    } else if path.is_dir() {
        path
    } else {
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .ok_or_else(|| "无效的保存路径".to_string())?;
        if !parent.is_dir() {
            return Err(format!("目录不存在：{}", parent.display()));
        }
        parent
    };
    let status = Command::new("xdg-open")
        .arg(dir)
        .status()
        .map_err(|err| format!("xdg-open: {err}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("xdg-open failed ({status})"))
    }
}

#[cfg(target_os = "linux")]
pub fn copy_file_to_clipboard(path: &Path) -> Result<(), String> {
    use std::process::Stdio;
    let absolute = std::fs::canonicalize(path)
        .map_err(|err| format!("resolve {}: {err}", path.display()))?;
    let uri = format!("file://{}", absolute.display());
    // Prefer wl-copy, then xclip.
    if let Ok(mut child) = Command::new("wl-copy")
        .arg("--type")
        .arg("text/uri-list")
        .stdin(Stdio::piped())
        .spawn()
    {
        use std::io::Write;
        if let Some(stdin) = child.stdin.as_mut() {
            let _ = writeln!(stdin, "{uri}");
        }
        let status = child.wait().map_err(|err| format!("wl-copy: {err}"))?;
        if status.success() {
            return Ok(());
        }
    }
    let mut child = Command::new("xclip")
        .args(["-selection", "clipboard", "-t", "text/uri-list"])
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|err| format!("xclip/wl-copy unavailable: {err}"))?;
    {
        use std::io::Write;
        let stdin = child
            .stdin
            .as_mut()
            .ok_or_else(|| "xclip stdin missing".to_string())?;
        writeln!(stdin, "{uri}").map_err(|err| format!("xclip write: {err}"))?;
    }
    let status = child.wait().map_err(|err| format!("xclip: {err}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("xclip failed ({status})"))
    }
}
