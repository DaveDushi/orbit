//! The egress proxy of agent sandboxes (`orbit egress-proxy`; docs/agent-sandbox-design.md, section 4). A thread
//! sandbox has no DNS and no route except TCP to this proxy on its own /30. Per connection:
//!
//! 1. `Proxy-Authorization: Basic <call id>:<proxy password>`. The broker writes a **call file** for each running call
//!    (`<calls dir>/<call id>`: the sandbox's address and the password's SHA-256) and deletes it at the call end. A
//!    password of an ended call, or one used from another sandbox's address, gets 407. Open tunnels of a call close
//!    within a second of its end.
//! 2. `CONNECT host:port` only (plain HTTP gets 405). Orbit decides (`POST /api/internal/agent/egress/check`):
//!    `allow`, `deny` or `ask`. Only after `allow` does the proxy resolve the name. Every address must be public
//!    (`orbit_platform::is_public_ip`), and it connects to the address it checked (no DNS rebinding).
//! 3. After `200`, the TLS ClientHello must name the same host (SNI). Then it relays and counts bytes, at most
//!    `call_max_bytes` and `call_max_connections` per call, and reports them to Orbit (`.../egress/done`).
//!
//! `orbit.internal` (plain HTTP only, `ORBIT_URL=http://orbit.internal`) goes to Orbit's internal address without an
//! egress decision: Orbit checks the call token of each request itself, and it is not egress. The proxy is the trusted
//! proxy boundary there (Orbit trusts loopback's forwarding headers): it passes exactly one request per connection
//! (the head, then `Content-Length` body bytes; chunked uploads get 411), removes the sandbox's own forwarding headers,
//! and sets `X-Forwarded-For` to the sandbox's address and `X-Forwarded-Proto: https`. A raw tunnel to Orbit
//! (`CONNECT orbit.internal:…`) is refused like any other `orbit.internal` egress.

use std::collections::HashMap;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use futures_util::future::BoxFuture;
use orbit_platform::{Id, is_public_ip};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Semaphore;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

/// The fixed name of Orbit inside a sandbox (`ORBIT_URL=http://orbit.internal`).
pub const ORBIT_INTERNAL: &str = "orbit.internal";
const HEAD_MAX: usize = 8 * 1024;
const HEAD_TIMEOUT: Duration = Duration::from_secs(10);
const RESOLVE_TIMEOUT: Duration = Duration::from_secs(5);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const HELLO_TIMEOUT: Duration = Duration::from_secs(10);
/// A TLS record holds at most 16 KiB of plaintext (plus a little).
const RECORD_MAX: usize = 16 * 1024 + 256;
/// All connections of the proxy together (sandboxes are few; this only bounds a flood).
const MAX_CONNECTIONS: usize = 512;
/// How often open tunnels are checked against their call file.
const SWEEP: Duration = Duration::from_secs(1);

pub struct ProxyConfig {
    /// Orbit's loopback URL for the egress routes.
    pub orbit_url: String,
    /// The service bearer derived from the broker key (not the key: the proxy cannot sign tickets).
    pub service_token: String,
    pub calls_dir: PathBuf,
    /// Where `orbit.internal` goes.
    pub orbit_internal: SocketAddr,
    pub call_max_bytes: u64,
    pub call_max_connections: usize,
}

/// Name lookups and outgoing connections; tests count lookups and fake answers.
pub trait Dialer: Send + Sync + 'static {
    fn resolve<'a>(
        &'a self,
        host: &'a str,
        port: u16,
    ) -> BoxFuture<'a, io::Result<Vec<SocketAddr>>>;
    fn connect(&self, address: SocketAddr) -> BoxFuture<'_, io::Result<TcpStream>>;
}

pub struct SystemDialer;

impl Dialer for SystemDialer {
    fn resolve<'a>(
        &'a self,
        host: &'a str,
        port: u16,
    ) -> BoxFuture<'a, io::Result<Vec<SocketAddr>>> {
        Box::pin(async move { Ok(tokio::net::lookup_host((host, port)).await?.collect()) })
    }

    fn connect(&self, address: SocketAddr) -> BoxFuture<'_, io::Result<TcpStream>> {
        Box::pin(TcpStream::connect(address))
    }
}

// ---------------------------------------------------------------------------------------------
// Calls: open connections and bytes per call, and the end of a call (its file is gone).

struct Call {
    bytes: AtomicU64,
    open: AtomicUsize,
    ended: CancellationToken,
}

struct Slot(Arc<Call>);

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.open.fetch_sub(1, Ordering::SeqCst);
    }
}

#[derive(Default)]
struct Calls(Mutex<HashMap<Id, Arc<Call>>>);

impl Calls {
    fn open(&self, id: Id, max_connections: usize, max_bytes: u64) -> Result<Slot, &'static str> {
        let mut calls = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let call = Arc::clone(calls.entry(id).or_insert_with(|| {
            Arc::new(Call {
                bytes: AtomicU64::new(0),
                open: AtomicUsize::new(0),
                ended: CancellationToken::new(),
            })
        }));
        if call.ended.is_cancelled() {
            return Err("the call ended");
        }
        if call.bytes.load(Ordering::SeqCst) >= max_bytes {
            return Err("the call's egress bytes are used up");
        }
        if call.open.fetch_add(1, Ordering::SeqCst) >= max_connections {
            call.open.fetch_sub(1, Ordering::SeqCst);
            return Err("the call has too many open connections");
        }
        Ok(Slot(call))
    }

    /// Ends the calls whose file is gone (their tunnels close) and forgets them once nothing is open.
    fn sweep(&self, dir: &Path) {
        let mut calls = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        calls.retain(|id, call| {
            if !dir.join(id.to_string()).exists() {
                call.ended.cancel();
            }
            !(call.ended.is_cancelled() && call.open.load(Ordering::SeqCst) == 0)
        });
    }
}

#[derive(Deserialize)]
struct CallFile {
    addr: IpAddr,
    password_sha256: String,
}

// ---------------------------------------------------------------------------------------------
// The request head.

struct Head {
    method: String,
    target: String,
    version: String,
    headers: Vec<(String, String)>,
}

impl Head {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

fn parse_head(raw: &[u8]) -> Option<Head> {
    let text = std::str::from_utf8(raw).ok()?;
    let mut lines = text.split("\r\n");
    let mut first = lines.next()?.split(' ');
    let (method, target, version) = (first.next()?, first.next()?, first.next()?);
    if first.next().is_some() || !version.starts_with("HTTP/1.") {
        return None;
    }
    let headers = lines
        .filter(|line| !line.is_empty())
        .map(|line| {
            let (name, value) = line.split_once(':')?;
            Some((name.trim().to_owned(), value.trim().to_owned()))
        })
        .collect::<Option<Vec<_>>>()?;
    Some(Head {
        method: method.to_owned(),
        target: target.to_owned(),
        version: version.to_owned(),
        headers,
    })
}

/// The head, and the bytes the client sent after it.
async fn read_head(stream: &mut TcpStream) -> io::Result<(Head, Vec<u8>)> {
    let mut buffer = Vec::with_capacity(1024);
    let mut chunk = [0_u8; 1024];
    loop {
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(end) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            let head = parse_head(&buffer[..end]).ok_or(io::ErrorKind::InvalidData)?;
            return Ok((head, buffer[end + 4..].to_vec()));
        }
        if buffer.len() > HEAD_MAX {
            return Err(io::ErrorKind::InvalidData.into());
        }
    }
}

/// `Proxy-Authorization: Basic base64(<call id>:<password>)`.
fn credentials(head: &Head) -> Option<(Id, String)> {
    let (scheme, encoded) = head.header("proxy-authorization")?.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("basic") {
        return None;
    }
    let decoded = String::from_utf8(STANDARD.decode(encoded.trim()).ok()?).ok()?;
    let (user, password) = decoded.split_once(':')?;
    Some((user.parse().ok()?, password.to_owned()))
}

enum Target {
    /// One plain HTTP request to `orbit.internal`.
    Orbit,
    Connect {
        host: String,
        port: u16,
    },
}

fn target(head: &Head) -> Option<Target> {
    if head.method == "CONNECT" {
        let (host, port) = head.target.rsplit_once(':')?;
        let port = port.parse().ok()?;
        return Some(Target::Connect {
            host: host.to_owned(),
            port,
        });
    }
    // Plain HTTP only to Orbit: `http://orbit.internal[:80]/…`, nothing else (no userinfo tricks).
    let authority = orbit_path(&head.target).map(|(authority, _)| authority)?;
    (authority.eq_ignore_ascii_case(ORBIT_INTERNAL)
        || authority.eq_ignore_ascii_case("orbit.internal:80"))
    .then_some(Target::Orbit)
}

/// `http://<authority><path>` split; `None` when it is not an absolute `http://` URL.
fn orbit_path(target: &str) -> Option<(&str, &str)> {
    let scheme = target.get(..7)?;
    if !scheme.eq_ignore_ascii_case("http://") {
        return None;
    }
    let rest = &target[7..];
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    Some((&rest[..end], &rest[end..]))
}

/// The plain request for Orbit: origin form, one request per connection, none of the sandbox's proxy or forwarding
/// headers, and the proxy's own: the sandbox's address and `https` (the proxy is the trusted boundary).
fn orbit_request(head: &Head, sandbox: IpAddr) -> Vec<u8> {
    let path = orbit_path(&head.target).map_or("/", |(_, path)| path);
    let path = if path.starts_with('/') {
        path.to_owned()
    } else {
        format!("/{path}")
    };
    let mut out = format!("{} {path} {}\r\n", head.method, head.version);
    for (name, value) in &head.headers {
        let lower = name.to_ascii_lowercase();
        if lower.starts_with("proxy-")
            || lower.starts_with("x-forwarded-")
            || [
                "connection",
                "keep-alive",
                "upgrade",
                "te",
                "forwarded",
                "x-real-ip",
                "x-request-id",
            ]
            .contains(&lower.as_str())
        {
            continue;
        }
        out.push_str(&format!("{name}: {value}\r\n"));
    }
    out.push_str(&format!(
        "X-Forwarded-For: {}\r\nX-Forwarded-Proto: https\r\nConnection: close\r\n\r\n",
        sandbox.to_canonical()
    ));
    out.into_bytes()
}

/// The body length of a request to Orbit: `Content-Length` (once) or none. `None`: chunked or invalid.
fn body_length(head: &Head) -> Option<u64> {
    if head.header("transfer-encoding").is_some() {
        return None;
    }
    let mut lengths = head
        .headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"));
    let length = match lengths.next() {
        Some((_, value)) => value.parse().ok()?,
        None => 0,
    };
    lengths.next().is_none().then_some(length)
}

// ---------------------------------------------------------------------------------------------
// TLS ClientHello.

fn take(data: &[u8], at: usize, len: usize) -> Option<&[u8]> {
    data.get(at..at.checked_add(len)?)
}

fn u16_at(data: &[u8], at: usize) -> Option<usize> {
    take(data, at, 2).map(|bytes| usize::from(u16::from_be_bytes([bytes[0], bytes[1]])))
}

/// The server name (SNI) of a TLS ClientHello that fits in its first record, lower case without a trailing dot.
#[must_use]
pub fn client_hello_sni(record: &[u8]) -> Option<String> {
    // Record: handshake (22), version, length.
    if record.first() != Some(&22) {
        return None;
    }
    let body = take(record, 5, u16_at(record, 3)?)?;
    // Handshake: ClientHello (1), 24-bit length.
    if body.first() != Some(&1) {
        return None;
    }
    let length = (usize::from(*body.get(1)?) << 16) | u16_at(body, 2)?;
    let hello = take(body, 4, length)?;
    // Version (2), random (32), session id, cipher suites, compression methods, extensions.
    let mut at = 34;
    at += 1 + usize::from(*hello.get(at)?);
    at += 2 + u16_at(hello, at)?;
    at += 1 + usize::from(*hello.get(at)?);
    let extensions = take(hello, at + 2, u16_at(hello, at)?)?;
    let mut at = 0;
    while at + 4 <= extensions.len() {
        let kind = u16_at(extensions, at)?;
        let data = take(extensions, at + 4, u16_at(extensions, at + 2)?)?;
        at += 4 + data.len();
        if kind != 0 {
            continue;
        }
        // server_name: list length, then entries of type (0 = host name), length, name.
        let list = take(data, 2, u16_at(data, 0)?)?;
        if list.first() != Some(&0) {
            return None;
        }
        let name = std::str::from_utf8(take(list, 3, u16_at(list, 1)?)?).ok()?;
        return Some(name.trim_end_matches('.').to_ascii_lowercase());
    }
    None
}

/// The first TLS record (after `early`, the bytes that came with the head), and any bytes after it.
async fn read_record(stream: &mut TcpStream, early: Vec<u8>) -> io::Result<Vec<u8>> {
    let mut buffer = early;
    let mut chunk = [0_u8; 4096];
    loop {
        if buffer.first().is_some_and(|kind| *kind != 22) {
            return Err(io::ErrorKind::InvalidData.into());
        }
        if buffer.len() >= 5 {
            let length = usize::from(u16::from_be_bytes([buffer[3], buffer[4]]));
            if length > RECORD_MAX {
                return Err(io::ErrorKind::InvalidData.into());
            }
            if buffer.len() >= 5 + length {
                return Ok(buffer);
            }
        }
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
}

// ---------------------------------------------------------------------------------------------
// The proxy.

struct Proxy {
    config: ProxyConfig,
    dialer: Arc<dyn Dialer>,
    http: reqwest::Client,
    calls: Calls,
}

enum Check {
    Answer(Value),
    /// No such running call, or a wrong password.
    Unknown,
    Unavailable,
}

/// What became of an allowed connection.
#[derive(Default)]
struct Outcome {
    up: u64,
    down: u64,
    blocked: Option<&'static str>,
}

/// Serves until `shutdown`.
pub async fn serve(
    config: ProxyConfig,
    dialer: Arc<dyn Dialer>,
    listener: TcpListener,
    shutdown: CancellationToken,
) -> io::Result<()> {
    let http = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(io::Error::other)?;
    let proxy = Arc::new(Proxy {
        config,
        dialer,
        http,
        calls: Calls::default(),
    });
    let sweeper = Arc::clone(&proxy);
    let stop = shutdown.clone();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                () = tokio::time::sleep(SWEEP) => sweeper.calls.sweep(&sweeper.config.calls_dir),
                () = stop.cancelled() => return,
            }
        }
    });
    let slots = Arc::new(Semaphore::new(MAX_CONNECTIONS));
    loop {
        let accepted = tokio::select! {
            accepted = listener.accept() => accepted,
            () = shutdown.cancelled() => return Ok(()),
        };
        let (stream, peer) = match accepted {
            Ok(accepted) => accepted,
            Err(error) => {
                tracing::warn!(%error, "egress proxy accept");
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        let Ok(permit) = Arc::clone(&slots).try_acquire_owned() else {
            continue;
        };
        let proxy = Arc::clone(&proxy);
        tokio::spawn(async move {
            proxy.handle(stream, peer).await;
            drop(permit);
        });
    }
}

async fn reply(stream: &mut TcpStream, status: &str, body: &Value) {
    let body = body.to_string();
    let challenge = if status.starts_with("407") {
        "Proxy-Authenticate: Basic realm=\"orbit-sandbox\"\r\n"
    } else {
        ""
    };
    let response = format!(
        "HTTP/1.1 {status}\r\n{challenge}Content-Type: application/json\r\nContent-Length: {}\r\n\
         Connection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}

const ESTABLISHED: &[u8] = b"HTTP/1.1 200 Connection established\r\n\r\n";

impl Proxy {
    async fn handle(&self, mut client: TcpStream, peer: SocketAddr) {
        let _ = client.set_nodelay(true);
        let Ok(Ok((head, early))) = timeout(HEAD_TIMEOUT, read_head(&mut client)).await else {
            return;
        };
        let refused = json!({ "error": "proxy_auth_required" });
        let Some((call, password)) = credentials(&head) else {
            return reply(&mut client, "407 Proxy Authentication Required", &refused).await;
        };
        if !self.known_call(call, &password, peer.ip()).await {
            tracing::warn!(%call, peer = %peer.ip(), "egress proxy: no running call with this password at this address");
            return reply(&mut client, "407 Proxy Authentication Required", &refused).await;
        }
        let Some(target) = target(&head) else {
            let body = json!({ "error": "connect_only", "detail": "Only HTTPS through CONNECT." });
            return reply(&mut client, "405 Method Not Allowed", &body).await;
        };
        // Orbit traffic is not egress: no byte cap, but it ends with the call too.
        let max_bytes = match target {
            Target::Orbit => u64::MAX,
            Target::Connect { .. } => self.config.call_max_bytes,
        };
        let slot = match self
            .calls
            .open(call, self.config.call_max_connections, max_bytes)
        {
            Ok(slot) => slot,
            Err(reason) => {
                let body = json!({ "error": "egress_limit", "detail": reason });
                return reply(&mut client, "403 Forbidden", &body).await;
            }
        };
        match target {
            Target::Orbit => self.orbit(client, &head, early, &slot.0, peer.ip()).await,
            Target::Connect { host, port } => {
                self.connect(client, early, &slot.0, call, &password, &host, port)
                    .await;
            }
        }
    }

    /// A running call's file names this password and the peer's address.
    async fn known_call(&self, call: Id, password: &str, peer: IpAddr) -> bool {
        let path = self.config.calls_dir.join(call.to_string());
        let Ok(raw) = tokio::fs::read(path).await else {
            return false;
        };
        let Ok(file) = serde_json::from_slice::<CallFile>(&raw) else {
            return false;
        };
        let given: String = Sha256::digest(password.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let password_ok: bool = given
            .as_bytes()
            .ct_eq(file.password_sha256.to_ascii_lowercase().as_bytes())
            .into();
        password_ok && peer.to_canonical() == file.addr.to_canonical()
    }

    /// One request to Orbit: the cleaned head, exactly its body, then Orbit's answer until it closes (or the call ends).
    async fn orbit(
        &self,
        mut client: TcpStream,
        head: &Head,
        mut early: Vec<u8>,
        call: &Call,
        sandbox: IpAddr,
    ) {
        let Some(length) = body_length(head) else {
            let body = json!({ "error": "length_required", "detail": "Send a Content-Length body, not chunked." });
            return reply(&mut client, "411 Length Required", &body).await;
        };
        let Ok(Ok(upstream)) = timeout(
            CONNECT_TIMEOUT,
            TcpStream::connect(self.config.orbit_internal),
        )
        .await
        else {
            let body = json!({ "error": "orbit_unavailable" });
            return reply(&mut client, "502 Bad Gateway", &body).await;
        };
        let _ = upstream.set_nodelay(true);
        // Bytes past the body (a second, pipelined request) never reach Orbit.
        early.truncate(usize::try_from(length).unwrap_or(usize::MAX));
        let rest = length - early.len() as u64;
        let (mut client_read, mut client_write) = client.into_split();
        let (mut upstream_read, mut upstream_write) = upstream.into_split();
        let upload = async {
            upstream_write
                .write_all(&orbit_request(head, sandbox))
                .await?;
            upstream_write.write_all(&early).await?;
            tokio::io::copy(&mut (&mut client_read).take(rest), &mut upstream_write).await
        };
        let download = tokio::io::copy(&mut upstream_read, &mut client_write);
        tokio::select! {
            _ = async { tokio::join!(upload, download) } => {}
            () = call.ended.cancelled() => {}
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn connect(
        &self,
        mut client: TcpStream,
        early: Vec<u8>,
        call: &Call,
        call_id: Id,
        password: &str,
        host: &str,
        port: u16,
    ) {
        let answer = match self.check(call_id, password, host, port).await {
            Check::Answer(answer) => answer,
            Check::Unknown => {
                let body = json!({ "error": "proxy_auth_required" });
                return reply(&mut client, "407 Proxy Authentication Required", &body).await;
            }
            Check::Unavailable => {
                let body = json!({ "error": "egress_unavailable", "detail": "Orbit did not answer; try again." });
                return reply(&mut client, "503 Service Unavailable", &body).await;
            }
        };
        let named = answer["host"].as_str().unwrap_or(host).to_owned();
        match answer["decision"].as_str() {
            Some("allow") => {}
            Some("ask") => {
                let body =
                    json!({ "error": "egress_approval_required", "host": named, "port": port });
                return reply(&mut client, "403 Forbidden", &body).await;
            }
            _ => {
                let body = json!({ "error": "egress_denied", "host": named, "port": port, "reason": answer["reason"] });
                return reply(&mut client, "403 Forbidden", &body).await;
            }
        }
        let (Some(log_id), Some(host)) = (answer["log_id"].as_str(), answer["host"].as_str())
        else {
            let body = json!({ "error": "egress_unavailable" });
            return reply(&mut client, "503 Service Unavailable", &body).await;
        };
        let started = Instant::now();
        let outcome = self.tunnel(client, early, call, host, port).await;
        if let Some(reason) = outcome.blocked {
            tracing::warn!(call = %call_id, host, reason, "egress proxy: blocked after the yes");
        }
        self.report(log_id, call_id, &outcome, started.elapsed())
            .await;
    }

    async fn check(&self, call: Id, password: &str, host: &str, port: u16) -> Check {
        let response = self
            .http
            .post(format!(
                "{}/api/internal/agent/egress/check",
                self.config.orbit_url
            ))
            .bearer_auth(&self.config.service_token)
            .json(
                &json!({ "call_id": call, "proxy_password": password, "host": host, "port": port }),
            )
            .send()
            .await;
        match response {
            Ok(response) if response.status().is_success() => response
                .json()
                .await
                .map_or(Check::Unavailable, Check::Answer),
            Ok(response) if response.status() == reqwest::StatusCode::FORBIDDEN => Check::Unknown,
            Ok(response) => {
                tracing::warn!(status = %response.status(), "egress proxy: Orbit's check failed");
                Check::Unavailable
            }
            Err(error) => {
                tracing::warn!(%error, "egress proxy: Orbit's check failed");
                Check::Unavailable
            }
        }
    }

    /// After Orbit's yes: resolve, refuse non-public addresses, connect to a checked address, check the SNI, relay.
    async fn tunnel(
        &self,
        mut client: TcpStream,
        early: Vec<u8>,
        call: &Call,
        host: &str,
        port: u16,
    ) -> Outcome {
        let blocked = |reason| Outcome {
            blocked: Some(reason),
            ..Outcome::default()
        };
        let addresses = match timeout(RESOLVE_TIMEOUT, self.dialer.resolve(host, port)).await {
            Ok(Ok(addresses)) if !addresses.is_empty() => addresses,
            _ => {
                let body = json!({ "error": "egress_unresolved", "host": host });
                reply(&mut client, "502 Bad Gateway", &body).await;
                return Outcome::default();
            }
        };
        if addresses.iter().any(|address| !is_public_ip(address.ip())) {
            let body = json!({ "error": "egress_blocked", "host": host, "reason": "the host has a non-public address" });
            reply(&mut client, "403 Forbidden", &body).await;
            return blocked("the host has a non-public address");
        }
        let mut upstream = None;
        for address in addresses.iter().take(4) {
            if let Ok(Ok(stream)) = timeout(CONNECT_TIMEOUT, self.dialer.connect(*address)).await {
                upstream = Some(stream);
                break;
            }
        }
        let Some(mut upstream) = upstream else {
            let body = json!({ "error": "egress_connect_failed", "host": host });
            reply(&mut client, "502 Bad Gateway", &body).await;
            return Outcome::default();
        };
        let _ = upstream.set_nodelay(true);
        if client.write_all(ESTABLISHED).await.is_err() {
            return Outcome::default();
        }
        let hello = tokio::select! {
            hello = timeout(HELLO_TIMEOUT, read_record(&mut client, early)) => hello,
            () = call.ended.cancelled() => return Outcome::default(),
        };
        let Ok(Ok(hello)) = hello else {
            return blocked("no TLS ClientHello");
        };
        if client_hello_sni(&hello).as_deref() != Some(host) {
            return blocked("the TLS server name (SNI) is missing or not the host");
        }
        let sent = hello.len() as u64;
        call.bytes.fetch_add(sent, Ordering::SeqCst);
        if upstream.write_all(&hello).await.is_err() {
            return Outcome {
                up: sent,
                ..Outcome::default()
            };
        }
        let (up, down) = relay(client, upstream, call, Some(self.config.call_max_bytes)).await;
        Outcome {
            up: up + sent,
            down,
            blocked: None,
        }
    }

    async fn report(&self, log_id: &str, call: Id, outcome: &Outcome, elapsed: Duration) {
        let body = json!({
            "log_id": log_id,
            "call_id": call,
            "bytes_up": outcome.up,
            "bytes_down": outcome.down,
            "duration_ms": u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX),
            "blocked": outcome.blocked,
        });
        let sent = self
            .http
            .post(format!(
                "{}/api/internal/agent/egress/done",
                self.config.orbit_url
            ))
            .bearer_auth(&self.config.service_token)
            .json(&body)
            .send()
            .await;
        if !sent.is_ok_and(|response| response.status().is_success()) {
            tracing::warn!(%call, "egress proxy: the byte report failed");
        }
    }
}

/// Relays until both sides close, the call ends, or (with `cap`) the call's bytes pass the cap. Returns the bytes
/// sent up and down.
async fn relay(
    client: TcpStream,
    upstream: TcpStream,
    call: &Call,
    cap: Option<u64>,
) -> (u64, u64) {
    let (mut client_read, mut client_write) = client.into_split();
    let (mut upstream_read, mut upstream_write) = upstream.into_split();
    let (up, down) = (AtomicU64::new(0), AtomicU64::new(0));
    let pumps = async {
        tokio::try_join!(
            pump(&mut client_read, &mut upstream_write, &up, call, cap),
            pump(&mut upstream_read, &mut client_write, &down, call, cap),
        )
    };
    tokio::select! {
        _ = pumps => {}
        () = call.ended.cancelled() => {}
    }
    (up.load(Ordering::SeqCst), down.load(Ordering::SeqCst))
}

async fn pump(
    from: &mut (impl AsyncRead + Unpin),
    to: &mut (impl AsyncWrite + Unpin),
    counter: &AtomicU64,
    call: &Call,
    cap: Option<u64>,
) -> io::Result<()> {
    let mut buffer = vec![0_u8; 16 * 1024];
    loop {
        let read = from.read(&mut buffer).await?;
        if read == 0 {
            let _ = to.shutdown().await;
            return Ok(());
        }
        let size = read as u64;
        if let Some(cap) = cap
            && call.bytes.fetch_add(size, Ordering::SeqCst) + size > cap
        {
            return Err(io::Error::other("the call's egress bytes are used up"));
        }
        to.write_all(&buffer[..read]).await?;
        counter.fetch_add(size, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::{Target, body_length, client_hello_sni, orbit_request, parse_head, target};

    /// A ClientHello record with the given SNI extension bytes (or none).
    fn hello(sni: Option<&str>) -> Vec<u8> {
        let mut extensions = Vec::new();
        // An unrelated extension first (supported_versions).
        extensions.extend_from_slice(&[0x00, 0x2b, 0x00, 0x03, 0x02, 0x03, 0x04]);
        if let Some(name) = sni {
            let name = name.as_bytes();
            let entry_len = 3 + name.len();
            extensions.extend_from_slice(&[0x00, 0x00]);
            extensions.extend_from_slice(&u16::try_from(entry_len + 2).unwrap().to_be_bytes());
            extensions.extend_from_slice(&u16::try_from(entry_len).unwrap().to_be_bytes());
            extensions.push(0);
            extensions.extend_from_slice(&u16::try_from(name.len()).unwrap().to_be_bytes());
            extensions.extend_from_slice(name);
        }
        let mut body = vec![0x03, 0x03];
        body.extend_from_slice(&[7; 32]);
        body.extend_from_slice(&[4, 1, 2, 3, 4]); // session id
        body.extend_from_slice(&[0, 2, 0x13, 0x01]); // one cipher suite
        body.extend_from_slice(&[1, 0]); // null compression
        body.extend_from_slice(&u16::try_from(extensions.len()).unwrap().to_be_bytes());
        body.extend_from_slice(&extensions);
        let mut handshake = vec![1, 0];
        handshake.extend_from_slice(&u16::try_from(body.len()).unwrap().to_be_bytes());
        handshake.extend_from_slice(&body);
        let mut record = vec![22, 3, 1];
        record.extend_from_slice(&u16::try_from(handshake.len()).unwrap().to_be_bytes());
        record.extend_from_slice(&handshake);
        record
    }

    #[test]
    fn the_server_name_comes_from_a_whole_client_hello_only() {
        assert_eq!(
            client_hello_sni(&hello(Some("API.Example.com."))).as_deref(),
            Some("api.example.com")
        );
        assert_eq!(client_hello_sni(&hello(None)), None);
        let whole = hello(Some("api.example.com"));
        for cut in [0, 5, 20, whole.len() - 1] {
            assert_eq!(client_hello_sni(&whole[..cut]), None, "cut at {cut}");
        }
        let mut not_handshake = whole.clone();
        not_handshake[0] = 23;
        assert_eq!(client_hello_sni(&not_handshake), None);
        assert_eq!(client_hello_sni(b"GET / HTTP/1.1\r\n\r\n"), None);
    }

    #[test]
    fn only_connect_and_plain_requests_to_orbit_internal_pass() {
        let head = |line: &str| parse_head(format!("{line}\r\nHost: x").as_bytes()).unwrap();
        assert!(matches!(
            target(&head("CONNECT api.example.com:443 HTTP/1.1")),
            Some(Target::Connect { ref host, port: 443 }) if host == "api.example.com"
        ));
        // A raw tunnel to Orbit is an egress request (Orbit denies it); plain HTTP is one cleaned request.
        assert!(matches!(
            target(&head("CONNECT Orbit.Internal:80 HTTP/1.1")),
            Some(Target::Connect { ref host, port: 80 }) if host == "Orbit.Internal"
        ));
        assert!(matches!(
            target(&head("GET http://orbit.internal/api/v1/me HTTP/1.1")),
            Some(Target::Orbit)
        ));
        for refused in [
            "GET http://example.com/ HTTP/1.1",
            "GET http://orbit.internal@example.com/ HTTP/1.1",
            "GET http://orbit.internal.example.com/ HTTP/1.1",
            "GET http://orbit.internal:8000/ HTTP/1.1",
            "GET / HTTP/1.1",
            "CONNECT api.example.com HTTP/1.1",
        ] {
            assert!(target(&head(refused)).is_none(), "{refused}");
        }
        assert!(parse_head(b"GET / HTTP/2\r\nHost: x").is_none());
    }

    #[test]
    fn a_request_to_orbit_carries_the_proxys_forwarding_headers_only() {
        let head = parse_head(
            b"POST http://orbit.internal?x=1 HTTP/1.1\r\nHost: orbit.internal\r\nProxy-Authorization: Basic abc\r\n\
              Proxy-Connection: keep-alive\r\nConnection: keep-alive, Upgrade\r\nUpgrade: websocket\r\n\
              X-Forwarded-For: 1.2.3.4\r\nx-forwarded-proto: http\r\nForwarded: for=1.2.3.4\r\nX-Real-IP: 1.2.3.4\r\n\
              Authorization: Bearer t\r\nContent-Length: 2",
        )
        .unwrap();
        let request =
            String::from_utf8(orbit_request(&head, "10.201.3.2".parse().unwrap())).unwrap();
        assert_eq!(
            request,
            "POST /?x=1 HTTP/1.1\r\nHost: orbit.internal\r\nAuthorization: Bearer t\r\nContent-Length: 2\r\n\
             X-Forwarded-For: 10.201.3.2\r\nX-Forwarded-Proto: https\r\nConnection: close\r\n\r\n"
        );
        assert_eq!(body_length(&head), Some(2));
    }

    #[test]
    fn a_request_to_orbit_needs_one_plain_body_length() {
        let length = |headers: &str| {
            body_length(&parse_head(format!("POST / HTTP/1.1\r\n{headers}").as_bytes()).unwrap())
        };
        assert_eq!(length("Host: x"), Some(0));
        assert_eq!(length("Content-Length: 10"), Some(10));
        assert_eq!(length("Transfer-Encoding: chunked"), None);
        assert_eq!(
            length("Content-Length: 10\r\nTransfer-Encoding: chunked"),
            None
        );
        assert_eq!(length("Content-Length: 10\r\nContent-Length: 10"), None);
        assert_eq!(length("Content-Length: -1"), None);
    }
}
