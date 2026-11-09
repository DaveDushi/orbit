//! "Start with Windows" (macOS: "Open at login"; Linux: "Start at login"). Off by default; the tray
//! menu turns it on. The entry starts the app with `--hidden`: in the tray, without a window.
//!
//! - Windows: the value `Orbit` under `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`.
//! - macOS: `~/Library/LaunchAgents/com.wellrox.orbit.plist` (`RunAtLoad`).
//! - Linux: `~/.config/autostart/orbit-desktop.desktop` (XDG autostart).
//!
//! The entry names the exe where it is now; the app writes it again at each start while the
//! setting is on, so a moved app still starts.

use std::path::Path;

pub const HIDDEN_FLAG: &str = "--hidden";

pub fn label() -> &'static str {
    if cfg!(windows) {
        "Start with Windows"
    } else if cfg!(target_os = "macos") {
        "Open at login"
    } else {
        "Start at login"
    }
}

/// Turns the entry on or off for the running exe.
pub fn set(on: bool) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("cannot find the app: {e}"))?;
    platform::set(on, &exe)
}

#[cfg_attr(not(windows), allow(dead_code))] // Each OS uses one entry; the tests check all of them.
/// The Windows `Run` value: the quoted exe path and the flag.
pub fn windows_command(exe: &Path) -> String {
    format!("\"{}\" {HIDDEN_FLAG}", exe.display())
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
/// The macOS launch agent.
pub fn launch_agent(exe: &Path) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>com.wellrox.orbit</string>
  <key>ProgramArguments</key>
  <array><string>{}</string><string>{HIDDEN_FLAG}</string></array>
  <key>RunAtLoad</key><true/>
</dict>
</plist>
"#,
        xml_escape(&exe.display().to_string())
    )
}

#[cfg_attr(any(windows, target_os = "macos"), allow(dead_code))]
/// The XDG autostart entry. `Exec` quotes the path by the Desktop Entry rules.
pub fn desktop_entry(exe: &Path) -> String {
    let quoted: String = exe.display().to_string().chars().flat_map(|c| match c {
        '"' | '`' | '$' | '\\' => vec!['\\', c],
        '%' => vec!['%', '%'],
        c => vec![c],
    }).collect();
    format!("[Desktop Entry]\nType=Application\nName=Orbit\nExec=\"{quoted}\" {HIDDEN_FLAG}\nX-GNOME-Autostart-enabled=true\n")
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

#[cfg(not(windows))]
fn write_or_remove(path: Option<std::path::PathBuf>, on: bool, content: String) -> Result<(), String> {
    let path = path.ok_or("no home folder")?;
    if on {
        std::fs::create_dir_all(path.parent().expect("a file in a folder")).map_err(|e| e.to_string())?;
        std::fs::write(&path, content).map_err(|e| format!("{}: {e}", path.display()))
    } else {
        match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(format!("{}: {e}", path.display())),
            _ => Ok(()),
        }
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use std::path::Path;

    pub fn set(on: bool, exe: &Path) -> Result<(), String> {
        let path = dirs::home_dir().map(|h| h.join("Library/LaunchAgents/com.wellrox.orbit.plist"));
        super::write_or_remove(path, on, super::launch_agent(exe))
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod platform {
    use std::path::Path;

    pub fn set(on: bool, exe: &Path) -> Result<(), String> {
        let path = dirs::config_dir().map(|d| d.join("autostart/orbit-desktop.desktop"));
        super::write_or_remove(path, on, super::desktop_entry(exe))
    }
}

#[cfg(windows)]
mod platform {
    use std::path::Path;

    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, REG_SZ, RegDeleteKeyValueW, RegSetKeyValueW};

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(Some(0)).collect()
    }

    pub fn set(on: bool, exe: &Path) -> Result<(), String> {
        let key = wide(r"Software\Microsoft\Windows\CurrentVersion\Run");
        let name = wide("Orbit");
        // SAFETY: every pointer is a NUL-terminated UTF-16 buffer that lives across the call.
        let status = unsafe {
            if on {
                let value = wide(&super::windows_command(exe));
                RegSetKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr(), REG_SZ, value.as_ptr().cast(), (value.len() * 2) as u32)
            } else {
                match RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr()) {
                    ERROR_FILE_NOT_FOUND => ERROR_SUCCESS,
                    status => status,
                }
            }
        };
        if status == ERROR_SUCCESS { Ok(()) } else { Err(format!("the registry refused (error {status})")) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_start_the_app_hidden_with_its_path_quoted() {
        let exe = Path::new(r"C:\Users\Ada Lovelace\Apps\Orbit\Orbit.exe");
        assert_eq!(windows_command(exe), r#""C:\Users\Ada Lovelace\Apps\Orbit\Orbit.exe" --hidden"#);

        let plist = launch_agent(Path::new("/Applications/R&D <Orbit>.app/Contents/MacOS/orbit-desktop"));
        assert!(plist.contains("<string>/Applications/R&amp;D &lt;Orbit&gt;.app/Contents/MacOS/orbit-desktop</string><string>--hidden</string>"));
        assert!(plist.contains("<key>RunAtLoad</key><true/>"));

        let entry = desktop_entry(Path::new("/home/ada/My $Apps/100%/orbit-desktop"));
        assert!(entry.contains("Exec=\"/home/ada/My \\$Apps/100%%/orbit-desktop\" --hidden\n"), "{entry}");
    }
}
