//! Orbit desktop (GPUI + gpui-component). Without arguments it opens the app: server → gateway
//! sign-in window (when the server has an exe.dev gateway) → Orbit sign-in → the first
//! workspace's conversations, messages and threads, kept current by its live and events sockets.
//!
//! - `--fixture <dir>`: signed in on synthetic data, no network.
//! - `--snap out.png [--scene <name>] [--theme light|dark]` (with `--fixture`, feature `snap`):
//!   renders one frame headless to a PNG (scenes in `snap.rs`).
//! - `--gateway-signin <server>`: the sign-in window helper (the app runs it; see `signin.rs`).
//! - `--check-gateway <server>`: the risk-4b check (see `check.rs`).
//! - `--version`: prints the version.

// Release builds on Windows are GUI programs (no console window behind the app).
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod check;
mod keychain;
mod signin;
#[cfg(feature = "snap")]
mod snap;
mod state;
mod view;

use std::path::Path;

use app::OrbitApp;
use gpui_kit::component::Theme;
use gpui_kit::{AppContext as _, Bounds, TitlebarOptions, WindowBounds, WindowOptions, px, size};
use orbit_client::Fixture;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    #[cfg(windows)]
    if args.iter().any(|a| matches!(a.as_str(), "--check-gateway" | "--version" | "--snap")) {
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
    let fixture = value("--fixture").map(|dir| Fixture::load(Path::new(&dir)).map_err(|e| e.to_string()));
    if let Some(out) = value("--snap") {
        #[cfg(feature = "snap")]
        std::process::exit(snap::run(&out, fixture, value("--scene").as_deref(), value("--theme").as_deref()));
        #[cfg(not(feature = "snap"))]
        {
            eprintln!("--snap {out}: this build has no headless renderer (build with --features snap)");
            std::process::exit(2);
        }
    }

    gpui_kit::application().with_assets(gpui_kit::assets::Assets).run(move |cx| {
        gpui_kit::init(cx);
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        let bounds = Bounds::centered(None, size(px(1100.), px(720.)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            window_min_size: Some(size(px(720.), px(480.))),
            titlebar: Some(TitlebarOptions { title: Some("Orbit".into()), ..Default::default() }),
            app_id: Some("com.wellrox.orbit".into()),
            ..Default::default()
        };
        let opened = gpui_kit::open_window(options, cx, |window, cx| {
            Theme::sync_system_appearance(Some(window), cx);
            cx.new(|cx| OrbitApp::new(fixture, window, cx))
        });
        if let Err(e) = opened {
            eprintln!("orbit-desktop: cannot open a window: {e}");
            cx.quit();
        }
        cx.activate(true);
    });
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
