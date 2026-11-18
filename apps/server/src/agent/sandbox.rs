//! Agent sandboxes, Orbit's side (docs/agent-sandbox-design.md, sections 3-5). A custom agent with the
//! `sandbox_shell` tool runs code in a gVisor sandbox per chat thread and driver (`deploy/agent-sandbox/broker.py`).
//! Orbit stays the judge:
//!
//! - `POST /api/internal/agent/sandbox/calls` (sidecar secret): checks the driver, the agent, its settings, the
//!   running thread and the monthly budget, then issues a **call token** (an agent session bound to this call, alive
//!   for the call's timeout + 60 s) and a **ticket** that the broker checks: an HMAC under the broker key over the
//!   call, agent, work id, limits and expiry. The call token and the call's proxy password are sealed inside, so the
//!   sidecar never sees them; Orbit keeps only their hashes.
//! - `POST /api/internal/agent/sandbox/calls/{id}/end` (broker): ends the call once (idempotent), deletes its token,
//!   adds its CPU time to the month, and returns the egress requests that wait for a person's answer.
//! - `POST /api/internal/agent/egress/check` (egress proxy): with the call's proxy password, decides one
//!   `host:port`: `allow`, `deny` or `ask` (section 4, decision order), and logs it.
//! - `POST /api/internal/agent/egress/done` (egress proxy): an allowed connection's bytes, once, into its log row, the
//!   call and the month; or `blocked` when the proxy refused it after the yes (`agent::egress_proxy`).
//!
//! The broker and the proxy authenticate with a bearer derived from the broker key; the key file is
//! `ORBIT_AGENT_SANDBOX_KEY_FILE`. Without it, or with `ORBIT_AGENT_SANDBOX=off`, every route answers `sandbox_off`.

use axum::extract::{Extension, Path, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, Mac};
use orbit_platform::{Flag, Id, RequestId, Row, TimestampMillis, generate_opaque_token};
use rand_core::{OsRng, RngCore};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use super::AgentState;
use super::routes::sidecar_only;
use super::scope::TOKEN_PREFIX;
use crate::task_routes::ApiError;

type RequestIdExtension = Option<Extension<RequestId>>;
type HmacSha256 = Hmac<Sha256>;

/// The tool that lets an agent use its sandbox (ticked per agent; its catalog row comes with the sidecar tool).
pub const TOOL: &str = "sandbox_shell";
/// A ticket must reach the broker within this time.
const TICKET_LIFETIME_MS: i64 = 5 * 60 * 1_000;
/// Per-call limits of the pilot (section 6). The broker caps them again.
const MAX_TIMEOUT_SECONDS: i64 = 15 * 60;
const MEMORY_MB: i64 = 1024;
const DISK_MB: i64 = 4096;
/// A call without an end after its timeout and this long is closed as `lost`.
const LOST_AFTER_MS: i64 = 5 * 60 * 1_000;
/// An "Allow once" lasts this long (as write approvals).
pub const ALLOW_ONCE_MS: i64 = 30 * 60 * 1_000;

/// Hosts that always ask, also with an "Allow always" rule: paste and file-drop sites, shorteners, request catchers
/// and tunnels, personal and raw object storage, gist and upload endpoints. Each also covers its subdomains.
const RISKY: &[&str] = &[
    "webhook.site",
    "requestbin.com",
    "requestbin.net",
    "pipedream.net",
    "ngrok.io",
    "ngrok.app",
    "ngrok-free.app",
    "ngrok.dev",
    "trycloudflare.com",
    "localtunnel.me",
    "pastebin.com",
    "paste.ee",
    "hastebin.com",
    "transfer.sh",
    "file.io",
    "0x0.st",
    "bit.ly",
    "tinyurl.com",
    "is.gd",
    "t.co",
    "dropbox.com",
    "dropboxusercontent.com",
    "drive.google.com",
    "wetransfer.com",
    "we.tl",
    "s3.amazonaws.com",
    "storage.googleapis.com",
    "blob.core.windows.net",
    "gist.github.com",
    "gist.githubusercontent.com",
    "uploads.github.com",
];

/// The broker key, and the keys derived from it (one per use).
#[derive(Clone)]
pub struct SandboxKeys {
    seal: [u8; 32],
    ticket: [u8; 32],
    service: String,
}

impl std::fmt::Debug for SandboxKeys {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SandboxKeys([REDACTED])")
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn hmac(key: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC takes any key length");
    for part in parts {
        mac.update(part);
    }
    mac.finalize().into_bytes().into()
}

impl SandboxKeys {
    /// From the contents of the broker key file (at least 32 characters, surrounding white space ignored).
    pub fn new(key: &[u8]) -> Result<Self, String> {
        let key = key.trim_ascii();
        if key.len() < 32 {
            return Err("the agent sandbox key must have at least 32 characters".to_owned());
        }
        Ok(Self {
            seal: hmac(key, &[b"orbit-sandbox/seal"]),
            ticket: hmac(key, &[b"orbit-sandbox/ticket"]),
            service: hex(&hmac(key, &[b"orbit-sandbox/service"])),
        })
    }

    /// `ORBIT_AGENT_SANDBOX_KEY_FILE`; `None` when it is not set or `ORBIT_AGENT_SANDBOX=off` (the installation's
    /// off switch).
    pub fn from_env() -> Result<Option<Self>, String> {
        if std::env::var("ORBIT_AGENT_SANDBOX").is_ok_and(|value| value.trim() == "off") {
            return Ok(None);
        }
        let Some(path) = std::env::var("ORBIT_AGENT_SANDBOX_KEY_FILE")
            .ok()
            .filter(|path| !path.trim().is_empty())
        else {
            return Ok(None);
        };
        let key = std::fs::read(path.trim())
            .map_err(|error| format!("ORBIT_AGENT_SANDBOX_KEY_FILE {path}: {error}"))?;
        Self::new(&key).map(Some)
    }

    /// Encrypt-then-MAC with HMAC-SHA256 only (the broker is Python without crypto packages): an HMAC counter
    /// keystream under the seal key; the ticket MAC covers the result.
    fn seal(&self, plain: &[u8]) -> String {
        let mut nonce = [0_u8; 16];
        OsRng.fill_bytes(&mut nonce);
        let mut sealed = nonce.to_vec();
        for (index, chunk) in plain.chunks(32).enumerate() {
            let counter = u32::try_from(index).unwrap_or(u32::MAX).to_be_bytes();
            let block = hmac(&self.seal, &[&nonce, &counter]);
            sealed.extend(chunk.iter().zip(block).map(|(a, b)| a ^ b));
        }
        URL_SAFE_NO_PAD.encode(sealed)
    }

    fn sign(&self, claims: &Value) -> String {
        let payload = URL_SAFE_NO_PAD.encode(claims.to_string());
        let mac = hmac(&self.ticket, &[b"v1.", payload.as_bytes()]);
        format!("v1.{payload}.{}", URL_SAFE_NO_PAD.encode(mac))
    }

    fn service_allows(&self, headers: &HeaderMap) -> bool {
        let given = orbit_platform::bearer_token(headers).unwrap_or_default();
        given.as_bytes().ct_eq(self.service.as_bytes()).into()
    }
}

pub(crate) fn router(state: AgentState) -> Router {
    Router::new()
        .route("/api/internal/agent/sandbox/calls", post(issue_call))
        .route(
            "/api/internal/agent/sandbox/calls/{call_id}/end",
            post(end_call),
        )
        .route("/api/internal/agent/egress/check", post(check_egress))
        .route("/api/internal/agent/egress/done", post(egress_done))
        .with_state(state)
}

fn error(
    status: StatusCode,
    code: &'static str,
    detail: &'static str,
    uri: &Uri,
    request_id: &RequestIdExtension,
) -> ApiError {
    let title = status.canonical_reason().unwrap_or("Error");
    ApiError::new(status, code, title, detail, uri.path(), request_id.as_ref())
}

fn unavailable(failure: &sqlx::Error, uri: &Uri, request_id: &RequestIdExtension) -> ApiError {
    tracing::warn!(error = %failure, "agent sandbox");
    error(
        StatusCode::SERVICE_UNAVAILABLE,
        "unavailable",
        "Try again in a moment.",
        uri,
        request_id,
    )
}

fn keys<'a>(
    state: &'a AgentState,
    uri: &Uri,
    request_id: &RequestIdExtension,
) -> Result<&'a SandboxKeys, ApiError> {
    state.sandbox.as_ref().ok_or_else(|| {
        error(
            StatusCode::NOT_FOUND,
            "sandbox_off",
            "Agent sandboxes are off on this server.",
            uri,
            request_id,
        )
    })
}

/// The broker and the egress proxy: the service bearer derived from the broker key.
fn service_only<'a>(
    state: &'a AgentState,
    headers: &HeaderMap,
    uri: &Uri,
    request_id: &RequestIdExtension,
) -> Result<&'a SandboxKeys, ApiError> {
    let keys = keys(state, uri, request_id)?;
    if keys.service_allows(headers) {
        Ok(keys)
    } else {
        Err(error(
            StatusCode::UNAUTHORIZED,
            "authentication_required",
            "The sandbox service key is missing or wrong.",
            uri,
            request_id,
        ))
    }
}

fn month(now: i64) -> String {
    jiff::Timestamp::from_millisecond(now)
        .map(|time| time.strftime("%Y-%m").to_string())
        .unwrap_or_default()
}

/// The thread sandbox of one agent, thread and driver: nothing is shared between them.
fn work_id(agent: Id, thread_key: &str, driver: Id) -> String {
    let digest = Sha256::digest(format!("{agent}:{thread_key}:{driver}"));
    hex(&digest[..16])
}

/// Calls without an end long after their timeout become `lost`; their tokens go.
pub async fn close_lost_calls(conn: &mut sqlx::AnyConnection, now: i64) -> Result<(), sqlx::Error> {
    sqlx::query(
        "DELETE FROM sessions WHERE id IN (SELECT session_id FROM agent_sandbox_calls \
         WHERE status = 'running' AND started_at + timeout_seconds * 1000 + $1 < $2)",
    )
    .bind(LOST_AFTER_MS)
    .bind(now)
    .execute(&mut *conn)
    .await?;
    sqlx::query(
        "UPDATE agent_sandbox_calls SET status = 'lost', ended_at = $2, session_id = NULL \
         WHERE status = 'running' AND started_at + timeout_seconds * 1000 + $1 < $2",
    )
    .bind(LOST_AFTER_MS)
    .bind(now)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Ticket and call token.

#[derive(Deserialize)]
struct CallBody {
    workspace_id: Id,
    /// The driver: the person the thread's run acts for.
    user_id: Id,
    agent_user_id: Id,
    thread_key: String,
    #[serde(default)]
    script_sha256: Option<String>,
    #[serde(default)]
    timeout_seconds: Option<i64>,
}

enum Refused {
    Off,
    NotAllowed,
    Budget,
}

async fn issue_call(
    State(state): State<AgentState>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
    Json(body): Json<CallBody>,
) -> Result<Response, ApiError> {
    sidecar_only(&state, &headers, &uri, &request_id)?;
    let keys = keys(&state, &uri, &request_id)?;
    let timeout = body
        .timeout_seconds
        .unwrap_or(120)
        .clamp(1, MAX_TIMEOUT_SECONDS);
    let script_sha256 = body
        .script_sha256
        .filter(|hash| hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()));
    match issue(
        &state,
        keys,
        &body.workspace_id,
        body.user_id,
        body.agent_user_id,
        &body.thread_key,
        timeout,
        script_sha256,
    )
    .await
    {
        Ok(Ok(answer)) => Ok(Json(answer).into_response()),
        Ok(Err(Refused::Off)) => Err(error(
            StatusCode::FORBIDDEN,
            "sandbox_off",
            "The sandbox is off for this agent.",
            &uri,
            &request_id,
        )),
        Ok(Err(Refused::Budget)) => Err(error(
            StatusCode::FORBIDDEN,
            "sandbox_budget_spent",
            "This agent used its sandbox time for this month.",
            &uri,
            &request_id,
        )),
        Ok(Err(Refused::NotAllowed)) => Err(error(
            StatusCode::FORBIDDEN,
            "sandbox_refused",
            "The person cannot use this agent's sandbox in this thread now.",
            &uri,
            &request_id,
        )),
        Err(failure) => Err(unavailable(&failure, &uri, &request_id)),
    }
}

#[allow(clippy::too_many_arguments)]
async fn issue(
    state: &AgentState,
    keys: &SandboxKeys,
    workspace_id: &Id,
    driver: Id,
    agent: Id,
    thread_key: &str,
    timeout: i64,
    script_sha256: Option<String>,
) -> Result<Result<Value, Refused>, sqlx::Error> {
    let now = TimestampMillis::now().as_millis();
    let mut tx = state.database().immediate_transaction().await?;
    close_lost_calls(&mut tx, now).await?;
    // The driver: an active member (not a guest) of a live workspace.
    let member = sqlx::query(
        "SELECT 1 FROM memberships JOIN users ON users.id = memberships.user_id \
         JOIN workspaces ON workspaces.id = memberships.workspace_id \
         WHERE memberships.user_id = $1 AND memberships.workspace_id = $2 AND memberships.role <> 'guest' \
         AND users.suspended_at IS NULL AND workspaces.deleted_at IS NULL",
    )
    .bind(driver.to_string())
    .bind(workspace_id.to_string())
    .fetch_optional(&mut *tx)
    .await?;
    // The agent: a live custom agent of this workspace with the sandbox tool.
    let tools = sqlx::query_scalar::<_, String>(
        "SELECT tools FROM agent_profiles WHERE workspace_id = $1 AND user_id = $2 AND deleted_at IS NULL",
    )
    .bind(workspace_id.to_string())
    .bind(agent.to_string())
    .fetch_optional(&mut *tx)
    .await?;
    let ticked = tools
        .and_then(|tools| serde_json::from_str::<Vec<String>>(&tools).ok())
        .is_some_and(|tools| tools.iter().any(|tool| tool == TOOL));
    // The thread: running now, for this agent, driven by this person.
    let thread = sqlx::query(
        "SELECT EXISTS (SELECT 1 FROM agent_routine_runs r WHERE r.thread_key = t.thread_key AND r.writes = 'dry') \
         AS dry FROM agent_threads t WHERE t.thread_key = $1 AND t.workspace_id = $2 AND t.driver_user_id = $3 \
         AND t.agent_user_id = $4 AND t.status = 'running'",
    )
    .bind(thread_key)
    .bind(workspace_id.to_string())
    .bind(driver.to_string())
    .bind(agent.to_string())
    .fetch_optional(&mut *tx)
    .await?;
    let (Some(_), true, Some(thread)) = (member, ticked, thread) else {
        return Ok(Err(Refused::NotAllowed));
    };
    let dry_run = thread.get::<Flag, _>("dry").0;
    let settings = sqlx::query(
        "SELECT s.idle_minutes, s.cpu_minutes_month, COALESCE(u.cpu_ms, 0) AS cpu_ms \
         FROM agent_sandbox_settings s \
         LEFT JOIN agent_sandbox_usage u ON u.agent_user_id = s.agent_user_id AND u.month = $3 \
         WHERE s.agent_user_id = $1 AND s.workspace_id = $2 AND s.enabled = 1",
    )
    .bind(agent.to_string())
    .bind(workspace_id.to_string())
    .bind(month(now))
    .fetch_optional(&mut *tx)
    .await?;
    let Some(settings) = settings else {
        return Ok(Err(Refused::Off));
    };
    if settings.get::<i64, _>("cpu_ms") >= settings.get::<i64, _>("cpu_minutes_month") * 60_000 {
        return Ok(Err(Refused::Budget));
    }

    // The call token: an agent session that the scope layer judges like any other, bound to this call.
    let call_id = Id::new_v7();
    let session_id = Id::new_v7();
    let token = format!("{TOKEN_PREFIX}{}", generate_opaque_token());
    let proxy_password = generate_opaque_token();
    let token_hash: [u8; 32] = Sha256::digest(token.as_bytes()).into();
    let password_hash: [u8; 32] = Sha256::digest(proxy_password.as_bytes()).into();
    let token_expires = now + (timeout + 60) * 1_000;
    sqlx::query(
        "INSERT INTO sessions (id, token_hash, user_id, created_at, last_activity_at, idle_expires_at, \
         absolute_expires_at) VALUES ($1, $2, $3, $4, $4, $5, $5)",
    )
    .bind(session_id.to_string())
    .bind(token_hash.to_vec())
    .bind(driver.to_string())
    .bind(now)
    .bind(token_expires)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO agent_sessions (session_id, user_id, workspace_id, expires_at, agent_user_id, dry_run, \
         sandbox_call_id) VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(session_id.to_string())
    .bind(driver.to_string())
    .bind(workspace_id.to_string())
    .bind(token_expires)
    .bind(agent.to_string())
    .bind(i64::from(dry_run))
    .bind(call_id.to_string())
    .execute(&mut *tx)
    .await?;
    let work = work_id(agent, thread_key, driver);
    sqlx::query(
        "INSERT INTO agent_sandbox_calls (id, workspace_id, agent_user_id, driver_user_id, thread_key, work_id, \
         session_id, proxy_password_hash, script_sha256, dry_run, status, timeout_seconds, started_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, 'running', $11, $12)",
    )
    .bind(call_id.to_string())
    .bind(workspace_id.to_string())
    .bind(agent.to_string())
    .bind(driver.to_string())
    .bind(thread_key)
    .bind(&work)
    .bind(session_id.to_string())
    .bind(password_hash.to_vec())
    .bind(script_sha256)
    .bind(i64::from(dry_run))
    .bind(timeout)
    .bind(now)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    let limits = json!({
        "timeout_s": timeout,
        "memory_mb": MEMORY_MB,
        "cpu": 1,
        "disk_mb": DISK_MB,
        "idle_minutes": settings.get::<i64, _>("idle_minutes"),
    });
    let sealed = keys.seal(
        json!({ "token": token, "proxy_password": proxy_password })
            .to_string()
            .as_bytes(),
    );
    let expires_at = now + TICKET_LIFETIME_MS;
    let ticket = keys.sign(&json!({
        "call": call_id.to_string(),
        "agent": agent.to_string(),
        "workspace": workspace_id.to_string(),
        "work": work,
        "exp": expires_at,
        "limits": limits,
        "sealed": sealed,
    }));
    Ok(Ok(json!({
        "call_id": call_id.to_string(),
        "ticket": ticket,
        "expires_at": expires_at,
        "dry_run": dry_run,
        "limits": limits,
    })))
}

// ---------------------------------------------------------------------------------------------
// Call end.

#[derive(Deserialize)]
struct EndBody {
    #[serde(default)]
    exit: Option<i64>,
    #[serde(default)]
    timed_out: bool,
    #[serde(default)]
    interrupted: Option<String>,
    #[serde(default)]
    cpu_ms: i64,
}

async fn end_call(
    State(state): State<AgentState>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
    Path(call_id): Path<String>,
    Json(body): Json<EndBody>,
) -> Result<Response, ApiError> {
    service_only(&state, &headers, &uri, &request_id)?;
    let not_found = || {
        error(
            StatusCode::NOT_FOUND,
            "not_found",
            "Not found.",
            &uri,
            &request_id,
        )
    };
    let call_id: Id = call_id.parse().map_err(|_| not_found())?;
    let now = TimestampMillis::now().as_millis();
    let interrupted = body
        .interrupted
        .map(|reason| reason.chars().take(200).collect::<String>());
    let result: Result<Option<Vec<Value>>, sqlx::Error> = async {
        let mut tx = state.database().immediate_transaction().await?;
        let Some(call) = sqlx::query(
            "SELECT agent_user_id, session_id, status FROM agent_sandbox_calls WHERE id = $1",
        )
        .bind(call_id.to_string())
        .fetch_optional(&mut *tx)
        .await?
        else {
            return Ok(None);
        };
        // Only the first end counts (the broker may retry; a lost call stays lost).
        if call.get::<String, _>("status") == "running" {
            if let Some(session) = call.get::<Option<String>, _>("session_id") {
                sqlx::query("DELETE FROM sessions WHERE id = $1")
                    .bind(session)
                    .execute(&mut *tx)
                    .await?;
            }
            sqlx::query(
                "UPDATE agent_sandbox_calls SET status = 'done', session_id = NULL, ended_at = $2, exit_code = $3, \
                 timed_out = $4, interrupted = $5, cpu_ms = $6 WHERE id = $1",
            )
            .bind(call_id.to_string())
            .bind(now)
            .bind(body.exit)
            .bind(i64::from(body.timed_out))
            .bind(interrupted)
            .bind(body.cpu_ms.max(0))
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "INSERT INTO agent_sandbox_usage (agent_user_id, month, cpu_ms) VALUES ($1, $2, $3) \
                 ON CONFLICT (agent_user_id, month) DO UPDATE SET cpu_ms = agent_sandbox_usage.cpu_ms + $3",
            )
            .bind(call.get::<String, _>("agent_user_id"))
            .bind(month(now))
            .bind(body.cpu_ms.max(0))
            .execute(&mut *tx)
            .await?;
        }
        let pending = sqlx::query(
            "SELECT id, host, port FROM agent_egress_requests WHERE call_id = $1 AND status = 'pending' \
             ORDER BY created_at, id",
        )
        .bind(call_id.to_string())
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .map(|row| {
            json!({
                "id": row.get::<String, _>("id"),
                "host": row.get::<String, _>("host"),
                "port": row.get::<i64, _>("port"),
            })
        })
        .collect();
        tx.commit().await?;
        Ok(Some(pending))
    }
    .await;
    match result {
        Ok(Some(pending)) => Ok(Json(json!({ "egress_pending": pending })).into_response()),
        Ok(None) => Err(not_found()),
        Err(failure) => Err(unavailable(&failure, &uri, &request_id)),
    }
}

// ---------------------------------------------------------------------------------------------
// Egress decisions.

/// A host as rules see it: lower case, punycode, no trailing dot. `None` for IP literals and invalid names: no rule
/// can allow them.
#[must_use]
pub fn normalize_host(host: &str) -> Option<String> {
    let host = host.trim().trim_end_matches('.');
    if host.is_empty() || host.starts_with('[') {
        return None;
    }
    match url::Host::parse(host).ok()? {
        url::Host::Domain(domain) if domain.contains('.') => Some(domain),
        _ => None,
    }
}

/// `api.example.com` matches only itself; `*.example.com` any subdomain at any depth, never `example.com`.
#[must_use]
pub fn rule_matches(rule: &str, host: &str) -> bool {
    match rule.strip_prefix("*.") {
        Some(base) => host
            .strip_suffix(base)
            .is_some_and(|rest| rest.len() > 1 && rest.ends_with('.')),
        None => rule == host,
    }
}

/// On the risky list (section 4): itself or a subdomain of a listed host.
#[must_use]
pub fn is_risky(host: &str) -> bool {
    RISKY
        .iter()
        .any(|risky| host == *risky || rule_matches(&format!("*.{risky}"), host))
}

#[derive(Deserialize)]
struct CheckBody {
    call_id: Id,
    proxy_password: String,
    host: String,
    port: i64,
}

async fn check_egress(
    State(state): State<AgentState>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
    Json(body): Json<CheckBody>,
) -> Result<Response, ApiError> {
    service_only(&state, &headers, &uri, &request_id)?;
    let own_host = state
        .public_origin
        .as_deref()
        .and_then(|origin| url::Url::parse(origin).ok())
        .and_then(|origin| origin.host_str().map(str::to_owned));
    match decide(&state, &body, own_host.as_deref()).await {
        Ok(Some(decision)) => Ok(Json(decision).into_response()),
        Ok(None) => Err(error(
            StatusCode::FORBIDDEN,
            "egress_call_unknown",
            "No such call, or a wrong proxy password.",
            &uri,
            &request_id,
        )),
        Err(failure) => Err(unavailable(&failure, &uri, &request_id)),
    }
}

/// Section 4, decision order: (1) agent off or budget spent → deny; (2) deny rule, or a Deny for this thread and
/// driver → deny; (3) risky host → ask, unless Allowed once here; (4) allow rule → allow; (5) Allowed once for this
/// thread and driver → allow; (6) else ask (one pending request per thread, driver and host). `None`: the call or
/// its password is wrong.
async fn decide(
    state: &AgentState,
    body: &CheckBody,
    own_host: Option<&str>,
) -> Result<Option<Value>, sqlx::Error> {
    let now = TimestampMillis::now().as_millis();
    let mut tx = state.database().immediate_transaction().await?;
    let Some(call) = sqlx::query(
        "SELECT c.workspace_id, c.agent_user_id, c.driver_user_id, c.thread_key, c.status, c.proxy_password_hash, \
         s.enabled, s.cpu_minutes_month, s.egress_bytes_month, p.deleted_at, \
         COALESCE(u.cpu_ms, 0) AS cpu_ms, COALESCE(u.egress_bytes, 0) AS egress_bytes \
         FROM agent_sandbox_calls c \
         LEFT JOIN agent_sandbox_settings s ON s.agent_user_id = c.agent_user_id \
         LEFT JOIN agent_profiles p ON p.user_id = c.agent_user_id \
         LEFT JOIN agent_sandbox_usage u ON u.agent_user_id = c.agent_user_id AND u.month = $2 \
         WHERE c.id = $1",
    )
    .bind(body.call_id.to_string())
    .bind(month(now))
    .fetch_optional(&mut *tx)
    .await?
    else {
        return Ok(None);
    };
    let given: [u8; 32] = Sha256::digest(body.proxy_password.as_bytes()).into();
    if !bool::from(
        given
            .as_slice()
            .ct_eq(&call.get::<Vec<u8>, _>("proxy_password_hash")),
    ) {
        return Ok(None);
    }
    let workspace: String = call.get("workspace_id");
    let agent: String = call.get("agent_user_id");
    let driver: String = call.get("driver_user_id");
    let thread: String = call.get("thread_key");
    let host = normalize_host(&body.host);
    let port = body.port;

    let deny = |reason: &'static str| ("deny", reason, None::<String>);
    let (decision, reason, rule) = 'decision: {
        if call.get::<String, _>("status") != "running" {
            break 'decision deny("the call ended");
        }
        let enabled = call.get::<Option<i64>, _>("enabled") == Some(1)
            && call.get::<Option<i64>, _>("deleted_at").is_none();
        if !enabled {
            break 'decision deny("the sandbox is off for this agent");
        }
        let cpu_spent = call.get::<i64, _>("cpu_ms")
            >= call.get::<Option<i64>, _>("cpu_minutes_month").unwrap_or(0) * 60_000;
        let egress_spent = call.get::<i64, _>("egress_bytes")
            >= call
                .get::<Option<i64>, _>("egress_bytes_month")
                .unwrap_or(0);
        if cpu_spent || egress_spent {
            break 'decision deny("the agent's monthly budget is spent");
        }
        let Some(host) = host.as_deref() else {
            break 'decision deny("not a host name (IP addresses are never allowed)");
        };
        if !(1..=65_535).contains(&port) {
            break 'decision deny("not a port");
        }
        if host == "orbit.internal" || Some(host) == own_host {
            break 'decision deny("Orbit is reached through orbit.internal only");
        }
        let rules = sqlx::query(
            "SELECT id, host, action FROM agent_egress_rules WHERE agent_user_id = $1 AND workspace_id = $2 \
             AND port = $3 ORDER BY action DESC, id",
        )
        .bind(&agent)
        .bind(&workspace)
        .bind(port)
        .fetch_all(&mut *tx)
        .await?;
        let matching = |action: &str| {
            rules
                .iter()
                .find(|row| {
                    row.get::<String, _>("action") == action
                        && rule_matches(&row.get::<String, _>("host"), host)
                })
                .map(|row| row.get::<String, _>("id"))
        };
        if let Some(rule) = matching("deny") {
            break 'decision ("deny", "a deny rule", Some(rule));
        }
        let answers = sqlx::query(
            "SELECT status FROM agent_egress_requests WHERE agent_user_id = $1 AND thread_key = $2 \
             AND driver_user_id = $3 AND host = $4 AND port = $5 AND status IN ('allowed', 'denied') \
             AND (expires_at IS NULL OR expires_at > $6)",
        )
        .bind(&agent)
        .bind(&thread)
        .bind(&driver)
        .bind(host)
        .bind(port)
        .bind(now)
        .fetch_all(&mut *tx)
        .await?;
        let answered = |status: &str| {
            answers
                .iter()
                .any(|row| row.get::<String, _>("status") == status)
        };
        if answered("denied") {
            break 'decision deny("denied in this thread");
        }
        let allowed_once = answered("allowed");
        if is_risky(host) && !allowed_once {
            break 'decision ("ask", "a risky host: always asks", None);
        }
        if let Some(rule) = matching("allow") {
            sqlx::query("UPDATE agent_egress_rules SET last_used_at = $2 WHERE id = $1")
                .bind(&rule)
                .bind(now)
                .execute(&mut *tx)
                .await?;
            break 'decision ("allow", "an allow rule", Some(rule));
        }
        if allowed_once {
            break 'decision ("allow", "allowed once in this thread", None);
        }
        ("ask", "no rule for this host", None)
    };

    let mut request_id = None;
    if decision == "ask"
        && let Some(host) = host.as_deref()
    {
        let pending = sqlx::query_scalar::<_, String>(
            "SELECT id FROM agent_egress_requests WHERE agent_user_id = $1 AND thread_key = $2 \
             AND driver_user_id = $3 AND host = $4 AND port = $5 AND status = 'pending'",
        )
        .bind(&agent)
        .bind(&thread)
        .bind(&driver)
        .bind(host)
        .bind(port)
        .fetch_optional(&mut *tx)
        .await?;
        let id = match pending {
            Some(id) => {
                sqlx::query("UPDATE agent_egress_requests SET call_id = $2 WHERE id = $1")
                    .bind(&id)
                    .bind(body.call_id.to_string())
                    .execute(&mut *tx)
                    .await?;
                id
            }
            None => {
                let id = Id::new_v7().to_string();
                sqlx::query(
                    "INSERT INTO agent_egress_requests (id, workspace_id, agent_user_id, driver_user_id, thread_key, \
                     call_id, host, port, status, created_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'pending', $9)",
                )
                .bind(&id)
                .bind(&workspace)
                .bind(&agent)
                .bind(&driver)
                .bind(&thread)
                .bind(body.call_id.to_string())
                .bind(host)
                .bind(port)
                .bind(now)
                .execute(&mut *tx)
                .await?;
                id
            }
        };
        request_id = Some(id);
    }
    // The log keeps what was asked: the normalized host, or the raw value cut short when it is not one.
    let logged_host = host
        .clone()
        .unwrap_or_else(|| body.host.chars().take(100).collect());
    let log_id = Id::new_v7().to_string();
    sqlx::query(
        "INSERT INTO agent_egress_log (id, call_id, workspace_id, agent_user_id, driver_user_id, host, port, \
         decision, reason, rule_id, created_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
    )
    .bind(&log_id)
    .bind(body.call_id.to_string())
    .bind(&workspace)
    .bind(&agent)
    .bind(&driver)
    .bind(logged_host)
    .bind(port)
    .bind(decision)
    .bind(reason)
    .bind(rule.as_deref())
    .bind(now)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Some(json!({
        "decision": decision,
        "reason": reason,
        "host": host,
        "rule_id": rule,
        "request_id": request_id,
        "log_id": log_id,
    })))
}

#[derive(Deserialize)]
struct DoneBody {
    log_id: Id,
    call_id: Id,
    bytes_up: i64,
    bytes_down: i64,
    duration_ms: i64,
    /// Why the proxy refused an allowed connection (a non-public address, a TLS name that is not the host).
    #[serde(default)]
    blocked: Option<String>,
}

/// The proxy's report of one allowed connection. Counts once per log row (a retry changes nothing): the bytes go into
/// the row, the call and the agent's month.
async fn egress_done(
    State(state): State<AgentState>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
    Json(body): Json<DoneBody>,
) -> Result<Response, ApiError> {
    service_only(&state, &headers, &uri, &request_id)?;
    let now = TimestampMillis::now().as_millis();
    let bytes = body.bytes_up.max(0).saturating_add(body.bytes_down.max(0));
    let blocked = body
        .blocked
        .map(|reason| reason.chars().take(200).collect::<String>());
    let result: Result<bool, sqlx::Error> = async {
        let mut tx = state.database().immediate_transaction().await?;
        let Some(agent) = sqlx::query_scalar::<_, String>(
            "UPDATE agent_egress_log SET bytes_up = $3, bytes_down = $4, duration_ms = $5, done_at = $6, \
             decision = CASE WHEN $7 = 1 THEN 'blocked' ELSE decision END, reason = COALESCE($8, reason) \
             WHERE id = $1 AND call_id = $2 AND decision = 'allow' AND done_at IS NULL RETURNING agent_user_id",
        )
        .bind(body.log_id.to_string())
        .bind(body.call_id.to_string())
        .bind(body.bytes_up.max(0))
        .bind(body.bytes_down.max(0))
        .bind(body.duration_ms.max(0))
        .bind(now)
        .bind(i64::from(blocked.is_some()))
        .bind(blocked)
        .fetch_optional(&mut *tx)
        .await?
        else {
            return Ok(false);
        };
        sqlx::query(
            "UPDATE agent_sandbox_calls SET egress_bytes = egress_bytes + $2 WHERE id = $1",
        )
        .bind(body.call_id.to_string())
        .bind(bytes)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO agent_sandbox_usage (agent_user_id, month, egress_bytes) VALUES ($1, $2, $3) \
             ON CONFLICT (agent_user_id, month) DO UPDATE SET egress_bytes = agent_sandbox_usage.egress_bytes + $3",
        )
        .bind(agent)
        .bind(month(now))
        .bind(bytes)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(true)
    }
    .await;
    match result {
        Ok(counted) => Ok(Json(json!({ "counted": counted })).into_response()),
        Err(failure) => Err(unavailable(&failure, &uri, &request_id)),
    }
}

#[cfg(test)]
mod tests {
    use super::{SandboxKeys, is_risky, normalize_host, rule_matches};

    #[test]
    fn hosts_are_normalized_and_ip_literals_refused() {
        assert_eq!(
            normalize_host("API.Example.COM.").as_deref(),
            Some("api.example.com")
        );
        assert_eq!(
            normalize_host("bücher.example").as_deref(),
            Some("xn--bcher-kva.example")
        );
        for refused in [
            "1.2.3.4",
            "[::1]",
            "::1",
            "localhost",
            "",
            "a b.com",
            "0x7f.1",
            "2130706433",
        ] {
            assert_eq!(normalize_host(refused), None, "{refused}");
        }
    }

    #[test]
    fn wildcards_match_subdomains_only() {
        assert!(rule_matches("api.example.com", "api.example.com"));
        assert!(!rule_matches("api.example.com", "x.api.example.com"));
        assert!(rule_matches("*.example.com", "a.example.com"));
        assert!(rule_matches("*.example.com", "a.b.example.com"));
        assert!(!rule_matches("*.example.com", "example.com"));
        assert!(!rule_matches("*.example.com", "badexample.com"));
        assert!(!rule_matches("*.example.com", "example.com.evil.net"));
    }

    #[test]
    fn the_risky_list_covers_subdomains() {
        assert!(is_risky("webhook.site"));
        assert!(is_risky("abc.ngrok-free.app"));
        assert!(is_risky("bucket.s3.amazonaws.com"));
        assert!(!is_risky("api.github.com"));
        assert!(!is_risky("notwebhook.site"));
    }

    #[test]
    fn keys_need_32_characters_and_hide_in_debug() {
        assert!(SandboxKeys::new(b"short").is_err());
        let keys = SandboxKeys::new(b"0123456789abcdef0123456789abcdef\n").unwrap();
        assert_eq!(format!("{keys:?}"), "SandboxKeys([REDACTED])");
    }
}
