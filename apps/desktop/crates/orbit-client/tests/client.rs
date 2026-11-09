//! The HTTP client and the sockets against a scripted local server: what goes on the wire
//! (headers, no `Origin`, no secret in a URL), how answers are classified, and how sockets resume,
//! reconnect and stop.

mod mock;

use std::sync::mpsc;
use std::time::Duration;

use mock::{Mock, http, ws};
use orbit_client::live::{Ended, LiveFrame, WorkspaceEventKind};
use orbit_client::{
    ApiError, Client, GatewayCookie, GatewayCookies, LoginOutcome, Secret, ServerUrl, SocketConfig, SocketEvent, SocketKind,
};
use tungstenite::Message;
use tungstenite::protocol::CloseFrame;
use tungstenite::protocol::frame::coding::CloseCode;

const SESSION_TOKEN: &str = "sess-SYNTHETIC-0123456789";
const TWO_FACTOR_TOKEN: &str = "twofa-SYNTHETIC-abcdef";
const COOKIE: &str = "exe-gw-SYNTHETIC";
const JSON: &[(&str, &str)] = &[("Content-Type", "application/json")];
const PROBLEM: &[(&str, &str)] = &[("Content-Type", "application/problem+json")];

fn gateway_cookies(server: &ServerUrl) -> GatewayCookies {
    GatewayCookies {
        server: server.origin().to_owned(),
        cookies: vec![GatewayCookie { name: "exe-proxy".into(), value: Secret::new(COOKIE), domain: String::new(), path: "/".into(), secure: false, expires: None }],
    }
}

fn client(mock: &Mock) -> Client {
    let server = ServerUrl::parse(&mock.origin).unwrap();
    let mut client = Client::new(server.clone());
    client.set_gateway(Some(gateway_cookies(&server)));
    client
}

#[test]
fn a_native_sign_in_with_two_factor_then_logout() {
    let login = r#"{"user":{"id":"u1","email":"ada@example.test","display_name":"Ada","installation_admin":false,"root":false,"status":{}},"session_id":"s1","session_token":"sess-SYNTHETIC-0123456789"}"#;
    let mock = Mock::start(vec![
        http(202, JSON, r#"{"two_factor_token":"twofa-SYNTHETIC-abcdef"}"#),
        http(200, JSON, login),
        http(200, JSON, r#"{"id":"u1","email":"ada@example.test","display_name":"Ada","future":1}"#),
        http(204, &[], ""),
    ]);
    let mut client = client(&mock);
    let outcome = client.login("ada@example.test", &Secret::new("correct horse")).unwrap();
    let LoginOutcome::TwoFactor(challenge) = outcome else { panic!("{outcome:?}") };
    let outcome = client.login_two_factor(&challenge, " 123456 ").unwrap();
    assert!(!format!("{outcome:?}").contains(SESSION_TOKEN), "the outcome prints redacted");
    let LoginOutcome::SignedIn { session, user } = outcome else { panic!() };
    assert_eq!((user.display_name.as_str(), session.session_id.as_str()), ("Ada", "s1"));
    assert_eq!(client.me().unwrap().email, "ada@example.test");
    client.logout().unwrap();
    assert!(client.session().is_none());

    let seen = mock.finish();
    let lines: Vec<&str> = seen.iter().map(|s| s.line.as_str()).collect();
    assert_eq!(lines, [
        "POST /api/v1/auth/login HTTP/1.1",
        "POST /api/v1/auth/login/two-factor HTTP/1.1",
        "GET /api/v1/auth/me HTTP/1.1",
        "POST /api/v1/auth/logout HTTP/1.1",
    ]);
    for s in &seen {
        assert_eq!(s.header("origin"), None, "{}: a native client sends no Origin", s.line);
        assert_eq!(s.header("x-orbit-contract"), Some("orbit-api-v1"));
        assert_eq!(s.header("cookie"), Some(format!("exe-proxy={COOKIE}").as_str()), "{}: the gateway cookie", s.line);
        assert!(!s.line.contains(SESSION_TOKEN) && !s.line.contains(TWO_FACTOR_TOKEN), "no secret in a URL");
    }
    assert_eq!(seen[0].header("authorization"), None, "no session yet");
    assert!(seen[0].body.contains(r#""password":"correct horse""#));
    assert_eq!(seen[1].body, format!(r#"{{"two_factor_token":"{TWO_FACTOR_TOKEN}","code":"123456"}}"#));
    let bearer = format!("Bearer {SESSION_TOKEN}");
    assert_eq!(seen[2].header("authorization"), Some(bearer.as_str()));
    assert_eq!(seen[3].header("authorization"), Some(bearer.as_str()));
}

#[test]
fn gateway_and_orbit_refusals_are_told_apart() {
    let mock = Mock::start(vec![
        http(307, &[("Location", "/__exe.dev/login?exedev_host=x&redirect=%2Fapi%2Fv1%2Fsetup%2Fstatus")], ""),
        http(401, &[("Content-Type", "text/plain"), ("WWW-Authenticate", "Bearer, Basic")], "invalid or missing authentication"),
        http(401, PROBLEM, r#"{"code":"invalid_credentials","title":"Authentication failed","detail":"The email or password is not correct."}"#),
        http(409, PROBLEM, r#"{"code":"contract_mismatch","title":"API contract mismatch","detail":"x"}"#),
        http(200, JSON, r#"{"complete":true}"#),
    ]);
    let mut client = client(&mock);
    assert_eq!(client.setup_status(), Err(ApiError::GatewaySignIn), "the 307 is not followed");
    assert_eq!(client.me(), Err(ApiError::GatewayRejected));
    let wrong = client.login("ada@example.test", &Secret::new("wrong")).unwrap_err();
    assert!(matches!(&wrong, ApiError::SignedOut(p) if p.code == "invalid_credentials"), "{wrong:?}");
    assert_eq!(wrong.to_string(), "The email or password is not correct.");
    assert_eq!(client.me(), Err(ApiError::ContractMismatch));
    assert!(client.setup_status().unwrap().complete);
    let seen = mock.finish();
    assert_eq!(seen.len(), 5, "one request each, no redirect followed");
}

#[test]
fn a_client_without_a_server_answer_reports_the_network() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let client = Client::new(ServerUrl::parse(&origin).unwrap());
    assert!(matches!(client.setup_status(), Err(ApiError::Network(_))));
}

fn config() -> SocketConfig {
    SocketConfig { base_delay: Duration::from_millis(10), silence: Duration::from_secs(5), poll: Duration::from_millis(50) }
}

fn signed_in_headers(origin: &str) -> (ServerUrl, Vec<(&'static str, Secret)>) {
    let server = ServerUrl::parse(origin).unwrap();
    let mut client = Client::new(server.clone());
    client.set_gateway(Some(gateway_cookies(&server)));
    client.set_session(Some(orbit_client::Session { session_id: "s1".into(), token: Secret::new(SESSION_TOKEN), browser: false }));
    let headers = client.auth_headers("/api/v1/workspaces");
    (server, headers)
}

fn close(code: u16) -> Message {
    Message::Close(Some(CloseFrame { code: CloseCode::from(code), reason: "".into() }))
}

/// Collects the socket's events until it ends.
fn until_ended(events: &mpsc::Receiver<SocketEvent>) -> Vec<SocketEvent> {
    let mut all = Vec::new();
    loop {
        let event = events.recv_timeout(Duration::from_secs(10)).expect("the socket ends");
        let done = matches!(event, SocketEvent::Ended(_));
        all.push(event);
        if done {
            return all;
        }
    }
}

#[test]
fn the_live_socket_resumes_after_a_restart_and_stops_when_the_session_ends() {
    let mock = Mock::start(vec![
        ws(|s| {
            s.send(Message::text(r#"{"type":"hello","epoch":"ep1","seq":5,"presence":[]}"#)).unwrap();
            s.send(Message::text(r#"{"type":"resync"}"#)).unwrap();
            s.send(Message::text(r#"{"type":"ping"}"#)).unwrap();
            s.send(Message::text(r#"{"seq":6,"topic":"chat","event":{"type":"message.created"}}"#)).unwrap();
            s.send(close(1012)).unwrap();
        }),
        ws(|s| {
            s.send(Message::text(r#"{"type":"hello","epoch":"ep1","seq":7}"#)).unwrap();
            s.send(Message::text(r#"{"seq":7,"topic":"chat","event":{"type":"message.updated"}}"#)).unwrap();
            s.send(close(4401)).unwrap();
        }),
    ]);
    let (server, headers) = signed_in_headers(&mock.origin);
    let (tx, rx) = mpsc::channel();
    let socket = orbit_client::socket::spawn(SocketKind::Live, server, "0190-ws", headers, config(), move |e| { let _ = tx.send(e); });
    let events = until_ended(&rx);
    drop(socket);
    let seen = mock.finish();

    assert_eq!(seen[0].line, "GET /api/v1/workspaces/0190-ws/live", "first connection: no resume");
    assert_eq!(seen[1].line, "GET /api/v1/workspaces/0190-ws/live?epoch=ep1&after=6", "resumes after the last event");
    for s in &seen {
        assert_eq!(s.header("origin"), None);
        assert_eq!(s.header("authorization"), Some(format!("Bearer {SESSION_TOKEN}").as_str()));
        assert_eq!(s.header("cookie"), Some(format!("exe-proxy={COOKIE}").as_str()));
        assert!(!s.line.contains(SESSION_TOKEN));
    }
    let frames: Vec<&LiveFrame> = events.iter().filter_map(|e| match e { SocketEvent::Live(f) => Some(f), _ => None }).collect();
    assert_eq!(frames.len(), 5, "hello, resync, event; hello, event; no ping: {frames:?}");
    assert_eq!(frames[1], &LiveFrame::Resync);
    assert!(events.iter().any(|e| matches!(e, SocketEvent::Retrying { close_code: Some(1012), .. })), "1012 retries");
    assert_eq!(events.last(), Some(&SocketEvent::Ended(Ended::SignedOut)), "4401 stops for good");
}

#[test]
fn the_events_socket_resumes_after_a_resync_and_stops_without_access() {
    let mock = Mock::start(vec![
        ws(|s| {
            s.send(Message::text(r#"{"version":1,"kind":"resync_required","sequence":"12","workspaces_changed":true,"profile_changed":true,"views_only":false}"#)).unwrap();
            s.send(Message::text(r#"{"version":1,"kind":"workspace.changed","sequence":"13"}"#)).unwrap();
            s.send(close(1000)).unwrap();
        }),
        http(403, &[], ""),
    ]);
    let (server, headers) = signed_in_headers(&mock.origin);
    let (tx, rx) = mpsc::channel();
    let socket = orbit_client::socket::spawn(SocketKind::Events, server, "0190-ws", headers, config(), move |e| { let _ = tx.send(e); });
    let events = until_ended(&rx);
    drop(socket);
    let seen = mock.finish();

    assert_eq!(seen[0].line, "GET /api/v1/workspaces/0190-ws/events");
    assert!(seen[1].line.starts_with("GET /api/v1/workspaces/0190-ws/events?after=13 "), "{}", seen[1].line);
    let kinds: Vec<WorkspaceEventKind> = events.iter().filter_map(|e| match e { SocketEvent::Workspace(w) => Some(w.kind), _ => None }).collect();
    assert_eq!(kinds, [WorkspaceEventKind::ResyncRequired, WorkspaceEventKind::Changed]);
    assert_eq!(events.last(), Some(&SocketEvent::Ended(Ended::Forbidden)));
}

#[test]
fn a_socket_behind_the_gateway_stops_and_asks_for_the_sign_in_window() {
    let mock = Mock::start(vec![
        http(502, &[("Content-Type", "text/html")], "<html>bad gateway</html>"),
        http(307, &[("Location", "https://orbit.example.com/__exe.dev/login?redirect=%2F")], ""),
    ]);
    let (server, headers) = signed_in_headers(&mock.origin);
    let (tx, rx) = mpsc::channel();
    let socket = orbit_client::socket::spawn(SocketKind::Live, server, "w", headers, config(), move |e| { let _ = tx.send(e); });
    let events = until_ended(&rx);
    drop(socket);
    assert_eq!(mock.finish().len(), 2, "a 502 retries, the 307 stops");
    assert!(matches!(events[1], SocketEvent::Retrying { close_code: None, .. }));
    assert_eq!(events.last(), Some(&SocketEvent::Ended(Ended::Gateway(ApiError::GatewaySignIn))));
}

#[test]
fn a_stopped_socket_ends_its_thread_without_reconnecting() {
    let (opened_tx, opened_rx) = mpsc::channel();
    let mock = Mock::start(vec![ws(move |s| {
        s.send(Message::text(r#"{"type":"hello","epoch":"e","seq":0}"#)).unwrap();
        opened_tx.send(()).unwrap();
        // Waits for the client's close.
        while let Ok(m) = s.read() { if m.is_close() { break; } }
    })]);
    let (server, headers) = signed_in_headers(&mock.origin);
    let socket = orbit_client::socket::spawn(SocketKind::Live, server, "w", headers, config(), |_| {});
    opened_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    socket.stop();
    assert_eq!(mock.finish().len(), 1);
}

#[test]
fn the_browser_style_sign_out_sends_origin_and_the_session_cookie_but_no_bearer() {
    let mock = Mock::start(vec![http(204, &[], "")]);
    let mut client = client(&mock);
    client.set_session(Some(orbit_client::Session { session_id: "s1".into(), token: Secret::new(SESSION_TOKEN), browser: false }));
    client.logout_as_browser().unwrap();
    assert!(client.session().is_none());
    let origin = mock.origin.clone();
    let seen = mock.finish();
    assert_eq!(seen[0].line, "POST /api/v1/auth/logout HTTP/1.1");
    assert_eq!(seen[0].header("origin"), Some(origin.as_str()));
    assert_eq!(seen[0].header("authorization"), None);
    assert_eq!(seen[0].header("cookie"), Some(format!("exe-proxy={COOKIE}; orbit_session_dev={SESSION_TOKEN}").as_str()));
}

#[test]
fn a_server_that_drops_at_once_gets_growing_delays() {
    let drop_at_once = || ws(|s| s.send(close(1000)).unwrap());
    let mock = Mock::start(vec![drop_at_once(), drop_at_once(), drop_at_once(), ws(|s| s.send(close(4403)).unwrap())]);
    let (server, headers) = signed_in_headers(&mock.origin);
    let (tx, rx) = mpsc::channel();
    let socket = orbit_client::socket::spawn(SocketKind::Live, server, "w", headers, config(), move |e| { let _ = tx.send(e); });
    let events = until_ended(&rx);
    drop(socket);
    mock.finish();
    let delays: Vec<Duration> = events.iter().filter_map(|e| match e { SocketEvent::Retrying { after, .. } => Some(*after), _ => None }).collect();
    assert_eq!(delays.len(), 3);
    assert!(delays[0] < Duration::from_millis(16) && delays[1] >= Duration::from_millis(20) && delays[2] >= Duration::from_millis(40), "{delays:?}");
    assert_eq!(events.last(), Some(&SocketEvent::Ended(Ended::Forbidden)));
}
