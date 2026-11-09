//! The exe.dev gateway's cookies for one server, and the hand-off that brings them from the sign-in
//! window (`orbit-desktop --gateway-signin <server>`, a separate process with a web view).
//!
//! The helper writes exactly one JSON line to its stdout, which the app reads from a pipe:
//! - `{"version":1,"server":"https://host","cookies":[{"name":…,"value":…,"domain":…,"path":…,"secure":…,"expires":…}]}`
//!   after the sign-in reached the server (`expires`: Unix seconds, absent for a session cookie);
//! - `{"version":1,"server":"https://host","cancelled":true}` when the person closed the window;
//! - `{"version":1,"server":"https://host","error":"…"}` when the web view failed.
//!
//! [`parse_hand_off`] trusts nothing in it: the server must be the one asked for, and only
//! well-formed, unexpired cookies that belong to the server's host are kept.

use serde::{Deserialize, Serialize};

use crate::secret::Secret;
use crate::server::ServerUrl;

pub const HAND_OFF_VERSION: u32 = 1;

/// One cookie as the helper writes it and the keychain keeps it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatewayCookie {
    pub name: String,
    pub value: Secret,
    /// Empty: host-only, for the server's host.
    #[serde(default)]
    pub domain: String,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub secure: bool,
    /// Unix seconds; `None` for a session cookie.
    #[serde(default)]
    pub expires: Option<i64>,
}

/// The gateway cookies of one server.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatewayCookies {
    pub server: String,
    pub cookies: Vec<GatewayCookie>,
}

/// What the sign-in window handed back.
#[derive(Debug, PartialEq, Eq)]
pub enum HandOff {
    SignedIn(GatewayCookies),
    Cancelled,
    Failed(String),
}

#[derive(Debug, PartialEq, Eq)]
pub enum HandOffError {
    Malformed,
    UnknownVersion(u32),
    /// The line names another server than the one the window was opened for.
    WrongServer,
    /// The sign-in finished but left no usable cookie for the server.
    NoCookies,
}

impl std::fmt::Display for HandOffError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HandOffError::Malformed => f.write_str("the sign-in window sent something unreadable"),
            HandOffError::UnknownVersion(v) => write!(f, "the sign-in window speaks version {v}"),
            HandOffError::WrongServer => f.write_str("the sign-in window answered for another server"),
            HandOffError::NoCookies => f.write_str("the sign-in finished without a gateway cookie"),
        }
    }
}

impl std::error::Error for HandOffError {}

#[derive(Serialize, Deserialize)]
struct Line {
    version: u32,
    server: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cookies: Option<Vec<GatewayCookie>>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    cancelled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

/// The line the helper writes (no newline).
pub fn hand_off_line(server: &ServerUrl, outcome: &HandOff) -> String {
    let mut line = Line { version: HAND_OFF_VERSION, server: server.origin().to_owned(), cookies: None, cancelled: false, error: None };
    match outcome {
        HandOff::SignedIn(jar) => line.cookies = Some(jar.cookies.clone()),
        HandOff::Cancelled => line.cancelled = true,
        HandOff::Failed(e) => line.error = Some(e.clone()),
    }
    serde_json::to_string(&line).expect("plain data serializes")
}

/// Reads the helper's line for `server`, at `now` (Unix seconds).
pub fn parse_hand_off(line: &str, server: &ServerUrl, now: i64) -> Result<HandOff, HandOffError> {
    let line: Line = serde_json::from_str(line.trim()).map_err(|_| HandOffError::Malformed)?;
    if line.version != HAND_OFF_VERSION {
        return Err(HandOffError::UnknownVersion(line.version));
    }
    if line.server != server.origin() {
        return Err(HandOffError::WrongServer);
    }
    if line.cancelled {
        return Ok(HandOff::Cancelled);
    }
    if let Some(error) = line.error {
        return Ok(HandOff::Failed(error.chars().take(200).collect()));
    }
    let cookies: Vec<GatewayCookie> = line.cookies.unwrap_or_default().into_iter()
        .filter(|c| well_formed(c) && domain_matches(&c.domain, server.host()) && !expired(c, now))
        .filter(|c| server.is_secure() || !c.secure)
        .collect();
    if cookies.is_empty() {
        return Err(HandOffError::NoCookies);
    }
    Ok(HandOff::SignedIn(GatewayCookies { server: server.origin().to_owned(), cookies }))
}

impl GatewayCookies {
    /// The `Cookie` header for a request to `path` on `server` at `now`, or `None` when no cookie
    /// applies (all expired: the app opens the sign-in window again).
    pub fn header(&self, server: &ServerUrl, path: &str, now: i64) -> Option<Secret> {
        if self.server != server.origin() {
            return None;
        }
        let pairs: Vec<String> = self.cookies.iter()
            .filter(|c| !expired(c, now) && path_matches(&c.path, path) && (server.is_secure() || !c.secure))
            .map(|c| format!("{}={}", c.name, c.value.expose()))
            .collect();
        (!pairs.is_empty()).then(|| Secret::new(pairs.join("; ")))
    }

    /// The earliest expiry among cookies that have one.
    pub fn expires(&self) -> Option<i64> {
        self.cookies.iter().filter_map(|c| c.expires).min()
    }
}

fn well_formed(c: &GatewayCookie) -> bool {
    let token = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_graphic() && !b"()<>@,;:\\\"/[]?={}".contains(&b));
    let octets = |s: &str| s.bytes().all(|b| b.is_ascii_graphic() && !matches!(b, b'"' | b',' | b';' | b'\\'));
    token(&c.name) && octets(c.value.expose())
}

/// RFC 6265 §5.1.3: the host itself, or a parent domain on a dot boundary.
fn domain_matches(domain: &str, host: &str) -> bool {
    let domain = domain.trim_start_matches('.').to_ascii_lowercase();
    domain.is_empty() || domain == host || (host.ends_with(&domain) && host[..host.len() - domain.len()].ends_with('.'))
}

/// RFC 6265 §5.1.4.
fn path_matches(cookie_path: &str, path: &str) -> bool {
    if cookie_path.is_empty() || cookie_path == "/" || cookie_path == path {
        return true;
    }
    path.starts_with(cookie_path) && (cookie_path.ends_with('/') || path[cookie_path.len()..].starts_with('/'))
}

fn expired(c: &GatewayCookie, now: i64) -> bool {
    c.expires.is_some_and(|at| at <= now)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000;

    fn server() -> ServerUrl {
        ServerUrl::parse("https://orbit.example.com").unwrap()
    }

    fn cookie(name: &str, value: &str, domain: &str) -> serde_json::Value {
        serde_json::json!({ "name": name, "value": value, "domain": domain, "path": "/", "secure": true })
    }

    fn line(cookies: serde_json::Value) -> String {
        serde_json::json!({ "version": 1, "server": "https://orbit.example.com", "cookies": cookies }).to_string()
    }

    #[test]
    fn a_hand_off_keeps_only_the_servers_good_cookies() {
        let mut expired = cookie("old", "x", "orbit.example.com");
        expired["expires"] = (NOW - 1).into();
        let cookies = serde_json::json!([
            cookie("exe-session", "abc", "orbit.example.com"),
            cookie("parent", "p", ".example.com"),
            cookie("other", "o", "evil.example.org"),
            cookie("suffix", "s", "bit.example.com"),
            cookie("bad;name", "v", ""),
            cookie("badvalue", "a;b", ""),
            expired,
        ]);
        let HandOff::SignedIn(jar) = parse_hand_off(&line(cookies), &server(), NOW).unwrap() else { panic!() };
        let names: Vec<&str> = jar.cookies.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["exe-session", "parent"]);
        assert_eq!(jar.header(&server(), "/api/v1/auth/me", NOW).unwrap().expose(), "exe-session=abc; parent=p");
    }

    #[test]
    fn a_hand_off_for_another_server_or_without_cookies_is_refused() {
        let other = serde_json::json!({ "version": 1, "server": "https://evil.example.org", "cookies": [cookie("a", "b", "")] });
        assert_eq!(parse_hand_off(&other.to_string(), &server(), NOW), Err(HandOffError::WrongServer));
        assert_eq!(parse_hand_off(&line(serde_json::json!([])), &server(), NOW), Err(HandOffError::NoCookies));
        assert_eq!(parse_hand_off("not json", &server(), NOW), Err(HandOffError::Malformed));
        let v2 = serde_json::json!({ "version": 2, "server": "https://orbit.example.com" });
        assert_eq!(parse_hand_off(&v2.to_string(), &server(), NOW), Err(HandOffError::UnknownVersion(2)));
    }

    #[test]
    fn the_helpers_lines_round_trip() {
        let jar = GatewayCookies {
            server: server().origin().to_owned(),
            cookies: vec![GatewayCookie { name: "a".into(), value: Secret::new("b"), domain: String::new(), path: "/".into(), secure: true, expires: Some(NOW + 60) }],
        };
        for outcome in [HandOff::SignedIn(jar), HandOff::Cancelled, HandOff::Failed("no web view".into())] {
            assert_eq!(parse_hand_off(&hand_off_line(&server(), &outcome), &server(), NOW), Ok(outcome));
        }
    }

    #[test]
    fn the_header_drops_expired_and_out_of_path_cookies() {
        let jar = GatewayCookies {
            server: server().origin().to_owned(),
            cookies: vec![
                GatewayCookie { name: "a".into(), value: Secret::new("1"), domain: String::new(), path: "/api".into(), secure: true, expires: Some(NOW + 10) },
                GatewayCookie { name: "b".into(), value: Secret::new("2"), domain: String::new(), path: "/docs".into(), secure: true, expires: None },
            ],
        };
        assert_eq!(jar.header(&server(), "/api/v1/x", NOW).unwrap().expose(), "a=1");
        assert_eq!(jar.header(&server(), "/apix", NOW), None);
        assert_eq!(jar.header(&server(), "/api/v1/x", NOW + 10), None, "expired at its time");
        assert_eq!(jar.expires(), Some(NOW + 10));
        let other = ServerUrl::parse("https://other.example.com").unwrap();
        assert_eq!(jar.header(&other, "/api", NOW), None, "never sent to another server");
    }
}
