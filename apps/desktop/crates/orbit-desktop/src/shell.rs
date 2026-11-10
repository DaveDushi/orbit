//! The main window: the server's own web UI in a system web view (WebView2 on Windows, WKWebView
//! on macOS, WebKitGTK on Linux), with a tray icon, OS notifications, the unread badge, downloads
//! and "Start with Windows" around it.
//!
//! The web view keeps its cookies in the same per-server profile as the gateway sign-in window
//! (`signin::data_dir`), so the exe.dev sign-in and then Orbit's own sign-in happen in the window,
//! like in a browser, and stay. Without a server the window shows a start page that asks for one;
//! a new server starts the app again on that server's profile.
//!
//! Closing the window hides it to the tray; Quit in the tray menu ends the app.

use std::path::PathBuf;
use std::process::Command;

use orbit_client::ServerUrl;
use orbit_client::update::{Latest, newer};
use tao::dpi::{PhysicalPosition, PhysicalSize};
use tao::event::{Event, StartCause, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tao::window::{Window, WindowBuilder};
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use wry::{NewWindowResponse, WebContext, WebView, WebViewBuilder};

use crate::bridge::{self, Message, Unread};
use crate::instance::Instance;
use crate::route::{self, Route};
use crate::settings::{Area, MIN_HEIGHT, MIN_WIDTH, Settings, WindowState};
use crate::{autostart, signin};

const ICON_PNG: &[u8] = include_bytes!("../assets/orbit-64.png");
const PICK_HTML: &str = include_str!("../assets/pick.html");

enum UserEvent {
    Tray(TrayIconEvent),
    Menu(MenuEvent),
    /// A bridge message and the URL of the page that sent it.
    Ipc { from: String, body: String },
    /// A link the page wants in a new window.
    NewWindow(String),
    Title(String),
    NotificationClicked(u64),
    /// A second start of the app (see `instance.rs`).
    Show,
    Downloaded { path: Option<PathBuf>, ok: bool },
}

struct TrayMenu {
    open: MenuItem,
    autostart: CheckMenuItem,
    update: MenuItem,
    server: MenuItem,
    quit: MenuItem,
}

/// Opens the app. `hidden`: start in the tray (the autostart entry passes `--hidden`).
/// `instance`: this app's single-instance lock, freed before a restart.
pub fn run(hidden: bool, mut instance: Option<Instance>) -> ! {
    let mut settings = Settings::load();
    let server = settings.server.as_deref().and_then(|s| ServerUrl::parse(s).ok());
    if settings.start_with_os
        && let Err(e) = autostart::set(true)
    {
        eprintln!("orbit-desktop: autostart: {e}");
    }

    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    TrayIconEvent::set_event_handler(Some({
        let proxy = proxy.clone();
        move |event| {
            let _ = proxy.send_event(UserEvent::Tray(event));
        }
    }));
    if let Some(instance) = &instance {
        let proxy = proxy.clone();
        if let Err(e) = instance.serve(move || {
            let _ = proxy.send_event(UserEvent::Show);
        }) {
            eprintln!("orbit-desktop: a second start cannot reach this one: {e}");
        }
    }
    MenuEvent::set_event_handler(Some({
        let proxy = proxy.clone();
        move |event| {
            let _ = proxy.send_event(UserEvent::Menu(event));
        }
    }));

    let (icon_rgba, icon_size) = app_icon();
    let monitors: Vec<Area> = event_loop.available_monitors().map(|m| {
        let (at, size) = (m.position(), m.size());
        Area { x: at.x, y: at.y, width: size.width, height: size.height }
    }).collect();
    let saved = settings.window.and_then(|w| w.fit(&monitors));
    let mut builder = WindowBuilder::new()
        .with_title("Orbit")
        .with_visible(false)
        .with_min_inner_size(tao::dpi::LogicalSize::new(MIN_WIDTH, MIN_HEIGHT))
        .with_inner_size(tao::dpi::LogicalSize::new(1200.0, 800.0))
        .with_window_icon(tao::window::Icon::from_rgba(icon_rgba.clone(), icon_size, icon_size).ok());
    if let Some(state) = saved {
        builder = builder
            .with_position(PhysicalPosition::new(state.x, state.y))
            .with_inner_size(PhysicalSize::new(state.width, state.height))
            .with_maximized(state.maximized);
    }
    let window = builder.build(&event_loop).unwrap_or_else(|e| fail(&format!("cannot open a window: {e}")));
    let mut normal = saved;

    let mut context = WebContext::new(server.as_ref().map_or_else(start_page_dir, signin::data_dir));
    let webview = build_webview(&window, &mut context, server.as_ref(), &proxy).unwrap_or_else(|e| fail(&format!("cannot open the web view: {e}")));

    #[cfg(target_os = "macos")]
    let _app_menu = mac_app_menu();
    let menu = TrayMenu {
        open: MenuItem::new("Open Orbit", true, None),
        autostart: CheckMenuItem::new(autostart::label(), true, settings.start_with_os, None),
        update: MenuItem::new("Update available", false, None),
        server: MenuItem::new("Change server…", true, None),
        quit: MenuItem::new("Quit Orbit", true, None),
    };
    let mut tray: Option<TrayIcon> = None;
    let mut unread = Unread::default();
    let mut update_url: Option<String> = None;
    // The start page is showing: only it may pick a server (it has no URL of its own).
    let mut picking = server.is_none();

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        let _keep = &context;
        let before = unread;
        // The Windows overlay is lost while the window is hidden (its taskbar button is gone):
        // set it again when the window is back in front.
        let mut redraw_badge = false;
        match event {
            Event::NewEvents(StartCause::Init) => {
                // The tray exists once the loop runs (macOS needs that; it is fine elsewhere).
                tray = build_tray(&menu, &icon_rgba, icon_size);
                if !hidden || tray.is_none() {
                    show(&window);
                }
            }
            Event::WindowEvent { event, .. } => match event {
                WindowEvent::CloseRequested => {
                    save_window(&mut settings, &window, normal);
                    if tray.is_some() {
                        window.set_visible(false);
                    } else {
                        *control_flow = ControlFlow::Exit;
                    }
                }
                WindowEvent::Moved(_) | WindowEvent::Resized(_) => {
                    if window.is_visible() && !window.is_maximized() && !window.is_minimized() {
                        normal = window_state(&window).or(normal);
                    }
                }
                // the overlay's size follows the display's scale
                WindowEvent::ScaleFactorChanged { .. } => redraw_badge = true,
                WindowEvent::Focused(true) => {
                    unread.seen();
                    // `show` focuses the window; by then its taskbar button is back.
                    redraw_badge = true;
                }
                _ => {}
            },
            Event::UserEvent(UserEvent::Tray(TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. }) | UserEvent::Show) => {
                show(&window);
            }
            Event::UserEvent(UserEvent::Tray(_)) => {}
            Event::UserEvent(UserEvent::Menu(event)) => {
                let id = event.id();
                if id == menu.open.id() {
                    show(&window);
                } else if id == menu.quit.id() {
                    save_window(&mut settings, &window, normal);
                    *control_flow = ControlFlow::Exit;
                } else if id == menu.autostart.id() {
                    let on = menu.autostart.is_checked();
                    match autostart::set(on) {
                        Ok(()) => {
                            settings.start_with_os = on;
                            settings.save();
                        }
                        Err(e) => {
                            eprintln!("orbit-desktop: autostart: {e}");
                            menu.autostart.set_checked(!on);
                        }
                    }
                } else if id == menu.update.id() {
                    if let Some(url) = &update_url {
                        open_outside(url);
                    }
                } else if id == menu.server.id() {
                    let _ = webview.load_html(&pick_page(server.as_ref()));
                    picking = true;
                    unread.reset();
                    show(&window);
                }
            }
            Event::UserEvent(UserEvent::Ipc { from, body }) => {
                let Some(message) = bridge::parse(&body) else { return };
                let from_orbit = server.as_ref().is_some_and(|s| route::orbit_page(s, &from));
                match message {
                    Message::Server(input) if picking && !from.starts_with("http") => match ServerUrl::parse(&input) {
                        Ok(picked) => {
                            settings.server = Some(picked.origin().to_owned());
                            save_window(&mut settings, &window, normal);
                            // Free the lock first, or the new start hands itself back to this one.
                            drop(instance.take());
                            restart(hidden);
                            *control_flow = ControlFlow::Exit;
                        }
                        Err(e) => {
                            let _ = webview.evaluate_script(&format!("showError({})", serde_json::to_string(&e.to_string()).expect("a string")));
                        }
                    },
                    _ if !from_orbit => {}
                    Message::Notify { id, title, body, .. } => {
                        unread.notified(window.is_visible() && window.is_focused());
                        notify(title, body, Some(id), &proxy);
                    }
                    Message::Badge(badge) => unread.set_web(badge),
                    Message::Open(url) => follow(server.as_ref(), &webview, &url),
                    Message::Latest(json) => {
                        let latest = serde_json::from_str::<Latest>(&json).ok();
                        if update_url.is_none()
                            && let Some(latest) = latest.as_ref().and_then(|l| newer(env!("CARGO_PKG_VERSION"), l))
                            && let Some(url) = latest.download()
                        {
                            update_url = Some(url.to_owned());
                            menu.update.set_text(format!("Download Orbit {}", latest.version));
                            menu.update.set_enabled(true);
                            notify("Orbit update available".into(), format!("Version {} is ready. Use the tray menu to download it.", latest.version), None, &proxy);
                        }
                    }
                    Message::Server(_) => {}
                }
            }
            Event::UserEvent(UserEvent::NewWindow(url)) => follow(server.as_ref(), &webview, &url),
            Event::UserEvent(UserEvent::Title(title)) => {
                let title = title.trim();
                window.set_title(if title.is_empty() { "Orbit" } else { title });
            }
            Event::UserEvent(UserEvent::NotificationClicked(id)) => {
                show(&window);
                let _ = webview.evaluate_script(&format!("window.__orbitDesktop && window.__orbitDesktop.clicked({id})"));
            }
            Event::UserEvent(UserEvent::Downloaded { path, ok }) => {
                let name = path.as_deref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let folder = path.as_deref().and_then(|p| p.parent()).map(|d| d.display().to_string()).unwrap_or_default();
                if ok {
                    notify("Download finished".into(), format!("{name} is saved in {folder}."), None, &proxy);
                } else {
                    notify("Download failed".into(), name, None, &proxy);
                }
            }
            _ => {}
        }
        if unread != before || redraw_badge {
            show_badge(&window, tray.as_ref(), &unread, &icon_rgba, icon_size);
        }
    })
}

fn build_webview(window: &Window, context: &mut WebContext, server: Option<&ServerUrl>, proxy: &EventLoopProxy<UserEvent>) -> wry::Result<WebView> {
    let popups = server.cloned();
    let ipc = proxy.clone();
    let new_window = proxy.clone();
    let title = proxy.clone();
    let downloaded = proxy.clone();
    let mut builder = WebViewBuilder::new_with_web_context(context)
        .with_ipc_handler(move |request| {
            let from = request.uri().to_string();
            let _ = ipc.send_event(UserEvent::Ipc { from, body: request.into_body() });
        })
        .with_new_window_req_handler(move |url, _| {
            // A blank popup is the web view's own window (same profile and cookies); the page keeps its handle
            // and loads an embedded app's sign-in in it. Any other new window is a link (`follow`).
            if popups.as_ref().is_some_and(|server| route::new_window(server, &url) == Route::Popup) {
                return NewWindowResponse::Allow;
            }
            let _ = new_window.send_event(UserEvent::NewWindow(url));
            NewWindowResponse::Deny
        })
        .with_document_title_changed_handler(move |text| {
            let _ = title.send_event(UserEvent::Title(text));
        })
        .with_download_started_handler(|_url, path| {
            let dir = dirs::download_dir().or_else(dirs::home_dir).unwrap_or_default();
            let suggested = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            *path = route::download_path(&dir, &suggested, |p| p.exists());
            true
        })
        .with_download_completed_handler(move |_url, path, ok| {
            let _ = downloaded.send_event(UserEvent::Downloaded { path, ok });
        });
    builder = match server {
        Some(server) => builder
            .with_initialization_script_for_main_only(bridge::script(server.origin()), true)
            .with_url(server.origin()),
        None => builder.with_html(pick_page(None)),
    };
    #[cfg(not(target_os = "linux"))]
    return builder.build(window);
    #[cfg(target_os = "linux")]
    {
        use tao::platform::unix::WindowExtUnix;
        use wry::WebViewBuilderExtUnix;
        builder.build_gtk(window.default_vbox().expect("tao windows have a vbox on Linux"))
    }
}

fn build_tray(menu: &TrayMenu, rgba: &[u8], size: u32) -> Option<TrayIcon> {
    let items = Menu::new();
    let separator = PredefinedMenuItem::separator();
    let added = items.append_items(&[&menu.open, &separator, &menu.autostart, &menu.update, &menu.server, &PredefinedMenuItem::separator(), &menu.quit]);
    let icon = tray_icon::Icon::from_rgba(rgba.to_vec(), size, size).ok()?;
    let built = added.map_err(|e| e.to_string()).and_then(|()| {
        TrayIconBuilder::new()
            .with_menu(Box::new(items))
            .with_menu_on_left_click(cfg!(target_os = "macos"))
            .with_tooltip("Orbit")
            .with_icon(icon)
            .build()
            .map_err(|e| e.to_string())
    });
    // No tray (a Linux desktop without an indicator host): the window stays the only way in.
    built.map_err(|e| eprintln!("orbit-desktop: no tray icon: {e}")).ok()
}

/// The page asked for `url` (a click on a link to another site, or a new window).
fn follow(server: Option<&ServerUrl>, webview: &WebView, url: &str) {
    let Some(server) = server else { return };
    match route::route(server, url) {
        Route::InApp => {
            let _ = webview.load_url(url);
        }
        Route::External => open_outside(url),
        Route::Deny | Route::Popup => {}
    }
}

fn open_outside(url: &str) {
    if let Err(e) = open::that_detached(url) {
        eprintln!("orbit-desktop: cannot open the link in the browser: {e}");
    }
}

/// An OS notification. A click brings the window to the front and, for a page's notice (`id`),
/// does what the web app does on a click: the bridge opens that conversation.
fn notify(title: String, body: String, id: Option<u64>, proxy: &EventLoopProxy<UserEvent>) {
    let proxy = proxy.clone();
    // The thread waits for the click (or the close) of its notification.
    std::thread::spawn(move || {
        let mut notification = notify_rust::Notification::new();
        notification.summary(&title).body(&body).appname("Orbit");
        // D-Bus servers report a body click only for a "default" action (Windows and macOS always
        // do; there an action would add a button).
        #[cfg(all(unix, not(target_os = "macos")))]
        notification.action("default", "Open");
        #[cfg(windows)]
        if let Some(app_id) = crate::app_id::for_toasts() {
            notification.app_id(app_id);
        }
        let shown = notification.show().map_err(|e| e.to_string()).and_then(|handle| {
            handle
                .wait_for_response(|response: &notify_rust::NotificationResponse| {
                    if clicked(response) {
                        let _ = proxy.send_event(UserEvent::NotificationClicked(id.unwrap_or(0)));
                    }
                })
                .map_err(|e| e.to_string())
        });
        if let Err(e) = shown {
            eprintln!("orbit-desktop: notification: {e}");
        }
    });
}

/// A click on the notification's body (`Default`), or on the "default" action of a D-Bus server
/// that reports it as an action.
fn clicked(response: &notify_rust::NotificationResponse) -> bool {
    use notify_rust::NotificationResponse::{Action, Default};
    matches!(response, Default) || matches!(response, Action(key) if key == "default")
}

fn show_badge(window: &Window, tray: Option<&TrayIcon>, unread: &Unread, icon: &[u8], size: u32) {
    let label = unread.label();
    if let Some(tray) = tray {
        let _ = tray.set_tooltip(Some(match &label {
            Some(label) if label != "•" => format!("Orbit: {label} unread"),
            Some(_) => "Orbit: unread messages".to_owned(),
            None => "Orbit".to_owned(),
        }));
        let rgba = if label.is_some() { with_dot(icon, size) } else { icon.to_vec() };
        let _ = tray.set_icon(tray_icon::Icon::from_rgba(rgba, size, size).ok());
    }
    #[cfg(windows)]
    {
        use tao::platform::windows::WindowExtWindows;
        // the taskbar draws the overlay at its small-icon size: 16 px × the display scale
        let px = (16.0 * window.scale_factor()).round().max(16.0) as u32;
        let overlay = crate::overlay::mark(unread.badge()).and_then(|m| tao::window::Icon::from_rgba(crate::overlay::rgba(&m, px), px, px).ok());
        window.set_overlay_icon(overlay.as_ref());
    }
    #[cfg(target_os = "macos")]
    {
        use tao::platform::macos::WindowExtMacOS;
        window.set_badge_label(label);
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        use tao::platform::unix::WindowExtUnix;
        let count = match unread.badge() {
            bridge::Badge::Count(n) => i64::from(n),
            bridge::Badge::Flag => 1,
        };
        window.set_badge_count((count > 0).then_some(count), Some("orbit-desktop.desktop".into()));
        let _ = label;
    }
}

fn show(window: &Window) {
    window.set_visible(true);
    window.set_minimized(false);
    window.set_focus();
}

fn window_state(window: &Window) -> Option<WindowState> {
    let at = window.outer_position().ok()?;
    let size = window.inner_size();
    Some(WindowState { x: at.x, y: at.y, width: size.width, height: size.height, maximized: false })
}

fn save_window(settings: &mut Settings, window: &Window, normal: Option<WindowState>) {
    let maximized = window.is_maximized();
    let Some(state) = (if maximized { normal } else { window_state(window).or(normal) }) else { return };
    settings.window = Some(WindowState { maximized, ..state });
    settings.save();
}

fn pick_page(server: Option<&ServerUrl>) -> String {
    let value = server.map(|s| s.origin().replace('&', "&amp;").replace('"', "&quot;").replace('<', "&lt;")).unwrap_or_default();
    PICK_HTML.replace("__ORBIT_SERVER__", &value)
}

/// The start page's own web view profile: it never holds a server's cookies.
fn start_page_dir() -> Option<PathBuf> {
    Some(dirs::data_dir()?.join("orbit-desktop").join("start"))
}

/// Starts the app again, for a new server (each server has its own web view profile).
fn restart(hidden: bool) {
    let Ok(exe) = std::env::current_exe() else { return };
    let mut command = Command::new(exe);
    if hidden {
        command.arg(autostart::HIDDEN_FLAG);
    }
    if let Err(e) = command.spawn() {
        eprintln!("orbit-desktop: cannot start again: {e}");
    }
}

fn app_icon() -> (Vec<u8>, u32) {
    let image = image::load_from_memory(ICON_PNG).expect("the embedded icon is a PNG").into_rgba8();
    let size = image.width();
    (image.into_raw(), size)
}

/// The app icon with a red dot at its top right (the tray, while something is unread).
fn with_dot(icon: &[u8], size: u32) -> Vec<u8> {
    let mut rgba = icon.to_vec();
    let r = size as f32 * 0.22;
    paint_dot(&mut rgba, size, size as f32 - r, r, r);
    rgba
}

fn paint_dot(rgba: &mut [u8], size: u32, cx: f32, cy: f32, r: f32) {
    for y in 0..size {
        for x in 0..size {
            let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
            let cover = (r - d + 0.5).clamp(0.0, 1.0);
            if cover > 0.0 {
                let i = ((y * size + x) * 4) as usize;
                for (c, v) in [220.0, 38.0, 38.0].into_iter().enumerate() {
                    rgba[i + c] = (v * cover + f32::from(rgba[i + c]) * (1.0 - cover)) as u8;
                }
                rgba[i + 3] = (255.0 * cover + f32::from(rgba[i + 3]) * (1.0 - cover)) as u8;
            }
        }
    }
}

/// macOS: the app menu with Edit, so ⌘C / ⌘V / ⌘A work in the web view, and ⌘Q.
#[cfg(target_os = "macos")]
fn mac_app_menu() -> Menu {
    use tray_icon::menu::Submenu;
    let menu = Menu::new();
    let app = Submenu::with_items("Orbit", true, &[&PredefinedMenuItem::hide(None), &PredefinedMenuItem::separator(), &PredefinedMenuItem::quit(None)]);
    let edit = Submenu::with_items("Edit", true, &[
        &PredefinedMenuItem::undo(None),
        &PredefinedMenuItem::redo(None),
        &PredefinedMenuItem::separator(),
        &PredefinedMenuItem::cut(None),
        &PredefinedMenuItem::copy(None),
        &PredefinedMenuItem::paste(None),
        &PredefinedMenuItem::select_all(None),
    ]);
    if let (Ok(app), Ok(edit)) = (app, edit) {
        let _ = menu.append_items(&[&app, &edit]);
    }
    menu.init_for_nsapp();
    menu
}

fn fail(message: &str) -> ! {
    eprintln!("orbit-desktop: {message}");
    std::process::exit(1);
}

#[cfg(test)]
mod tests {
    use notify_rust::{CloseReason, NotificationResponse};

    use super::clicked;

    #[test]
    fn only_a_click_on_the_notification_opens_it() {
        assert!(clicked(&NotificationResponse::Default));
        assert!(clicked(&NotificationResponse::Action("default".into())));
        assert!(!clicked(&NotificationResponse::Action("__closed".into())));
        assert!(!clicked(&NotificationResponse::Closed(CloseReason::Dismissed)));
        assert!(!clicked(&NotificationResponse::Closed(CloseReason::Expired)));
    }
}
