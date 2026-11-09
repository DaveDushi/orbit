//! Orbit desktop: the team's Orbit web UI in a native window (wry), with a tray icon, OS
//! notifications, an unread badge on the taskbar / dock icon, downloads to the Downloads folder,
//! external links in the default browser and "Start with Windows" (see `shell.rs`).
//!
//! - no arguments: opens the app (the start page asks for the server the first time).
//! - `--hidden`: starts in the tray, without a window (the autostart entry uses it).
//! - `--gateway-signin <server>`: the sign-in window helper of `--check-gateway` (see `signin.rs`).
//! - `--check-gateway <server>`: the risk-4b check (see `check.rs`).
//! - `--version`: prints the version.

// Release builds on Windows are GUI programs (no console window behind the app).
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod autostart;
mod bridge;
mod check;
mod route;
mod settings;
mod shell;
mod signin;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    #[cfg(windows)]
    if args.iter().any(|a| matches!(a.as_str(), "--check-gateway" | "--version")) {
        attach_console();
    }
    let value = |flag: &str| args.iter().position(|a| a == flag).map(|i| args.get(i + 1).cloned().unwrap_or_default());
    if let Some(server) = value("--gateway-signin") {
        signin::run(&server);
    }
    if let Some(server) = value("--check-gateway") {
        std::process::exit(check::run(&server));
    }
    if args.iter().any(|a| a == "--version") {
        println!("orbit-desktop {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    #[cfg(target_os = "macos")]
    let _ = notify_rust::set_application("com.wellrox.orbit");
    shell::run(args.iter().any(|a| a == autostart::HIDDEN_FLAG));
}

/// Prints of a command-line use go to the console the GUI program was started from
/// (`start /wait Orbit.exe --check-gateway …`), unless its output is already redirected.
#[cfg(windows)]
fn attach_console() {
    use windows_sys::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_OUTPUT_HANDLE};
    // SAFETY: plain Win32 calls without pointers; failure leaves the process as it was.
    unsafe {
        if GetStdHandle(STD_OUTPUT_HANDLE).is_null() {
            AttachConsole(ATTACH_PARENT_PROCESS);
        }
    }
}
