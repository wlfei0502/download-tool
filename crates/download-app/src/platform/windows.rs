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
            data: "URL:Download Tool Protocol".into(),
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
