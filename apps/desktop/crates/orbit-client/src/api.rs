//! The HTTP client: one server, its gateway cookies and the Orbit session.
//!
//! Every request sends `X-Orbit-Contract`, the gateway's `Cookie` (when the server has a gateway)
//! and `Authorization: Bearer <session>` (once signed in). It never sends `Origin`: Orbit treats a
//! request without it as a native client and returns the session token in the sign-in body
//! (`SessionDelivery::Token`, `apps/server/src/auth_routes.rs`). Redirects are not followed, so the
//! gateway's `307` to its sign-in page reaches [`classify_response`] and nothing is sent twice.
//! Tokens and cookies are [`Secret`]s; they appear in headers only, never in a URL or an error.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::cookies::GatewayCookies;
use crate::gateway::{GatewayReply, classify};
use crate::secret::Secret;
use crate::server::ServerUrl;
use crate::types::{AuthUser, SetupStatus, WorkspaceRecord};
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
        Client { server, agent, gateway: None, session: None }
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
        if let Some(cookie) = self.gateway.as_ref().and_then(|g| g.header(&self.server, path, now())) {
            headers.push(("Cookie", cookie));
        }
        if let Some(session) = &self.session {
            headers.push(("Authorization", Secret::new(format!("Bearer {}", session.token.expose()))));
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
        let (status, body) = self.send("POST", "/api/v1/auth/login", Some(&Body { email, password: password.expose() }))?;
        if status == 202 {
            let challenge: TwoFactorChallenge = decode(&body)?;
            return Ok(LoginOutcome::TwoFactor(challenge.two_factor_token));
        }
        self.signed_in(&body)
    }

    pub fn login_two_factor(&mut self, two_factor_token: &Secret, code: &str) -> Result<LoginOutcome, ApiError> {
        #[derive(Serialize)]
        struct Body<'a> {
            two_factor_token: &'a str,
            code: &'a str,
        }
        let body = Body { two_factor_token: two_factor_token.expose(), code: code.trim() };
        let (_, body) = self.send("POST", "/api/v1/auth/login/two-factor", Some(&body))?;
        self.signed_in(&body)
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

    /// A GET of `path` (which starts with `/api/`), decoded as `T`.
    pub fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, ApiError> {
        let (_, body) = self.send::<()>("GET", path, None)?;
        decode(&body)
    }

    fn signed_in(&mut self, body: &str) -> Result<LoginOutcome, ApiError> {
        let login: LoginResponse = decode(body)?;
        let token = login.session_token.filter(|t| !t.is_empty())
            .ok_or_else(|| ApiError::Decode("the sign-in returned no session token".into()))?;
        let session = Session { session_id: login.session_id, token };
        self.session = Some(session.clone());
        Ok(LoginOutcome::SignedIn { session, user: login.user })
    }

    fn send<B: Serialize>(&self, method: &str, path: &str, body: Option<&B>) -> Result<(u16, String), ApiError> {
        self.send_with(method, path, body, self.auth_headers(path))
    }

    fn send_with<B: Serialize>(
        &self,
        method: &str,
        path: &str,
        body: Option<&B>,
        headers: Vec<(&'static str, Secret)>,
    ) -> Result<(u16, String), ApiError> {
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
                let mut request = self.agent.post(&url).header("Accept", "application/json");
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
        let text = response.body_mut().with_config().limit(BODY_LIMIT).read_to_string().map_err(network)?;
        classify_response(status, location.as_deref(), challenge.as_deref(), content_type.as_deref(), &text)?;
        Ok((status, text))
    }
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
        client.set_session(Some(Session { session_id: "s".into(), token: Secret::new("t") }));
        let headers = client.auth_headers("/api/v1/auth/me");
        let values: Vec<(&str, &str)> = headers.iter().map(|(n, v)| (*n, v.expose())).collect();
        assert_eq!(values, [("X-Orbit-Contract", CONTRACT_ID), ("Cookie", "exe=c"), ("Authorization", "Bearer t")]);
        assert!(!format!("{headers:?}").contains("Bearer t"), "headers print redacted");
    }
}
