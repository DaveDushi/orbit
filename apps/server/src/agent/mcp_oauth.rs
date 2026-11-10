//! "Connect" for remote MCP servers that use OAuth: one shared sign-in per integration, which an
//! owner or admin starts. It follows the MCP authorization spec
//! (<https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization>):
//!
//! 1. Discovery: the server's 401 names its protected resource metadata (RFC 9728), else the
//!    well-known URLs; that names the authorization server, whose metadata (RFC 8414 or OIDC) Orbit
//!    reads in the spec's order and keeps only when its `issuer` is exact.
//! 2. Orbit registers itself with dynamic client registration (RFC 7591) at every Connect.
//! 3. Authorization code with PKCE (S256 only), the `resource` parameter (RFC 8707), and a
//!    `state` that is bound to the admin's session, works once and expires after 10 minutes. The
//!    PKCE verifier and the client stay sealed in the database.
//! 4. The tokens are sealed with the app key in `agent_integrations.oauth_sealed_connection`. A
//!    call renews the access token a minute before it expires (or once after a 401); a refresh the
//!    server refuses marks the connection "needs reconnect".
//!
//! No token ever goes to the browser, a log, a URL or an error message.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use orbit_platform::{Database, Id, Row, TimestampMillis};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use url::Url;

use super::AgentState;
use super::credentials::{open, seal};
use super::oauth::AccessToken;
use super::outbound::{Outbound, OutboundError, TestHosts, check_url, token_failure, usable_token};
use crate::mail::{Mailer, templates};

/// Where the authorization server sends the admin back, below the public URL.
pub(crate) const CALLBACK_PATH: &str = "/api/v1/agent-integrations/oauth/callback";
const STATE_LIFETIME_MS: i64 = 10 * 60 * 1_000;
/// An access token is renewed this long before it expires.
const RENEW_BEFORE_MS: i64 = 60 * 1_000;
/// Admins are warned this long before the grant ends.
pub(crate) const EXPIRY_WARNING_MS: i64 = 14 * 24 * 60 * 60 * 1_000;
/// Longer lifetimes are not believed (ten years).
const MAX_LIFETIME_SECONDS: i64 = 10 * 366 * 24 * 60 * 60;

// ---------------------------------------------------------------------------------------------
// What Orbit keeps.

/// The authorization server of one MCP server, from its metadata.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub(crate) struct Discovery {
    /// The `resource` of the protected resource metadata: sent as RFC 8707 `resource`.
    pub resource: String,
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub registration_endpoint: Option<String>,
    /// What Orbit asks for: the challenge's scope, else every scope the resource supports.
    pub scope: Option<String>,
    /// The server promised `iss` in its answer (RFC 9207), so it must be there.
    pub iss_required: bool,
    pub auth_methods: Vec<String>,
}

/// The client Orbit registered.
#[derive(Clone, Deserialize, Serialize)]
pub(crate) struct Client {
    pub client_id: String,
    pub client_secret: Option<String>,
    /// `none`, `client_secret_post` or `client_secret_basic`.
    pub auth_method: String,
}

impl Client {
    /// The client's part of a token request: its ID in the form, and the secret as it registered.
    fn form<'a>(&'a self, form: &mut Vec<(&'static str, &'a str)>) -> Option<(&'a str, &'a str)> {
        match (self.auth_method.as_str(), &self.client_secret) {
            ("client_secret_basic", Some(secret)) => Some((&self.client_id, secret)),
            ("client_secret_post", Some(secret)) => {
                form.push(("client_id", &self.client_id));
                form.push(("client_secret", secret));
                None
            }
            _ => {
                form.push(("client_id", &self.client_id));
                None
            }
        }
    }
}

/// A started Connect, sealed in `agent_integration_oauth_states`.
#[derive(Deserialize, Serialize)]
struct Pending {
    verifier: String,
    discovery: Discovery,
    client: Client,
    /// The integration's URL at the start: a change in between voids the sign-in.
    mcp_url: String,
    redirect_uri: String,
}

/// The sealed connection of an integration.
#[derive(Clone, Deserialize, Serialize)]
pub(crate) struct Connection {
    pub issuer: String,
    token_endpoint: String,
    resource: String,
    client: Client,
    access_token: String,
    access_expires_at: Option<i64>,
    refresh_token: Option<String>,
}

impl Connection {
    fn fresh(&self, now: i64) -> bool {
        self.access_expires_at
            .is_none_or(|expires_at| expires_at - RENEW_BEFORE_MS > now)
    }
}

/// A token answer, checked.
pub(crate) struct Tokens {
    access_token: String,
    access_expires_at: Option<i64>,
    refresh_token: Option<String>,
    refresh_expires_at: Option<i64>,
    /// Who signed in, when the answer tells.
    pub account: Option<String>,
}

impl Tokens {
    /// When the admin must connect again: the end of the refresh token, or of the access token
    /// when there is no refresh token; `None` when the server does not tell.
    pub(crate) fn grant_expires_at(&self) -> Option<i64> {
        self.refresh_expires_at.or(if self.refresh_token.is_none() {
            self.access_expires_at
        } else {
            None
        })
    }
}

macro_rules! redacted_debug {
    ($($name:ident),*) => {$(
        impl std::fmt::Debug for $name {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str(concat!(stringify!($name), "([REDACTED])"))
            }
        }
    )*};
}
redacted_debug!(Client, Pending, Connection, Tokens);

// ---------------------------------------------------------------------------------------------
// Parsing and checks.

/// `resource_metadata` and `scope` of a `WWW-Authenticate: Bearer …` challenge.
pub(crate) fn challenge_params(header: &str) -> (Option<String>, Option<String>) {
    let mut resource_metadata = None;
    let mut scope = None;
    let mut rest = header.trim();
    if rest
        .get(..6)
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("bearer"))
    {
        rest = &rest[6..];
    }
    loop {
        rest = rest.trim_start_matches([' ', ',', '\t']);
        let Some(equals) = rest.find('=') else { break };
        let name = rest[..equals].trim().to_ascii_lowercase();
        rest = rest[equals + 1..].trim_start();
        let value;
        if let Some(quoted) = rest.strip_prefix('"') {
            let mut text = String::new();
            let mut characters = quoted.char_indices();
            let mut end = quoted.len();
            while let Some((index, character)) = characters.next() {
                match character {
                    '\\' => {
                        if let Some((_, escaped)) = characters.next() {
                            text.push(escaped);
                        }
                    }
                    '"' => {
                        end = index + 1;
                        break;
                    }
                    other => text.push(other),
                }
            }
            value = text;
            rest = &quoted[end.min(quoted.len())..];
        } else {
            let end = rest.find([',', ' ']).unwrap_or(rest.len());
            value = rest[..end].to_owned();
            rest = &rest[end..];
        }
        match name.as_str() {
            "resource_metadata" => resource_metadata = Some(value),
            "scope" => scope = Some(value).filter(|scope| !scope.trim().is_empty()),
            _ => {}
        }
    }
    (resource_metadata, scope)
}

/// The protected resource metadata URLs of an MCP server without a challenge: path-inserted, then
/// the root.
pub(crate) fn protected_resource_urls(mcp_url: &Url) -> Vec<Url> {
    let mut root = mcp_url.clone();
    root.set_query(None);
    root.set_fragment(None);
    let path = mcp_url.path().trim_end_matches('/').to_owned();
    let mut urls = Vec::new();
    if !path.is_empty() {
        let mut inserted = root.clone();
        inserted.set_path(&format!("/.well-known/oauth-protected-resource{path}"));
        urls.push(inserted);
    }
    root.set_path("/.well-known/oauth-protected-resource");
    urls.push(root);
    urls
}

/// The metadata URLs of an authorization server: the spec's order, with the path-appended RFC 8414
/// URL second. Some servers (TikTok's) answer only at the path-appended URLs.
pub(crate) fn authorization_server_urls(issuer: &Url) -> Vec<Url> {
    let path = issuer.path().trim_end_matches('/').to_owned();
    let at = |path: String| {
        let mut url = issuer.clone();
        url.set_query(None);
        url.set_fragment(None);
        url.set_path(&path);
        url
    };
    if path.is_empty() {
        vec![
            at("/.well-known/oauth-authorization-server".to_owned()),
            at("/.well-known/openid-configuration".to_owned()),
        ]
    } else {
        vec![
            at(format!("/.well-known/oauth-authorization-server{path}")),
            at(format!("{path}/.well-known/oauth-authorization-server")),
            at(format!("/.well-known/openid-configuration{path}")),
            at(format!("{path}/.well-known/openid-configuration")),
        ]
    }
}

/// Whether the metadata's `resource` names this MCP server: same origin, and its path is the
/// server's path or a parent of it.
pub(crate) fn resource_matches(resource: &str, mcp_url: &Url) -> bool {
    let Ok(resource) = Url::parse(resource) else {
        return false;
    };
    let path = resource.path().trim_end_matches('/');
    resource.scheme() == mcp_url.scheme()
        && resource.host() == mcp_url.host()
        && resource.port_or_known_default() == mcp_url.port_or_known_default()
        && resource.fragment().is_none()
        && resource.query().is_none()
        && (path.is_empty()
            || mcp_url.path().trim_end_matches('/') == path
            || mcp_url.path().starts_with(&format!("{path}/")))
}

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// The authorization server metadata, checked: the exact `issuer`, https endpoints (plain HTTP
/// only for test hosts), PKCE with S256 (a MUST of the MCP spec), and the code flow. OIDC fields
/// (`jwks_uri`, `subject_types_supported`, …) are not needed: an OAuth-shaped
/// `openid-configuration` is fine.
pub(crate) fn checked_metadata(
    document: &Value,
    issuer: &str,
    test_hosts: &TestHosts,
) -> Result<Discovery, &'static str> {
    if document.get("issuer").and_then(Value::as_str) != Some(issuer) {
        return Err("The authorization server's metadata names another issuer.");
    }
    let endpoint = |name: &str| -> Result<Option<String>, &'static str> {
        match document.get(name).and_then(Value::as_str) {
            None => Ok(None),
            Some(raw) => check_url(raw, test_hosts)
                .map(|url| Some(url.to_string()))
                .map_err(|_| "An endpoint of the authorization server is not an https URL."),
        }
    };
    let authorization_endpoint = endpoint("authorization_endpoint")?
        .ok_or("The authorization server has no authorization endpoint.")?;
    let token_endpoint =
        endpoint("token_endpoint")?.ok_or("The authorization server has no token endpoint.")?;
    if !strings(&document["code_challenge_methods_supported"]).contains(&"S256".to_owned()) {
        return Err("The authorization server does not support PKCE with S256.");
    }
    let response_types = strings(&document["response_types_supported"]);
    if !response_types.is_empty() && !response_types.contains(&"code".to_owned()) {
        return Err("The authorization server does not offer the authorization code flow.");
    }
    Ok(Discovery {
        resource: String::new(),
        issuer: issuer.to_owned(),
        authorization_endpoint,
        token_endpoint,
        registration_endpoint: endpoint("registration_endpoint")?,
        scope: None,
        iss_required: document["authorization_response_iss_parameter_supported"] == json!(true),
        auth_methods: strings(&document["token_endpoint_auth_methods_supported"]),
    })
}

/// How Orbit authenticates at the token endpoint: as a public client when the server allows it
/// (no secret to keep), else with the secret it registers.
fn auth_method(supported: &[String]) -> &'static str {
    let offers = |method: &str| supported.iter().any(|item| item == method);
    if offers("none") {
        "none"
    } else if offers("client_secret_post") {
        "client_secret_post"
    } else {
        // RFC 8414: the default when the server names none.
        "client_secret_basic"
    }
}

fn random_token() -> String {
    let mut bytes = [0_u8; 32];
    OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

/// RFC 7636 S256.
pub(crate) fn code_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

fn state_hash(state: &str) -> String {
    super::integrations::hash(state)
}

/// The URL the admin's browser opens.
fn authorize_url(pending: &Pending, state: &str) -> Result<Url, &'static str> {
    let mut url = Url::parse(&pending.discovery.authorization_endpoint)
        .map_err(|_| "The authorization endpoint is not valid.")?;
    {
        let mut pairs = url.query_pairs_mut();
        pairs
            .append_pair("response_type", "code")
            .append_pair("client_id", &pending.client.client_id)
            .append_pair("redirect_uri", &pending.redirect_uri)
            .append_pair("state", state)
            .append_pair("code_challenge", &code_challenge(&pending.verifier))
            .append_pair("code_challenge_method", "S256")
            .append_pair("resource", &pending.discovery.resource);
        if let Some(scope) = &pending.discovery.scope {
            pairs.append_pair("scope", scope);
        }
    }
    Ok(url)
}

/// RFC 9207: a present `iss` must be the issuer, exactly; a missing one only when not promised.
pub(crate) fn iss_ok(given: Option<&str>, issuer: &str, required: bool) -> bool {
    match given {
        Some(given) => given == issuer,
        None => !required,
    }
}

/// Seconds as a whole number or a numeric string, within reason.
fn seconds(value: Option<&Value>) -> Option<i64> {
    let seconds = match value? {
        Value::Number(number) => number.as_i64()?,
        Value::String(text) => text.trim().parse().ok()?,
        _ => return None,
    };
    (1..=MAX_LIFETIME_SECONDS)
        .contains(&seconds)
        .then_some(seconds)
}

/// A name to show: one line, no control characters, at most 100 characters.
fn shown(text: &str) -> Option<String> {
    let clean: String = text
        .chars()
        .filter(|character| !character.is_control())
        .take(100)
        .collect();
    let clean = clean.trim();
    (!clean.is_empty()).then(|| clean.to_owned())
}

/// Who signed in: the ID token's name claims (only shown, so not verified), else the answer's
/// own `username`, `name` or `email`.
fn account_of(answer: &Value) -> Option<String> {
    let claims = answer
        .get("id_token")
        .and_then(Value::as_str)
        .and_then(|token| token.split('.').nth(1))
        .and_then(|payload| URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok())
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .unwrap_or(Value::Null);
    ["preferred_username", "name", "email"]
        .iter()
        .find_map(|claim| claims.get(*claim).and_then(Value::as_str).and_then(shown))
        .or_else(|| {
            ["username", "display_name", "name", "email"]
                .iter()
                .find_map(|field| answer.get(*field).and_then(Value::as_str).and_then(shown))
        })
}

/// A token answer: a bearer access token Orbit can send, and the lifetimes it gives.
pub(crate) fn parse_tokens(answer: &Value, now: i64) -> Result<Tokens, &'static str> {
    if answer
        .get("token_type")
        .and_then(Value::as_str)
        .is_some_and(|kind| !kind.eq_ignore_ascii_case("bearer"))
    {
        return Err("the token is not a bearer token");
    }
    let access_token = answer
        .get("access_token")
        .and_then(Value::as_str)
        .filter(|token| usable_token(token))
        .ok_or("the answer has no usable access_token")?;
    let refresh_token = answer
        .get("refresh_token")
        .and_then(Value::as_str)
        .filter(|token| !token.is_empty() && token.len() <= 8_192)
        .map(str::to_owned);
    Ok(Tokens {
        access_token: access_token.to_owned(),
        access_expires_at: seconds(answer.get("expires_in")).map(|seconds| now + seconds * 1_000),
        refresh_expires_at: seconds(answer.get("refresh_expires_in"))
            .filter(|_| refresh_token.is_some())
            .map(|seconds| now + seconds * 1_000),
        refresh_token,
        account: account_of(answer),
    })
}

// ---------------------------------------------------------------------------------------------
// Connect.

fn said(error: &OutboundError) -> String {
    super::integrations::capitalized(&error.to_string())
}

/// Reads the server's protected resource metadata and its authorization server's metadata.
pub(crate) async fn discover(outbound: &Outbound, mcp_url: &Url) -> Result<Discovery, String> {
    let test_hosts = outbound.test_hosts();
    let challenge = outbound
        .mcp_challenge(mcp_url)
        .await
        .map_err(|error| said(&error))?;
    let (metadata_url, challenge_scope) = challenge
        .as_deref()
        .map(challenge_params)
        .unwrap_or_default();
    let candidates = match metadata_url {
        Some(raw) => vec![check_url(&raw, test_hosts).map_err(|error| said(&error))?],
        None => protected_resource_urls(mcp_url),
    };
    let mut resource_metadata = None;
    for url in candidates {
        if let Ok((200, document)) = outbound.get_json(&url).await
            && document.is_object()
        {
            resource_metadata = Some(document);
            break;
        }
    }
    let resource_metadata = resource_metadata.ok_or(
        "The server publishes no OAuth metadata, so it does not use a sign-in Orbit knows. Use a fixed header credential.",
    )?;
    let resource = resource_metadata
        .get("resource")
        .and_then(Value::as_str)
        .filter(|resource| resource_matches(resource, mcp_url))
        .ok_or("The server's OAuth metadata names another resource.")?
        .to_owned();
    let issuer = resource_metadata
        .get("authorization_servers")
        .and_then(Value::as_array)
        .and_then(|servers| servers.first())
        .and_then(Value::as_str)
        .ok_or("The server's OAuth metadata names no authorization server.")?
        .to_owned();
    let issuer_url = check_url(&issuer, test_hosts).map_err(|error| said(&error))?;
    let mut discovery = None;
    for url in authorization_server_urls(&issuer_url) {
        if let Ok((200, document)) = outbound.get_json(&url).await
            && document.is_object()
        {
            discovery = Some(checked_metadata(&document, &issuer, test_hosts)?);
            break;
        }
    }
    let mut discovery =
        discovery.ok_or("The authorization server publishes no metadata Orbit can read.")?;
    discovery.resource = resource;
    discovery.scope = challenge_scope.or_else(|| {
        let supported = strings(&resource_metadata["scopes_supported"]);
        (!supported.is_empty()).then(|| supported.join(" "))
    });
    Ok(discovery)
}

/// Dynamic client registration (RFC 7591).
async fn register(
    outbound: &Outbound,
    discovery: &Discovery,
    redirect_uri: &str,
) -> Result<Client, String> {
    let endpoint = discovery.registration_endpoint.as_deref().ok_or(
        "The authorization server offers no dynamic client registration, which Orbit needs to connect.",
    )?;
    let endpoint = Url::parse(endpoint).map_err(|_| "The registration endpoint is not valid.")?;
    let method = auth_method(&discovery.auth_methods);
    let mut body = json!({
        "client_name": "Orbit",
        "redirect_uris": [redirect_uri],
        "grant_types": ["authorization_code", "refresh_token"],
        "response_types": ["code"],
        "token_endpoint_auth_method": method,
        "application_type": "web",
    });
    if let Some(scope) = &discovery.scope {
        body["scope"] = json!(scope);
    }
    let (status, answer) = outbound
        .post_json(&endpoint, &body)
        .await
        .map_err(|error| said(&error))?;
    if !(200..300).contains(&status) {
        return Err(format!(
            "The client registration failed: {}.",
            token_failure(status, &answer)
        ));
    }
    let client_id = answer
        .get("client_id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty() && id.len() <= 1_024)
        .ok_or("The client registration answered without a client ID.")?;
    let client_secret = answer
        .get("client_secret")
        .and_then(Value::as_str)
        .filter(|secret| !secret.is_empty() && secret.len() <= 4_096)
        .map(str::to_owned);
    let auth_method = answer
        .get("token_endpoint_auth_method")
        .and_then(Value::as_str)
        .filter(|method| {
            matches!(
                *method,
                "none" | "client_secret_post" | "client_secret_basic"
            )
        })
        .unwrap_or(method);
    Ok(Client {
        client_id: client_id.to_owned(),
        auth_method: if client_secret.is_some() {
            auth_method.to_owned()
        } else {
            "none".to_owned()
        },
        client_secret,
    })
}

/// Starts a Connect for `session`: discovery, registration, and a single-use `state`. Returns the
/// authorization URL for the admin's browser, or what went wrong (no secrets in it).
pub(crate) async fn start(
    state: &AgentState,
    (integration_id, mcp_url): (&str, &str),
    (session_id, user_id): (Id, Id),
) -> Result<Url, String> {
    let key = state
        .app_key
        .as_ref()
        .ok_or("The server has no ORBIT_APP_KEY, so it cannot keep a sign-in.")?;
    let origin = state
        .public_origin
        .as_deref()
        .ok_or("The server has no public URL to come back to.")?;
    let url = Url::parse(mcp_url).map_err(|_| "The URL is not valid.")?;
    let discovery = discover(&state.outbound, &url).await?;
    let redirect_uri = format!("{origin}{CALLBACK_PATH}");
    let client = register(&state.outbound, &discovery, &redirect_uri).await?;
    let pending = Pending {
        verifier: random_token(),
        discovery,
        client,
        mcp_url: mcp_url.to_owned(),
        redirect_uri,
    };
    let state_value = random_token();
    let authorize = authorize_url(&pending, &state_value)?;
    let sealed = serde_json::to_value(&pending)
        .ok()
        .and_then(|value| seal(key, &value).ok())
        .ok_or("The sign-in cannot be sealed.")?;
    let now = TimestampMillis::now().as_millis();
    let database = state.database();
    let stored = async {
        sqlx::query("DELETE FROM agent_integration_oauth_states WHERE expires_at <= $1")
            .bind(now)
            .execute(database.pool())
            .await?;
        sqlx::query(
            "INSERT INTO agent_integration_oauth_states \
             (state_hash, integration_id, session_id, user_id, sealed, created_at, expires_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(state_hash(&state_value))
        .bind(integration_id)
        .bind(session_id.to_string())
        .bind(user_id.to_string())
        .bind(sealed)
        .bind(now)
        .bind(now + STATE_LIFETIME_MS)
        .execute(database.pool())
        .await
    }
    .await;
    if let Err(error) = stored {
        tracing::warn!(%error, "agent integration connect");
        return Err("Try again in a moment.".to_owned());
    }
    Ok(authorize)
}

/// A started Connect that the callback took (and so used up).
pub(crate) struct Started {
    pub integration_id: String,
    pub session_id: String,
    pub user_id: String,
    pending: Pending,
}

impl Started {
    pub(crate) fn mcp_url(&self) -> &str {
        &self.pending.mcp_url
    }
}

/// Takes the Connect of `state` out of the database: it works once. `None` when it is unknown,
/// expired or cannot be opened.
pub(crate) async fn take(
    database: &Database,
    key: &[u8; 32],
    state: &str,
) -> Result<Option<Started>, sqlx::Error> {
    let row = sqlx::query(
        "DELETE FROM agent_integration_oauth_states WHERE state_hash = $1 \
         RETURNING integration_id, session_id, user_id, sealed, expires_at",
    )
    .bind(state_hash(state))
    .fetch_optional(database.pool())
    .await?;
    let Some(row) = row else { return Ok(None) };
    if row.get::<i64, _>("expires_at") <= TimestampMillis::now().as_millis() {
        return Ok(None);
    }
    let pending = open(key, &row.get::<Vec<u8>, _>("sealed"))
        .ok()
        .and_then(|value| serde_json::from_value(value).ok());
    Ok(pending.map(|pending| Started {
        integration_id: row.get("integration_id"),
        session_id: row.get("session_id"),
        user_id: row.get("user_id"),
        pending,
    }))
}

/// Exchanges the code for tokens. The error is fixed text, without anything from the answer.
pub(crate) async fn finish(
    outbound: &Outbound,
    started: &Started,
    code: &str,
    iss: Option<&str>,
) -> Result<(Connection, Tokens), String> {
    let pending = &started.pending;
    let discovery = &pending.discovery;
    if !iss_ok(iss, &discovery.issuer, discovery.iss_required) {
        return Err("The answer came from another authorization server.".to_owned());
    }
    let token_url =
        Url::parse(&discovery.token_endpoint).map_err(|_| "The token endpoint is not valid.")?;
    let mut form = vec![
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", pending.redirect_uri.as_str()),
        ("code_verifier", pending.verifier.as_str()),
        ("resource", discovery.resource.as_str()),
    ];
    let basic = pending.client.form(&mut form);
    let (status, answer) = outbound
        .token_request(&token_url, &form, basic)
        .await
        .map_err(|error| said(&error))?;
    if !(200..300).contains(&status) {
        return Err(format!(
            "The token request failed: {}.",
            token_failure(status, &answer)
        ));
    }
    let now = TimestampMillis::now().as_millis();
    let tokens = parse_tokens(&answer, now).map_err(|reason| {
        super::integrations::capitalized(&format!("the token request failed: {reason}"))
    })?;
    let connection = Connection {
        issuer: discovery.issuer.clone(),
        token_endpoint: discovery.token_endpoint.clone(),
        resource: discovery.resource.clone(),
        client: pending.client.clone(),
        access_token: tokens.access_token.clone(),
        access_expires_at: tokens.access_expires_at,
        refresh_token: tokens.refresh_token.clone(),
    };
    Ok((connection, tokens))
}

/// Saves a new connection, unless the integration's URL changed since the start. The header
/// credential goes: the token is the credential now.
pub(crate) async fn save_new(
    database: &Database,
    key: &[u8; 32],
    started: &Started,
    connection: &Connection,
    tokens: &Tokens,
) -> Result<bool, sqlx::Error> {
    let Ok(sealed) = serde_json::to_value(connection)
        .map_err(|_| ())
        .and_then(|value| seal(key, &value).map_err(|_| ()))
    else {
        return Ok(false);
    };
    let now = TimestampMillis::now().as_millis();
    let saved = sqlx::query(
        "UPDATE agent_integrations SET oauth_sealed_connection = $1, oauth_account = $2, \
         oauth_connected_at = $3, oauth_grant_expires_at = $4, oauth_invalid_at = NULL, \
         oauth_expiry_notice_at = NULL, sealed_secret = NULL, auth_header = NULL, \
         updated_at = CASE WHEN updated_at >= $3 THEN updated_at + 1 ELSE $3 END \
         WHERE id = $5 AND url = $6 AND kind = 'mcp'",
    )
    .bind(sealed)
    .bind(&tokens.account)
    .bind(now)
    .bind(tokens.grant_expires_at())
    .bind(&started.integration_id)
    .bind(started.mcp_url())
    .execute(database.pool())
    .await?;
    Ok(saved.rows_affected() == 1)
}

// ---------------------------------------------------------------------------------------------
// Calls.

/// One refresh at a time per integration on this server. Several servers are handled by saving
/// only over the connection that was read (see [`access`]).
#[derive(Clone, Default)]
pub(crate) struct RefreshLocks(Arc<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>>);

impl RefreshLocks {
    fn of(&self, integration_id: &str) -> Arc<tokio::sync::Mutex<()>> {
        let mut locks = self.0.lock().unwrap_or_else(|error| error.into_inner());
        locks.retain(|_, lock| Arc::strong_count(lock) > 1);
        Arc::clone(locks.entry(integration_id.to_owned()).or_default())
    }
}

/// The sealed connection and whether a refresh failed.
async fn load(
    database: &Database,
    integration_id: &str,
) -> Result<Option<(Vec<u8>, bool)>, sqlx::Error> {
    let row = sqlx::query(
        "SELECT oauth_sealed_connection, oauth_invalid_at FROM agent_integrations WHERE id = $1",
    )
    .bind(integration_id)
    .fetch_optional(database.pool())
    .await?;
    Ok(row.and_then(|row| {
        let sealed: Option<Vec<u8>> = row.get("oauth_sealed_connection");
        sealed.map(|sealed| {
            (
                sealed,
                row.get::<Option<i64>, _>("oauth_invalid_at").is_some(),
            )
        })
    }))
}

fn unavailable(error: &sqlx::Error) -> OutboundError {
    tracing::warn!(%error, "agent integration connection");
    OutboundError::Refused("Try again in a moment.")
}

/// The access token of an integration's connection: the saved one while it is fresh and not
/// `failed` (a token the server refused), else a renewed one.
pub(crate) async fn access(
    state: &AgentState,
    integration_id: &str,
    failed: Option<&AccessToken>,
) -> Result<AccessToken, OutboundError> {
    let key = state.app_key.as_ref().ok_or(OutboundError::Refused(
        "The server has no ORBIT_APP_KEY to open the credential.",
    ))?;
    let database = state.database();
    let usable = |sealed: &[u8]| -> Result<Option<AccessToken>, OutboundError> {
        let connection = opened(key, sealed)?;
        let token = AccessToken::new(connection.access_token.clone());
        Ok(
            (connection.fresh(TimestampMillis::now().as_millis()) && failed != Some(&token))
                .then_some(token),
        )
    };
    let (sealed, invalid) = load(database, integration_id)
        .await
        .map_err(|error| unavailable(&error))?
        .ok_or(OutboundError::Reconnect)?;
    if invalid {
        return Err(OutboundError::Reconnect);
    }
    if let Some(token) = usable(&sealed)? {
        return Ok(token);
    }
    let lock = state.connect_locks.of(integration_id);
    let _guard = lock.lock().await;
    // Another call may have renewed it while this one waited.
    let (sealed, invalid) = load(database, integration_id)
        .await
        .map_err(|error| unavailable(&error))?
        .ok_or(OutboundError::Reconnect)?;
    if invalid {
        return Err(OutboundError::Reconnect);
    }
    if let Some(token) = usable(&sealed)? {
        return Ok(token);
    }
    refresh(state, key, integration_id, &sealed).await
}

fn opened(key: &[u8; 32], sealed: &[u8]) -> Result<Connection, OutboundError> {
    open(key, sealed)
        .ok()
        .and_then(|value| serde_json::from_value(value).ok())
        .ok_or(OutboundError::Refused(
            "The saved sign-in cannot be opened. Click Reconnect.",
        ))
}

/// Renews the access token with the refresh token and saves it over `sealed` only. A refusal by
/// the server marks the connection "needs reconnect", unless another server saved a new one.
async fn refresh(
    state: &AgentState,
    key: &[u8; 32],
    integration_id: &str,
    sealed: &[u8],
) -> Result<AccessToken, OutboundError> {
    let database = state.database();
    let connection = opened(key, sealed)?;
    let Some(refresh_token) = connection.refresh_token.as_deref() else {
        return mark_invalid(state, key, integration_id, sealed).await;
    };
    let token_url = Url::parse(&connection.token_endpoint)
        .map_err(|_| OutboundError::Refused("The token endpoint is not valid."))?;
    let mut form = vec![
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
        ("resource", connection.resource.as_str()),
    ];
    let basic = connection.client.form(&mut form);
    let (status, answer) = state
        .outbound
        .token_request(&token_url, &form, basic)
        .await?;
    if matches!(status, 400 | 401) {
        return mark_invalid(state, key, integration_id, sealed).await;
    }
    if !(200..300).contains(&status) {
        return Err(OutboundError::Token(token_failure(status, &answer)));
    }
    let now = TimestampMillis::now().as_millis();
    let tokens =
        parse_tokens(&answer, now).map_err(|reason| OutboundError::Token(reason.to_owned()))?;
    let renewed = Connection {
        access_token: tokens.access_token.clone(),
        access_expires_at: tokens.access_expires_at,
        // Without a new refresh token (no rotation), the old one stays.
        refresh_token: tokens
            .refresh_token
            .clone()
            .or_else(|| connection.refresh_token.clone()),
        ..connection
    };
    let sealed_new = serde_json::to_value(&renewed)
        .ok()
        .and_then(|value| seal(key, &value).ok())
        .ok_or(OutboundError::Refused("The sign-in cannot be sealed."))?;
    sqlx::query(
        "UPDATE agent_integrations SET oauth_sealed_connection = $1, \
         oauth_grant_expires_at = COALESCE($2, oauth_grant_expires_at) \
         WHERE id = $3 AND oauth_sealed_connection = $4",
    )
    .bind(sealed_new)
    .bind(tokens.refresh_expires_at)
    .bind(integration_id)
    .bind(sealed)
    .execute(database.pool())
    .await
    .map_err(|error| unavailable(&error))?;
    Ok(AccessToken::new(renewed.access_token))
}

/// Marks the connection "needs reconnect" when it is still `sealed`. When another server saved a
/// new one meanwhile, its token is used instead.
async fn mark_invalid(
    state: &AgentState,
    key: &[u8; 32],
    integration_id: &str,
    sealed: &[u8],
) -> Result<AccessToken, OutboundError> {
    let database = state.database();
    let marked = sqlx::query(
        "UPDATE agent_integrations SET oauth_invalid_at = $1 \
         WHERE id = $2 AND oauth_sealed_connection = $3 AND oauth_invalid_at IS NULL",
    )
    .bind(TimestampMillis::now().as_millis())
    .bind(integration_id)
    .bind(sealed)
    .execute(database.pool())
    .await
    .map_err(|error| unavailable(&error))?;
    if marked.rows_affected() == 0
        && let Some((current, false)) = load(database, integration_id)
            .await
            .map_err(|error| unavailable(&error))?
        && current != sealed
    {
        return Ok(AccessToken::new(opened(key, &current)?.access_token));
    }
    tracing::info!(
        integration_id,
        "agent integration sign-in needs a reconnect"
    );
    Err(OutboundError::Reconnect)
}

// ---------------------------------------------------------------------------------------------
// Notices.

/// Emails the owners and admins of each workspace once when a connection's grant ends within 14
/// days, or a refresh was refused. Without a mail server, only the admin page warns.
pub(crate) async fn send_notices(database: &Database, mailer: &Mailer) -> Result<(), sqlx::Error> {
    if !mailer.enabled().await? {
        return Ok(());
    }
    let now = TimestampMillis::now().as_millis();
    let due = sqlx::query(
        "SELECT i.id, i.workspace_id, i.name, i.oauth_grant_expires_at, i.oauth_invalid_at, \
         w.name AS workspace FROM agent_integrations i JOIN workspaces w ON w.id = i.workspace_id \
         WHERE i.oauth_sealed_connection IS NOT NULL AND i.oauth_expiry_notice_at IS NULL \
         AND (i.oauth_invalid_at IS NOT NULL OR i.oauth_grant_expires_at <= $1)",
    )
    .bind(now + EXPIRY_WARNING_MS)
    .fetch_all(database.pool())
    .await?;
    for row in due {
        let id: String = row.get("id");
        // One server sends it: the one whose claim changes the row.
        let claimed = sqlx::query(
            "UPDATE agent_integrations SET oauth_expiry_notice_at = $1 \
             WHERE id = $2 AND oauth_expiry_notice_at IS NULL",
        )
        .bind(now)
        .bind(&id)
        .execute(database.pool())
        .await?;
        if claimed.rows_affected() == 0 {
            continue;
        }
        let workspace_id: String = row.get("workspace_id");
        let admins = sqlx::query_scalar::<_, String>(
            "SELECT u.email FROM memberships m JOIN users u ON u.id = m.user_id \
             WHERE m.workspace_id = $1 AND m.role IN ('owner', 'admin') AND u.suspended_at IS NULL",
        )
        .bind(&workspace_id)
        .fetch_all(database.pool())
        .await?;
        let tool: String = row.get("name");
        let workspace: String = row.get("workspace");
        let days_left = row
            .get::<Option<i64>, _>("oauth_grant_expires_at")
            .filter(|_| row.get::<Option<i64>, _>("oauth_invalid_at").is_none())
            .map(|expires_at| ((expires_at - now).max(0) + 86_399_999) / 86_400_000);
        let link = mailer.link("/settings/agentic/workspace");
        for email in admins {
            mailer.send_later(templates::agent_tool_sign_in(
                &email, &workspace, &tool, days_left, &link,
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use url::Url;

    use super::{
        Client, Connection, Discovery, Pending, TestHosts, authorization_server_urls,
        authorize_url, challenge_params, checked_metadata, code_challenge, iss_ok, parse_tokens,
        protected_resource_urls, resource_matches,
    };

    const TIKTOK: &str = "https://business-api.tiktok.com/open_mcp/tt-ads-mcp-organic-social";

    /// What TikTok's server published on 2026-11-10.
    fn tiktok_metadata() -> serde_json::Value {
        let base = format!("{TIKTOK}/oauth");
        json!({
            "authorization_endpoint": format!("{base}/tiktok/authorize"),
            "code_challenge_methods_supported": ["S256"],
            "grant_types_supported": ["authorization_code", "refresh_token"],
            "issuer": base,
            "registration_endpoint": format!("{base}/tiktok/register"),
            "response_types_supported": ["code"],
            "scopes_supported": ["user.info.basic", "video.list"],
            "token_endpoint": format!("{base}/tiktok/token"),
            "token_endpoint_auth_methods_supported": ["none"],
        })
    }

    #[test]
    fn a_bearer_challenge_names_the_metadata_and_the_scope() {
        let (metadata, scope) = challenge_params(
            "Bearer resource_metadata=\"https://business-api.tiktok.com/.well-known/oauth-protected-resource/open_mcp/x\"",
        );
        assert_eq!(
            metadata.as_deref(),
            Some("https://business-api.tiktok.com/.well-known/oauth-protected-resource/open_mcp/x")
        );
        assert_eq!(scope, None);
        let (metadata, scope) = challenge_params(
            "bearer error=\"invalid_token\", scope=\"files:read files:write\",resource_metadata=https://a.example/m, realm=\"a \\\"b\\\"\"",
        );
        assert_eq!(metadata.as_deref(), Some("https://a.example/m"));
        assert_eq!(scope.as_deref(), Some("files:read files:write"));
        assert_eq!(challenge_params("Bearer"), (None, None));
        assert_eq!(challenge_params("Bearer scope=\"\""), (None, None));
    }

    #[test]
    fn metadata_urls_follow_the_specs_order() {
        let mcp = Url::parse(&format!("{TIKTOK}?x=1")).unwrap();
        let urls: Vec<String> = protected_resource_urls(&mcp)
            .iter()
            .map(Url::to_string)
            .collect();
        assert_eq!(
            urls,
            [
                "https://business-api.tiktok.com/.well-known/oauth-protected-resource/open_mcp/tt-ads-mcp-organic-social",
                "https://business-api.tiktok.com/.well-known/oauth-protected-resource",
            ]
        );
        let issuer = Url::parse(&format!("{TIKTOK}/oauth")).unwrap();
        let urls: Vec<String> = authorization_server_urls(&issuer)
            .iter()
            .map(Url::to_string)
            .collect();
        assert_eq!(
            urls,
            [
                "https://business-api.tiktok.com/.well-known/oauth-authorization-server/open_mcp/tt-ads-mcp-organic-social/oauth",
                "https://business-api.tiktok.com/open_mcp/tt-ads-mcp-organic-social/oauth/.well-known/oauth-authorization-server",
                "https://business-api.tiktok.com/.well-known/openid-configuration/open_mcp/tt-ads-mcp-organic-social/oauth",
                "https://business-api.tiktok.com/open_mcp/tt-ads-mcp-organic-social/oauth/.well-known/openid-configuration",
            ]
        );
        let urls: Vec<String> =
            authorization_server_urls(&Url::parse("https://auth.example.com/").unwrap())
                .iter()
                .map(Url::to_string)
                .collect();
        assert_eq!(
            urls,
            [
                "https://auth.example.com/.well-known/oauth-authorization-server",
                "https://auth.example.com/.well-known/openid-configuration",
            ]
        );
    }

    #[test]
    fn the_resource_must_name_this_server() {
        let mcp = Url::parse(TIKTOK).unwrap();
        assert!(resource_matches(TIKTOK, &mcp));
        assert!(resource_matches("https://business-api.tiktok.com", &mcp));
        assert!(resource_matches(
            "https://business-api.tiktok.com/open_mcp/",
            &mcp
        ));
        for other in [
            "https://evil.example.com/open_mcp/tt-ads-mcp-organic-social",
            "http://business-api.tiktok.com/open_mcp/tt-ads-mcp-organic-social",
            "https://business-api.tiktok.com:8443/open_mcp/tt-ads-mcp-organic-social",
            "https://business-api.tiktok.com/open_mcp/tt-ads",
            "https://business-api.tiktok.com/other",
            "https://business-api.tiktok.com/open_mcp/tt-ads-mcp-organic-social?x=1",
            "not a url",
        ] {
            assert!(!resource_matches(other, &mcp), "{other}");
        }
    }

    #[test]
    fn authorization_server_metadata_is_checked() {
        let none = TestHosts::default();
        let issuer = format!("{TIKTOK}/oauth");
        let discovery = checked_metadata(&tiktok_metadata(), &issuer, &none).unwrap();
        assert_eq!(discovery.token_endpoint, format!("{issuer}/tiktok/token"));
        assert_eq!(
            discovery.registration_endpoint.as_deref(),
            Some(format!("{issuer}/tiktok/register").as_str())
        );
        assert_eq!(discovery.auth_methods, ["none"]);
        assert!(!discovery.iss_required);

        let mut other_issuer = tiktok_metadata();
        other_issuer["issuer"] = json!(format!("{issuer}/"));
        let mut no_pkce = tiktok_metadata();
        no_pkce
            .as_object_mut()
            .unwrap()
            .remove("code_challenge_methods_supported");
        let mut plain_only = tiktok_metadata();
        plain_only["code_challenge_methods_supported"] = json!(["plain"]);
        let mut http_token = tiktok_metadata();
        http_token["token_endpoint"] = json!("http://business-api.tiktok.com/token");
        let mut local_token = tiktok_metadata();
        local_token["token_endpoint"] = json!("https://127.0.0.1/token");
        let mut no_code = tiktok_metadata();
        no_code["response_types_supported"] = json!(["token"]);
        let mut no_authorize = tiktok_metadata();
        no_authorize
            .as_object_mut()
            .unwrap()
            .remove("authorization_endpoint");
        for bad in [
            other_issuer,
            no_pkce,
            plain_only,
            http_token,
            local_token,
            no_code,
            no_authorize,
        ] {
            assert!(checked_metadata(&bad, &issuer, &none).is_err(), "{bad}");
        }

        let hosts = TestHosts::new(["127.0.0.1"]);
        let local = json!({
            "issuer": "http://127.0.0.1:4000",
            "authorization_endpoint": "http://127.0.0.1:4000/authorize",
            "token_endpoint": "http://127.0.0.1:4000/token",
            "code_challenge_methods_supported": ["S256"],
            "authorization_response_iss_parameter_supported": true,
        });
        assert!(
            checked_metadata(&local, "http://127.0.0.1:4000", &hosts)
                .unwrap()
                .iss_required
        );
    }

    #[test]
    fn pkce_uses_s256_and_the_authorize_url_carries_state_resource_and_scope() {
        // RFC 7636, appendix B.
        assert_eq!(
            code_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
        let pending = Pending {
            verifier: "verifier-value".to_owned(),
            discovery: Discovery {
                resource: TIKTOK.to_owned(),
                issuer: format!("{TIKTOK}/oauth"),
                authorization_endpoint: format!("{TIKTOK}/oauth/tiktok/authorize?a=1"),
                token_endpoint: format!("{TIKTOK}/oauth/tiktok/token"),
                registration_endpoint: None,
                scope: Some("user.info.basic video.list".to_owned()),
                iss_required: false,
                auth_methods: vec![],
            },
            client: Client {
                client_id: "client-1".to_owned(),
                client_secret: None,
                auth_method: "none".to_owned(),
            },
            mcp_url: TIKTOK.to_owned(),
            redirect_uri: "https://orbit.example.com/api/v1/agent-integrations/oauth/callback"
                .to_owned(),
        };
        let url = authorize_url(&pending, "state-1").unwrap();
        let pairs: std::collections::HashMap<String, String> =
            url.query_pairs().into_owned().collect();
        assert_eq!(pairs["a"], "1");
        assert_eq!(pairs["response_type"], "code");
        assert_eq!(pairs["state"], "state-1");
        assert_eq!(pairs["code_challenge_method"], "S256");
        assert_eq!(pairs["code_challenge"], code_challenge("verifier-value"));
        assert_eq!(pairs["resource"], TIKTOK);
        assert_eq!(pairs["scope"], "user.info.basic video.list");
        assert_eq!(pairs["redirect_uri"], pending.redirect_uri);
        assert!(!url.as_str().contains("verifier-value"));
    }

    #[test]
    fn iss_must_match_exactly_and_be_there_when_promised() {
        assert!(iss_ok(Some("https://a.example"), "https://a.example", true));
        assert!(iss_ok(None, "https://a.example", false));
        assert!(!iss_ok(None, "https://a.example", true));
        assert!(!iss_ok(
            Some("https://a.example/"),
            "https://a.example",
            false
        ));
        assert!(!iss_ok(
            Some("https://A.example"),
            "https://a.example",
            false
        ));
    }

    #[test]
    fn token_answers_give_the_lifetimes_and_who_signed_in() {
        let now = 1_000_000;
        let tokens = parse_tokens(
            &json!({
                "access_token": "a", "token_type": "Bearer", "expires_in": 86_400,
                "refresh_token": "r", "refresh_expires_in": "31536000", "username": "brand\u{7}_account",
            }),
            now,
        )
        .unwrap();
        assert_eq!(tokens.access_expires_at, Some(now + 86_400_000));
        assert_eq!(tokens.grant_expires_at(), Some(now + 31_536_000_000));
        assert_eq!(tokens.account.as_deref(), Some("brand_account"));

        // No refresh token: the grant ends with the access token. No lifetimes: unknown.
        let tokens = parse_tokens(&json!({ "access_token": "a", "expires_in": 60 }), now).unwrap();
        assert_eq!(tokens.grant_expires_at(), Some(now + 60_000));
        let tokens = parse_tokens(
            &json!({ "access_token": "a", "refresh_token": "r", "expires_in": -5 }),
            now,
        )
        .unwrap();
        assert_eq!(
            (tokens.access_expires_at, tokens.grant_expires_at()),
            (None, None)
        );

        // The ID token's claims name the account.
        let claims = base64::Engine::encode(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD,
            br#"{"email":"brand@example.com"}"#,
        );
        let tokens = parse_tokens(
            &json!({ "access_token": "a", "id_token": format!("x.{claims}.y"), "username": "other" }),
            now,
        )
        .unwrap();
        assert_eq!(tokens.account.as_deref(), Some("brand@example.com"));

        for bad in [
            json!({}),
            json!({ "access_token": "" }),
            json!({ "access_token": "a\nb" }),
            json!({ "access_token": "a", "token_type": "mac" }),
        ] {
            assert!(parse_tokens(&bad, now).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_connection_renews_a_minute_before_it_expires() {
        let connection = |expires_at| Connection {
            issuer: String::new(),
            token_endpoint: String::new(),
            resource: String::new(),
            client: Client {
                client_id: String::new(),
                client_secret: None,
                auth_method: "none".to_owned(),
            },
            access_token: "a".to_owned(),
            access_expires_at: expires_at,
            refresh_token: None,
        };
        assert!(connection(None).fresh(0));
        assert!(connection(Some(60_001)).fresh(0));
        assert!(!connection(Some(60_000)).fresh(0));
        assert!(!connection(Some(10)).fresh(0));
    }

    #[test]
    fn debug_output_redacts_the_connection_and_the_client() {
        let client = Client {
            client_id: "client-value".to_owned(),
            client_secret: Some("secret-value".to_owned()),
            auth_method: "client_secret_post".to_owned(),
        };
        let connection = Connection {
            issuer: "issuer".to_owned(),
            token_endpoint: String::new(),
            resource: String::new(),
            client: client.clone(),
            access_token: "access-value".to_owned(),
            access_expires_at: None,
            refresh_token: Some("refresh-value".to_owned()),
        };
        let tokens = parse_tokens(&json!({ "access_token": "access-value" }), 0).unwrap();
        let text = format!("{client:?} {connection:?} {tokens:?}");
        assert!(!text.contains("value"), "{text}");
    }

    /// A local server with TikTok's shapes: the MCP path answers 401 with `resource_metadata`, and
    /// the authorization server's metadata answers only at the paths in `serves`. Returns its base
    /// URL and the paths that were asked for.
    async fn tiktok_like(
        serves: &'static [&'static str],
    ) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        use axum::http::{StatusCode, Uri, header};
        use axum::response::IntoResponse;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let asked = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let (seen, origin) = (std::sync::Arc::clone(&asked), base.clone());
        let router = axum::Router::new().fallback(move |uri: Uri| {
            let (seen, base) = (std::sync::Arc::clone(&seen), origin.clone());
            async move {
                let path = uri.path().to_owned();
                seen.lock().unwrap().push(path.clone());
                let mcp = format!("{base}/open_mcp/social");
                if path == "/open_mcp/social" {
                    let metadata = format!(
                        "Bearer resource_metadata=\"{base}/.well-known/oauth-protected-resource/open_mcp/social\""
                    );
                    return (StatusCode::UNAUTHORIZED, [(header::WWW_AUTHENTICATE, metadata)], "unauthorized")
                        .into_response();
                }
                if path == "/.well-known/oauth-protected-resource/open_mcp/social" {
                    return axum::Json(json!({
                        "authorization_servers": [format!("{mcp}/oauth")],
                        "bearer_methods_supported": ["header"],
                        "resource": mcp,
                        "scopes_supported": ["user.info.basic", "video.list"],
                    }))
                    .into_response();
                }
                if serves.contains(&path.as_str()) {
                    let issuer = format!("{mcp}/oauth");
                    // TikTok's answer, OAuth-shaped also at openid-configuration: no jwks_uri,
                    // subject_types_supported or id_token_signing_alg_values_supported.
                    return axum::Json(json!({
                        "authorization_endpoint": format!("{issuer}/tiktok/authorize"),
                        "code_challenge_methods_supported": ["S256"],
                        "grant_types_supported": ["authorization_code", "refresh_token"],
                        "issuer": issuer,
                        "registration_endpoint": format!("{issuer}/tiktok/register"),
                        "response_types_supported": ["code"],
                        "revocation_endpoint": format!("{issuer}/tiktok/revoke"),
                        "token_endpoint": format!("{issuer}/tiktok/token"),
                        "token_endpoint_auth_methods_supported": ["none"],
                    }))
                    .into_response();
                }
                (StatusCode::NOT_FOUND, "<html>404 Not Found</html>").into_response()
            }
        });
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        (base, asked)
    }

    fn metadata_paths(asked: &std::sync::Mutex<Vec<String>>) -> Vec<String> {
        asked
            .lock()
            .unwrap()
            .iter()
            .filter(|path| {
                path.contains("oauth-authorization-server") || path.contains("openid-configuration")
            })
            .cloned()
            .collect()
    }

    #[tokio::test]
    async fn discovery_finds_tiktoks_metadata_at_the_path_appended_url() {
        let outbound = super::Outbound::new(TestHosts::new(["127.0.0.1"]));
        let (base, asked) =
            tiktok_like(&["/open_mcp/social/oauth/.well-known/oauth-authorization-server"]).await;
        let mcp = Url::parse(&format!("{base}/open_mcp/social")).unwrap();
        let discovery = super::discover(&outbound, &mcp).await.unwrap();
        assert_eq!(discovery.issuer, format!("{base}/open_mcp/social/oauth"));
        assert_eq!(discovery.resource, mcp.as_str());
        assert_eq!(
            discovery.token_endpoint,
            format!("{base}/open_mcp/social/oauth/tiktok/token")
        );
        assert_eq!(
            discovery.scope.as_deref(),
            Some("user.info.basic video.list")
        );
        // The inserted path answered 404, the appended one 200, and nothing more was asked.
        assert_eq!(
            metadata_paths(&asked),
            [
                "/.well-known/oauth-authorization-server/open_mcp/social/oauth",
                "/open_mcp/social/oauth/.well-known/oauth-authorization-server",
            ]
        );
    }

    #[tokio::test]
    async fn discovery_accepts_an_oauth_shaped_openid_configuration() {
        let outbound = super::Outbound::new(TestHosts::new(["127.0.0.1"]));
        let (base, asked) =
            tiktok_like(&["/open_mcp/social/oauth/.well-known/openid-configuration"]).await;
        let mcp = Url::parse(&format!("{base}/open_mcp/social")).unwrap();
        let discovery = super::discover(&outbound, &mcp).await.unwrap();
        assert_eq!(
            discovery.registration_endpoint.as_deref(),
            Some(format!("{base}/open_mcp/social/oauth/tiktok/register").as_str())
        );
        assert_eq!(metadata_paths(&asked).len(), 4);

        // Nothing answers: a clear error, no panic.
        let (base, _) = tiktok_like(&[]).await;
        let mcp = Url::parse(&format!("{base}/open_mcp/social")).unwrap();
        assert!(super::discover(&outbound, &mcp).await.is_err());
    }

    #[test]
    fn only_issuer_and_the_two_endpoints_are_required_besides_pkce() {
        let none = TestHosts::default();
        let minimal = json!({
            "issuer": "https://auth.example.com",
            "authorization_endpoint": "https://auth.example.com/authorize",
            "token_endpoint": "https://auth.example.com/token",
            "code_challenge_methods_supported": ["S256"],
        });
        let discovery = checked_metadata(&minimal, "https://auth.example.com", &none).unwrap();
        assert_eq!(discovery.registration_endpoint, None);
        assert!(discovery.auth_methods.is_empty());
    }
}
