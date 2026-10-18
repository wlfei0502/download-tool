//! macOS `dltool://` registration.
//!
//! A bundled app declares the scheme with `CFBundleURLTypes` in `Info.plist`.
//! `cargo run` produces a bare executable, which Launch Services will not
//! associate with a scheme. See the README for the `lsregister` step.

use std::path::{Path, PathBuf};
#[cfg(target_os = "macos")]
use std::process::Command;

pub const BUNDLE_NOTE: &str = "\
not an app bundle; add CFBundleURLTypes for dltool to Info.plist and run lsregister (see README)";

const LSREGISTER: &str = "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister";

#[cfg(any(target_os = "macos", test))]
pub fn url_types_plist() -> &'static str {
    "\
<key>CFBundleURLTypes</key>
<array>
    <dict>
        <key>CFBundleURLName</key>
        <string>下载器</string>
        <key>CFBundleURLSchemes</key>
        <array>
            <string>dltool</string>
        </array>
    </dict>
</array>
"
}

#[cfg(any(target_os = "macos", test))]
pub fn plist_declares_dltool(plist: &str) -> bool {
    let compact: String = plist.split_whitespace().collect();
    compact.contains("CFBundleURLTypes") && compact.contains("<string>dltool</string>")
}

/// `Foo.app` when `exe` lives in `Foo.app/Contents/MacOS/`.
#[cfg(any(target_os = "macos", test))]
pub fn app_bundle(exe: &Path) -> Option<PathBuf> {
    let macos = exe.parent()?;
    if !macos.ends_with("MacOS") {
        return None;
    }
    let contents = macos.parent()?;
    if contents.file_name()? != "Contents" {
        return None;
    }
    let bundle = contents.parent()?;
    if bundle.extension()? != "app" {
        return None;
    }
    Some(bundle.to_path_buf())
}

#[cfg(target_os = "macos")]
pub fn register(exe: &Path) -> Result<(), String> {
    let Some(bundle) = app_bundle(exe) else {
        return Err(BUNDLE_NOTE.into());
    };
    let plist_path = bundle.join("Contents/Info.plist");
    let plist = std::fs::read_to_string(&plist_path)
        .map_err(|err| format!("read {}: {err}. {BUNDLE_NOTE}", plist_path.display()))?;
    if !plist_declares_dltool(&plist) {
        return Err(format!(
            "{} does not declare the dltool scheme.\n{}\n{BUNDLE_NOTE}",
            plist_path.display(),
            url_types_plist()
        ));
    }
    let status = Command::new(LSREGISTER)
        .arg("-f")
        .arg(&bundle)
        .status()
        .map_err(|err| format!("lsregister: {err}. {BUNDLE_NOTE}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("lsregister exited with {status}. {BUNDLE_NOTE}"))
    }
}

#[cfg(any(target_os = "macos", test))]
pub const LAUNCH_AGENT_LABEL: &str = "com.download-tool.autostart";

#[cfg(any(target_os = "macos", test))]
pub fn launch_agent_path(home: &Path) -> PathBuf {
    home.join("Library/LaunchAgents")
        .join(format!("{LAUNCH_AGENT_LABEL}.plist"))
}

/// Launch Agent that starts this executable at graphical login.
#[cfg(any(target_os = "macos", test))]
pub fn launch_agent_plist(exe: &Path) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n\
         <dict>\n\
         \t<key>Label</key>\n\
         \t<string>{label}</string>\n\
         \t<key>ProgramArguments</key>\n\
         \t<array>\n\
         \t\t<string>{program}</string>\n\
         \t</array>\n\
         \t<key>RunAtLoad</key>\n\
         \t<true/>\n\
         \t<key>LimitLoadToSessionType</key>\n\
         \t<string>Aqua</string>\n\
         </dict>\n\
         </plist>\n",
        label = LAUNCH_AGENT_LABEL,
        program = xml_escape(&exe.display().to_string()),
    )
}

#[cfg(any(target_os = "macos", test))]
fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(target_os = "macos")]
pub fn set_autostart(enabled: bool, exe: &Path) -> Result<(), String> {
    let home = std::env::var_os("HOME").ok_or_else(|| "HOME is not set".to_string())?;
    let path = launch_agent_path(Path::new(&home));
    if !enabled {
        if path.exists() {
            let _ = launchctl_bootout();
            std::fs::remove_file(&path)
                .map_err(|err| format!("remove {}: {err}", path.display()))?;
        }
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("create {}: {err}", parent.display()))?;
    }
    let plist = launch_agent_plist(exe);
    let current = std::fs::read_to_string(&path).ok();
    let unchanged = current.as_deref() == Some(plist.as_str());
    if !unchanged {
        std::fs::write(&path, &plist).map_err(|err| format!("write {}: {err}", path.display()))?;
    }
    // A loaded job with the same plist is already the login item. Reloading
    // it would start a second process because RunAtLoad is set.
    if unchanged && launch_agent_loaded() {
        return Ok(());
    }
    let _ = launchctl_bootout();
    let uid = user_id()?;
    let status = Command::new("launchctl")
        .arg("bootstrap")
        .arg(format!("gui/{uid}"))
        .arg(&path)
        .status()
        .map_err(|err| format!("launchctl bootstrap: {err}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "launchctl bootstrap exited with {status}; wrote {}",
            path.display()
        ))
    }
}

#[cfg(target_os = "macos")]
fn launch_agent_loaded() -> bool {
    let Ok(uid) = user_id() else {
        return false;
    };
    Command::new("launchctl")
        .args(["print", &format!("gui/{uid}/{LAUNCH_AGENT_LABEL}")])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

#[cfg(target_os = "macos")]
fn launchctl_bootout() -> Result<(), String> {
    let uid = user_id()?;
    let status = Command::new("launchctl")
        .args(["bootout", &format!("gui/{uid}/{LAUNCH_AGENT_LABEL}")])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map_err(|err| format!("launchctl bootout: {err}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("launchctl bootout exited with {status}"))
    }
}

#[cfg(target_os = "macos")]
fn user_id() -> Result<String, String> {
    let output = Command::new("id")
        .arg("-u")
        .output()
        .map_err(|err| format!("id -u: {err}"))?;
    if !output.status.success() {
        return Err(format!("id -u exited with {}", output.status));
    }
    let id = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if id.is_empty() || !id.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(format!("id -u returned {id:?}"));
    }
    Ok(id)
}

#[cfg(target_os = "macos")]
pub fn reveal_in_folder(path: &Path) -> Result<(), String> {
    if path.exists() {
        let status = Command::new("open")
            .arg("-R")
            .arg(path)
            .status()
            .map_err(|err| format!("open -R: {err}"))?;
        if status.success() {
            return Ok(());
        }
        return Err(format!("open -R failed ({status})"));
    }
    let dir = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| "无效的保存路径".to_string())?;
    if !dir.is_dir() {
        return Err(format!("目录不存在：{}", dir.display()));
    }
    let status = Command::new("open")
        .arg(dir)
        .status()
        .map_err(|err| format!("open: {err}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("open failed ({status})"))
    }
}

#[cfg(target_os = "macos")]
pub fn copy_file_to_clipboard(path: &Path) -> Result<(), String> {
    let absolute = std::fs::canonicalize(path)
        .map_err(|err| format!("resolve {}: {err}", path.display()))?;
    let posix = absolute.to_string_lossy();
    // AppleScript string escaping for POSIX file paths.
    let escaped = posix.replace('\\', "\\\\").replace('"', "\\\"");
    let script = format!("set the clipboard to (POSIX file \"{escaped}\")");
    let output = Command::new("osascript")
        .arg("-e")
        .arg(&script)
        .output()
        .map_err(|err| format!("osascript: {err}"))?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("osascript failed ({}) {stderr}", output.status))
    }
}
