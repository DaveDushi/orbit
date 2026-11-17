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

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

use orbit_client::ServerUrl;
use orbit_client::update::{Latest, newer};
use tao::dpi::{PhysicalPosition, PhysicalSize};
use tao::event::{Event, StartCause, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tao::window::{Window, WindowBuilder};
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use wry::{NewWindowResponse, WebContext, WebView, WebViewBuilder};

use crate::bridge::{self, Action, Message, Unread};
use crate::instance::Instance;
use crate::route::{self, Route};
use crate::settings::{Area, MIN_HEIGHT, MIN_WIDTH, Settings, WindowState};
use crate::{autostart, diagnostics, signin};

const ICON_PNG: &[u8] = include_bytes!("../assets/orbit-64.png");
const PICK_HTML: &str = include_str!("../assets/pick.html");
const CONNECT_HTML: &str = include_str!("../assets/connect.html");
const VERSION: &str = env!("CARGO_PKG_VERSION");
/// Windows installs an update itself (the Inno Setup installer, silent, then Orbit starts again); macOS and
/// Linux open the download in the browser.
const INSTALLS_UPDATES: bool = cfg!(windows);

enum UserEvent {
    Tray(TrayIconEvent),
    Menu(MenuEvent),
    /// A bridge message and the URL of the page that sent it.
    Ipc { from: String, body: String },
    /// A link the page wants in a new window.
    NewWindow(String),
    Title(String),
    NotificationClicked(Click),
    /// A second start of the app (see `instance.rs`).
    Show,
    Downloaded { url: String, path: Option<PathBuf>, ok: bool },
    /// Words, an error or the end of a dictation on Windows' recognizer (`speech.rs`), as JSON for the page.
    #[cfg_attr(not(windows), allow(dead_code))]
    Speech(String),
}

/// What a click on an OS notification does.
enum Click {
    Show,
    /// The page's own notice: the bridge runs its click handler.
    Page(u64),
    /// A finished download: show the file in its folder.
    Reveal(PathBuf),
    Update,
}

/// A newer version from `latest.json`.
struct Update {
    version: String,
    url: String,
}

struct TrayMenu {
    open: MenuItem,
    autostart: CheckMenuItem,
    update: MenuItem,
    server: MenuItem,
    diagnostics: MenuItem,
    quit: MenuItem,
}

/// Opens the app. `hidden`: start in the tray (the autostart entry passes `--hidden`).
/// `instance`: this app's single-instance lock, freed before a restart.
pub fn run(hidden: bool, mut instance: Option<Instance>) -> ! {
    let mut settings = Settings::load();
    let server = settings.server.as_deref().and_then(|s| ServerUrl::parse(s).ok());
    diagnostics::log(&format!("start: Orbit {} on {}, server {}", env!("CARGO_PKG_VERSION"), std::env::consts::OS, server.as_ref().map_or("none", |s| s.origin())));
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
    // The update installer's URL while it downloads: it goes to the temp folder, not to Downloads.
    let pending: Arc<Mutex<Option<String>>> = Arc::default();
    let webview = build_webview(&window, &mut context, server.as_ref(), &proxy, &pending).unwrap_or_else(|e| fail(&format!("cannot open the web view: {e}")));

    #[cfg(target_os = "macos")]
    let _app_menu = mac_app_menu();
    let menu = TrayMenu {
        open: MenuItem::new("Open Orbit", true, None),
        autostart: CheckMenuItem::new(autostart::label(), true, settings.start_with_os, None),
        update: MenuItem::new("Check for updates", true, None),
        server: MenuItem::new("Change server…", true, None),
        diagnostics: MenuItem::new("Open diagnostics log", true, None),
        quit: MenuItem::new("Quit Orbit", true, None),
    };
    let mut tray: Option<TrayIcon> = None;
    let mut unread = Unread::default();
    let mut update: Option<Update> = None;
    // The installer is downloading (Windows).
    let mut updating = false;
    // The person asked for a check: say the result also when there is no update.
    let mut manual_check = false;
    #[cfg(windows)]
    let mut dictation = crate::speech::Dictation::default();
    // The start page is showing: only it may pick a server (it has no URL of its own).
    let mut picking = server.is_none();

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        let _keep = &context;
        let before = unread;
        // The Windows overlay is lost while the window is hidden (its taskbar button is gone):
        // set it again when the window is back in front.
        let mut redraw_badge = false;
        // A menu item of the tray, the page or a notification: one place handles them all (below).
        let mut action: Option<Action> = None;
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
                    if tray.is_some() {
                        // Once: a hidden window looks like a quit, and Windows 11 hides the tray icon behind ^.
                        if !settings.close_hint_shown {
                            settings.close_hint_shown = true;
                            diagnostics::log("close: hidden to the tray, first time (notice shown)");
                            notify("Orbit is still running".into(), CLOSE_HINT.into(), Click::Show, &proxy);
                        }
                        save_window(&mut settings, &window, normal);
                        window.set_visible(false);
                    } else {
                        save_window(&mut settings, &window, normal);
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
                    action = Some(Action::Quit);
                } else if id == menu.autostart.id() {
                    action = Some(Action::Autostart(menu.autostart.is_checked()));
                } else if id == menu.update.id() {
                    action = Some(if update.is_some() { Action::Update } else { Action::Check });
                } else if id == menu.diagnostics.id() {
                    action = Some(Action::Diagnostics);
                } else if id == menu.server.id() {
                    action = Some(Action::ChangeServer);
                }
            }
            Event::UserEvent(UserEvent::Ipc { from, body }) => {
                let Some(message) = bridge::parse(&body) else { return };
                let from_orbit = server.as_ref().is_some_and(|s| route::orbit_page(s, &from));
                // The app's own pages (start page, connect page) have no http URL.
                let own_page = !from.starts_with("http");
                match message {
                    Message::Server(input) if picking && own_page => match ServerUrl::parse(&input) {
                        Ok(picked) => {
                            diagnostics::log(&format!("server picked: {}", picked.origin()));
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
                    Message::Action(a @ (Action::ChangeServer | Action::Home)) if own_page => action = Some(a),
                    Message::Log(text) if own_page => diagnostics::log(&text),
                    _ if !from_orbit => {}
                    Message::Action(a) => action = Some(a),
                    Message::Ready => {
                        // A download keeps the page; a new page means the installer link showed a page instead
                        // (the gateway's sign-in after its cookie expired): the button works again.
                        updating = false;
                        push_state(&webview, &settings, update.as_ref(), updating);
                    }
                    Message::Notify { id, title, body, .. } => {
                        unread.notified(window.is_visible() && window.is_focused());
                        notify(title, body, Click::Page(id), &proxy);
                    }
                    Message::Badge(badge) => unread.set_web(badge),
                    Message::Open(url) => follow(server.as_ref(), &webview, &url),
                    Message::Latest(json) => {
                        let latest = serde_json::from_str::<Latest>(&json).ok();
                        let manual = std::mem::take(&mut manual_check);
                        let found = latest.as_ref().and_then(|l| newer(VERSION, l)).and_then(|l| Some((l.version.clone(), l.download()?.to_owned())));
                        match found {
                            // The check runs every 6 hours: tell once per new version (and after a check from the menu).
                            Some((version, url)) => {
                                if manual || update.as_ref().is_none_or(|u| u.url != url) {
                                    diagnostics::log(&format!("update: {version} is available"));
                                    menu.update.set_text(if INSTALLS_UPDATES { format!("Restart to update to Orbit {version}") } else { format!("Download Orbit {version}") });
                                    let body = if INSTALLS_UPDATES { format!("Orbit {version} is ready. Click here to restart and update.") } else { format!("Orbit {version} is ready. Click here to download it.") };
                                    notify("Orbit update available".into(), body, Click::Update, &proxy);
                                    update = Some(Update { version, url });
                                    push_state(&webview, &settings, update.as_ref(), updating);
                                }
                            }
                            None if manual => {
                                diagnostics::log(&format!("update check from the menu: {}", if latest.is_some() { "up to date" } else { "failed" }));
                                let (title, body) = match latest {
                                    Some(_) => ("Orbit is up to date".to_owned(), format!("You have the newest version, {VERSION}.")),
                                    None => ("Cannot check for updates".to_owned(), "Orbit could not read the update information. Try again later.".to_owned()),
                                };
                                notify(title, body, Click::Show, &proxy);
                            }
                            None => {}
                        }
                    }
                    Message::Log(text) => diagnostics::log(&format!("page: {text}")),
                    #[cfg(windows)]
                    Message::Speech(command) => {
                        let proxy = std::sync::Mutex::new(proxy.clone());
                        dictation.handle(command, move |event| {
                            let _ = proxy.lock().map(|p| p.send_event(UserEvent::Speech(event.to_string())));
                        });
                    }
                    #[cfg(not(windows))]
                    Message::Speech(_) => {}
                    Message::Server(_) => {}
                }
            }
            Event::UserEvent(UserEvent::Speech(json)) => {
                let _ = webview.evaluate_script(&format!("window.__orbitDesktop && window.__orbitDesktop.speech({json})"));
            }
            Event::UserEvent(UserEvent::NewWindow(url)) => follow(server.as_ref(), &webview, &url),
            Event::UserEvent(UserEvent::Title(title)) => {
                let title = title.trim();
                window.set_title(if title.is_empty() { "Orbit" } else { title });
            }
            Event::UserEvent(UserEvent::NotificationClicked(click)) => match click {
                Click::Show => show(&window),
                Click::Page(id) => {
                    show(&window);
                    let _ = webview.evaluate_script(&format!("window.__orbitDesktop && window.__orbitDesktop.clicked({id})"));
                }
                Click::Reveal(path) => reveal(&path),
                Click::Update => action = Some(Action::Update),
            },
            Event::UserEvent(UserEvent::Downloaded { url, path, ok }) => {
                let is_update = pending.lock().map(|mut p| if p.as_deref() == Some(url.as_str()) { p.take() } else { None }).ok().flatten().is_some();
                if is_update {
                    updating = false;
                    match path.filter(|_| ok).map(|p| run_installer(&p)) {
                        Some(Ok(())) => {
                            diagnostics::log("update: installer started, Orbit quits");
                            save_window(&mut settings, &window, normal);
                            drop(instance.take());
                            *control_flow = ControlFlow::Exit;
                            return;
                        }
                        failed => {
                            let reason = match failed {
                                Some(Err(e)) => format!("the installer did not start: {e}"),
                                _ => "the download failed".to_owned(),
                            };
                            diagnostics::log(&format!("update: {reason}"));
                            notify("Update failed".into(), format!("Orbit could not update: {reason}. Try again from the account menu or the tray icon."), Click::Show, &proxy);
                            push_state(&webview, &settings, update.as_ref(), updating);
                        }
                    }
                } else {
                    let name = path.as_deref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    let folder = path.as_deref().and_then(|p| p.parent()).map(|d| d.display().to_string()).unwrap_or_default();
                    match path {
                        Some(path) if ok => notify("Download finished".into(), format!("{name} is saved in {folder}. Click to show it."), Click::Reveal(path), &proxy),
                        _ => notify("Download failed".into(), name, Click::Show, &proxy),
                    }
                }
            }
            _ => {}
        }
        if let Some(action) = action {
            diagnostics::log(&format!("action: {action:?}"));
            match action {
                Action::Quit => {
                    save_window(&mut settings, &window, normal);
                    *control_flow = ControlFlow::Exit;
                }
                Action::Autostart(on) => {
                    match autostart::set(on) {
                        Ok(()) => {
                            settings.start_with_os = on;
                            settings.save();
                        }
                        Err(e) => diagnostics::log(&format!("autostart: {e}")),
                    }
                    menu.autostart.set_checked(settings.start_with_os);
                    push_state(&webview, &settings, update.as_ref(), updating);
                }
                Action::Update => match &update {
                    // Only the server's own file loads in the web view (with its gateway cookies); anything else
                    // opens in the browser.
                    Some(u) if INSTALLS_UPDATES && server.as_ref().is_some_and(|s| route::same_origin(s, &u.url)) => {
                        if !updating {
                            updating = true;
                            *pending.lock().unwrap_or_else(|e| e.into_inner()) = Some(u.url.clone());
                            diagnostics::log(&format!("update: downloading {}", u.version));
                            let _ = webview.load_url(&u.url);
                            push_state(&webview, &settings, update.as_ref(), updating);
                        }
                    }
                    Some(u) => open_outside(&u.url),
                    None => {}
                },
                Action::Check => {
                    manual_check = true;
                    let _ = webview.evaluate_script("window.__orbitDesktop && window.__orbitDesktop.checkUpdate()");
                }
                Action::ChangeServer => {
                    let _ = webview.load_html(&pick_page(server.as_ref()));
                    picking = true;
                    unread.reset();
                    show(&window);
                }
                Action::Home => {
                    if let Some(server) = &server {
                        picking = false;
                        go_home(&webview, server);
                    }
                }
                Action::Diagnostics => {
                    if let Some(path) = diagnostics::path()
                        && let Err(e) = open::that_detached(&path)
                    {
                        eprintln!("orbit-desktop: cannot open {}: {e}", path.display());
                    }
                }
            }
        }
        if unread != before || redraw_badge {
            show_badge(&window, tray.as_ref(), &unread, &icon_rgba, icon_size);
        }
    })
}

const CLOSE_HINT: &str = if cfg!(windows) {
    "Orbit keeps running in the background, so notifications still come. Open it again from the Start menu. To quit, use Quit Orbit in the account menu."
} else {
    "Orbit keeps running in the background, so notifications still come. To quit, use Quit Orbit in the account menu."
};

/// Sends the app's state to the page (the account menu's "Desktop app" items and the update banner).
fn push_state(webview: &WebView, settings: &Settings, update: Option<&Update>, updating: bool) {
    let state = serde_json::json!({
        "app": VERSION,
        "os": std::env::consts::OS,
        "autostart": settings.start_with_os,
        "autostartLabel": autostart::label(),
        "update": update.map(|u| &u.version),
        "updating": updating,
        "install": INSTALLS_UPDATES,
    });
    let _ = webview.evaluate_script(&format!("window.__orbitDesktop && window.__orbitDesktop.setState && window.__orbitDesktop.setState({state})"));
}

/// The update installer, silent (a progress window, no questions); `/relaunch=1` starts Orbit again at its end
/// (`packaging/windows/orbit.iss`).
fn run_installer(path: &Path) -> std::io::Result<()> {
    Command::new(path).args(["/SILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/relaunch=1"]).spawn().map(drop)
}

/// Shows a downloaded file in Explorer (selected), Finder, or its folder.
fn reveal(path: &Path) {
    diagnostics::log("download: shown in its folder");
    #[cfg(windows)]
    let shown = {
        use std::os::windows::process::CommandExt;
        // explorer reads `/select,"<path>"` as one raw argument (a Windows path has no quotes in it).
        Command::new("explorer").raw_arg(format!("/select,\"{}\"", path.display())).spawn().map(drop)
    };
    #[cfg(target_os = "macos")]
    let shown = Command::new("open").arg("-R").arg(path).spawn().map(drop);
    #[cfg(all(unix, not(target_os = "macos")))]
    let shown = open::that_detached(path.parent().unwrap_or(path));
    if let Err(e) = shown {
        eprintln!("orbit-desktop: cannot show {}: {e}", path.display());
    }
}

/// The page that checks the server and then loads it (`assets/connect.html`). `http://` servers (this computer
/// only) load at once: a page without an origin may not reach a loopback address.
fn connect_page(server: &ServerUrl) -> Option<String> {
    server.origin().starts_with("https://").then(|| CONNECT_HTML.replace("__ORBIT_SERVER__", &serde_json::to_string(server.origin()).expect("a string")))
}

fn go_home(webview: &WebView, server: &ServerUrl) {
    let _ = match connect_page(server) {
        Some(html) => webview.load_html(&html),
        None => webview.load_url(server.origin()),
    };
}

fn build_webview(window: &Window, context: &mut WebContext, server: Option<&ServerUrl>, proxy: &EventLoopProxy<UserEvent>, pending: &Arc<Mutex<Option<String>>>) -> wry::Result<WebView> {
    let popups = server.cloned();
    let ipc = proxy.clone();
    let new_window = proxy.clone();
    let title = proxy.clone();
    let downloaded = proxy.clone();
    let update = pending.clone();
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
        .with_navigation_handler(|url| {
            let allowed = route::navigation_allowed(&url);
            if !allowed {
                diagnostics::log("navigation to a file refused (a file dropped outside a drop zone)");
            }
            allowed
        })
        // Ctrl + / Ctrl − / Ctrl 0 and Ctrl + wheel, as in a browser (Windows; WebKit has no switch for it).
        .with_hotkeys_zoom(true)
        .with_download_started_handler(move |url, path| {
            // The update installer goes to the temp folder; the person's downloads go to Downloads.
            let is_update = update.lock().is_ok_and(|p| p.as_deref() == Some(url.as_str()));
            let dir = if is_update { std::env::temp_dir() } else { dirs::download_dir().or_else(dirs::home_dir).unwrap_or_default() };
            let suggested = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            *path = route::download_path(&dir, &suggested, |p| p.exists());
            true
        })
        .with_download_completed_handler(move |url, path, ok| {
            let _ = downloaded.send_event(UserEvent::Downloaded { url, path, ok });
        });
    #[cfg(windows)]
    {
        use wry::WebViewBuilderExtWindows;
        builder = builder.with_additional_browser_args(signin::WEBVIEW2_ARGS);
    }
    builder = match server {
        Some(server) => {
            let builder = builder.with_initialization_script_for_main_only(bridge::script(server.origin()), true);
            match connect_page(server) {
                Some(html) => builder.with_html(html),
                None => builder.with_url(server.origin()),
            }
        }
        None => builder.with_html(pick_page(None)),
    };
    #[cfg(windows)]
    {
        let webview = builder.build(window)?;
        if let Some(server) = server {
            crate::microphone::allow(&webview, server);
        }
        Ok(webview)
    }
    #[cfg(target_os = "macos")]
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
    let added = items.append_items(&[&menu.open, &separator, &menu.autostart, &menu.update, &menu.server, &menu.diagnostics, &PredefinedMenuItem::separator(), &menu.quit]);
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
fn notify(title: String, body: String, click: Click, proxy: &EventLoopProxy<UserEvent>) {
    let proxy = proxy.clone();
    // The thread waits for the click (or the close) of its notification.
    std::thread::spawn(move || {
        let mut click = click;
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
                        let _ = proxy.send_event(UserEvent::NotificationClicked(std::mem::replace(&mut click, Click::Show)));
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
    let current = if window.is_minimized() { None } else { window_state(window) };
    if let Some(state) = WindowState::to_save(window.is_maximized(), current, normal) {
        settings.window = Some(state);
    }
    settings.save();
}

/// The start page. `server`: the saved one (Change server: its address and a Cancel button). The first start
/// shows the build's default server (`ORBIT_DEFAULT_SERVER`, set by the release workflow), if any.
fn pick_page(server: Option<&ServerUrl>) -> String {
    let value = server.map(ServerUrl::origin).or(option_env!("ORBIT_DEFAULT_SERVER")).unwrap_or_default();
    let value = value.replace('&', "&amp;").replace('"', "&quot;").replace('<', "&lt;");
    PICK_HTML.replace("__ORBIT_SERVER__", &value).replace("__ORBIT_CANCEL__", if server.is_some() { "" } else { "hidden" })
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
    // ⌘W closes the window to the tray, ⌘M minimizes it.
    let window = Submenu::with_items("Window", true, &[&PredefinedMenuItem::minimize(None), &PredefinedMenuItem::close_window(None)]);
    let edit = Submenu::with_items("Edit", true, &[
        &PredefinedMenuItem::undo(None),
        &PredefinedMenuItem::redo(None),
        &PredefinedMenuItem::separator(),
        &PredefinedMenuItem::cut(None),
        &PredefinedMenuItem::copy(None),
        &PredefinedMenuItem::paste(None),
        &PredefinedMenuItem::select_all(None),
    ]);
    if let (Ok(app), Ok(edit), Ok(window)) = (app, edit, window) {
        let _ = menu.append_items(&[&app, &edit, &window]);
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
