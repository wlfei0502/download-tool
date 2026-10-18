//! Register `dltool://` and the login item with the operating system.
//!
//! Registration is best-effort. A missing bundle, registry, `xdg-mime`, or
//! login-item hook must not stop the app from starting.

use std::path::Path;

mod linux;
mod macos;
mod windows;

/// Best-effort protocol registration for the current OS.
pub fn register_protocol(exe: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        windows::register(exe)
    }
    #[cfg(target_os = "macos")]
    {
        macos::register(exe)
    }
    #[cfg(target_os = "linux")]
    {
        linux::register(exe)
    }
    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
    {
        let _ = exe;
        Err("dltool protocol registration is not implemented for this OS".into())
    }
}

/// Best-effort login item for the current OS.
///
/// The settings flag is the source of truth. This only registers or removes
/// the platform hook. A failure here must not stop the app from starting.
pub fn set_autostart(enabled: bool, exe: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        windows::set_autostart(enabled, exe)
    }
    #[cfg(target_os = "macos")]
    {
        macos::set_autostart(enabled, exe)
    }
    #[cfg(target_os = "linux")]
    {
        linux::set_autostart(enabled, exe)
    }
    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
    {
        let _ = (enabled, exe);
        Err("autostart is not implemented for this OS".into())
    }
}

/// Open the folder that contains `path`, selecting the file when possible.
pub fn reveal_in_folder(path: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        windows::reveal_in_folder(path)
    }
    #[cfg(target_os = "macos")]
    {
        macos::reveal_in_folder(path)
    }
    #[cfg(target_os = "linux")]
    {
        linux::reveal_in_folder(path)
    }
    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
    {
        let _ = path;
        Err("opening folders is not implemented for this OS".into())
    }
}

/// Put a local file on the system clipboard so Finder/Explorer can paste it.
pub fn copy_file_to_clipboard(path: &Path) -> Result<(), String> {
    if !path.is_file() {
        return Err(format!("文件不存在：{}", path.display()));
    }
    #[cfg(windows)]
    {
        windows::copy_file_to_clipboard(path)
    }
    #[cfg(target_os = "macos")]
    {
        macos::copy_file_to_clipboard(path)
    }
    #[cfg(target_os = "linux")]
    {
        linux::copy_file_to_clipboard(path)
    }
    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
    {
        let _ = path;
        Err("copying files is not implemented for this OS".into())
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{linux, macos, windows};

    #[test]
    fn windows_open_command_quotes_the_executable_and_url() {
        let command = windows::open_command(Path::new(
            r"C:\Program Files\Download Tool\download-tool.exe",
        ));
        assert_eq!(
            command,
            r#""C:\Program Files\Download Tool\download-tool.exe" "%1""#
        );
    }

    #[test]
    fn windows_registry_uses_the_dltool_open_command_key() {
        let entries = windows::registry_values(Path::new(r"C:\dltool\download-tool.exe"));
        assert!(entries.iter().any(|entry| {
            entry.key == r"HKCU\Software\Classes\dltool\shell\open\command"
                && entry.name.is_none()
                && entry.data == r#""C:\dltool\download-tool.exe" "%1""#
        }));
        assert!(entries.iter().any(|entry| {
            entry.key == r"HKCU\Software\Classes\dltool"
                && entry.name == Some("URL Protocol")
                && entry.data.is_empty()
        }));
    }

    #[test]
    fn linux_desktop_entry_handles_the_dltool_scheme() {
        let text = linux::desktop_entry(Path::new("/usr/local/bin/download-tool"));
        assert!(
            text.contains("Exec=/usr/local/bin/download-tool %u"),
            "{text}"
        );
        assert!(text.contains("MimeType=x-scheme-handler/dltool;"), "{text}");
        assert_eq!(linux::DESKTOP_FILE_NAME, "dltool.desktop");

        let quoted = linux::desktop_entry(Path::new("/home/me/My Apps/download-tool"));
        assert!(
            quoted.contains(r#"Exec="/home/me/My Apps/download-tool" %u"#),
            "{quoted}"
        );
    }

    #[test]
    fn linux_applications_dir_follows_xdg_then_home() {
        let xdg =
            linux::applications_dir_from(Some(Path::new("/tmp/xdg")), Some(Path::new("/home/me")))
                .unwrap();
        assert_eq!(xdg, Path::new("/tmp/xdg/applications"));

        let home = linux::applications_dir_from(None, Some(Path::new("/home/me"))).unwrap();
        assert_eq!(home, Path::new("/home/me/.local/share/applications"));

        let err = linux::applications_dir_from(None, None).unwrap_err();
        assert!(err.contains("HOME"), "{err}");
    }

    #[test]
    fn macos_plist_declares_the_dltool_scheme() {
        let xml = macos::url_types_plist();
        assert!(xml.contains("CFBundleURLTypes"), "{xml}");
        assert!(xml.contains("<string>dltool</string>"), "{xml}");
        assert!(macos::plist_declares_dltool(xml));
        assert!(!macos::plist_declares_dltool("<plist></plist>"));
    }

    #[test]
    fn macos_finds_the_enclosing_app_bundle() {
        let exe = Path::new("/Applications/Download Tool.app/Contents/MacOS/download-tool");
        assert_eq!(
            macos::app_bundle(exe),
            Some(Path::new("/Applications/Download Tool.app").to_path_buf())
        );
        assert_eq!(macos::app_bundle(Path::new("/tmp/download-tool")), None);
    }

    #[test]
    fn macos_launch_agent_runs_this_executable_at_login() {
        let plist = macos::launch_agent_plist(Path::new(
            "/Applications/Download Tool.app/Contents/MacOS/download-tool",
        ));
        assert!(plist.contains("com.download-tool.autostart"), "{plist}");
        assert!(plist.contains("<key>RunAtLoad</key>"), "{plist}");
        assert!(
            plist.contains(
                "<string>/Applications/Download Tool.app/Contents/MacOS/download-tool</string>"
            ),
            "{plist}"
        );
        let escaped = macos::launch_agent_plist(Path::new("/tmp/a&b<c>"));
        assert!(escaped.contains("/tmp/a&amp;b&lt;c&gt;"), "{escaped}");
        assert_eq!(
            macos::launch_agent_path(Path::new("/Users/me")),
            Path::new("/Users/me/Library/LaunchAgents/com.download-tool.autostart.plist")
        );
    }

    #[test]
    fn linux_autostart_desktop_file_is_enabled() {
        let text = linux::autostart_entry(Path::new("/usr/local/bin/download-tool"));
        assert!(
            text.contains("Exec=/usr/local/bin/download-tool\n"),
            "{text}"
        );
        assert!(text.contains("X-GNOME-Autostart-enabled=true"), "{text}");
        assert_eq!(linux::AUTOSTART_FILE_NAME, "download-tool.desktop");

        let quoted = linux::autostart_entry(Path::new("/home/me/My Apps/download-tool"));
        assert!(
            quoted.contains("Exec=\"/home/me/My Apps/download-tool\"\n"),
            "{quoted}"
        );

        let xdg = linux::autostart_dir_from(Some(Path::new("/tmp/xdg")), None).unwrap();
        assert_eq!(xdg, Path::new("/tmp/xdg/autostart"));
        let home = linux::autostart_dir_from(None, Some(Path::new("/home/me"))).unwrap();
        assert_eq!(home, Path::new("/home/me/.config/autostart"));
        let err = linux::autostart_dir_from(None, None).unwrap_err();
        assert!(err.contains("HOME"), "{err}");
    }

    #[test]
    fn windows_run_key_quotes_the_executable() {
        assert_eq!(
            windows::run_value(Path::new(
                r"C:\Program Files\Download Tool\download-tool.exe"
            )),
            r#""C:\Program Files\Download Tool\download-tool.exe""#
        );
        assert_eq!(
            windows::RUN_KEY,
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run"
        );
        assert_eq!(windows::RUN_VALUE_NAME, "下载器");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn cargo_run_binary_reports_the_bundle_step() {
        let err = super::register_protocol(Path::new("/tmp/download-tool")).unwrap_err();
        assert!(
            err.contains("Info.plist") || err.contains("lsregister") || err.contains("bundle"),
            "{err}"
        );
    }
}
