//! The app's state and its sign-in flow. Network work runs on threads and comes back as [`Msg`]s
//! through the inbox, which wakes Rune; `poll` applies them on the UI thread.
//!
//! Server → (gateway sign-in window, when the server has a gateway) → Orbit sign-in (→ two-factor
//! code) → signed in: the first workspace's conversations, with its live and events sockets.

use std::sync::{Arc, Mutex, OnceLock};

use orbit_client::live::Ended;
use orbit_client::{
    ApiError, AuthUser, Client, ConversationRecord, ConversationStateRecord, HandOff, LiveFrame, LoginOutcome, Secret,
    ServerUrl, SidebarRow, Socket, SocketConfig, SocketEvent, SocketKind, WorkspaceEventKind, sidebar,
};
use rune::{TextEdit, Waker};

use crate::{keychain, signin};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    /// Enter the server's address.
    Server,
    /// The gateway's sign-in window is open.
    Gateway,
    Login,
    TwoFactor,
    SignedIn,
}

/// The state of one socket, for the status line.
#[derive(Clone, Debug, PartialEq)]
pub enum Link {
    Off,
    Connecting,
    Connected,
    /// Next try in this many seconds.
    Retrying(u64),
    Ended(String),
}

pub struct State {
    pub screen: Screen,
    pub server_edit: TextEdit,
    pub email: TextEdit,
    pub password: TextEdit,
    pub code: TextEdit,
    pub busy: bool,
    pub error: Option<String>,
    pub server: Option<ServerUrl>,
    pub user: String,
    pub workspace: String,
    pub rows: Vec<SidebarRow>,
    pub selected: Option<String>,
    pub live: Link,
    pub events: Link,
    /// `None` in fixture runs: nothing touches the network or the keychain.
    pub net: Option<Net>,
}

impl Default for State {
    fn default() -> Self {
        State {
            screen: Screen::Server,
            server_edit: TextEdit::new(""),
            email: TextEdit::new(""),
            password: TextEdit::new("").secure(),
            code: TextEdit::new(""),
            busy: false,
            error: None,
            server: None,
            user: String::new(),
            workspace: String::new(),
            rows: Vec::new(),
            selected: None,
            live: Link::Off,
            events: Link::Off,
            net: None,
        }
    }
}

pub struct Net {
    inbox: Arc<Inbox>,
    client: Arc<Mutex<Option<Client>>>,
    two_factor: Option<Secret>,
    sockets: Vec<Socket>,
    /// Counts socket starts, so events of stopped sockets are dropped.
    generation: u64,
}

#[derive(Default)]
struct Inbox {
    messages: Mutex<Vec<Msg>>,
    waker: OnceLock<Waker>,
}

impl Inbox {
    fn post(&self, msg: Msg) {
        self.messages.lock().unwrap().push(msg);
        if let Some(wake) = self.waker.get() {
            wake();
        }
    }
}

pub enum Msg {
    Probed(Result<(), ApiError>),
    HandOff(Result<HandOff, String>),
    Login(Result<LoginOutcome, ApiError>),
    Loaded(Result<Loaded, ApiError>),
    Refreshed(Result<Vec<SidebarRow>, ApiError>),
    LoggedOut,
    Socket(u64, SocketKind, SocketEvent),
}

pub struct Loaded {
    user: AuthUser,
    workspace: Option<(String, String)>,
    rows: Vec<SidebarRow>,
    headers: Vec<(&'static str, Secret)>,
}

impl State {
    /// The real app: starts from the last server, if any.
    pub fn online() -> State {
        let mut state = State { net: Some(Net { inbox: Arc::default(), client: Arc::default(), two_factor: None, sockets: Vec::new(), generation: 0 }), ..State::default() };
        if let Some(server) = keychain::last_server() {
            state.server_edit.replace_text(server.origin());
        }
        state
    }

    /// Rune's `setup`: keep the waker, then resume the last server's session.
    pub fn setup(&mut self, waker: Waker) {
        let Some(net) = &self.net else { return };
        let _ = net.inbox.waker.set(waker);
        if !self.server_edit.text.is_empty() {
            self.submit_server();
        }
    }

    /// Rune's `poll`: applies what the threads sent.
    pub fn poll(&mut self) -> bool {
        let Some(net) = &self.net else { return false };
        let messages = std::mem::take(&mut *net.inbox.messages.lock().unwrap());
        let changed = !messages.is_empty();
        for msg in messages {
            self.apply(msg);
        }
        changed
    }

    fn job(&mut self, work: impl FnOnce(&mut Client) -> Msg + Send + 'static) {
        let Some(net) = &self.net else { return };
        self.busy = true;
        self.error = None;
        let (client, inbox) = (net.client.clone(), net.inbox.clone());
        std::thread::spawn(move || {
            let msg = match client.lock().unwrap().as_mut() {
                Some(client) => work(client),
                None => return,
            };
            inbox.post(msg);
        });
    }

    pub fn submit_server(&mut self) {
        let server = match ServerUrl::parse(&self.server_edit.text) {
            Ok(server) => server,
            Err(e) => {
                self.error = Some(e.to_string());
                return;
            }
        };
        let Some(net) = &mut self.net else { return };
        net.sockets.clear();
        let mut client = Client::new(server.clone());
        client.set_gateway(keychain::gateway(&server));
        client.set_session(keychain::session(&server));
        *net.client.lock().unwrap() = Some(client);
        keychain::remember_server(&server);
        self.server = Some(server);
        self.job(|c| Msg::Probed(c.setup_status().map(|_| ())));
    }

    pub fn change_server(&mut self) {
        self.stop_sockets();
        self.screen = Screen::Server;
        self.busy = false;
        self.error = None;
    }

    pub fn open_gateway(&mut self) {
        let (Some(server), Some(net)) = (self.server.clone(), &self.net) else { return };
        self.screen = Screen::Gateway;
        self.busy = true;
        self.error = None;
        let inbox = net.inbox.clone();
        std::thread::spawn(move || inbox.post(Msg::HandOff(signin::open(&server))));
    }

    pub fn submit_login(&mut self) {
        let email = self.email.text.trim().to_owned();
        let password = Secret::new(self.password.text.clone());
        if email.is_empty() || password.is_empty() {
            self.error = Some("Enter your email and password.".into());
            return;
        }
        self.password.clear();
        self.job(move |c| Msg::Login(c.login(&email, &password)));
    }

    pub fn submit_code(&mut self) {
        let Some(token) = self.net.as_ref().and_then(|n| n.two_factor.clone()) else { return };
        let code = self.code.text.trim().to_owned();
        if code.is_empty() {
            self.error = Some("Enter the code.".into());
            return;
        }
        self.code.clear();
        self.job(move |c| Msg::Login(c.login_two_factor(&token, &code)));
    }

    pub fn sign_out(&mut self) {
        self.stop_sockets();
        self.job(|c| {
            let _ = c.logout();
            Msg::LoggedOut
        });
    }

    fn load(&mut self) {
        self.job(|c| Msg::Loaded(load(c)));
    }

    fn apply(&mut self, msg: Msg) {
        self.busy = false;
        match msg {
            Msg::Probed(Ok(())) => {
                let has_session = self.net.as_ref().is_some_and(|n| n.client.lock().unwrap().as_ref().is_some_and(|c| c.session().is_some()));
                if has_session {
                    self.load();
                } else {
                    self.screen = Screen::Login;
                }
            }
            Msg::Probed(Err(e)) => self.failed(e, Screen::Server),
            Msg::HandOff(result) => {
                if self.screen != Screen::Gateway {
                    return;
                }
                match result {
                    Ok(HandOff::SignedIn(cookies)) => {
                        let server = self.server.clone().expect("a server before the gateway");
                        if let Err(e) = keychain::save_gateway(&server, &cookies) {
                            eprintln!("orbit-desktop: {e}");
                        }
                        self.job(move |c| {
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
                if let Some(net) = &mut self.net {
                    net.two_factor = Some(token);
                }
                self.screen = Screen::TwoFactor;
            }
            Msg::Login(Ok(LoginOutcome::SignedIn { session, .. })) => {
                if let Some(net) = &mut self.net {
                    net.two_factor = None;
                }
                if let Some(server) = &self.server
                    && let Err(e) = keychain::save_session(server, &session)
                {
                    eprintln!("orbit-desktop: {e}");
                }
                self.load();
            }
            Msg::Login(Err(ApiError::SignedOut(p))) => {
                // The sign-in route's 401: wrong password or code.
                self.error = Some(if p.detail.is_empty() { "The email or password is not correct.".into() } else { p.detail });
            }
            Msg::Login(Err(e)) => self.failed(e, self.screen),
            Msg::Loaded(Ok(loaded)) => {
                self.screen = Screen::SignedIn;
                self.user = loaded.user.display_name;
                self.rows = loaded.rows;
                match loaded.workspace {
                    Some((id, name)) => {
                        self.workspace = name;
                        self.start_sockets(&id, loaded.headers);
                    }
                    None => self.workspace = "No workspace".into(),
                }
            }
            Msg::Loaded(Err(e)) => self.failed(e, Screen::Server),
            Msg::Refreshed(Ok(rows)) => self.rows = rows,
            Msg::Refreshed(Err(e)) => self.failed(e, Screen::SignedIn),
            Msg::LoggedOut => {
                if let Some(server) = &self.server {
                    keychain::forget_session(server);
                }
                if let Some(net) = &self.net
                    && let Some(c) = net.client.lock().unwrap().as_mut()
                {
                    c.set_session(None);
                }
                self.rows.clear();
                self.screen = Screen::Login;
            }
            Msg::Socket(generation, kind, event) => {
                if self.net.as_ref().is_some_and(|n| n.generation == generation) {
                    self.socket_event(kind, event);
                }
            }
        }
    }

    /// Who refused decides where the person goes next.
    fn failed(&mut self, error: ApiError, otherwise: Screen) {
        match error {
            ApiError::GatewaySignIn | ApiError::GatewayRejected => {
                if let Some(server) = &self.server {
                    keychain::forget_gateway(server);
                }
                self.stop_sockets();
                self.open_gateway();
            }
            ApiError::SignedOut(_) => {
                if let Some(server) = &self.server {
                    keychain::forget_session(server);
                }
                if let Some(net) = &self.net
                    && let Some(c) = net.client.lock().unwrap().as_mut()
                {
                    c.set_session(None);
                }
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
        let (Some(server), Some(net)) = (self.server.clone(), &mut self.net) else { return };
        let generation = net.generation;
        for kind in [SocketKind::Live, SocketKind::Events] {
            let inbox = net.inbox.clone();
            net.sockets.push(orbit_client::socket::spawn(kind, server.clone(), workspace_id, headers.clone(), SocketConfig::default(), move |event| {
                inbox.post(Msg::Socket(generation, kind, event));
            }));
        }
    }

    fn stop_sockets(&mut self) {
        if let Some(net) = &mut self.net {
            net.sockets.clear();
            net.generation += 1;
        }
        self.live = Link::Off;
        self.events = Link::Off;
    }

    fn socket_event(&mut self, kind: SocketKind, event: SocketEvent) {
        let link = match kind {
            SocketKind::Live => &mut self.live,
            SocketKind::Events => &mut self.events,
        };
        match event {
            SocketEvent::Connecting { .. } => *link = Link::Connecting,
            SocketEvent::Connected => *link = Link::Connected,
            SocketEvent::Retrying { after, .. } => *link = Link::Retrying(after.as_secs().max(1)),
            SocketEvent::Live(LiveFrame::Resync) | SocketEvent::Live(LiveFrame::Event { .. }) => self.refresh(),
            SocketEvent::Workspace(w) if w.kind == WorkspaceEventKind::ResyncRequired => self.refresh(),
            SocketEvent::Live(_) | SocketEvent::Workspace(_) => {}
            SocketEvent::Ended(Ended::SignedOut) => self.failed(ApiError::SignedOut(Default::default()), Screen::Login),
            SocketEvent::Ended(Ended::Gateway(e)) => self.failed(e, Screen::Server),
            SocketEvent::Ended(Ended::Forbidden) => *link = Link::Ended("no access to this workspace".into()),
            SocketEvent::Ended(Ended::ContractMismatch) => *link = Link::Ended("update the desktop app".into()),
        }
    }

    /// Refetches the sidebar (M1 shows conversations only; messages come later).
    fn refresh(&mut self) {
        let Some(net) = &self.net else { return };
        let (client, inbox) = (net.client.clone(), net.inbox.clone());
        std::thread::spawn(move || {
            let rows = match client.lock().unwrap().as_ref() {
                Some(c) => first_workspace_rows(c).map(|(_, rows)| rows),
                None => return,
            };
            inbox.post(Msg::Refreshed(rows));
        });
    }
}

/// The first workspace (id, name), if any, and its sidebar.
type FirstWorkspace = (Option<(String, String)>, Vec<SidebarRow>);

fn first_workspace_rows(c: &Client) -> Result<FirstWorkspace, ApiError> {
    let Some(workspace) = c.workspaces()?.into_iter().next() else { return Ok((None, Vec::new())) };
    let conversations: Vec<ConversationRecord> = c.get(&format!("/api/v1/workspaces/{}/chat/conversations", workspace.id))?;
    let states: Vec<ConversationStateRecord> = c.get(&format!("/api/v1/workspaces/{}/chat/states", workspace.id))?;
    Ok((Some((workspace.id, workspace.name)), sidebar(&conversations, &states)))
}

fn load(c: &mut Client) -> Result<Loaded, ApiError> {
    let user = c.me()?;
    let (workspace, rows) = first_workspace_rows(c)?;
    Ok(Loaded { user, workspace, rows, headers: c.auth_headers("/api/v1/workspaces") })
}
