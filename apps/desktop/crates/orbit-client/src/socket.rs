//! Keeps one workspace socket open on its own thread: connects with the client's headers (gateway
//! cookie, `Authorization: Bearer`, no `Origin`), resumes with the cursor, reconnects after a drop
//! with [`Backoff`], and stops for good when the server says the session or access ended
//! ([`after_close`], [`after_refused_upgrade`]). The rules live in `live.rs`; this is the I/O.

use std::io::ErrorKind;
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use tungstenite::client::IntoClientRequest;
use tungstenite::handshake::HandshakeError;
use tungstenite::http::{HeaderName, HeaderValue};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Connector, Message, WebSocket};

use crate::backoff::Backoff;
use crate::live::{
    AfterClose, Ended, EventsCursor, LiveCursor, LiveFrame, WorkspaceEvent, after_close, after_refused_upgrade,
    parse_live_frame, parse_workspace_event,
};
use crate::secret::Secret;
use crate::server::ServerUrl;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SocketKind {
    /// `/api/v1/workspaces/{id}/live`: chat events.
    Live,
    /// `/api/v1/workspaces/{id}/events`: invalidations.
    Events,
}

/// What the socket thread reports.
#[derive(Clone, Debug, PartialEq)]
pub enum SocketEvent {
    Connecting { attempt: u32 },
    Connected,
    /// A live frame (pings are not reported). `Resync` means: refetch chat.
    Live(LiveFrame),
    /// An events frame; `ResyncRequired` means: refetch the workspace.
    Workspace(WorkspaceEvent),
    /// The connection is lost or was refused; the next try is after `after`.
    Retrying { after: Duration, close_code: Option<u16> },
    /// Stopped for good; the app acts (sign in again, leave the workspace).
    Ended(Ended),
}

#[derive(Clone, Copy, Debug)]
pub struct SocketConfig {
    /// The first reconnect delay.
    pub base_delay: Duration,
    /// A connection silent for this long is lost (the server sends something every 20-25 s).
    pub silence: Duration,
    /// The longest one blocking read waits, so a stop request is seen quickly.
    pub poll: Duration,
}

impl Default for SocketConfig {
    fn default() -> Self {
        SocketConfig { base_delay: Duration::from_secs(1), silence: Duration::from_secs(60), poll: Duration::from_millis(500) }
    }
}

/// How long a connection must stay open before the reconnect delays start again from the base.
const STABLE: Duration = Duration::from_secs(10);

/// A running socket. Dropping it asks the thread to stop without waiting (a connect can take
/// seconds); [`Socket::stop`] waits.
pub struct Socket {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Socket {
    /// Stops the thread and waits for it: the close frame is sent before this returns.
    pub fn stop(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Socket {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// Starts a socket of `workspace_id`. `headers` come from [`crate::Client::auth_headers`].
pub fn spawn(
    kind: SocketKind,
    server: ServerUrl,
    workspace_id: &str,
    headers: Vec<(&'static str, Secret)>,
    config: SocketConfig,
    mut on_event: impl FnMut(SocketEvent) + Send + 'static,
) -> Socket {
    let stop = Arc::new(AtomicBool::new(false));
    let path = format!(
        "/api/v1/workspaces/{}/{}",
        workspace_id.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect::<String>(),
        match kind { SocketKind::Live => "live", SocketKind::Events => "events" }
    );
    let thread = std::thread::Builder::new()
        .name(format!("orbit-{kind:?}").to_lowercase())
        .spawn({
            let stop = stop.clone();
            move || run(kind, &server, &path, &headers, config, &stop, &mut on_event)
        })
        .expect("spawning a thread");
    Socket { stop, thread: Some(thread) }
}

#[derive(Default)]
struct Cursors {
    live: LiveCursor,
    events: EventsCursor,
}

fn run(
    kind: SocketKind,
    server: &ServerUrl,
    path: &str,
    headers: &[(&'static str, Secret)],
    config: SocketConfig,
    stop: &AtomicBool,
    on_event: &mut dyn FnMut(SocketEvent),
) {
    let mut backoff = Backoff::new(config.base_delay);
    let mut cursors = Cursors::default();
    while !stop.load(Ordering::Relaxed) {
        on_event(SocketEvent::Connecting { attempt: backoff.attempt() });
        let query = match kind {
            SocketKind::Live => cursors.live.query(),
            SocketKind::Events => cursors.events.query(),
        };
        let mut close_code = None;
        match connect(&server.ws(&format!("{path}{query}")), headers) {
            Ok(mut ws) => {
                on_event(SocketEvent::Connected);
                let opened = Instant::now();
                close_code = serve(&mut ws, kind, &mut cursors, config, stop, on_event);
                // Only a connection that held starts the delays again: a server that accepts and
                // drops at once is not retried every second.
                if opened.elapsed() >= STABLE {
                    backoff.reset();
                }
                if stop.load(Ordering::Relaxed) {
                    return;
                }
                if let AfterClose::Stop(ended) = after_close(close_code) {
                    on_event(SocketEvent::Ended(ended));
                    return;
                }
            }
            Err(Refused::Stop(ended)) => {
                on_event(SocketEvent::Ended(ended));
                return;
            }
            Err(Refused::Retry) => {}
        }
        let delay = backoff.next_delay();
        on_event(SocketEvent::Retrying { after: delay, close_code });
        let until = Instant::now() + delay;
        while Instant::now() < until && !stop.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(50).min(until - Instant::now()));
        }
    }
}

enum Refused {
    Stop(Ended),
    Retry,
}

type Ws = WebSocket<MaybeTlsStream<TcpStream>>;

fn connect(url: &str, headers: &[(&'static str, Secret)]) -> Result<Ws, Refused> {
    let mut request = url.into_client_request().map_err(|_| Refused::Retry)?;
    for (name, value) in headers {
        let name = HeaderName::from_bytes(name.as_bytes()).map_err(|_| Refused::Retry)?;
        let mut value = HeaderValue::from_str(value.expose()).map_err(|_| Refused::Retry)?;
        value.set_sensitive(true);
        request.headers_mut().insert(name, value);
    }
    let uri = request.uri().clone();
    let secure = uri.scheme_str() == Some("wss");
    let host = uri.host().unwrap_or_default().trim_matches(['[', ']']).to_owned();
    let port = uri.port_u16().unwrap_or(if secure { 443 } else { 80 });
    let addrs = (host.as_str(), port).to_socket_addrs().map_err(|_| Refused::Retry)?;
    let stream = addrs
        .into_iter()
        .find_map(|addr| TcpStream::connect_timeout(&addr, Duration::from_secs(10)).ok())
        .ok_or(Refused::Retry)?;
    let _ = stream.set_read_timeout(Some(Duration::from_secs(15)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));
    let connector = if secure { Connector::Rustls(tls_config()) } else { Connector::Plain };
    match tungstenite::client_tls_with_config(request, stream, None, Some(connector)) {
        Ok((ws, _)) => Ok(ws),
        Err(HandshakeError::Failure(tungstenite::Error::Http(response))) => {
            let header = |name: &str| response.headers().get(name).and_then(|v| v.to_str().ok());
            let body = response.body().as_deref().map(String::from_utf8_lossy).unwrap_or_default();
            match after_refused_upgrade(
                response.status().as_u16(),
                header("location"),
                header("www-authenticate"),
                header("content-type"),
                &body,
            ) {
                AfterClose::Stop(ended) => Err(Refused::Stop(ended)),
                AfterClose::Reconnect => Err(Refused::Retry),
            }
        }
        Err(_) => Err(Refused::Retry),
    }
}

/// Reads until the connection ends; returns the close code the server sent, if any.
fn serve(
    ws: &mut Ws,
    kind: SocketKind,
    cursors: &mut Cursors,
    config: SocketConfig,
    stop: &AtomicBool,
    on_event: &mut dyn FnMut(SocketEvent),
) -> Option<u16> {
    let tcp = match ws.get_mut() {
        MaybeTlsStream::Plain(s) => Some(&*s),
        MaybeTlsStream::Rustls(s) => Some(&s.sock),
        _ => None,
    };
    if let Some(tcp) = tcp {
        let _ = tcp.set_read_timeout(Some(config.poll));
    }
    let mut heard = Instant::now();
    loop {
        if stop.load(Ordering::Relaxed) {
            let _ = ws.close(None);
            let _ = ws.flush();
            return None;
        }
        match ws.read() {
            Ok(Message::Text(text)) => {
                heard = Instant::now();
                match kind {
                    SocketKind::Live => {
                        if let Some(frame) = parse_live_frame(&text) {
                            cursors.live.read(&frame);
                            if frame != LiveFrame::Ping {
                                on_event(SocketEvent::Live(frame));
                            }
                        }
                    }
                    SocketKind::Events => {
                        if let Some(event) = parse_workspace_event(&text) {
                            cursors.events.read(&event);
                            on_event(SocketEvent::Workspace(event));
                        }
                    }
                }
            }
            Ok(Message::Close(frame)) => {
                let _ = ws.flush();
                return frame.map(|f| u16::from(f.code));
            }
            // Pings are answered by tungstenite on the next read or flush.
            Ok(_) => heard = Instant::now(),
            Err(tungstenite::Error::Io(e)) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                let _ = ws.flush();
                if heard.elapsed() > config.silence {
                    return None;
                }
            }
            Err(_) => return None,
        }
    }
}

/// rustls with ring and the webpki roots, built once.
fn tls_config() -> Arc<rustls::ClientConfig> {
    static CONFIG: OnceLock<Arc<rustls::ClientConfig>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            let roots = rustls::RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.to_vec() };
            let config = rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                .with_safe_default_protocol_versions()
                .expect("ring supports the default versions")
                .with_root_certificates(roots)
                .with_no_client_auth();
            Arc::new(config)
        })
        .clone()
}
