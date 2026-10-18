//! Windows `dltool://` registration under the current user.
//!
//! `HKCU\Software\Classes\dltool\shell\open\command` launches this executable
//! with the clicked URL. An empty `URL Protocol` value marks the key as a
//! scheme handler.

#[cfg(any(windows, test))]
use std::path::Path;
#[cfg(windows)]
use std::process::Command;

#[cfg(any(windows, test))]
pub struct RegistryValue {
    pub key: &'static str,
    pub name: Option<&'static str>,
    pub data: String,
}

#[cfg(any(windows, test))]
pub fn open_command(exe: &Path) -> String {
    format!("\"{}\" \"%1\"", exe.display())
}

#[cfg(any(windows, test))]
pub fn registry_values(exe: &Path) -> Vec<RegistryValue> {
    let command = open_command(exe);
    vec![
        RegistryValue {
            key: r"HKCU\Software\Classes\dltool",
            name: None,
            data: "URL:下载器协议".into(),
        },
        RegistryValue {
            key: r"HKCU\Software\Classes\dltool",
            name: Some("URL Protocol"),
            data: String::new(),
        },
        RegistryValue {
            key: r"HKCU\Software\Classes\dltool\shell\open\command",
            name: None,
            data: command,
        },
    ]
}

#[cfg(windows)]
pub fn register(exe: &Path) -> Result<(), String> {
    for entry in registry_values(exe) {
        let mut args = vec!["add".to_string(), entry.key.to_string()];
        match entry.name {
            Some(name) => {
                args.push("/v".into());
                args.push(name.into());
            }
            None => args.push("/ve".into()),
        }
        args.push("/d".into());
        args.push(entry.data);
        args.push("/f".into());
        let output = Command::new("reg")
            .args(&args)
            .output()
            .map_err(|err| format!("reg add {}: {err}", entry.key))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!(
                "reg add {} failed ({}) {stderr}",
                entry.key, output.status
            ));
        }
    }
    Ok(())
}

#[cfg(any(windows, test))]
pub const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";

#[cfg(any(windows, test))]
pub const RUN_VALUE_NAME: &str = "下载器";

#[cfg(any(windows, test))]
pub fn run_value(exe: &Path) -> String {
    format!("\"{}\"", exe.display())
}

#[cfg(windows)]
pub fn set_autostart(enabled: bool, exe: &Path) -> Result<(), String> {
    if !enabled {
        let output = Command::new("reg")
            .args(["delete", RUN_KEY, "/v", RUN_VALUE_NAME, "/f"])
            .output()
            .map_err(|err| format!("reg delete: {err}"))?;
        if output.status.success() || output.status.code() == Some(1) {
            return Ok(());
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "reg delete {RUN_KEY} failed ({}) {stderr}",
            output.status
        ));
    }
    let output = Command::new("reg")
        .args([
            "add",
            RUN_KEY,
            "/v",
            RUN_VALUE_NAME,
            "/t",
            "REG_SZ",
            "/d",
            &run_value(exe),
            "/f",
        ])
        .output()
        .map_err(|err| format!("reg add: {err}"))?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!(
            "reg add {RUN_KEY} failed ({}) {stderr}",
            output.status
        ))
    }
}


#[cfg(windows)]
pub fn reveal_in_folder(path: &Path) -> Result<(), String> {
    if path.exists() {
        let arg = format!("/select,{}", path.display());
        let status = Command::new("explorer")
            .arg(arg)
            .status()
            .map_err(|err| format!("explorer: {err}"))?;
        // explorer often exits non-zero after opening; treat spawn success as ok when status obtained
        let _ = status;
        return Ok(());
    }
    let dir = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| "无效的保存路径".to_string())?;
    if !dir.is_dir() {
        return Err(format!("目录不存在：{}", dir.display()));
    }
    let status = Command::new("explorer")
        .arg(dir)
        .status()
        .map_err(|err| format!("explorer: {err}"))?;
    let _ = status;
    Ok(())
}


#[cfg(windows)]
pub fn copy_file_to_clipboard(path: &Path) -> Result<(), String> {
    let absolute = std::fs::canonicalize(path)
        .map_err(|err| format!("resolve {}: {err}", path.display()))?;
    let path_str = absolute.to_string_lossy().replace('\'', "''");
    let script = format!("Set-Clipboard -Path '{path_str}'");
    let output = Command::new("powershell")
        .args(["-NoProfile", "-Command", &script])
        .output()
        .map_err(|err| format!("powershell: {err}"))?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("Set-Clipboard failed ({}) {stderr}", output.status))
    }
}
