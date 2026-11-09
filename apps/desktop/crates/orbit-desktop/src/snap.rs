//! Headless PNGs for goldens and screenshots: one frame of the app on the fixture backend,
//! rendered by GPUI's offscreen renderer (Metal on macOS, wgpu on Linux: lavapipe needs no GPU).
//! `--fixture fixtures/demo --snap out.png --scene <name> --theme dark`.

use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, HeadlessAppContext, Window, px, size};
use orbit_client::live::Ended;
use orbit_client::update::Latest;
use orbit_client::{ApiError, Fixture, SocketEvent, SocketKind};

use crate::app::{Msg, OrbitApp};
use crate::state::{Link, Screen};

const DESIGN: &str = "00000000-0000-4000-8000-0000000000a2";
const GENERAL: &str = "00000000-0000-4000-8000-0000000000a1";
const DONNA: &str = "00000000-0000-4000-8000-0000000000b1";
const THREAD_ROOT: &str = "00000000-0000-4000-8000-000000000003";

/// A name, the conversation to open first (its messages load before the step runs), the step.
type Scene = (&'static str, Option<&'static str>, fn(&mut OrbitApp, &mut Window, &mut gpui_kit::Context<OrbitApp>));

/// Named states for headless snapshots.
const SCENES: &[Scene] = &[
    ("shell", None, |_, _, _| {}),
    ("signed-out", None, |app, _, _| app.screen = Screen::Server),
    ("login-error", None, |app, window, cx| {
        app.screen = Screen::Login;
        app.set_field(&app.inputs.email.clone(), "ada@example.test", window, cx);
        app.error = Some("The email or password is not correct.".into());
    }),
    ("channel", Some(DESIGN), |_, _, _| {}),
    ("markdown", Some(DONNA), |_, _, _| {}),
    ("thread", Some(DESIGN), |app, _, cx| {
        app.open_thread(THREAD_ROOT, cx);
    }),
    ("composer", Some(GENERAL), |app, window, cx| {
        set_composer(app, "Shipping the **desktop** build today", window, cx);
    }),
    ("sent", Some(GENERAL), |app, window, cx| {
        set_composer(app, "The first message from the GPUI app 🎉", window, cx);
        app.submit_composer(false, window, cx);
    }),
    ("send-failed", Some(GENERAL), |app, _, cx| {
        // As the network would fail it (the fixture backend never fails a send).
        let now = crate::app::now_rfc3339();
        if app.chat.queue_send("This one did not go out", false, "desk-snap".into(), &now).is_ok() {
            app.chat.sent("desk-snap", Err(ApiError::Network("connection refused".into()).to_string()));
        }
        app.sync_lists(false, cx);
    }),
    ("editing", Some(DESIGN), |app, window, cx| {
        if let Some(id) = app.chat.last_own_message() {
            app.start_edit(&id, window, cx);
        }
    }),
    ("empty-error", Some(GENERAL), |app, window, cx| {
        app.submit_composer(false, window, cx);
    }),
    ("live", Some(GENERAL), |app, window, cx| {
        let mut message = app.chat.messages[0].message.clone();
        message.id = "live-1".into();
        message.body = "Arrived over the **live** socket just now.".into();
        message.created_at = crate::app::now_rfc3339();
        app.apply(Msg::Socket(0, SocketKind::Live, SocketEvent::Live(orbit_client::LiveFrame::Event {
            seq: Some(1),
            topic: "chat".into(),
            event: serde_json::to_value(serde_json::json!({"type": "message.created", "message": message})).unwrap(),
        })), window, cx);
    }),
    ("reconnecting", Some(GENERAL), |app, _, _| {
        app.live = Link::Retrying(4);
        app.events = Link::Connecting;
    }),
    ("browser-mode", Some(GENERAL), |app, _, _| {
        app.browser_mode = true;
    }),
    ("stopped", Some(GENERAL), |app, window, cx| {
        app.apply(Msg::Socket(0, SocketKind::Events, SocketEvent::Ended(Ended::Forbidden)), window, cx);
    }),
    ("update", Some(GENERAL), |app, window, cx| {
        app.apply(Msg::Update(Latest { version: "9.0.0".into(), macos: Some("https://example.invalid/Orbit.dmg".into()),
            windows: Some("https://example.invalid/Orbit.zip".into()), linux: Some("https://example.invalid/orbit.tar.gz".into()),
            ..Latest::default() }), window, cx);
    }),
];

fn set_composer(app: &mut OrbitApp, text: &str, window: &mut Window, cx: &mut gpui_kit::Context<OrbitApp>) {
    let text = text.to_owned();
    app.inputs.composer.update(cx, |f, cx| f.set_value(text, window, cx));
}

pub fn run(out: &str, fixture: Option<Result<Fixture, String>>, scene: Option<&str>, theme: Option<&str>) -> i32 {
    let scene_name = scene.unwrap_or("shell");
    let Some(&(_, open, scene)) = SCENES.iter().find(|(name, _, _)| *name == scene_name) else {
        let names: Vec<&str> = SCENES.iter().map(|(n, _, _)| *n).collect();
        eprintln!("--scene {scene_name}: unknown; one of {}", names.join(", "));
        return 2;
    };
    let Some(fixture) = fixture else {
        eprintln!("--snap needs --fixture <dir>");
        return 2;
    };
    let mut cx = HeadlessAppContext::with_platform(
        gpui_kit::platform::current_platform(true).text_system(),
        std::sync::Arc::new(gpui_kit::assets::Assets),
        gpui_kit::platform::current_headless_renderer,
    );
    cx.update(gpui_kit::init);
    let dark = theme == Some("dark");
    cx.update(|cx| Theme::change(if dark { ThemeMode::Dark } else { ThemeMode::Light }, None, cx));
    let opened = cx.update(|cx| {
        gpui_kit::open_window(
            gpui_kit::WindowOptions {
                window_bounds: Some(gpui_kit::WindowBounds::Windowed(gpui_kit::Bounds {
                    origin: Default::default(),
                    size: size(px(1100.), px(720.)),
                })),
                focus: false,
                show: false,
                ..Default::default()
            },
            cx,
            |window, cx| cx.new(|cx| OrbitApp::new(Some(fixture), window, cx)),
        )
    });
    let (handle, app) = match opened {
        Ok(opened) => opened,
        Err(e) => {
            eprintln!("--snap: cannot open a headless window: {e}");
            return 1;
        }
    };
    // Let the fixture's answers (channel messages) arrive, then set the scene.
    cx.run_until_parked();
    if let Some(id) = open {
        let _ = cx.update_window(handle, |_, window, cx| app.update(cx, |app, cx| app.select(id, window, cx)));
        cx.run_until_parked();
    }
    let _ = cx.update_window(handle, |_, window, cx| app.update(cx, |app, cx| scene(app, window, cx)));
    for _ in 0..3 {
        cx.run_until_parked();
        let _ = cx.update_window(handle, |_, window, cx| window.render_frame(cx));
    }
    let image = match cx.capture_screenshot(handle) {
        Ok(image) => image,
        Err(e) => {
            eprintln!("--snap: offscreen rendering failed (Linux needs a Vulkan driver, e.g. mesa lavapipe): {e}");
            return 1;
        }
    };
    if let Err(e) = image.save(out) {
        eprintln!("--snap: cannot write {out}: {e}");
        return 1;
    }
    println!("{out}: scene {scene_name}, {} theme, {}x{}", if dark { "dark" } else { "light" }, image.width(), image.height());
    0
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::Arc;

    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{AppContext as _, HeadlessAppContext};
    use orbit_client::Fixture;

    use crate::app::OrbitApp;

    /// Regression (2026-11-09): Up in the empty composer did nothing, because GPUI runs the text
    /// area's own Up binding before key listeners; the edited text then went out as a new message.
    #[test]
    fn up_in_the_empty_composer_edits_the_newest_own_message_and_esc_cancels() {
        let fixture = Fixture::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/demo")).unwrap();
        let mut cx = HeadlessAppContext::with_asset_source(
            gpui_kit::platform::current_platform(true).text_system(),
            Arc::new(gpui_kit::assets::Assets),
        );
        cx.update(gpui_kit::init);
        let (handle, app) = cx
            .update(|cx| gpui_kit::open_window(Default::default(), cx, |window, cx| cx.new(|cx| OrbitApp::new(Some(Ok(fixture)), window, cx))))
            .unwrap();
        cx.update_window(handle, |_, window, cx| {
            app.update(cx, |app, cx| app.select(super::DESIGN, window, cx));
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            let composer = app.read(cx).inputs.composer.clone();
            composer.update(cx, |f, cx| f.focus(window, cx));
            window.press("up", cx);
        })
        .unwrap();
        cx.run_until_parked();
        let (editing, text) = cx.update(|cx| {
            let app = app.read(cx);
            (app.chat.editing.clone(), app.inputs.composer.read(cx).value().to_string())
        });
        assert_eq!(editing.as_deref(), Some("00000000-0000-4000-8000-000000000004"), "Ada's message in #design");
        assert!(text.starts_with("I moved the mock"), "{text}");

        cx.update_window(handle, |_, window, cx| window.press("escape", cx)).unwrap();
        cx.run_until_parked();
        let (editing, text) = cx.update(|cx| {
            let app = app.read(cx);
            (app.chat.editing.clone(), app.inputs.composer.read(cx).value().to_string())
        });
        assert_eq!((editing, text.as_str()), (None, ""));
    }
}
