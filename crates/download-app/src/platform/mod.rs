//! Register `dltool://` with the operating system.
//!
//! Registration is best-effort. A missing bundle, registry, or `xdg-mime`
//! must not stop the app from starting.

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
