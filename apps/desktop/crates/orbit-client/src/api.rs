//! The HTTP client: one server, its gateway cookies and the Orbit session.
//!
//! Every request sends `X-Orbit-Contract`, the gateway's `Cookie` (when the server has a gateway)
//! and `Authorization: Bearer <session>` (once signed in). It never sends `Origin`: Orbit treats a
//! request without it as a native client and returns the session token in the sign-in body
//! (`SessionDelivery::Token`, `apps/server/src/auth_routes.rs`). Redirects are not followed, so the
//! gateway's `307` to its sign-in page reaches [`classify_response`] and nothing is sent twice.
//! Tokens and cookies are [`Secret`]s; they appear in headers only, never in a URL or an error.
//!
//! **Browser mode** (the risk-4b fallback, see `apps/desktop/README.md`): when the gateway does not
//! pass `Authorization` to Orbit, or passes its own cookie, the same session token goes as Orbit's
//! session cookie with `Origin: <server>`, as a browser on the server's own origin would send it.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::cookies::GatewayCookies;
use crate::gateway::{GatewayReply, classify};
use crate::secret::Secret;
use crate::server::ServerUrl;
use crate::types::{AuthUser, MemberPage, MemberRecord, MessagePage, MessageRecord, MessageWrite, SetupStatus, ThreadPage, WorkspaceRecord};
use crate::CONTRACT_ID;

/// Orbit's `application/problem+json` body (the fields the app reads).
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct Problem {
    #[serde(default)]
    pub code: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub detail: String,
}

/// Why a request failed, by who answered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApiError {
    /// The gateway wants its browser sign-in (no cookie, or it expired): open the sign-in window.
    GatewaySignIn,
    /// The gateway answered `401` itself: it refused what it read (an expired cookie, or a bearer
    /// token it took for its own).
    GatewayRejected,
    /// Orbit's `401`: no session, or it ended (expired, revoked, signed out elsewhere).
    SignedOut(Problem),
    /// Orbit's `409 contract_mismatch`: this app is too old or too new for the server.
    ContractMismatch,
    /// Any other Orbit problem (wrong password is `SignedOut` on the sign-in route).
    Problem(u16, Problem),
    /// A status without a problem body: not Orbit (a proxy error page, a wrong address).
    Status(u16),
    Network(String),
    Decode(String),
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApiError::GatewaySignIn => f.write_str("Sign in to the server's gateway first."),
            ApiError::GatewayRejected => f.write_str("The server's gateway refused the sign-in. Sign in again."),
            ApiError::SignedOut(p) if !p.detail.is_empty() => f.write_str(&p.detail),
            ApiError::SignedOut(_) => f.write_str("Your session ended. Sign in again."),
            ApiError::ContractMismatch => f.write_str("This app does not match the server's version. Update the desktop app."),
            ApiError::Problem(status, p) => write!(f, "{} (HTTP {status})", if p.detail.is_empty() { &p.title } else { &p.detail }),
            ApiError::Status(status) => write!(f, "The server answered HTTP {status}."),
            ApiError::Network(e) => write!(f, "Cannot reach the server: {e}"),
            ApiError::Decode(e) => write!(f, "The server sent something unexpected: {e}"),
        }
    }
}

impl std::error::Error for ApiError {}

/// Reads a finished response: `Ok` for a 2xx that reached Orbit, else who refused it and why.
/// Pure, so the rules are tested without a server.
pub fn classify_response(
    status: u16,
    location: Option<&str>,
    www_authenticate: Option<&str>,
    content_type: Option<&str>,
    body: &str,
) -> Result<(), ApiError> {
    match classify(status, location, www_authenticate) {
        GatewayReply::SignInRequired => return Err(ApiError::GatewaySignIn),
        GatewayReply::TokenRejected => return Err(ApiError::GatewayRejected),
        GatewayReply::Passed => {}
    }
    if (200..300).contains(&status) {
        return Ok(());
    }
    let problem = content_type
        .is_some_and(|t| t.trim().to_ascii_lowercase().starts_with("application/problem+json"))
        .then(|| serde_json::from_str::<Problem>(body).ok())
        .flatten();
    Err(match problem {
        Some(p) if status == 401 => ApiError::SignedOut(p),
        Some(p) if status == 409 && p.code == "contract_mismatch" => ApiError::ContractMismatch,
        Some(p) => ApiError::Problem(status, p),
        None => ApiError::Status(status),
    })
}

/// An Orbit session the app holds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub session_id: String,
    pub token: Secret,
    /// Send the token as Orbit's session cookie with `Origin` instead of as a bearer token.
    #[serde(default)]
    pub browser: bool,
}

/// Whether a failure is the gateway in the way of the bearer session (risk 4b), so the app should
/// switch to browser mode. `fresh_bearer`: the bearer session was just made and nothing has
/// reached Orbit with it yet, so a refusal now is not an ended session.
/// - `403 origin_forbidden` on sign-in: the proxy passes its own cookie, and Orbit refuses a
///   request without `Origin` that carries a cookie.
/// - `401` from the gateway (it read the bearer as its own token) or from Orbit (the proxy
///   stripped `Authorization`) on the first call after sign-in.
pub fn needs_browser_mode(error: &ApiError, fresh_bearer: bool) -> bool {
    match error {
        ApiError::Problem(403, p) => p.code == "origin_forbidden",
        ApiError::GatewayRejected | ApiError::SignedOut(_) => fresh_bearer,
        _ => false,
    }
}

/// The name of Orbit's session cookie (`apps/server/src/auth_routes.rs`).
fn session_cookie_name(server: &ServerUrl) -> &'static str {
    if server.is_secure() { "__Host-orbit_session" } else { "orbit_session_dev" }
}

/// The value of cookie `name` in `Set-Cookie` header values.
pub fn set_cookie_value(set_cookies: &[String], name: &str) -> Option<String> {
    set_cookies.iter().find_map(|line| {
        let pair = line.split(';').next()?.trim();
        let (key, value) = pair.split_once('=')?;
        (key.trim() == name && !value.is_empty()).then(|| value.trim().to_owned())
    })
}

/// A finished request that reached Orbit.
struct Reply {
    status: u16,
    body: String,
    set_cookies: Vec<String>,
}

#[derive(Debug, PartialEq)]
pub enum LoginOutcome {
    SignedIn { session: Session, user: AuthUser },
    /// The account has two-factor sign-in on: send a code with this token within five minutes.
    TwoFactor(Secret),
}

#[derive(Deserialize)]
struct LoginResponse {
    user: AuthUser,
    session_id: String,
    #[serde(default)]
    session_token: Option<Secret>,
}

#[derive(Deserialize)]
struct TwoFactorChallenge {
    two_factor_token: Secret,
}

const USER_AGENT: &str = concat!("orbit-desktop/", env!("CARGO_PKG_VERSION"));
/// Large enough for a page of chat; anything bigger is not an answer the app expects.
const BODY_LIMIT: u64 = 16 * 1024 * 1024;

pub struct Client {
    server: ServerUrl,
    agent: ureq::Agent,
    gateway: Option<GatewayCookies>,
    session: Option<Session>,
    /// Browser mode for the next sign-in (a session keeps its own flag).
    browser: bool,
}

impl Client {
    pub fn new(server: ServerUrl) -> Self {
        let agent = ureq::Agent::config_builder()
            .max_redirects(0)
            .http_status_as_error(false)
            .timeout_connect(Some(Duration::from_secs(10)))
            .timeout_global(Some(Duration::from_secs(30)))
            .user_agent(USER_AGENT)
            .build()
            .into();
        Client { server, agent, gateway: None, session: None, browser: false }
    }

    pub fn server(&self) -> &ServerUrl {
        &self.server
    }

    pub fn set_gateway(&mut self, cookies: Option<GatewayCookies>) {
        self.gateway = cookies;
    }

    pub fn gateway(&self) -> Option<&GatewayCookies> {
        self.gateway.as_ref()
    }

    pub fn set_session(&mut self, session: Option<Session>) {
        self.session = session;
    }

    pub fn session(&self) -> Option<&Session> {
        self.session.as_ref()
    }

    /// The headers every request and socket upgrade carries: name and value. Values that are
    /// secrets stay [`Secret`]s until the request is built.
    pub fn auth_headers(&self, path: &str) -> Vec<(&'static str, Secret)> {
        let mut headers = vec![("X-Orbit-Contract", Secret::new(CONTRACT_ID))];
        let gateway = self.gateway.as_ref().and_then(|g| g.header(&self.server, path, now()));
        match &self.session {
            Some(session) if session.browser => {
                let mut cookie = format!("{}={}", session_cookie_name(&self.server), session.token.expose());
                if let Some(gateway) = gateway {
                    cookie = format!("{}; {cookie}", gateway.expose());
                }
                headers.push(("Origin", Secret::new(self.server.origin())));
                headers.push(("Cookie", Secret::new(cookie)));
            }
            session => {
                if let Some(cookie) = gateway {
                    headers.push(("Cookie", cookie));
                }
                if let Some(session) = session {
                    headers.push(("Authorization", Secret::new(format!("Bearer {}", session.token.expose()))));
                }
            }
        }
        headers
    }

    /// Answers `Ok` when the address is an Orbit server the app can reach now.
    pub fn setup_status(&self) -> Result<SetupStatus, ApiError> {
        self.get("/api/v1/setup/status")
    }

    pub fn login(&mut self, email: &str, password: &Secret) -> Result<LoginOutcome, ApiError> {
        #[derive(Serialize)]
        struct Body<'a> {
            email: &'a str,
            password: &'a str,
        }
        let reply = self.send("POST", "/api/v1/auth/login", Some(&Body { email, password: password.expose() }))?;
        if reply.status == 202 {
            let challenge: TwoFactorChallenge = decode(&reply.body)?;
            return Ok(LoginOutcome::TwoFactor(challenge.two_factor_token));
        }
        self.signed_in(&reply)
    }

    /// Browser mode for the sign-in: the next sign-in sends `Origin`, so Orbit answers with its
    /// session cookie, and every later request carries that cookie and `Origin`.
    pub fn set_browser_mode(&mut self, on: bool) {
        self.browser = on;
        if let Some(session) = &mut self.session {
            session.browser = on;
        }
    }

    pub fn login_two_factor(&mut self, two_factor_token: &Secret, code: &str) -> Result<LoginOutcome, ApiError> {
        #[derive(Serialize)]
        struct Body<'a> {
            two_factor_token: &'a str,
            code: &'a str,
        }
        let body = Body { two_factor_token: two_factor_token.expose(), code: code.trim() };
        let reply = self.send("POST", "/api/v1/auth/login/two-factor", Some(&body))?;
        self.signed_in(&reply)
    }

    /// Ends the session on the server and forgets it here, also when the server already ended it.
    pub fn logout(&mut self) -> Result<(), ApiError> {
        let result = self.send::<()>("POST", "/api/v1/auth/logout", None).map(|_| ());
        if matches!(result, Ok(()) | Err(ApiError::SignedOut(_))) {
            self.session = None;
            return Ok(());
        }
        result
    }

    /// Signs out the way a browser does: `Origin` and Orbit's session cookie, no bearer. Only for
    /// cleanup when the bearer cannot reach Orbit (the risk-4b check): the session must not stay.
    pub fn logout_as_browser(&mut self) -> Result<(), ApiError> {
        let Some(session) = self.session.clone() else { return Ok(()) };
        let name = if self.server.is_secure() { "__Host-orbit_session" } else { "orbit_session_dev" };
        let path = "/api/v1/auth/logout";
        let mut cookie = format!("{name}={}", session.token.expose());
        if let Some(gateway) = self.gateway.as_ref().and_then(|g| g.header(&self.server, path, now())) {
            cookie = format!("{}; {cookie}", gateway.expose());
        }
        let headers = vec![
            ("X-Orbit-Contract", Secret::new(CONTRACT_ID)),
            ("Origin", Secret::new(self.server.origin())),
            ("Cookie", Secret::new(cookie)),
        ];
        self.send_with::<()>("POST", path, None, headers)?;
        self.session = None;
        Ok(())
    }

    pub fn me(&self) -> Result<AuthUser, ApiError> {
        self.get("/api/v1/auth/me")
    }

    pub fn workspaces(&self) -> Result<Vec<WorkspaceRecord>, ApiError> {
        self.get("/api/v1/workspaces")
    }

    /// The workspace's members (names of message authors), up to 500.
    pub fn members(&self, workspace: &str) -> Result<Vec<MemberRecord>, ApiError> {
        Ok(self.get::<MemberPage>(&format!("/api/v1/workspaces/{}/members?limit=500", id(workspace)))?.items)
    }

    /// The newest messages of a conversation, oldest first.
    pub fn messages(&self, workspace: &str, conversation: &str) -> Result<MessagePage, ApiError> {
        self.get(&format!("/api/v1/workspaces/{}/chat/conversations/{}/messages?limit=100", id(workspace), id(conversation)))
    }

    /// A thread: its root and its replies, oldest first.
    pub fn thread(&self, workspace: &str, root: &str) -> Result<ThreadPage, ApiError> {
        self.get(&format!("/api/v1/workspaces/{}/chat/threads/{}?limit=100", id(workspace), id(root)))
    }

    /// Sends a message. A retry with the same `nonce` returns the first message, never a second one.
    pub fn send_message(
        &self,
        workspace: &str,
        conversation: &str,
        thread_root_id: Option<&str>,
        nonce: &str,
        body: &str,
    ) -> Result<MessageRecord, ApiError> {
        #[derive(Serialize)]
        struct Body<'a> {
            body: &'a str,
            nonce: &'a str,
            thread_root_id: Option<&'a str>,
        }
        let path = format!("/api/v1/workspaces/{}/chat/conversations/{}/messages", id(workspace), id(conversation));
        let reply = self.send("POST", &path, Some(&Body { body, nonce, thread_root_id }))?;
        Ok(decode::<MessageWrite>(&reply.body)?.result)
    }

    /// Replaces the body of one of the caller's messages.
    pub fn edit_message(&self, workspace: &str, message: &str, body: &str) -> Result<MessageRecord, ApiError> {
        #[derive(Serialize)]
        struct Body<'a> {
            body: &'a str,
        }
        let path = format!("/api/v1/workspaces/{}/chat/messages/{}", id(workspace), id(message));
        let reply = self.send("PATCH", &path, Some(&Body { body }))?;
        Ok(decode::<MessageWrite>(&reply.body)?.result)
    }

    /// A GET of `path` (which starts with `/api/`), decoded as `T`.
    pub fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, ApiError> {
        decode(&self.send::<()>("GET", path, None)?.body)
    }

    fn signed_in(&mut self, reply: &Reply) -> Result<LoginOutcome, ApiError> {
        let login: LoginResponse = decode(&reply.body)?;
        // Browser mode: the token comes as Orbit's session cookie, not in the body.
        let from_cookie = || set_cookie_value(&reply.set_cookies, session_cookie_name(&self.server)).map(Secret::new);
        let token = login.session_token.filter(|t| !t.is_empty()).or_else(from_cookie)
            .ok_or_else(|| ApiError::Decode("the sign-in returned no session token".into()))?;
        let session = Session { session_id: login.session_id, token, browser: self.browser };
        self.session = Some(session.clone());
        Ok(LoginOutcome::SignedIn { session, user: login.user })
    }

    fn send<B: Serialize>(&self, method: &str, path: &str, body: Option<&B>) -> Result<Reply, ApiError> {
        let mut headers = self.auth_headers(path);
        // Browser mode before a session exists: the sign-in itself sends `Origin`.
        if self.browser && self.session.is_none() {
            headers.push(("Origin", Secret::new(self.server.origin())));
        }
        self.send_with(method, path, body, headers)
    }

    fn send_with<B: Serialize>(
        &self,
        method: &str,
        path: &str,
        body: Option<&B>,
        headers: Vec<(&'static str, Secret)>,
    ) -> Result<Reply, ApiError> {
        let url = self.server.http(path);
        let response = match method {
            "GET" => {
                let mut request = self.agent.get(&url).header("Accept", "application/json");
                for (name, value) in &headers {
                    request = request.header(*name, value.expose());
                }
                request.call()
            }
            _ => {
                let mut request = match method {
                    "PATCH" => self.agent.patch(&url),
                    _ => self.agent.post(&url),
                }
                .header("Accept", "application/json");
                for (name, value) in &headers {
                    request = request.header(*name, value.expose());
                }
                match body {
                    // Bytes, not `send_json`: a known length instead of a chunked body.
                    Some(body) => {
                        let bytes = serde_json::to_vec(body).map_err(|e| ApiError::Decode(e.to_string()))?;
                        request.header("Content-Type", "application/json").send(&bytes[..])
                    }
                    None => request.send_empty(),
                }
            }
        };
        let mut response = response.map_err(network)?;
        let status = response.status().as_u16();
        let header = |name: &str| response.headers().get(name).and_then(|v| v.to_str().ok()).map(str::to_owned);
        let (location, challenge, content_type) = (header("location"), header("www-authenticate"), header("content-type"));
        let set_cookies = response.headers().get_all("set-cookie").iter().filter_map(|v| v.to_str().ok()).map(str::to_owned).collect();
        let text = response.body_mut().with_config().limit(BODY_LIMIT).read_to_string().map_err(network)?;
        classify_response(status, location.as_deref(), challenge.as_deref(), content_type.as_deref(), &text)?;
        Ok(Reply { status, body: text, set_cookies })
    }
}

/// Ids go into paths as they are; anything that is not an id character is dropped.
fn id(value: &str) -> String {
    value.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect()
}

fn decode<T: DeserializeOwned>(body: &str) -> Result<T, ApiError> {
    serde_json::from_str(body).map_err(|e| ApiError::Decode(e.to_string()))
}

/// ureq's errors name the problem (refused, timed out, TLS), never a header.
fn network(e: ureq::Error) -> ApiError {
    ApiError::Network(e.to_string())
}

pub(crate) fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROBLEM: Option<&str> = Some("application/problem+json");

    #[test]
    fn gateway_answers_are_told_apart_from_orbits() {
        let login = Some("https://orbit.example.com/__exe.dev/login?exedev_host=orbit.example.com&redirect=%2F");
        assert_eq!(classify_response(307, login, None, Some("text/html"), ""), Err(ApiError::GatewaySignIn));
        let proxy_401 = "invalid or missing authentication (trace 1)";
        assert_eq!(classify_response(401, None, Some("Bearer, Basic"), Some("text/plain"), proxy_401), Err(ApiError::GatewayRejected));

        let orbit_401 = r#"{"code":"authentication_required","title":"Authentication required","detail":"Sign in.","status":401}"#;
        assert_eq!(
            classify_response(401, None, None, PROBLEM, orbit_401),
            Err(ApiError::SignedOut(Problem { code: "authentication_required".into(), title: "Authentication required".into(), detail: "Sign in.".into() }))
        );
        let mismatch = r#"{"code":"contract_mismatch","title":"API contract mismatch","detail":"x"}"#;
        assert_eq!(classify_response(409, None, None, PROBLEM, mismatch), Err(ApiError::ContractMismatch));
        let throttled = r#"{"code":"authentication_throttled","title":"Too many","detail":"Wait."}"#;
        assert!(matches!(classify_response(429, None, None, Some("application/problem+json; charset=utf-8"), throttled), Err(ApiError::Problem(429, p)) if p.code == "authentication_throttled"));
        // A 401 without a problem body or a challenge is neither: a wrong address or a broken proxy.
        assert_eq!(classify_response(401, None, None, Some("text/html"), "<html>"), Err(ApiError::Status(401)));
        assert_eq!(classify_response(502, None, None, Some("text/html"), "<html>"), Err(ApiError::Status(502)));
        assert_eq!(classify_response(204, None, None, None, ""), Ok(()));
        // Orbit's own redirects are not the gateway's.
        assert_eq!(classify_response(307, Some("/chat"), None, None, ""), Err(ApiError::Status(307)));
    }

    #[test]
    fn auth_headers_carry_the_gateway_cookie_and_the_session_and_never_origin() {
        use crate::cookies::GatewayCookie;
        let server = ServerUrl::parse("https://orbit.example.com").unwrap();
        let mut client = Client::new(server.clone());
        let names = |c: &Client| c.auth_headers("/api/v1/auth/me").iter().map(|(n, _)| *n).collect::<Vec<_>>();
        assert_eq!(names(&client), ["X-Orbit-Contract"]);
        client.set_gateway(Some(GatewayCookies {
            server: server.origin().to_owned(),
            cookies: vec![GatewayCookie { name: "exe".into(), value: Secret::new("c"), domain: String::new(), path: "/".into(), secure: true, expires: None }],
        }));
        client.set_session(Some(Session { session_id: "s".into(), token: Secret::new("t"), browser: false }));
        let headers = client.auth_headers("/api/v1/auth/me");
        let values: Vec<(&str, &str)> = headers.iter().map(|(n, v)| (*n, v.expose())).collect();
        assert_eq!(values, [("X-Orbit-Contract", CONTRACT_ID), ("Cookie", "exe=c"), ("Authorization", "Bearer t")]);
        assert!(!format!("{headers:?}").contains("Bearer t"), "headers print redacted");

        // Browser mode: the same token as Orbit's session cookie, with Origin, and no bearer.
        client.set_browser_mode(true);
        let headers = client.auth_headers("/api/v1/auth/me");
        let values: Vec<(&str, &str)> = headers.iter().map(|(n, v)| (*n, v.expose())).collect();
        assert_eq!(values, [
            ("X-Orbit-Contract", CONTRACT_ID),
            ("Origin", "https://orbit.example.com"),
            ("Cookie", "exe=c; __Host-orbit_session=t"),
        ]);
    }

    #[test]
    fn browser_mode_is_chosen_only_for_the_gateway_in_the_way() {
        let origin = ApiError::Problem(403, Problem { code: "origin_forbidden".into(), ..Default::default() });
        assert!(needs_browser_mode(&origin, false), "sign-in refused: the proxy passes its cookie");
        assert!(needs_browser_mode(&ApiError::GatewayRejected, true), "the proxy read the bearer as its own");
        assert!(needs_browser_mode(&ApiError::SignedOut(Problem::default()), true), "the proxy stripped it");
        // An older session that ends is just an ended session.
        assert!(!needs_browser_mode(&ApiError::SignedOut(Problem::default()), false));
        assert!(!needs_browser_mode(&ApiError::GatewayRejected, false));
        assert!(!needs_browser_mode(&ApiError::Problem(403, Problem { code: "forbidden".into(), ..Default::default() }), true));
        assert!(!needs_browser_mode(&ApiError::Network("down".into()), true));
    }

    #[test]
    fn the_session_cookie_is_read_from_set_cookie() {
        let lines = vec![
            "exe-session=x; Path=/".to_owned(),
            "__Host-orbit_session=tok123; Path=/; Secure; HttpOnly; SameSite=Lax".to_owned(),
        ];
        assert_eq!(set_cookie_value(&lines, "__Host-orbit_session").as_deref(), Some("tok123"));
        assert_eq!(set_cookie_value(&lines, "orbit_session_dev"), None);
        assert_eq!(set_cookie_value(&["__Host-orbit_session=; Max-Age=0".to_owned()], "__Host-orbit_session"), None, "a cleared cookie is none");
    }
}
