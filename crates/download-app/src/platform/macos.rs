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
        <string>Download Tool</string>
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
