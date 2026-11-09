//! Orbit desktop. Without arguments it opens the app: server → gateway sign-in window (when the
//! server has an exe.dev gateway) → Orbit sign-in → the first workspace's conversations, kept
//! fresh by its live and events sockets.
//!
//! - `--fixture <dir>`: signed in on synthetic data, no network; with `--snap out.png --scene <name>`
//!   it renders headless (scenes below).
//! - `--gateway-signin <server>`: the sign-in window helper (the app runs it; see `signin.rs`).
//! - `--check-gateway <server>`: the risk-4b check (see `check.rs`).

mod app;
mod check;
mod keychain;
mod signin;
mod view;

use std::path::Path;

use app::{Link, Screen, State};
use orbit_client::{Fixture, ServerUrl};
use rune::AppSpec;

fn fixture_state(dir: &Path) -> State {
    let mut s = State { server: ServerUrl::parse("orbit.example.com").ok(), ..State::default() };
    s.server_edit.replace_text("orbit.example.com");
    match Fixture::load(dir) {
        Ok(f) => {
            s.screen = Screen::SignedIn;
            s.workspace = f.workspace_name.clone();
            s.rows = f.sidebar();
            s.user = "Ada Example".into();
            s.live = Link::Connected;
            s.events = Link::Connected;
        }
        Err(e) => s.error = Some(e.to_string()),
    }
    s
}

fn init() -> State {
    let args = rune::extra_args();
    match args.iter().position(|a| a == "--fixture").and_then(|i| args.get(i + 1)) {
        Some(dir) => fixture_state(Path::new(dir)),
        None => State::online(),
    }
}

fn select_unread(s: &mut State) {
    s.selected = s.rows.iter().find(|r| r.unread > 0).map(|r| r.id.clone());
}

fn login(s: &mut State) {
    s.screen = Screen::Login;
    s.email.replace_text("ada@example.test");
    s.password.replace_text("correct horse battery");
}

/// Named states for headless snapshots (`--fixture fixtures/demo --snap out.png --scene <name>`).
type Scene = (&'static str, fn(&mut State));

const SCENES: &[Scene] = &[
    ("selected", select_unread),
    ("signed-out", |s| s.screen = Screen::Server),
    ("gateway", |s| {
        s.screen = Screen::Gateway;
        s.busy = true;
    }),
    ("login", login),
    ("login-error", |s| {
        login(s);
        s.password.clear();
        s.error = Some("The email or password is not correct.".into());
    }),
    ("two-factor", |s| {
        s.screen = Screen::TwoFactor;
        s.code.replace_text("4821");
    }),
    ("signed-in", select_unread),
    ("reconnecting", |s| {
        select_unread(s);
        s.live = Link::Retrying(4);
        s.events = Link::Connecting;
    }),
    ("session-ended", |s| {
        login(s);
        s.password.clear();
        s.error = Some("Your session ended. Sign in again.".into());
    }),
];

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let value = |flag: &str| args.iter().position(|a| a == flag).map(|i| args.get(i + 1).cloned().unwrap_or_default());
    if let Some(server) = value("--gateway-signin") {
        signin::run(&server);
    }
    if let Some(server) = value("--check-gateway") {
        std::process::exit(check::run(&server));
    }
    rune::run(AppSpec::<State> {
        scenes: SCENES,
        setup: Some(|s, waker| s.setup(waker)),
        poll: Some(|s, _, _| s.poll()),
        min_size: Some((640.0, 420.0)),
        ..AppSpec::new("Orbit", (900.0, 560.0), init, view::view)
    });
}
