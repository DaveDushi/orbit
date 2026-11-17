//! A small log for reports from people's machines: what the app sees and we cannot test here (the WebView2
//! version, its permission requests, the page's voice input errors). `<local data dir>/orbit-desktop/diagnostics.log`,
//! opened from the tray menu. It holds no message text and no cookies; above 64 KB it starts again.

use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_LEN: u64 = 64 * 1024;

pub fn path() -> Option<PathBuf> {
    Some(dirs::data_local_dir()?.join("orbit-desktop").join("diagnostics.log"))
}

/// Appends one line, after the time in Unix seconds.
pub fn log(line: &str) {
    let Some(path) = path() else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_LEN) {
        let _ = std::fs::remove_file(&path);
    }
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(file, "{secs} {}", line.replace(['\r', '\n'], " "));
    }
}
