//! The GPUI app: one entity that owns the state, the text fields and the network work.
//!
//! Network calls and sockets run on their own threads (blocking `ureq` / `tungstenite`) and send
//! [`Msg`]s into a channel; a task on GPUI's main thread applies them and redraws. Logic that can
//! break silently lives in `state.rs` (no UI) and is tested there.
//!
//! Server → (gateway sign-in window, when the server has a gateway) → Orbit sign-in (→ two-factor
//! code) → signed in: the first workspace's conversations, messages and threads, kept current by
//! its live and events sockets.

use std::sync::{Arc, RwLock};

use gpui_kit::component::input::{InputEvent, InputState, TextareaState};
use gpui_kit::component::message_scroller::MessageScrollerState;
use gpui_kit::{App, AppContext as _, Context, Entity, Focusable as _, KeystrokeEvent, Modifiers, Subscription, Window};
use orbit_client::update::{LATEST_PATH, Latest, newer};
use orbit_client::{
    ApiError, AuthUser, Client, ConversationRecord, ConversationStateRecord, Fixture, HandOff, LoginOutcome,
    MemberRecord, MessageRecord, Secret, ServerUrl, Socket, SocketConfig, SocketEvent, SocketKind, ThreadPage,
    needs_browser_mode,
};

use crate::state::{Chat, Link, Outgoing, Screen, SocketAction, first_change, row_signature, socket_event};
use crate::{keychain, signin};

pub enum Msg {
    Probed(Result<(), ApiError>),
    HandOff(Result<HandOff, String>),
    Login(Result<LoginOutcome, ApiError>),
    Loaded(Result<Loaded, ApiError>),
    Refreshed(Result<Sidebar, ApiError>),
    Messages(String, Result<Vec<MessageRecord>, ApiError>),
    Thread(String, Result<ThreadPage, ApiError>),
    Sent(String, Result<MessageRecord, ApiError>),
    Edited(Result<MessageRecord, ApiError>),
    Update(Latest),
    LoggedOut,
    Socket(u64, SocketKind, SocketEvent),
}

pub struct Sidebar {
    conversations: Vec<ConversationRecord>,
    states: Vec<ConversationStateRecord>,
}

pub struct Loaded {
    user: AuthUser,
    /// The first workspace (id, name), if any.
    workspace: Option<(String, String)>,
    sidebar: Sidebar,
    members: Vec<MemberRecord>,
    headers: Vec<(&'static str, Secret)>,
    /// The session as it works now (browser mode may have been switched on).
    session: Option<orbit_client::Session>,
}

/// Where the data comes from.
enum Backend {
    Net(Net),
    /// `--fixture <dir>`: synthetic data, no network, no keychain. Sends and edits "succeed" locally.
    Fixture(Box<Fixture>),
}

struct Net {
    client: Arc<RwLock<Option<Client>>>,
    two_factor: Option<Secret>,
    sockets: Vec<Socket>,
    /// Counts socket starts, so events of stopped sockets are dropped.
    generation: u64,
    /// The bearer session was just made: a refusal now may be the gateway in the way (risk 4b).
    fresh_session: bool,
}

pub struct Inputs {
    pub server: Entity<InputState>,
    pub email: Entity<InputState>,
    pub password: Entity<InputState>,
    pub code: Entity<InputState>,
    pub composer: Entity<TextareaState>,
    pub reply: Entity<TextareaState>,
}

pub struct OrbitApp {
    pub screen: Screen,
    pub busy: bool,
    pub error: Option<String>,
    pub server: Option<ServerUrl>,
    pub user: String,
    pub chat: Chat,
    pub live: Link,
    pub events: Link,
    /// The session sends `Origin` and Orbit's cookie instead of a bearer token (risk-4b fallback).
    pub browser_mode: bool,
    pub update: Option<Latest>,
    /// An error of the composer (empty message, too long), shown above it.
    pub composer_error: Option<String>,
    pub inputs: Inputs,
    pub scroller: Entity<MessageScrollerState>,
    pub thread_scroller: Entity<MessageScrollerState>,
    shown: Vec<u64>,
    shown_thread: Vec<u64>,
    backend: Backend,
    inbox: async_channel::Sender<Msg>,
    _subscriptions: Vec<Subscription>,
}

impl OrbitApp {
    /// The real app (`fixture: None`) or the fixture backend.
    pub fn new(fixture: Option<Result<Fixture, String>>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let field = |placeholder: &str, window: &mut Window, cx: &mut Context<Self>| {
            let placeholder = placeholder.to_owned();
            cx.new(|cx| InputState::new(window, cx).placeholder(placeholder))
        };
        let inputs = Inputs {
            server: field("orbit.example.com", window, cx),
            email: field("you@example.com", window, cx),
            password: cx.new(|cx| InputState::new(window, cx).masked(true)),
            code: field("123456", window, cx),
            composer: cx.new(|cx| TextareaState::new(window, cx).auto_grow(1, 8).submit_on_enter(true).placeholder("Write a message")),
            reply: cx.new(|cx| TextareaState::new(window, cx).auto_grow(1, 6).submit_on_enter(true).placeholder("Reply in thread")),
        };
        let subscriptions = vec![
            cx.subscribe_in(&inputs.server, window, |this, _, e: &InputEvent, window, cx| {
                if matches!(e, InputEvent::PressEnter { .. }) {
                    this.submit_server(window, cx);
                }
            }),
            cx.subscribe_in(&inputs.password, window, |this, _, e: &InputEvent, window, cx| {
                if matches!(e, InputEvent::PressEnter { .. }) {
                    this.submit_login(window, cx);
                }
            }),
            cx.subscribe_in(&inputs.code, window, |this, _, e: &InputEvent, window, cx| {
                if matches!(e, InputEvent::PressEnter { .. }) {
                    this.submit_code(window, cx);
                }
            }),
            cx.subscribe_in(&inputs.composer, window, |this, _, e: &InputEvent, window, cx| match e {
                InputEvent::PressEnter { shift: false, .. } => this.submit_composer(false, window, cx),
                InputEvent::Change => this.composer_error = None,
                _ => {}
            }),
            cx.subscribe_in(&inputs.reply, window, |this, _, e: &InputEvent, window, cx| {
                if matches!(e, InputEvent::PressEnter { shift: false, .. }) {
                    this.submit_composer(true, window, cx);
                }
            }),
        ];
        // Up and Esc in the composer: GPUI runs the text area's own key bindings (Up moves the
        // caret) before key listeners, so they are taken here, before the bindings.
        let (weak, composer) = (cx.weak_entity(), inputs.composer.clone());
        let mut subscriptions = subscriptions;
        subscriptions.push(cx.intercept_keystrokes(move |e: &KeystrokeEvent, window, cx| {
            let key = e.keystroke.key.clone();
            if !matches!(key.as_str(), "up" | "escape") || e.keystroke.modifiers != Modifiers::default() {
                return;
            }
            if !composer.read(cx).focus_handle(cx).is_focused(window) {
                return;
            }
            let _ = weak.update(cx, |app, cx| {
                if app.composer_key(&key, window, cx) {
                    cx.stop_propagation();
                }
            });
        }));
        let scroller = cx.new(|cx| MessageScrollerState::new(0, cx));
        let thread_scroller = cx.new(|cx| MessageScrollerState::new(0, cx));
        cx.observe(&scroller, |_, _, cx| cx.notify()).detach();
        cx.observe(&thread_scroller, |_, _, cx| cx.notify()).detach();

        let (inbox, outbox) = async_channel::unbounded::<Msg>();
        cx.spawn_in(window, async move |this, cx| {
            while let Ok(msg) = outbox.recv().await {
                if this.update_in(cx, |app, window, cx| app.apply(msg, window, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();

        let mut app = OrbitApp {
            screen: Screen::Server,
            busy: false,
            error: None,
            server: None,
            user: String::new(),
            chat: Chat::default(),
            live: Link::Off,
            events: Link::Off,
            browser_mode: false,
            update: None,
            composer_error: None,
            inputs,
            scroller,
            thread_scroller,
            shown: Vec::new(),
            shown_thread: Vec::new(),
            backend: Backend::Net(Net {
                client: Arc::default(),
                two_factor: None,
                sockets: Vec::new(),
                generation: 0,
                fresh_session: false,
            }),
            inbox,
            _subscriptions: subscriptions,
        };
        match fixture {
            Some(Ok(fixture)) => app.start_fixture(fixture, window, cx),
            Some(Err(e)) => {
                app.backend = Backend::Fixture(Box::default());
                app.error = Some(e);
            }
            None => {
                if let Some(server) = keychain::last_server() {
                    app.set_field(&app.inputs.server.clone(), server.origin(), window, cx);
                    app.submit_server(window, cx);
                }
            }
        }
        app
    }

    fn start_fixture(&mut self, fixture: Fixture, window: &mut Window, cx: &mut Context<Self>) {
        self.server = ServerUrl::parse("orbit.example.com").ok();
        self.set_field(&self.inputs.server.clone(), "orbit.example.com", window, cx);
        self.screen = Screen::SignedIn;
        self.chat = Chat::new(&fixture.me, "fixture", &fixture.workspace_name);
        self.chat.set_sidebar(fixture.conversations.clone(), fixture.states.clone());
        self.chat.set_members(&fixture.members);
        self.user = self.chat.name_of(&fixture.me);
        self.live = Link::Connected;
        self.events = Link::Connected;
        self.backend = Backend::Fixture(Box::new(fixture));
    }

    pub fn set_field(&self, field: &Entity<InputState>, text: &str, window: &mut Window, cx: &mut App) {
        let text = text.to_owned();
        field.update(cx, |f, cx| f.set_value(text, window, cx));
    }

    fn set_area(&self, field: &Entity<TextareaState>, text: &str, window: &mut Window, cx: &mut App) {
        let text = text.to_owned();
        field.update(cx, |f, cx| f.set_value(text, window, cx));
    }

    fn net(&mut self) -> Option<&mut Net> {
        match &mut self.backend {
            Backend::Net(net) => Some(net),
            Backend::Fixture(_) => None,
        }
    }

    /// Runs `work` with the client on a thread and posts its answer.
    fn job(&self, work: impl FnOnce(&Client) -> Msg + Send + 'static) {
        let Backend::Net(net) = &self.backend else { return };
        let (client, inbox) = (net.client.clone(), self.inbox.clone());
        std::thread::spawn(move || {
            let msg = match client.read().unwrap().as_ref() {
                Some(client) => work(client),
                None => return,
            };
            let _ = inbox.send_blocking(msg);
        });
    }

    /// Like [`Self::job`], for work that changes the client (sign-in, sign-out, gateway cookies).
    fn job_mut(&mut self, work: impl FnOnce(&mut Client) -> Msg + Send + 'static) {
        let Backend::Net(net) = &self.backend else { return };
        self.busy = true;
        self.error = None;
        let (client, inbox) = (net.client.clone(), self.inbox.clone());
        std::thread::spawn(move || {
            let msg = match client.write().unwrap().as_mut() {
                Some(client) => work(client),
                None => return,
            };
            let _ = inbox.send_blocking(msg);
        });
    }

    pub fn submit_server(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let text = self.inputs.server.read(cx).value().to_string();
        let server = match ServerUrl::parse(&text) {
            Ok(server) => server,
            Err(e) => {
                self.error = Some(e.to_string());
                cx.notify();
                return;
            }
        };
        self.stop_sockets();
        let Some(net) = self.net() else { return };
        let mut client = Client::new(server.clone());
        client.set_gateway(keychain::gateway(&server));
        let session = keychain::session(&server);
        client.set_browser_mode(session.as_ref().is_some_and(|s| s.browser));
        client.set_session(session);
        *net.client.write().unwrap() = Some(client);
        net.fresh_session = false;
        keychain::remember_server(&server);
        self.server = Some(server);
        self.job_mut(|c| Msg::Probed(c.setup_status().map(|_| ())));
        cx.notify();
    }

    pub fn change_server(&mut self, cx: &mut Context<Self>) {
        self.stop_sockets();
        self.screen = Screen::Server;
        self.busy = false;
        self.error = None;
        cx.notify();
    }

    pub fn open_gateway(&mut self, cx: &mut Context<Self>) {
        let Some(server) = self.server.clone() else { return };
        if self.net().is_none() {
            return;
        }
        self.screen = Screen::Gateway;
        self.busy = true;
        self.error = None;
        let inbox = self.inbox.clone();
        std::thread::spawn(move || {
            let _ = inbox.send_blocking(Msg::HandOff(signin::open(&server)));
        });
        cx.notify();
    }

    pub fn submit_login(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let email = self.inputs.email.read(cx).value().trim().to_owned();
        let password = Secret::new(self.inputs.password.read(cx).value().to_string());
        if email.is_empty() || password.is_empty() {
            self.error = Some("Enter your email and password.".into());
            cx.notify();
            return;
        }
        self.set_field(&self.inputs.password.clone(), "", window, cx);
        self.job_mut(move |c| {
            let mut outcome = c.login(&email, &password);
            // The gateway passes its own cookie to Orbit, which then refuses a sign-in without
            // `Origin`: sign in as a browser of the server's own origin instead.
            if let Err(e) = &outcome
                && needs_browser_mode(e, false)
            {
                c.set_browser_mode(true);
                outcome = c.login(&email, &password);
            }
            Msg::Login(outcome)
        });
        cx.notify();
    }

    pub fn submit_code(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(token) = self.net().and_then(|n| n.two_factor.clone()) else { return };
        let code = self.inputs.code.read(cx).value().trim().to_owned();
        if code.is_empty() {
            self.error = Some("Enter the code.".into());
            cx.notify();
            return;
        }
        self.set_field(&self.inputs.code.clone(), "", window, cx);
        self.job_mut(move |c| Msg::Login(c.login_two_factor(&token, &code)));
        cx.notify();
    }

    pub fn sign_out(&mut self, cx: &mut Context<Self>) {
        self.stop_sockets();
        self.job_mut(|c| {
            let _ = c.logout();
            Msg::LoggedOut
        });
        cx.notify();
    }

    fn load(&mut self) {
        let fresh = self.net().is_some_and(|n| std::mem::take(&mut n.fresh_session));
        self.job_mut(move |c| {
            let mut loaded = load(c);
            // Risk 4b: the bearer from a fresh sign-in did not reach Orbit. Send the same session
            // as Orbit's cookie with `Origin` (browser mode) and try once more.
            if let Err(e) = &loaded
                && fresh
                && !c.session().is_some_and(|s| s.browser)
                && needs_browser_mode(e, true)
            {
                c.set_browser_mode(true);
                loaded = load(c);
            }
            Msg::Loaded(loaded)
        });
    }

    /// Opens a conversation and loads its messages.
    pub fn select(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if !self.chat.select(id) {
            return;
        }
        self.composer_error = None;
        self.set_area(&self.inputs.composer.clone(), "", window, cx);
        self.load_messages(id);
        self.sync_lists(true, cx);
        cx.notify();
    }

    fn load_messages(&mut self, id: &str) {
        let (workspace, id) = (self.chat.workspace_id.clone(), id.to_owned());
        match &self.backend {
            Backend::Fixture(f) => {
                let items = f.messages.get(&id).cloned().unwrap_or_default();
                let _ = self.inbox.try_send(Msg::Messages(id, Ok(items)));
            }
            Backend::Net(_) => self.job(move |c| Msg::Messages(id.clone(), c.messages(&workspace, &id).map(|p| p.items))),
        }
    }

    pub fn open_thread(&mut self, root_id: &str, cx: &mut Context<Self>) {
        if !self.chat.open_thread(root_id) {
            return;
        }
        let (workspace, root) = (self.chat.workspace_id.clone(), root_id.to_owned());
        match &self.backend {
            Backend::Fixture(f) => {
                let root_message = f.messages.values().flatten().find(|m| m.id == root).cloned();
                let page = root_message.map(|root_message| ThreadPage {
                    root: root_message,
                    items: f.threads.get(&root).cloned().unwrap_or_default(),
                    before: None,
                    after: None,
                });
                if let Some(page) = page {
                    let _ = self.inbox.try_send(Msg::Thread(root, Ok(page)));
                }
            }
            Backend::Net(_) => self.job(move |c| Msg::Thread(root.clone(), c.thread(&workspace, &root))),
        }
        self.sync_lists(false, cx);
        cx.notify();
    }

    pub fn close_thread(&mut self, cx: &mut Context<Self>) {
        self.chat.close_thread();
        self.sync_lists(false, cx);
        cx.notify();
    }

    /// Enter in the composer (or the thread reply field): send, or save the edit.
    pub fn submit_composer(&mut self, thread: bool, window: &mut Window, cx: &mut Context<Self>) {
        let field = if thread { self.inputs.reply.clone() } else { self.inputs.composer.clone() };
        let text = field.read(cx).value().to_string();
        if !thread && self.chat.editing.is_some() {
            match self.chat.finish_edit(&text) {
                Ok((id, body)) => {
                    self.set_area(&field, "", window, cx);
                    self.edit(id, body);
                }
                Err(e) => self.composer_error = Some(e),
            }
            cx.notify();
            return;
        }
        let nonce = new_nonce();
        match self.chat.queue_send(&text, thread, nonce, &now_rfc3339()) {
            Ok(out) => {
                self.composer_error = None;
                self.set_area(&field, "", window, cx);
                self.send(out);
            }
            Err(e) => self.composer_error = Some(e),
        }
        self.sync_lists(false, cx);
        cx.notify();
    }

    pub fn retry(&mut self, nonce: &str, cx: &mut Context<Self>) {
        if let Some(out) = self.chat.retry(nonce) {
            self.send(out);
        }
        self.sync_lists(false, cx);
        cx.notify();
    }

    fn send(&self, out: Outgoing) {
        let workspace = self.chat.workspace_id.clone();
        match &self.backend {
            Backend::Fixture(_) => {
                if let Some(mut message) = self.local_copy(&out.nonce) {
                    message.id = format!("fixture-{}", out.nonce);
                    let _ = self.inbox.try_send(Msg::Sent(out.nonce, Ok(message)));
                }
            }
            Backend::Net(_) => self.job(move |c| {
                let result = c.send_message(&workspace, &out.conversation_id, out.thread_root_id.as_deref(), &out.nonce, &out.body);
                Msg::Sent(out.nonce, result)
            }),
        }
    }

    fn local_copy(&self, nonce: &str) -> Option<MessageRecord> {
        let thread = self.chat.thread.iter().flat_map(|t| t.replies.iter());
        self.chat.messages.iter().chain(thread).find(|r| r.message.nonce.as_deref() == Some(nonce)).map(|r| r.message.clone())
    }

    /// Edit one of the person's messages in the main composer.
    pub fn start_edit(&mut self, message_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(body) = self.chat.start_edit(message_id) {
            self.composer_error = None;
            let composer = self.inputs.composer.clone();
            self.set_area(&composer, &body, window, cx);
            composer.update(cx, |f, cx| f.focus(window, cx));
        }
        cx.notify();
    }

    /// Up in an empty composer edits the person's newest message; Esc cancels an edit.
    /// True when the key was used.
    fn composer_key(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        match key {
            "escape" if self.chat.editing.is_some() => self.cancel_edit(window, cx),
            "up" if self.chat.editing.is_none() && self.inputs.composer.read(cx).value().is_empty() => {
                let Some(id) = self.chat.last_own_message() else { return false };
                self.start_edit(&id, window, cx);
            }
            _ => return false,
        }
        true
    }

    pub fn cancel_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.chat.cancel_edit();
        self.composer_error = None;
        self.set_area(&self.inputs.composer.clone(), "", window, cx);
        cx.notify();
    }

    fn edit(&self, id: String, body: String) {
        let workspace = self.chat.workspace_id.clone();
        match &self.backend {
            Backend::Fixture(_) => {
                let row = self.chat.messages.iter().find(|r| r.message.id == id).map(|r| r.message.clone());
                if let Some(mut message) = row {
                    message.body = body;
                    message.edited_at = Some(now_rfc3339());
                    let _ = self.inbox.try_send(Msg::Edited(Ok(message)));
                }
            }
            Backend::Net(_) => self.job(move |c| Msg::Edited(c.edit_message(&workspace, &id, &body))),
        }
    }

    pub fn open_url(&self, url: &str, cx: &mut Context<Self>) {
        cx.open_url(url);
    }

    /// Opens the server's web app (for what the desktop app does not do yet).
    pub fn open_in_browser(&self, cx: &mut Context<Self>) {
        if let Some(server) = &self.server {
            cx.open_url(&server.http("/"));
        }
    }

    pub fn apply(&mut self, msg: Msg, _window: &mut Window, cx: &mut Context<Self>) {
        match msg {
            Msg::Probed(Ok(())) => {
                self.busy = false;
                let has_session = self.net().is_some_and(|n| n.client.read().unwrap().as_ref().is_some_and(|c| c.session().is_some()));
                if has_session {
                    self.load();
                } else {
                    self.screen = Screen::Login;
                }
            }
            Msg::Probed(Err(e)) => self.failed(e, Screen::Server, cx),
            Msg::HandOff(result) => {
                self.busy = false;
                if self.screen != Screen::Gateway {
                    return;
                }
                match result {
                    Ok(HandOff::SignedIn(cookies)) => {
                        let server = self.server.clone().expect("a server before the gateway");
                        if let Err(e) = keychain::save_gateway(&server, &cookies) {
                            eprintln!("orbit-desktop: {e}");
                        }
                        self.job_mut(move |c| {
                            c.set_gateway(Some(cookies));
                            Msg::Probed(c.setup_status().map(|_| ()))
                        });
                    }
                    Ok(HandOff::Cancelled) => {
                        self.screen = Screen::Server;
                        self.error = Some("The sign-in window was closed before the sign-in finished.".into());
                    }
                    Ok(HandOff::Failed(e)) | Err(e) => {
                        self.screen = Screen::Server;
                        self.error = Some(format!("The sign-in window failed: {e}"));
                    }
                }
            }
            Msg::Login(Ok(LoginOutcome::TwoFactor(token))) => {
                self.busy = false;
                if let Some(net) = self.net() {
                    net.two_factor = Some(token);
                }
                self.screen = Screen::TwoFactor;
            }
            Msg::Login(Ok(LoginOutcome::SignedIn { .. })) => {
                if let Some(net) = self.net() {
                    net.two_factor = None;
                    net.fresh_session = true;
                }
                self.load();
            }
            Msg::Login(Err(ApiError::SignedOut(p))) => {
                // The sign-in route's 401: wrong password or code.
                self.busy = false;
                self.error = Some(if p.detail.is_empty() { "The email or password is not correct.".into() } else { p.detail });
            }
            Msg::Login(Err(e)) => self.failed(e, self.screen, cx),
            Msg::Loaded(Ok(loaded)) => {
                self.busy = false;
                self.screen = Screen::SignedIn;
                self.error = None;
                self.user = loaded.user.display_name;
                self.browser_mode = loaded.session.as_ref().is_some_and(|s| s.browser);
                if let (Some(server), Some(session)) = (&self.server, &loaded.session)
                    && let Err(e) = keychain::save_session(server, session)
                {
                    eprintln!("orbit-desktop: {e}");
                }
                let (id, name) = loaded.workspace.clone().unwrap_or_else(|| (String::new(), "No workspace".into()));
                self.chat = Chat::new(&loaded.user.id, &id, &name);
                self.chat.set_sidebar(loaded.sidebar.conversations, loaded.sidebar.states);
                self.chat.set_members(&loaded.members);
                self.sync_lists(true, cx);
                if loaded.workspace.is_some() {
                    self.start_sockets(&id, loaded.headers);
                }
                self.job(|c| match c.get::<Latest>(LATEST_PATH) {
                    Ok(latest) => Msg::Update(latest),
                    // No file (404) or no gateway pass: no update to show.
                    Err(_) => Msg::Update(Latest::default()),
                });
            }
            Msg::Loaded(Err(e)) => self.failed(e, Screen::Server, cx),
            Msg::Refreshed(Ok(sidebar)) => {
                self.chat.set_sidebar(sidebar.conversations, sidebar.states);
                // Events may be missing: reload the open lists too.
                if let Some(id) = self.chat.selected.clone() {
                    self.load_messages(&id);
                }
                if let Some(root) = self.chat.thread.as_ref().map(|t| t.root_id.clone()) {
                    self.chat.thread = None;
                    self.open_thread(&root, cx);
                }
            }
            Msg::Refreshed(Err(e)) => self.failed(e, Screen::SignedIn, cx),
            Msg::Messages(id, Ok(items)) => self.chat.set_messages(&id, items),
            Msg::Messages(id, Err(e)) => {
                if is_session_error(&e) {
                    self.failed(e, Screen::SignedIn, cx);
                } else {
                    self.chat.load_failed(&id, e.to_string());
                }
            }
            Msg::Thread(_, Ok(page)) => self.chat.set_thread(page.root, page.items),
            Msg::Thread(root, Err(e)) => {
                if is_session_error(&e) {
                    self.failed(e, Screen::SignedIn, cx);
                } else if self.chat.thread.as_ref().is_some_and(|t| t.root_id == root) {
                    self.chat.error = Some(format!("The thread did not load: {e}"));
                    self.chat.close_thread();
                }
            }
            Msg::Sent(nonce, result) => {
                if let Err(e) = &result
                    && is_session_error(e)
                {
                    self.chat.sent(&nonce, Err(e.to_string()));
                    self.failed(e.clone(), Screen::SignedIn, cx);
                } else {
                    self.chat.sent(&nonce, result.map_err(|e| e.to_string()));
                }
            }
            Msg::Edited(result) => self.chat.edited(result.map_err(|e| e.to_string())),
            Msg::Update(latest) => {
                self.update = newer(env!("CARGO_PKG_VERSION"), &latest).filter(|l| l.download().is_some()).cloned();
            }
            Msg::LoggedOut => {
                self.busy = false;
                if let Some(server) = &self.server {
                    keychain::forget_session(server);
                }
                self.clear_session();
                self.chat = Chat::default();
                self.sync_lists(true, cx);
                self.screen = Screen::Login;
            }
            Msg::Socket(generation, kind, event) => {
                // Events of stopped sockets are dropped (the fixture backend has no sockets).
                if self.net().is_none_or(|n| n.generation == generation) {
                    match socket_event(&mut self.live, &mut self.events, kind, event) {
                        SocketAction::None => {}
                        SocketAction::Refetch => self.refresh(),
                        SocketAction::Chat(event) => self.chat.apply(event),
                        SocketAction::SignedOut => self.failed(ApiError::SignedOut(Default::default()), Screen::Login, cx),
                        SocketAction::Gateway(e) => self.failed(e, Screen::Server, cx),
                    }
                }
            }
        }
        self.sync_lists(false, cx);
        cx.notify();
    }

    fn clear_session(&mut self) {
        if let Some(net) = self.net()
            && let Some(c) = net.client.write().unwrap().as_mut()
        {
            c.set_session(None);
            c.set_browser_mode(false);
        }
        self.browser_mode = false;
    }

    /// Who refused decides where the person goes next.
    fn failed(&mut self, error: ApiError, otherwise: Screen, cx: &mut Context<Self>) {
        self.busy = false;
        match error {
            ApiError::GatewaySignIn | ApiError::GatewayRejected => {
                if let Some(server) = &self.server {
                    keychain::forget_gateway(server);
                }
                self.stop_sockets();
                self.open_gateway(cx);
            }
            ApiError::SignedOut(_) => {
                if let Some(server) = &self.server {
                    keychain::forget_session(server);
                }
                self.clear_session();
                self.stop_sockets();
                self.screen = Screen::Login;
                self.error = Some("Your session ended. Sign in again.".into());
            }
            other => {
                self.screen = otherwise;
                self.error = Some(other.to_string());
            }
        }
    }

    fn start_sockets(&mut self, workspace_id: &str, headers: Vec<(&'static str, Secret)>) {
        self.stop_sockets();
        let server = self.server.clone();
        let inbox = self.inbox.clone();
        let (Some(server), Some(net)) = (server, self.net()) else { return };
        let generation = net.generation;
        for kind in [SocketKind::Live, SocketKind::Events] {
            let inbox = inbox.clone();
            net.sockets.push(orbit_client::socket::spawn(kind, server.clone(), workspace_id, headers.clone(), SocketConfig::default(), move |event| {
                let _ = inbox.send_blocking(Msg::Socket(generation, kind, event));
            }));
        }
    }

    fn stop_sockets(&mut self) {
        if let Some(net) = self.net() {
            net.sockets.clear();
            net.generation += 1;
        }
        self.live = Link::Off;
        self.events = Link::Off;
    }

    /// Refetches the sidebar after a resync (the open lists follow in `Refreshed`).
    fn refresh(&self) {
        let workspace = self.chat.workspace_id.clone();
        self.job(move |c| Msg::Refreshed(sidebar_of(c, &workspace)));
    }

    /// Tells the two virtual lists which rows changed, so they keep the scroll position and
    /// measure only those rows again. `reset`: another conversation, start at the newest message.
    pub fn sync_lists(&mut self, reset: bool, cx: &mut Context<Self>) {
        let main: Vec<u64> = self.chat.messages.iter().map(row_signature).collect();
        sync(&self.scroller, &mut self.shown, main, reset, cx);
        let thread: Vec<u64> = self.chat.thread.iter().flat_map(|t| t.replies.iter()).map(row_signature).collect();
        let thread_reset = reset || self.chat.thread.is_none();
        sync(&self.thread_scroller, &mut self.shown_thread, thread, thread_reset, cx);
    }
}

fn sync(scroller: &Entity<MessageScrollerState>, shown: &mut Vec<u64>, now: Vec<u64>, reset: bool, cx: &mut App) {
    if reset {
        let count = now.len();
        scroller.update(cx, |s, cx| s.reset(count, cx));
    } else if *shown != now {
        let keep = first_change(shown, &now);
        let (old, new) = (shown.len(), now.len());
        scroller.update(cx, |s, cx| {
            s.splice(keep..old, new - keep, cx);
        });
    }
    *shown = now;
}

fn is_session_error(e: &ApiError) -> bool {
    matches!(e, ApiError::SignedOut(_) | ApiError::GatewaySignIn | ApiError::GatewayRejected | ApiError::ContractMismatch)
}

fn sidebar_of(c: &Client, workspace: &str) -> Result<Sidebar, ApiError> {
    Ok(Sidebar {
        conversations: c.get(&format!("/api/v1/workspaces/{workspace}/chat/conversations"))?,
        states: c.get(&format!("/api/v1/workspaces/{workspace}/chat/states"))?,
    })
}

fn load(c: &mut Client) -> Result<Loaded, ApiError> {
    let user = c.me()?;
    let workspace = c.workspaces()?.into_iter().next().map(|w| (w.id, w.name));
    let (sidebar, members) = match &workspace {
        Some((id, _)) => (sidebar_of(c, id)?, c.members(id)?),
        None => (Sidebar { conversations: Vec::new(), states: Vec::new() }, Vec::new()),
    };
    Ok(Loaded { user, workspace, sidebar, members, headers: c.auth_headers("/api/v1/workspaces"), session: c.session().cloned() })
}

/// `desk-` and 32 random hex digits (the server takes 1–64 bytes).
fn new_nonce() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut out = String::from("desk-");
    for _ in 0..2 {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos()));
        out.push_str(&format!("{:016x}", h.finish()));
    }
    out
}

/// Now as RFC 3339 UTC (`2026-11-09T10:00:00Z`), for the local copy of a message being sent.
pub fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}
