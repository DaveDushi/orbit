//! The gateway sign-in window: `orbit-desktop --gateway-signin <server>`, a separate process
//! (tao + wry) because GPUI's event loop owns the app's main thread.
//!
//! It opens `<server>/api/v1/setup/status`. Without a gateway session the exe.dev proxy redirects
//! to its sign-in page (`/__exe.dev/login`); after the person signs in it redirects back. A page
//! that finishes loading on the server's origin outside `/__exe.dev/` means the gateway let the
//! window through: the helper reads the server's cookies from the web view and writes one hand-off
//! line to stdout (`orbit_client::cookies`), then exits. Closing the window writes `cancelled`.
//!
//! The cookies go to a pipe only: the helper refuses to write them to a terminal. Each server
//! has its own web view data directory (Linux, Windows), so a person stays signed in to the
//! gateway between sign-ins; macOS uses the default WebKit store.

use std::io::{BufRead, BufReader, IsTerminal, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};

use orbit_client::{GatewayCookie, GatewayCookies, HandOff, Secret, ServerUrl, hand_off_line, parse_hand_off};
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::window::WindowBuilder;
use wry::{NewWindowResponse, PageLoadEvent, WebContext, WebViewBuilder};

/// Opens the sign-in window for `server` and waits for its answer (call it off the UI thread).
pub fn open(server: &ServerUrl) -> Result<HandOff, String> {
    let exe = std::env::current_exe().map_err(|e| format!("cannot find the app: {e}"))?;
    let mut child = Command::new(exe)
        .arg("--gateway-signin")
        .arg(server.origin())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| format!("cannot open the sign-in window: {e}"))?;
    let mut line = String::new();
    let read = BufReader::new(child.stdout.take().expect("piped")).read_line(&mut line);
    let _ = child.wait();
    if read.is_err() || line.trim().is_empty() {
        return Err("the sign-in window ended without an answer".into());
    }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    parse_hand_off(&line, server, now).map_err(|e| e.to_string())
}

enum UserEvent {
    Loaded(String),
}

pub fn run(server_arg: &str) -> ! {
    let server = match ServerUrl::parse(server_arg) {
        Ok(server) => server,
        Err(e) => {
            eprintln!("orbit-desktop --gateway-signin: {e}");
            std::process::exit(2);
        }
    };
    if std::io::stdout().is_terminal() {
        eprintln!("orbit-desktop --gateway-signin writes the gateway cookies to its stdout. The app runs it; do not run it in a terminal.");
        std::process::exit(2);
    }

    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    let window = match WindowBuilder::new()
        .with_title(format!("Sign in to {}", server.host()))
        .with_inner_size(tao::dpi::LogicalSize::new(520.0, 720.0))
        .build(&event_loop)
    {
        Ok(window) => window,
        Err(e) => finish(&server, HandOff::Failed(format!("cannot open the window: {e}"))),
    };
    let mut context = WebContext::new(data_dir(&server));
    let origin = server.origin().to_owned();
    let builder = WebViewBuilder::new_with_web_context(&mut context)
        .with_url(server.http("/api/v1/setup/status"))
        .with_on_page_load_handler(move |event, url| {
            if matches!(event, PageLoadEvent::Finished) {
                let _ = proxy.send_event(UserEvent::Loaded(url));
            }
        })
        // The sign-in may pass through other https sites (an identity provider); nothing else.
        .with_navigation_handler(move |url| url.starts_with("https://") || url.starts_with(&origin) || url == "about:blank")
        .with_new_window_req_handler(|_, _| NewWindowResponse::Deny);

    #[cfg(not(target_os = "linux"))]
    let webview = builder.build(&window);
    #[cfg(target_os = "linux")]
    let webview = {
        use tao::platform::unix::WindowExtUnix;
        use wry::WebViewBuilderExtUnix;
        builder.build_gtk(window.default_vbox().expect("tao windows have a vbox on Linux"))
    };
    let webview = match webview {
        Ok(webview) => webview,
        Err(e) => finish(&server, HandOff::Failed(format!("cannot open the web view: {e}"))),
    };

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::UserEvent(UserEvent::Loaded(url)) if signed_in_at(&server, &url) => {
                let outcome = match webview.cookies_for_url(&server.http("/")) {
                    Ok(cookies) => HandOff::SignedIn(GatewayCookies {
                        server: server.origin().to_owned(),
                        cookies: cookies.iter().map(gateway_cookie).collect(),
                    }),
                    Err(e) => HandOff::Failed(format!("cannot read the cookies: {e}")),
                };
                finish(&server, outcome);
            }
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => finish(&server, HandOff::Cancelled),
            _ => {}
        }
    })
}

/// The page is the server's own, not the gateway's sign-in.
fn signed_in_at(server: &ServerUrl, url: &str) -> bool {
    let Some(rest) = url.strip_prefix(server.origin()) else { return false };
    (rest.is_empty() || rest.starts_with('/')) && !rest.starts_with("/__exe.dev")
}

fn gateway_cookie(c: &wry::cookie::Cookie<'static>) -> GatewayCookie {
    GatewayCookie {
        name: c.name().to_owned(),
        value: Secret::new(c.value()),
        domain: c.domain().unwrap_or_default().to_owned(),
        path: c.path().unwrap_or("/").to_owned(),
        secure: c.secure().unwrap_or(false),
        expires: c.expires_datetime().map(|at| at.unix_timestamp()),
    }
}

fn finish(server: &ServerUrl, outcome: HandOff) -> ! {
    let mut stdout = std::io::stdout().lock();
    let _ = writeln!(stdout, "{}", hand_off_line(server, &outcome));
    let _ = stdout.flush();
    std::process::exit(0);
}

/// `<data dir>/orbit-desktop/gateway/<host_port>`.
fn data_dir(server: &ServerUrl) -> Option<PathBuf> {
    let name: String = server.origin().split_once("://").map_or("", |(_, rest)| rest)
        .chars().map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' { c } else { '_' }).collect();
    Some(dirs::data_dir()?.join("orbit-desktop").join("gateway").join(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_servers_own_pages_end_the_sign_in() {
        let server = ServerUrl::parse("https://orbit.example.com").unwrap();
        assert!(signed_in_at(&server, "https://orbit.example.com/api/v1/setup/status"));
        assert!(signed_in_at(&server, "https://orbit.example.com"));
        assert!(!signed_in_at(&server, "https://orbit.example.com/__exe.dev/login?redirect=%2F"));
        assert!(!signed_in_at(&server, "https://orbit.example.com.evil.test/api"));
        assert!(!signed_in_at(&server, "https://exe.dev/login"));
    }
}
