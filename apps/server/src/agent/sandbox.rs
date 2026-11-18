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
//!   adds its CPU time to the month, and returns the egress requests that wait for a person's answer. The sidecar may
//!   end a call too (secret), as interrupted, when it gave up on the broker (a stopped run, a broker that did not
//!   answer): the token dies and the proxy denies the call's password at once.
//! - `POST /api/internal/agent/egress/check` (egress proxy): with the call's proxy password, decides one
//!   `host:port`: `allow`, `deny` or `ask` (section 4, decision order), and logs it.
//! - `POST /api/internal/agent/egress/done` (egress proxy): an allowed connection's bytes, once, into its log row, the
//!   call and the month; or `blocked` when the proxy refused it after the yes (`agent::egress_proxy`).
//! - `POST /api/v1/workspaces/{id}/agents/egress-requests/{request}` (a person; agent sessions never reach `agents/`):
//!   the answer to the egress card. Allow once: the driver only. Allow always: owners and admins, never a risky host.
//!   Deny: the driver or an owner/admin (`always`: a deny rule, owners and admins). A request waits [`ASK_WAIT_MS`]; then
//!   it is expired and the next call asks again. The answer queues an `action` input for the **original driver**, so
//!   the run continues as theirs, whoever clicked.
//! - `GET /api/v1/workspaces/{id}/agents/egress-requests?conversation_id=…` (members who can read the conversation):
//!   the egress cards of one conversation, each with its card message (the sidecar's nonce `egress:<call>`), its
//!   state, and what the viewer may answer. Web and mobile draw the buttons from it.
//! - `GET|PUT /api/v1/workspaces/{id}/agents/{agent}/sandbox`, `POST …/egress-rules`, `DELETE …/egress-rules/{rule}`
//!   (owners and admins; `agent` is the profile id): the agent card's sandbox switch and limits, and the Internet
//!   access page (rules, pending requests, the connection log). A rule may not be an IP, `*`, a wildcard on a public
//!   suffix (Public Suffix List), Orbit's own host, or an allow rule for a risky host.
//!
//! - `POST /api/internal/agent/runs/{run}/usage` (sidecar secret): the model tokens of one agent run, counted once
//!   into the agent's month. Inputs of an agent past its monthly tokens are flagged (`inputs`), and the sidecar tells
//!   the driver instead of running.
//! - `POST /api/internal/agent/sandbox/stops/{id}/done` (sidecar secret): the sidecar asked the broker to stop an
//!   agent's thread sandboxes ([`switch_off`]; the stops come with its inputs).
//!
//! Budgets (section 5): CPU time refuses new tickets, internet traffic makes the proxy deny, model tokens refuse new
//! runs. At 80 % and at 100 % of a monthly limit the agent sends its workspace's owners and admins one DM each, once
//! per month ([`send_budget_notices`]). Calls without an end are closed by the scheduler ([`sweep`]).
//!
//! The broker and the proxy authenticate with a bearer derived from the broker key; the key file is
//! `ORBIT_AGENT_SANDBOX_KEY_FILE`. Without it, or with `ORBIT_AGENT_SANDBOX=off`, every route answers `sandbox_off`.

use axum::extract::{Extension, Path, Query, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, Mac};
use orbit_domain::Permission;
use orbit_platform::{Flag, Id, RequestId, Row, TimestampMillis, generate_opaque_token};
use rand_core::{OsRng, RngCore};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use super::AgentState;
use super::routes::sidecar_only;
use super::scope::TOKEN_PREFIX;
use crate::audit::{self, AuditOutcome};
use crate::repositories::membership;
use crate::task_routes::{ApiError, authenticate_session, request_id_value};

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
/// A thread sandbox's longest life (the broker's `MAX_LIFE_S`): without a call since, none can be alive.
const SANDBOX_LIFE_MS: i64 = 4 * 60 * 60 * 1_000;
/// Budget notices go out at these shares of a monthly limit (percent).
const NOTICE_LEVELS: [i64; 2] = [80, 100];
/// An "Allow once" lasts this long (as write approvals).
pub const ALLOW_ONCE_MS: i64 = 30 * 60 * 1_000;
/// A card's question waits this long for an answer (as write approvals); later answers are refused and the next call
/// asks again.
pub const ASK_WAIT_MS: i64 = 30 * 60 * 1_000;

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
        .route("/api/internal/agent/runs/{run_id}/usage", post(run_usage))
        .route(
            "/api/internal/agent/sandbox/stops/{stop_id}/done",
            post(stop_done),
        )
        .route(
            "/api/v1/workspaces/{workspace_id}/agents/egress-requests",
            get(conversation_requests),
        )
        .route(
            "/api/v1/workspaces/{workspace_id}/agents/egress-requests/{request_id}",
            post(answer_egress),
        )
        .route(
            "/api/v1/workspaces/{workspace_id}/agents/{agent_id}/sandbox",
            get(read_sandbox).put(save_sandbox),
        )
        .route(
            "/api/v1/workspaces/{workspace_id}/agents/{agent_id}/egress-rules",
            post(add_rule),
        )
        .route(
            "/api/v1/workspaces/{workspace_id}/agents/{agent_id}/egress-rules/{rule_id}",
            delete(delete_rule),
        )
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

/// The month a use counts in (UTC, `YYYY-MM`): the key of the monthly limits.
#[must_use]
pub fn month(now: i64) -> String {
    jiff::Timestamp::from_millisecond(now)
        .map(|time| time.strftime("%Y-%m").to_string())
        .unwrap_or_default()
}

/// The thread sandbox of one agent, thread and driver: nothing is shared between them.
fn work_id(agent: Id, thread_key: &str, driver: Id) -> String {
    let digest = Sha256::digest(format!("{agent}:{thread_key}:{driver}"));
    hex(&digest[..16])
}

/// The month after `now`'s, as people read it ("1 December 2026"): when the monthly limits start again (UTC).
#[must_use]
pub fn next_month_text(now: i64) -> String {
    jiff::Timestamp::from_millisecond(now)
        .ok()
        .map(|time| {
            time.to_zoned(jiff::tz::TimeZone::UTC)
                .date()
                .first_of_month()
        })
        .and_then(|first| first.checked_add(jiff::Span::new().months(1)).ok())
        .map(|next| next.strftime("1 %B %Y").to_string())
        .unwrap_or_default()
}

/// Calls without an end long after their timeout become `lost`; their tokens go (the proxy denies a call that does not
/// run). The broker's late end report still counts the call's CPU time ([`end_call`]). Returns how many it closed.
pub async fn close_lost_calls(
    conn: &mut sqlx::AnyConnection,
    now: i64,
) -> Result<u64, sqlx::Error> {
    sqlx::query(
        "DELETE FROM sessions WHERE id IN (SELECT session_id FROM agent_sandbox_calls \
         WHERE status = 'running' AND started_at + timeout_seconds * 1000 + $1 < $2)",
    )
    .bind(LOST_AFTER_MS)
    .bind(now)
    .execute(&mut *conn)
    .await?;
    Ok(sqlx::query(
        "UPDATE agent_sandbox_calls SET status = 'lost', ended_at = $2, session_id = NULL \
         WHERE status = 'running' AND started_at + timeout_seconds * 1000 + $1 < $2",
    )
    .bind(LOST_AFTER_MS)
    .bind(now)
    .execute(&mut *conn)
    .await?
    .rows_affected())
}

/// Ends running calls now, as interrupted with `reason` (`agent`: only that agent's): their tokens die, and the proxy
/// denies their passwords at the next connection.
async fn interrupt_calls(
    conn: &mut sqlx::AnyConnection,
    agent: Option<&str>,
    reason: &str,
    now: i64,
) -> Result<u64, sqlx::Error> {
    sqlx::query(
        "DELETE FROM sessions WHERE id IN (SELECT session_id FROM agent_sandbox_calls \
         WHERE status = 'running' AND ($1 IS NULL OR agent_user_id = $1))",
    )
    .bind(agent)
    .execute(&mut *conn)
    .await?;
    Ok(sqlx::query(
        "UPDATE agent_sandbox_calls SET status = 'done', ended_at = $2, session_id = NULL, interrupted = $3 \
         WHERE status = 'running' AND ($1 IS NULL OR agent_user_id = $1)",
    )
    .bind(agent)
    .bind(now)
    .bind(reason)
    .execute(&mut *conn)
    .await?
    .rows_affected())
}

/// The kill path (section 5, off switch), in the caller's transaction: the agent's running calls end now, and when a
/// thread sandbox of the agent may still live (a call within a sandbox's longest life), a stop for the broker is
/// queued. The sidecar takes it with its inputs and calls the broker's `stop_agent`, which kills the agent's thread
/// sandboxes (running calls and background processes) and removes the calls' proxy files, so the proxy drops their
/// tunnels. The model reads `reason` as the call's interruption. The broker's end reports come later and still count
/// the calls' CPU time.
pub(crate) async fn switch_off(
    conn: &mut sqlx::AnyConnection,
    agent: Id,
    reason: &str,
    now: i64,
) -> Result<(), sqlx::Error> {
    let agent = agent.to_string();
    interrupt_calls(conn, Some(&agent), reason, now).await?;
    let recent = sqlx::query_scalar::<_, Flag>(
        "SELECT EXISTS (SELECT 1 FROM agent_sandbox_calls WHERE agent_user_id = $1 AND started_at > $2)",
    )
    .bind(&agent)
    .bind(now - SANDBOX_LIFE_MS)
    .fetch_one(&mut *conn)
    .await?;
    if recent.0 {
        sqlx::query(
            "INSERT INTO agent_sandbox_stops (id, agent_user_id, reason, created_at) VALUES ($1, $2, $3, $4)",
        )
        .bind(Id::new_v7().to_string())
        .bind(&agent)
        .bind(reason)
        .bind(now)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

/// The stops the sidecar has not done yet (oldest first), for its inputs poll.
pub(crate) async fn pending_stops(
    database: &orbit_platform::Database,
) -> Result<Vec<Value>, sqlx::Error> {
    Ok(sqlx::query(
        "SELECT id, agent_user_id, reason FROM agent_sandbox_stops ORDER BY created_at, id LIMIT 50",
    )
    .fetch_all(database.pool())
    .await?
    .iter()
    .map(|row| {
        json!({
            "id": row.get::<String, _>("id"),
            "agent_user_id": row.get::<String, _>("agent_user_id"),
            "reason": row.get::<String, _>("reason"),
        })
    })
    .collect())
}

async fn stop_done(
    State(state): State<AgentState>,
    Path(stop_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
) -> Result<StatusCode, ApiError> {
    sidecar_only(&state, &headers, &uri, &request_id)?;
    sqlx::query("DELETE FROM agent_sandbox_stops WHERE id = $1")
        .bind(stop_id)
        .execute(state.database().pool())
        .await
        .map_err(|failure| unavailable(&failure, &uri, &request_id))?;
    Ok(StatusCode::NO_CONTENT)
}

/// What one [`sweep`] did.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Sweep {
    /// Calls without an end after their timeout and grace, now `lost`.
    pub lost: u64,
    /// Running calls ended because agent sandboxes are off on this server.
    pub interrupted: u64,
}

/// The scheduler's job (every round of `scheduled::run_due`, on every server): closes lost calls, and with agent
/// sandboxes off on this server (no broker key, `ORBIT_AGENT_SANDBOX=off`) ends every running call. A cheap read
/// first: the write transaction holds every writer, so it runs only when there is work. Idempotent: a closed call is
/// not running any more, and its CPU time is counted only by the broker's report.
pub async fn sweep(state: &AgentState, now: i64) -> Result<Sweep, sqlx::Error> {
    let off = state.sandbox.is_none();
    let due = sqlx::query_scalar::<_, Flag>(
        "SELECT EXISTS (SELECT 1 FROM agent_sandbox_calls WHERE status = 'running' \
         AND ($3 = 1 OR started_at + timeout_seconds * 1000 + $1 < $2))",
    )
    .bind(LOST_AFTER_MS)
    .bind(now)
    .bind(i64::from(off))
    .fetch_one(state.database().pool())
    .await?;
    if !due.0 {
        return Ok(Sweep::default());
    }
    let mut tx = state.database().immediate_transaction().await?;
    let lost = close_lost_calls(&mut tx, now).await?;
    let interrupted = if off {
        interrupt_calls(&mut tx, None, "agent sandboxes are off on this server", now).await?
    } else {
        0
    };
    tx.commit().await?;
    Ok(Sweep { lost, interrupted })
}

/// The highest notice level (80 or 100 %) that `used` has reached of `limit`. A limit of 0 sends none: an admin set
/// it to stop the counter, not to be warned.
#[must_use]
pub fn notice_level(used: i64, limit: i64) -> Option<i64> {
    if limit <= 0 {
        return None;
    }
    NOTICE_LEVELS
        .iter()
        .rev()
        .copied()
        .find(|level| i128::from(used) * 100 >= i128::from(limit) * i128::from(*level))
}

/// A byte count as people read it.
fn bytes_text(bytes: i64) -> String {
    const MIB: f64 = 1024.0 * 1024.0;
    #[allow(clippy::cast_precision_loss)]
    let mib = bytes as f64 / MIB;
    if mib >= 1024.0 {
        format!("{:.1} GiB", mib / 1024.0)
    } else {
        format!("{mib:.0} MiB")
    }
}

/// CPU time in minutes: whole minutes, with one decimal under 10 (a small limit would read "0 of 1").
fn minutes_text(ms: i64) -> String {
    if ms < 600_000 && ms % 60_000 != 0 {
        #[allow(clippy::cast_precision_loss)]
        let minutes = ms as f64 / 60_000.0;
        format!("{minutes:.1}")
    } else {
        (ms / 60_000).to_string()
    }
}

/// A counter's use and limit, as the notice says them.
#[must_use]
pub fn usage_text(counter: &str, used: i64, limit: i64) -> String {
    match counter {
        "cpu" => format!(
            "{} of {} CPU minutes",
            minutes_text(used),
            minutes_text(limit)
        ),
        "egress" => format!("{} of {}", bytes_text(used), bytes_text(limit)),
        _ => format!("{used} of {limit} tokens"),
    }
}

/// The text of one budget notice.
fn notice_text(
    agent_name: &str,
    counter: &str,
    level: i64,
    used: i64,
    limit: i64,
    now: i64,
) -> String {
    let (what, effect) = match counter {
        "cpu" => ("sandbox CPU time", "my sandbox commands are refused"),
        "egress" => ("internet traffic", "my sandbox cannot reach the internet"),
        _ => ("model tokens", "I cannot start new runs"),
    };
    let month = jiff::Timestamp::from_millisecond(now)
        .map(|time| time.strftime("%B %Y").to_string())
        .unwrap_or_default();
    let usage = usage_text(counter, used, limit);
    let next = next_month_text(now);
    let change = format!(
        "Owners and admins can change the limit in Settings → Agentic → Agents → {agent_name}."
    );
    if level >= 100 {
        format!(
            "I used all of my {what} for {month} ({usage}): {effect} until {next} (UTC). {change}"
        )
    } else {
        format!(
            "Heads-up: I used {level} % of my {what} for {month} ({usage}). At 100 %, {effect} until {next} (UTC). {change}"
        )
    }
}

/// After a counter of `agent` grew: the notices it reached this month, each once (the notices table's key), as a DM
/// from the agent to each owner and admin of its workspace. Never fails the count: errors are logged. A notice whose
/// row was written but whose DM failed is not sent again (no spam; the agent card shows the use).
pub(crate) async fn send_budget_notices(state: &AgentState, agent: &str, now: i64) {
    if let Err(error) = budget_notices(state, agent, now).await {
        tracing::warn!(%error, "agent sandbox budget notice");
    }
}

async fn budget_notices(state: &AgentState, agent: &str, now: i64) -> Result<(), sqlx::Error> {
    let month = month(now);
    let Some(row) = sqlx::query(
        "SELECT s.workspace_id, s.cpu_minutes_month, s.egress_bytes_month, s.model_tokens_month, \
         COALESCE(u.cpu_ms, 0) AS cpu_ms, COALESCE(u.egress_bytes, 0) AS egress_bytes, \
         COALESCE(u.model_tokens, 0) AS model_tokens, a.display_name \
         FROM agent_sandbox_settings s JOIN users a ON a.id = s.agent_user_id \
         JOIN agent_profiles p ON p.user_id = s.agent_user_id AND p.deleted_at IS NULL \
         LEFT JOIN agent_sandbox_usage u ON u.agent_user_id = s.agent_user_id AND u.month = $2 \
         WHERE s.agent_user_id = $1",
    )
    .bind(agent)
    .bind(&month)
    .fetch_optional(state.database().pool())
    .await?
    else {
        return Ok(());
    };
    let counters = [
        (
            "cpu",
            row.get::<i64, _>("cpu_ms"),
            row.get::<i64, _>("cpu_minutes_month")
                .saturating_mul(60_000),
        ),
        (
            "egress",
            row.get::<i64, _>("egress_bytes"),
            row.get::<i64, _>("egress_bytes_month"),
        ),
        (
            "tokens",
            row.get::<i64, _>("model_tokens"),
            row.get::<i64, _>("model_tokens_month"),
        ),
    ];
    let reached: Vec<_> = counters
        .into_iter()
        .filter_map(|(counter, used, limit)| {
            notice_level(used, limit).map(|level| (counter, level, used, limit))
        })
        .collect();
    if reached.is_empty() {
        return Ok(());
    }
    // A jump past 100 % marks 80 % as sent too; only the highest new level is sent.
    let mut due = Vec::new();
    let mut tx = state.database().immediate_transaction().await?;
    for &(counter, level, used, limit) in &reached {
        let mut highest_new = false;
        for notice in NOTICE_LEVELS.into_iter().filter(|notice| *notice <= level) {
            let written = sqlx::query(
                "INSERT INTO agent_sandbox_notices (agent_user_id, month, counter, level, sent_at) \
                 VALUES ($1, $2, $3, $4, $5) ON CONFLICT DO NOTHING",
            )
            .bind(agent)
            .bind(&month)
            .bind(counter)
            .bind(notice)
            .bind(now)
            .execute(&mut *tx)
            .await?
            .rows_affected();
            highest_new = written == 1 && notice == level;
        }
        if highest_new {
            due.push((counter, level, used, limit));
        }
    }
    let admins: Vec<String> = sqlx::query_scalar(
        "SELECT m.user_id FROM memberships m JOIN users u ON u.id = m.user_id \
         WHERE m.workspace_id = $1 AND m.role IN ('owner', 'admin') AND u.suspended_at IS NULL ORDER BY m.user_id",
    )
    .bind(row.get::<String, _>("workspace_id"))
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;

    let (Ok(workspace_id), Ok(agent_id)) = (
        row.get::<String, _>("workspace_id").parse::<Id>(),
        agent.parse::<Id>(),
    ) else {
        return Ok(());
    };
    let name: String = row.get("display_name");
    let chat = &state.chat;
    for (counter, level, used, limit) in due {
        let body = notice_text(&name, counter, level, used, limit, now);
        for admin in admins.iter().filter_map(|admin| admin.parse::<Id>().ok()) {
            let dm = match chat
                .publish(
                    workspace_id,
                    agent_id,
                    chat.chat.open_dm(workspace_id, agent_id, vec![admin]),
                )
                .await
            {
                Ok((written, _)) => written.value.id,
                Err(error) => {
                    tracing::warn!(%error, "agent sandbox budget notice DM");
                    continue;
                }
            };
            let send = crate::repositories::chat::SendInput {
                conversation_id: dm,
                thread_root_id: None,
                reply_to_id: None,
                body: body.clone(),
                sticker_id: None,
                file_ids: Vec::new(),
                online: chat.hub.online(workspace_id),
                also_in_channel: false,
                nonce: format!("budget:{month}:{counter}:{level}"),
                page_context: None,
            };
            match chat
                .publish(
                    workspace_id,
                    agent_id,
                    chat.chat.send_message(workspace_id, agent_id, send),
                )
                .await
            {
                Ok((written, _)) => {
                    crate::push::PushService::of(state.database()).chat_message(&written.value);
                }
                Err(error) => tracing::warn!(%error, "agent sandbox budget notice"),
            }
        }
    }
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
    /// The month's CPU time is spent: (used ms, limit ms).
    Budget(i64, i64),
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
        Ok(Err(Refused::Budget(used, limit))) => {
            let now = TimestampMillis::now().as_millis();
            Err(error(
                StatusCode::FORBIDDEN,
                "sandbox_budget_spent",
                "This agent used its sandbox time for this month.",
                &uri,
                &request_id,
            )
            .with_detail(format!(
                "This agent used its sandbox CPU time for this month ({}): sandbox commands are refused until {} \
                 (UTC). Chat tools still work. Owners and admins can raise the limit on the agent's card.",
                usage_text("cpu", used, limit),
                next_month_text(now),
            )))
        }
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
    let (used, limit) = (
        settings.get::<i64, _>("cpu_ms"),
        settings
            .get::<i64, _>("cpu_minutes_month")
            .saturating_mul(60_000),
    );
    if used >= limit {
        return Ok(Err(Refused::Budget(used, limit)));
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
    Json(mut body): Json<EndBody>,
) -> Result<Response, ApiError> {
    // The sidecar ends a call only as interrupted: the broker alone knows its exit and CPU time.
    let broker = keys(&state, &uri, &request_id)?.service_allows(&headers);
    if !broker {
        sidecar_only(&state, &headers, &uri, &request_id)?;
        body = EndBody {
            exit: None,
            timed_out: false,
            interrupted: Some(
                body.interrupted
                    .unwrap_or_else(|| "the run stopped the call".to_owned()),
            ),
            cpu_ms: 0,
        };
    }
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
    let cpu_ms = body.cpu_ms.max(0);
    let result: Result<Option<(Value, Option<String>)>, sqlx::Error> = async {
        let mut tx = state.database().immediate_transaction().await?;
        let Some(call) = sqlx::query(
            "SELECT agent_user_id, session_id, status, cpu_counted FROM agent_sandbox_calls WHERE id = $1",
        )
        .bind(call_id.to_string())
        .fetch_optional(&mut *tx)
        .await?
        else {
            return Ok(None);
        };
        let agent: String = call.get("agent_user_id");
        // Only the first end ends the call (the broker may retry; a lost or interrupted call stays as it is).
        if call.get::<String, _>("status") == "running" {
            if let Some(session) = call.get::<Option<String>, _>("session_id") {
                sqlx::query("DELETE FROM sessions WHERE id = $1")
                    .bind(session)
                    .execute(&mut *tx)
                    .await?;
            }
            sqlx::query(
                "UPDATE agent_sandbox_calls SET status = 'done', session_id = NULL, ended_at = $2, exit_code = $3, \
                 timed_out = $4, interrupted = $5 WHERE id = $1",
            )
            .bind(call_id.to_string())
            .bind(now)
            .bind(body.exit)
            .bind(i64::from(body.timed_out))
            .bind(interrupted)
            .execute(&mut *tx)
            .await?;
        }
        // The broker's CPU time counts once, also when the sidecar, the off switch or the lost-call job ended the
        // call first.
        let mut counted = None;
        if broker && call.get::<i64, _>("cpu_counted") == 0 {
            sqlx::query("UPDATE agent_sandbox_calls SET cpu_ms = $2, cpu_counted = 1 WHERE id = $1")
                .bind(call_id.to_string())
                .bind(cpu_ms)
                .execute(&mut *tx)
                .await?;
            sqlx::query(
                "INSERT INTO agent_sandbox_usage (agent_user_id, month, cpu_ms) VALUES ($1, $2, $3) \
                 ON CONFLICT (agent_user_id, month) DO UPDATE SET cpu_ms = agent_sandbox_usage.cpu_ms + $3",
            )
            .bind(&agent)
            .bind(month(now))
            .bind(cpu_ms)
            .execute(&mut *tx)
            .await?;
            counted = Some(agent);
        }
        let pending: Vec<Value> = sqlx::query(
            "SELECT id, host, port, expires_at FROM agent_egress_requests WHERE call_id = $1 AND status = 'pending' \
             AND (expires_at IS NULL OR expires_at > $2) ORDER BY created_at, id",
        )
        .bind(call_id.to_string())
        .bind(now)
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .map(|row| {
            json!({
                "id": row.get::<String, _>("id"),
                "host": row.get::<String, _>("host"),
                "port": row.get::<i64, _>("port"),
                "expires_at": row.get::<Option<i64>, _>("expires_at"),
            })
        })
        .collect();
        // What the proxy refused, with Orbit's reason: in the sandbox a refusal is only "403" (or curl's exit 56).
        let denied: Vec<Value> = sqlx::query(
            "SELECT host, port, MIN(reason) AS reason FROM agent_egress_log WHERE call_id = $1 \
             AND decision IN ('deny', 'blocked') GROUP BY host, port ORDER BY host, port LIMIT 10",
        )
        .bind(call_id.to_string())
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .map(|row| {
            json!({
                "host": row.get::<String, _>("host"),
                "port": row.get::<i64, _>("port"),
                "reason": row.get::<String, _>("reason"),
            })
        })
        .collect();
        tx.commit().await?;
        Ok(Some((json!({ "egress_pending": pending, "egress_denied": denied }), counted)))
    }
    .await;
    match result {
        Ok(Some((answer, counted))) => {
            if let Some(agent) = counted {
                send_budget_notices(&state, &agent, now).await;
            }
            Ok(Json(answer).into_response())
        }
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
            >= call
                .get::<Option<i64>, _>("cpu_minutes_month")
                .unwrap_or(0)
                .saturating_mul(60_000);
        let egress_spent = call.get::<i64, _>("egress_bytes")
            >= call
                .get::<Option<i64>, _>("egress_bytes_month")
                .unwrap_or(0);
        if egress_spent {
            break 'decision deny("the agent's internet traffic for this month is spent");
        }
        if cpu_spent {
            break 'decision deny("the agent's sandbox CPU time for this month is spent");
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
             AND driver_user_id = $3 AND host = $4 AND port = $5 AND status = 'pending' \
             AND (expires_at IS NULL OR expires_at > $6)",
        )
        .bind(&agent)
        .bind(&thread)
        .bind(&driver)
        .bind(host)
        .bind(port)
        .bind(now)
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
                     call_id, host, port, status, created_at, expires_at) \
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'pending', $9, $10)",
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
                .bind(now + ASK_WAIT_MS)
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
    let result: Result<Option<String>, sqlx::Error> = async {
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
            return Ok(None);
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
        .bind(&agent)
        .bind(month(now))
        .bind(bytes)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(Some(agent))
    }
    .await;
    match result {
        Ok(counted) => {
            if let Some(agent) = &counted {
                send_budget_notices(&state, agent, now).await;
            }
            Ok(Json(json!({ "counted": counted.is_some() })).into_response())
        }
        Err(failure) => Err(unavailable(&failure, &uri, &request_id)),
    }
}

#[derive(Deserialize)]
struct UsageBody {
    agent_user_id: Id,
    tokens: i64,
}

/// The sidecar's report of one agent run's model tokens (input and output, from Pi's usage). Counts once per run id (a
/// retry changes nothing) into the agent's month; only custom agents have a limit.
async fn run_usage(
    State(state): State<AgentState>,
    Path(run_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
    Json(body): Json<UsageBody>,
) -> Result<Response, ApiError> {
    sidecar_only(&state, &headers, &uri, &request_id)?;
    let run_id: Id = run_id.parse().map_err(|_| {
        error(
            StatusCode::NOT_FOUND,
            "not_found",
            "Not found.",
            &uri,
            &request_id,
        )
    })?;
    let now = TimestampMillis::now().as_millis();
    let agent = body.agent_user_id.to_string();
    let tokens = body.tokens.max(0);
    let result: Result<Option<bool>, sqlx::Error> = async {
        let mut tx = state.database().immediate_transaction().await?;
        let custom = sqlx::query_scalar::<_, Flag>(
            "SELECT EXISTS (SELECT 1 FROM agent_profiles WHERE user_id = $1)",
        )
        .bind(&agent)
        .fetch_one(&mut *tx)
        .await?;
        if !custom.0 {
            return Ok(None);
        }
        let written = sqlx::query(
            "INSERT INTO agent_run_usage (run_id, agent_user_id, month, tokens, created_at) VALUES ($1, $2, $3, $4, $5) \
             ON CONFLICT (run_id) DO NOTHING",
        )
        .bind(run_id.to_string())
        .bind(&agent)
        .bind(month(now))
        .bind(tokens)
        .bind(now)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if written == 1 {
            sqlx::query(
                "INSERT INTO agent_sandbox_usage (agent_user_id, month, model_tokens) VALUES ($1, $2, $3) \
                 ON CONFLICT (agent_user_id, month) DO UPDATE SET model_tokens = agent_sandbox_usage.model_tokens + $3",
            )
            .bind(&agent)
            .bind(month(now))
            .bind(tokens)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(Some(written == 1))
    }
    .await;
    match result {
        Ok(Some(counted)) => {
            if counted {
                send_budget_notices(&state, &agent, now).await;
            }
            Ok(Json(json!({ "counted": counted })).into_response())
        }
        Ok(None) => Err(error(
            StatusCode::NOT_FOUND,
            "not_found",
            "No such custom agent.",
            &uri,
            &request_id,
        )),
        Err(failure) => Err(unavailable(&failure, &uri, &request_id)),
    }
}

// ---------------------------------------------------------------------------------------------
// The card's answer.

#[derive(Deserialize)]
struct AnswerBody {
    /// `allow_once`, `allow_always` or `deny`.
    answer: String,
    /// With `deny`: also a deny rule for this agent (owners and admins).
    #[serde(default)]
    always: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Answer {
    Once,
    Always,
    Deny,
}

/// Why an answer is refused.
#[derive(Debug, Eq, PartialEq)]
enum AnswerRefused {
    NotFound,
    Answered,
    Expired,
    DriverOnly,
    AdminOnly,
    DriverOrAdmin,
    Risky,
}

/// Who may give `answer` (section 4, the card): Allow once only the driver; Allow always and a deny rule only
/// owners and admins, and never for a risky host; Deny the driver or an owner/admin.
fn may_answer(
    answer: Answer,
    always: bool,
    is_driver: bool,
    is_admin: bool,
    host: &str,
) -> Result<(), AnswerRefused> {
    match answer {
        Answer::Once if !is_driver => Err(AnswerRefused::DriverOnly),
        Answer::Always if !is_admin => Err(AnswerRefused::AdminOnly),
        Answer::Always if is_risky(host) => Err(AnswerRefused::Risky),
        Answer::Deny if !(is_driver || is_admin) => Err(AnswerRefused::DriverOrAdmin),
        Answer::Deny if always && !is_admin => Err(AnswerRefused::AdminOnly),
        _ => Ok(()),
    }
}

async fn answer_egress(
    State(state): State<AgentState>,
    Path((workspace_id, egress_request)): Path<(String, String)>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
    Json(body): Json<AnswerBody>,
) -> Result<Response, ApiError> {
    keys(&state, &uri, &request_id)?;
    let session = authenticate_session(
        &state.identity,
        state.cookie_mode,
        &headers,
        uri.path(),
        request_id.as_ref(),
    )
    .await?;
    let not_found = || {
        error(
            StatusCode::NOT_FOUND,
            "not_found",
            "Not found.",
            &uri,
            &request_id,
        )
    };
    let (Ok(workspace_id), Ok(egress_request)) =
        (workspace_id.parse::<Id>(), egress_request.parse::<Id>())
    else {
        return Err(not_found());
    };
    let answer = match body.answer.as_str() {
        "allow_once" => Answer::Once,
        "allow_always" => Answer::Always,
        "deny" => Answer::Deny,
        _ => {
            return Err(error(
                StatusCode::UNPROCESSABLE_ENTITY,
                "invalid_answer",
                "The answer is allow_once, allow_always or deny.",
                &uri,
                &request_id,
            ));
        }
    };
    let always = body.always && answer == Answer::Deny;
    let audit_request = request_id_value(request_id.as_ref()).to_owned();
    let result = answer_request(
        &state,
        workspace_id,
        session.user.id,
        egress_request,
        answer,
        always,
        &audit_request,
    )
    .await;
    let refused = |status, code, detail| Err(error(status, code, detail, &uri, &request_id));
    match result {
        Ok(Ok(view)) => Ok(Json(view).into_response()),
        Ok(Err(AnswerRefused::NotFound)) => Err(not_found()),
        Ok(Err(AnswerRefused::Answered)) => refused(
            StatusCode::CONFLICT,
            "egress_request_answered",
            "This request already has an answer.",
        ),
        Ok(Err(AnswerRefused::Expired)) => refused(
            StatusCode::GONE,
            "egress_request_expired",
            "This request waited too long. The agent asks again at its next try.",
        ),
        Ok(Err(AnswerRefused::DriverOnly)) => refused(
            StatusCode::FORBIDDEN,
            "egress_driver_only",
            "Only the person the agent works for in this thread can allow once.",
        ),
        Ok(Err(AnswerRefused::AdminOnly)) => refused(
            StatusCode::FORBIDDEN,
            "egress_admin_only",
            "Ask an admin: only workspace owners and admins can make a rule.",
        ),
        Ok(Err(AnswerRefused::DriverOrAdmin)) => refused(
            StatusCode::FORBIDDEN,
            "egress_driver_or_admin",
            "Only the person the agent works for, or an admin, can deny.",
        ),
        Ok(Err(AnswerRefused::Risky)) => refused(
            StatusCode::CONFLICT,
            "egress_risky_host",
            "This host always asks: it cannot be allowed always.",
        ),
        Err(failure) => Err(unavailable(&failure, &uri, &request_id)),
    }
}

/// Records the answer, a rule when it makes one, the audit row, and the continuation for the original driver.
async fn answer_request(
    state: &AgentState,
    workspace_id: Id,
    user: Id,
    egress_request: Id,
    answer: Answer,
    always: bool,
    audit_request: &str,
) -> Result<Result<Value, AnswerRefused>, sqlx::Error> {
    let now = TimestampMillis::now();
    let mut tx = state.database().immediate_transaction().await?;
    // Guests and people outside the workspace see nothing here.
    let Some(actor) = membership::actor(&mut *tx, workspace_id, user).await? else {
        return Ok(Err(AnswerRefused::NotFound));
    };
    let Some(row) = sqlx::query(
        "SELECT agent_user_id, driver_user_id, thread_key, host, port, status, expires_at \
         FROM agent_egress_requests WHERE id = $1 AND workspace_id = $2",
    )
    .bind(egress_request.to_string())
    .bind(workspace_id.to_string())
    .fetch_optional(&mut *tx)
    .await?
    else {
        return Ok(Err(AnswerRefused::NotFound));
    };
    if row.get::<String, _>("status") != "pending" {
        return Ok(Err(AnswerRefused::Answered));
    }
    if row
        .get::<Option<i64>, _>("expires_at")
        .is_some_and(|expires| expires <= now.as_millis())
    {
        return Ok(Err(AnswerRefused::Expired));
    }
    let agent: String = row.get("agent_user_id");
    let driver: String = row.get("driver_user_id");
    let thread_key: String = row.get("thread_key");
    let host: String = row.get("host");
    let port: i64 = row.get("port");
    let is_driver = driver == user.to_string();
    if let Err(refused) = may_answer(
        answer,
        always,
        is_driver,
        actor.can(Permission::WorkspaceUpdate),
        &host,
    ) {
        return Ok(Err(refused));
    }

    // Allowed (once, or with the new rule) for this thread and driver for ALLOW_ONCE_MS; denied here for good.
    let (status, expires_at) = match answer {
        Answer::Once | Answer::Always => ("allowed", Some(now.as_millis() + ALLOW_ONCE_MS)),
        Answer::Deny => ("denied", None),
    };
    sqlx::query(
        "UPDATE agent_egress_requests SET status = $2, decided_by = $3, decided_at = $4, expires_at = $5 \
         WHERE id = $1",
    )
    .bind(egress_request.to_string())
    .bind(status)
    .bind(user.to_string())
    .bind(now.as_millis())
    .bind(expires_at)
    .execute(&mut *tx)
    .await?;
    let rule = match answer {
        Answer::Always => Some("allow"),
        Answer::Deny if always => Some("deny"),
        _ => None,
    };
    if let Some(action) = rule {
        sqlx::query(
            "INSERT INTO agent_egress_rules (id, workspace_id, agent_user_id, host, port, action, created_by, \
             created_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8) ON CONFLICT (agent_user_id, host, port) \
             DO UPDATE SET action = excluded.action, created_by = excluded.created_by, created_at = excluded.created_at",
        )
        .bind(Id::new_v7().to_string())
        .bind(workspace_id.to_string())
        .bind(&agent)
        .bind(&host)
        .bind(port)
        .bind(action)
        .bind(user.to_string())
        .bind(now.as_millis())
        .execute(&mut *tx)
        .await?;
    }
    let event = match (answer, rule) {
        (_, Some(_)) => "agent_egress.rule_create",
        (Answer::Once, None) => "agent_egress.allow_once",
        _ => "agent_egress.deny",
    };
    audit::record(
        &mut tx,
        workspace_id,
        Some(user),
        event,
        AuditOutcome::Success,
        "agent_egress_request",
        Some(egress_request),
        audit_request,
        json!({ "agent_user_id": agent, "host": host, "port": port, "rule": rule }),
        now,
    )
    .await?;

    // The run goes on as the original driver's, whoever clicked (they never become the driver).
    let thread = sqlx::query(
        "SELECT conversation_id, root_message_id FROM agent_threads WHERE thread_key = $1 AND workspace_id = $2",
    )
    .bind(&thread_key)
    .bind(workspace_id.to_string())
    .fetch_optional(&mut *tx)
    .await?;
    if let Some(thread) = thread {
        let by: String = sqlx::query_scalar("SELECT display_name FROM users WHERE id = $1")
            .bind(user.to_string())
            .fetch_one(&mut *tx)
            .await?;
        let answer_name = match (answer, rule) {
            (Answer::Once, _) => "allow_once",
            (Answer::Always, _) => "allow_always",
            (Answer::Deny, Some(_)) => "deny_always",
            (Answer::Deny, None) => "deny",
        };
        let action = json!({
            "type": "egress",
            "answer": answer_name,
            "host": host,
            "port": port,
            "by": by,
            "by_driver": is_driver,
            "request_id": egress_request.to_string(),
        });
        sqlx::query(
            "INSERT INTO agent_inputs (message_id, workspace_id, conversation_id, thread_key, root_message_id, \
             author_id, kind, created_at, action, agent_user_id) VALUES ($1, $2, $3, $4, $5, $6, 'action', $7, $8, $9)",
        )
        .bind(Id::new_v7().to_string())
        .bind(workspace_id.to_string())
        .bind(thread.get::<String, _>("conversation_id"))
        .bind(&thread_key)
        .bind(thread.get::<Option<String>, _>("root_message_id"))
        .bind(&driver)
        .bind(now.as_millis())
        .bind(action.to_string())
        .bind(&agent)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(Ok(json!({
        "status": status,
        "host": host,
        "port": port,
        "expires_at": expires_at,
        "rule": rule,
    })))
}

// ---------------------------------------------------------------------------------------------
// Cards in a conversation.

/// What the viewer may answer on a pending card: Allow once, Allow always, Deny (the same rules as [`may_answer`]).
fn viewer_may(is_driver: bool, is_admin: bool, host: &str) -> (bool, bool, bool) {
    let may = |answer| may_answer(answer, false, is_driver, is_admin, host).is_ok();
    (may(Answer::Once), may(Answer::Always), may(Answer::Deny))
}

/// A request's state as people see it: `pending`, `expired` (nobody answered in time), `allowed` or `denied`.
fn shown_status(status: &str, expires_at: Option<i64>, now: i64) -> &str {
    match status {
        "pending" if expires_at.is_some_and(|expires| expires <= now) => "expired",
        other => other,
    }
}

#[derive(Deserialize)]
struct ConversationQuery {
    conversation_id: String,
}

/// The caller as a member of the workspace (guests and outsiders: `None`) and whether they are an owner or admin.
async fn member_of(
    state: &AgentState,
    workspace_id: Id,
    user: Id,
) -> Result<Option<bool>, sqlx::Error> {
    let mut conn = state.database().pool().acquire().await?;
    Ok(membership::actor(&mut *conn, workspace_id, user)
        .await?
        .map(|actor| actor.can(Permission::WorkspaceUpdate)))
}

async fn conversation_requests(
    State(state): State<AgentState>,
    Path(workspace_id): Path<String>,
    Query(query): Query<ConversationQuery>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
) -> Result<Response, ApiError> {
    keys(&state, &uri, &request_id)?;
    let session = authenticate_session(
        &state.identity,
        state.cookie_mode,
        &headers,
        uri.path(),
        request_id.as_ref(),
    )
    .await?;
    let not_found = || {
        error(
            StatusCode::NOT_FOUND,
            "not_found",
            "Not found.",
            &uri,
            &request_id,
        )
    };
    let (Ok(workspace_id), Ok(conversation_id)) = (
        workspace_id.parse::<Id>(),
        query.conversation_id.parse::<Id>(),
    ) else {
        return Err(not_found());
    };
    let user = session.user.id;
    let result: Result<Option<Vec<Value>>, sqlx::Error> = async {
        let Some(is_admin) = member_of(&state, workspace_id, user).await? else {
            return Ok(None);
        };
        let Flag(readable) = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT EXISTS (SELECT 1 FROM chat_conversations c WHERE c.id = $1 AND c.workspace_id = $2 \
             AND (c.kind = 'public' OR EXISTS (SELECT 1 FROM chat_members cm WHERE cm.conversation_id = c.id \
             AND cm.user_id = $3)) AND {})",
            crate::repositories::chat::reachable_dm()
        )))
        .bind(conversation_id.to_string())
        .bind(workspace_id.to_string())
        .bind(user.to_string())
        .fetch_one(state.database().pool())
        .await?;
        if !readable {
            return Ok(None);
        }
        let now = TimestampMillis::now().as_millis();
        // A request belongs to the card of its latest call (a later call that asks the same host takes it over).
        let rows = sqlx::query(
            "SELECT r.id, r.host, r.port, r.status, r.driver_user_id, r.expires_at, r.decided_at, \
             m.id AS message_id, driver.display_name AS driver_name, decider.display_name AS decided_by_name, \
             (SELECT ru.action FROM agent_egress_rules ru WHERE ru.agent_user_id = r.agent_user_id \
              AND ru.host = r.host AND ru.port = r.port) AS rule \
             FROM agent_egress_requests r \
             JOIN agent_threads t ON t.thread_key = r.thread_key \
             JOIN chat_messages m ON m.conversation_id = t.conversation_id AND m.author_id = r.agent_user_id \
              AND m.nonce IS NOT NULL AND m.nonce = 'egress:' || r.call_id \
             JOIN users driver ON driver.id = r.driver_user_id \
             LEFT JOIN users decider ON decider.id = r.decided_by \
             WHERE r.workspace_id = $1 AND t.conversation_id = $2 \
             ORDER BY r.created_at DESC, r.id LIMIT 200",
        )
        .bind(workspace_id.to_string())
        .bind(conversation_id.to_string())
        .fetch_all(state.database().pool())
        .await?;
        Ok(Some(
            rows.iter()
                .map(|row| {
                    let host: String = row.get("host");
                    let expires_at: Option<i64> = row.get("expires_at");
                    let status = shown_status(&row.get::<String, _>("status"), expires_at, now)
                        .to_owned();
                    let is_driver = row.get::<String, _>("driver_user_id") == user.to_string();
                    let (once, always, deny) = if status == "pending" {
                        viewer_may(is_driver, is_admin, &host)
                    } else {
                        (false, false, false)
                    };
                    json!({
                        "id": row.get::<String, _>("id"),
                        "message_id": row.get::<String, _>("message_id"),
                        "risky": is_risky(&host),
                        "host": host,
                        "port": row.get::<i64, _>("port"),
                        "status": status,
                        "expires_at": expires_at,
                        "driver_id": row.get::<String, _>("driver_user_id"),
                        "driver_name": row.get::<String, _>("driver_name"),
                        "decided_by_name": row.get::<Option<String>, _>("decided_by_name"),
                        "decided_at": row.get::<Option<i64>, _>("decided_at"),
                        "rule": row.get::<Option<String>, _>("rule"),
                        "can_allow_once": once,
                        "can_allow_always": always,
                        "can_deny": deny,
                    })
                })
                .collect(),
        ))
    }
    .await;
    match result {
        Ok(Some(requests)) => Ok(Json(json!({ "requests": requests })).into_response()),
        Ok(None) => Err(not_found()),
        Err(failure) => Err(unavailable(&failure, &uri, &request_id)),
    }
}

// ---------------------------------------------------------------------------------------------
// The agent card's sandbox settings and the Internet access page (owners and admins).

/// Defaults of a settings row (the migration's), for an agent that has none yet.
const DEFAULT_IDLE_MINUTES: i64 = 30;
const DEFAULT_CPU_MINUTES: i64 = 600;
const DEFAULT_EGRESS_BYTES: i64 = 2 * 1024 * 1024 * 1024;
const DEFAULT_MODEL_TOKENS: i64 = 20_000_000;
/// Upper bounds an admin may set (a typo should not give a sandbox the whole host for a month).
const MAX_CPU_MINUTES: i64 = 100_000;
const MAX_EGRESS_BYTES: i64 = 1024 * 1024 * 1024 * 1024;
const MAX_MODEL_TOKENS: i64 = 10_000_000_000;
const LOG_ROWS: i64 = 100;

/// Why a manager route refuses.
enum ManageRefused {
    NotFound,
    Forbidden,
}

/// The agent's user id when the caller is an owner or admin of the workspace and `agent` is one of its live custom
/// agents (by profile id). Guests and outsiders: not found; members: forbidden.
async fn managed_agent(
    conn: &mut sqlx::AnyConnection,
    workspace_id: Id,
    user: Id,
    agent: &str,
) -> Result<Result<String, ManageRefused>, sqlx::Error> {
    let Some(actor) = membership::actor(&mut *conn, workspace_id, user).await? else {
        return Ok(Err(ManageRefused::NotFound));
    };
    let Some(agent_user) = sqlx::query_scalar::<_, String>(
        "SELECT user_id FROM agent_profiles WHERE id = $1 AND workspace_id = $2 AND deleted_at IS NULL",
    )
    .bind(agent)
    .bind(workspace_id.to_string())
    .fetch_optional(&mut *conn)
    .await?
    else {
        return Ok(Err(ManageRefused::NotFound));
    };
    if !actor.can(Permission::WorkspaceUpdate) {
        return Ok(Err(ManageRefused::Forbidden));
    }
    Ok(Ok(agent_user))
}

fn manage_refused(refused: &ManageRefused, uri: &Uri, request_id: &RequestIdExtension) -> ApiError {
    match refused {
        ManageRefused::NotFound => error(
            StatusCode::NOT_FOUND,
            "not_found",
            "Not found.",
            uri,
            request_id,
        ),
        ManageRefused::Forbidden => error(
            StatusCode::FORBIDDEN,
            "forbidden",
            "Only owners and admins can manage an agent's sandbox.",
            uri,
            request_id,
        ),
    }
}

/// The settings, this month's use, the rules, the pending requests and the latest connections of one agent.
async fn sandbox_view(
    conn: &mut sqlx::AnyConnection,
    agent_user: &str,
    viewer: Id,
) -> Result<Value, sqlx::Error> {
    let now = TimestampMillis::now().as_millis();
    let settings = sqlx::query(
        "SELECT enabled, idle_minutes, cpu_minutes_month, egress_bytes_month, model_tokens_month, updated_at \
         FROM agent_sandbox_settings WHERE agent_user_id = $1",
    )
    .bind(agent_user)
    .fetch_optional(&mut *conn)
    .await?;
    let settings = match settings {
        Some(row) => json!({
            "enabled": row.get::<i64, _>("enabled") == 1,
            "idle_minutes": row.get::<i64, _>("idle_minutes"),
            "cpu_minutes_month": row.get::<i64, _>("cpu_minutes_month"),
            "egress_bytes_month": row.get::<i64, _>("egress_bytes_month"),
            "model_tokens_month": row.get::<i64, _>("model_tokens_month"),
            "updated_at": row.get::<i64, _>("updated_at"),
        }),
        None => json!({
            "enabled": false,
            "idle_minutes": DEFAULT_IDLE_MINUTES,
            "cpu_minutes_month": DEFAULT_CPU_MINUTES,
            "egress_bytes_month": DEFAULT_EGRESS_BYTES,
            "model_tokens_month": DEFAULT_MODEL_TOKENS,
            "updated_at": null,
        }),
    };
    let month = month(now);
    let usage = sqlx::query(
        "SELECT cpu_ms, egress_bytes, model_tokens FROM agent_sandbox_usage WHERE agent_user_id = $1 AND month = $2",
    )
    .bind(agent_user)
    .bind(&month)
    .fetch_optional(&mut *conn)
    .await?;
    let used = |column: &str| usage.as_ref().map_or(0, |row| row.get::<i64, _>(column));
    let rules: Vec<Value> = sqlx::query(
        "SELECT r.id, r.host, r.port, r.action, r.created_at, r.last_used_at, u.display_name AS created_by_name \
         FROM agent_egress_rules r LEFT JOIN users u ON u.id = r.created_by \
         WHERE r.agent_user_id = $1 ORDER BY r.host, r.port",
    )
    .bind(agent_user)
    .fetch_all(&mut *conn)
    .await?
    .iter()
    .map(|row| {
        json!({
            "id": row.get::<String, _>("id"),
            "host": row.get::<String, _>("host"),
            "port": row.get::<i64, _>("port"),
            "action": row.get::<String, _>("action"),
            "created_by_name": row.get::<Option<String>, _>("created_by_name"),
            "created_at": row.get::<i64, _>("created_at"),
            "last_used_at": row.get::<Option<i64>, _>("last_used_at"),
        })
    })
    .collect();
    let pending: Vec<Value> = sqlx::query(
        "SELECT r.id, r.host, r.port, r.driver_user_id, r.created_at, r.expires_at, u.display_name AS driver_name \
         FROM agent_egress_requests r JOIN users u ON u.id = r.driver_user_id \
         WHERE r.agent_user_id = $1 AND r.status = 'pending' AND (r.expires_at IS NULL OR r.expires_at > $2) \
         ORDER BY r.created_at DESC, r.id",
    )
    .bind(agent_user)
    .bind(now)
    .fetch_all(&mut *conn)
    .await?
    .iter()
    .map(|row| {
        let host: String = row.get("host");
        let is_driver = row.get::<String, _>("driver_user_id") == viewer.to_string();
        let (once, always, deny) = viewer_may(is_driver, true, &host);
        json!({
            "id": row.get::<String, _>("id"),
            "risky": is_risky(&host),
            "host": host,
            "port": row.get::<i64, _>("port"),
            "driver_name": row.get::<String, _>("driver_name"),
            "created_at": row.get::<i64, _>("created_at"),
            "expires_at": row.get::<Option<i64>, _>("expires_at"),
            "can_allow_once": once,
            "can_allow_always": always,
            "can_deny": deny,
        })
    })
    .collect();
    let log: Vec<Value> = sqlx::query(
        "SELECT l.id, l.host, l.port, l.decision, l.reason, l.bytes_up, l.bytes_down, l.created_at, \
         u.display_name AS driver_name FROM agent_egress_log l LEFT JOIN users u ON u.id = l.driver_user_id \
         WHERE l.agent_user_id = $1 ORDER BY l.created_at DESC, l.id DESC LIMIT $2",
    )
    .bind(agent_user)
    .bind(LOG_ROWS)
    .fetch_all(&mut *conn)
    .await?
    .iter()
    .map(|row| {
        json!({
            "id": row.get::<String, _>("id"),
            "host": row.get::<String, _>("host"),
            "port": row.get::<i64, _>("port"),
            "decision": row.get::<String, _>("decision"),
            "reason": row.get::<String, _>("reason"),
            "bytes_up": row.get::<i64, _>("bytes_up"),
            "bytes_down": row.get::<i64, _>("bytes_down"),
            "driver_name": row.get::<Option<String>, _>("driver_name"),
            "created_at": row.get::<i64, _>("created_at"),
        })
    })
    .collect();
    Ok(json!({
        "settings": settings,
        "usage": {
            "month": month,
            "cpu_ms": used("cpu_ms"),
            "egress_bytes": used("egress_bytes"),
            "model_tokens": used("model_tokens"),
        },
        "rules": rules,
        "pending": pending,
        "log": log,
    }))
}

async fn read_sandbox(
    State(state): State<AgentState>,
    Path((workspace_id, agent)): Path<(String, String)>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
) -> Result<Response, ApiError> {
    keys(&state, &uri, &request_id)?;
    let session = authenticate_session(
        &state.identity,
        state.cookie_mode,
        &headers,
        uri.path(),
        request_id.as_ref(),
    )
    .await?;
    let workspace_id: Id = workspace_id
        .parse()
        .map_err(|_| manage_refused(&ManageRefused::NotFound, &uri, &request_id))?;
    let result: Result<Result<Value, ManageRefused>, sqlx::Error> = async {
        let mut conn = state.database().pool().acquire().await?;
        let agent_user =
            match managed_agent(&mut conn, workspace_id, session.user.id, &agent).await? {
                Ok(agent_user) => agent_user,
                Err(refused) => return Ok(Err(refused)),
            };
        Ok(Ok(
            sandbox_view(&mut conn, &agent_user, session.user.id).await?
        ))
    }
    .await;
    match result {
        Ok(Ok(view)) => Ok(Json(view).into_response()),
        Ok(Err(refused)) => Err(manage_refused(&refused, &uri, &request_id)),
        Err(failure) => Err(unavailable(&failure, &uri, &request_id)),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SettingsBody {
    enabled: bool,
    idle_minutes: i64,
    cpu_minutes_month: i64,
    egress_bytes_month: i64,
    model_tokens_month: i64,
}

/// The settings' limits, or what is wrong with them.
fn check_settings(body: &SettingsBody) -> Result<(), &'static str> {
    if !(5..=120).contains(&body.idle_minutes) {
        return Err("The idle time must be 5 to 120 minutes.");
    }
    if !(0..=MAX_CPU_MINUTES).contains(&body.cpu_minutes_month) {
        return Err("The CPU minutes per month must be 0 to 100,000.");
    }
    if !(0..=MAX_EGRESS_BYTES).contains(&body.egress_bytes_month) {
        return Err("The internet traffic per month must be 0 to 1 TiB.");
    }
    if !(0..=MAX_MODEL_TOKENS).contains(&body.model_tokens_month) {
        return Err("The model tokens per month must be 0 to 10 billion.");
    }
    Ok(())
}

async fn save_sandbox(
    State(state): State<AgentState>,
    Path((workspace_id, agent)): Path<(String, String)>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
    Json(body): Json<SettingsBody>,
) -> Result<Response, ApiError> {
    keys(&state, &uri, &request_id)?;
    let session = authenticate_session(
        &state.identity,
        state.cookie_mode,
        &headers,
        uri.path(),
        request_id.as_ref(),
    )
    .await?;
    let workspace_id: Id = workspace_id
        .parse()
        .map_err(|_| manage_refused(&ManageRefused::NotFound, &uri, &request_id))?;
    if let Err(detail) = check_settings(&body) {
        return Err(error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "validation_failed",
            detail,
            &uri,
            &request_id,
        ));
    }
    let user = session.user.id;
    let audit_request = request_id_value(request_id.as_ref()).to_owned();
    let result: Result<Result<Value, ManageRefused>, sqlx::Error> = async {
        let now = TimestampMillis::now();
        let mut tx = state.database().immediate_transaction().await?;
        let agent_user = match managed_agent(&mut tx, workspace_id, user, &agent).await? {
            Ok(agent_user) => agent_user,
            Err(refused) => return Ok(Err(refused)),
        };
        sqlx::query(
            "INSERT INTO agent_sandbox_settings (agent_user_id, workspace_id, enabled, idle_minutes, cpu_minutes_month, \
             egress_bytes_month, model_tokens_month, updated_by, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) \
             ON CONFLICT (agent_user_id) DO UPDATE SET enabled = excluded.enabled, idle_minutes = excluded.idle_minutes, \
             cpu_minutes_month = excluded.cpu_minutes_month, egress_bytes_month = excluded.egress_bytes_month, \
             model_tokens_month = excluded.model_tokens_month, updated_by = excluded.updated_by, \
             updated_at = excluded.updated_at",
        )
        .bind(&agent_user)
        .bind(workspace_id.to_string())
        .bind(i64::from(body.enabled))
        .bind(body.idle_minutes)
        .bind(body.cpu_minutes_month)
        .bind(body.egress_bytes_month)
        .bind(body.model_tokens_month)
        .bind(user.to_string())
        .bind(now.as_millis())
        .execute(&mut *tx)
        .await?;
        if !body.enabled
            && let Ok(agent_id) = agent_user.parse::<Id>()
        {
            switch_off(&mut tx, agent_id, "the sandbox was switched off", now.as_millis()).await?;
        }
        audit::record(
            &mut tx,
            workspace_id,
            Some(user),
            "agent_sandbox.settings_update",
            AuditOutcome::Success,
            "agent",
            agent_user.parse().ok(),
            &audit_request,
            json!({
                "enabled": body.enabled,
                "idle_minutes": body.idle_minutes,
                "cpu_minutes_month": body.cpu_minutes_month,
                "egress_bytes_month": body.egress_bytes_month,
                "model_tokens_month": body.model_tokens_month,
            }),
            now,
        )
        .await?;
        let view = sandbox_view(&mut tx, &agent_user, user).await?;
        tx.commit().await?;
        Ok(Ok(view))
    }
    .await;
    match result {
        Ok(Ok(view)) => Ok(Json(view).into_response()),
        Ok(Err(refused)) => Err(manage_refused(&refused, &uri, &request_id)),
        Err(failure) => Err(unavailable(&failure, &uri, &request_id)),
    }
}

/// A rule's host as stored (`api.example.com` or `*.example.com`), or why it is refused (section 4, the rules):
/// `*`, IP literals, `orbit.internal`, Orbit's own host (also under a wildcard), a public suffix or a wildcard on one
/// (the Public Suffix List, private part included: `*.co.uk`, `*.github.io`), and allow rules for risky hosts.
pub fn check_rule_host(
    raw: &str,
    allow: bool,
    own_host: Option<&str>,
) -> Result<String, &'static str> {
    let raw = raw.trim().to_ascii_lowercase();
    let (wildcard, base) = match raw.strip_prefix("*.") {
        Some(base) => (true, base),
        None => (false, raw.as_str()),
    };
    if base.contains('*') || raw == "*" {
        return Err("A wildcard is only allowed as `*.` at the start, for example *.example.com.");
    }
    let Some(base) = normalize_host(base) else {
        return Err("Not a host name. IP addresses are never allowed.");
    };
    let rule = if wildcard {
        format!("*.{base}")
    } else {
        base.clone()
    };
    let orbit = |host: &str| host == base || rule_matches(&rule, host);
    if orbit("orbit.internal") || own_host.is_some_and(orbit) {
        return Err("Orbit is reached through orbit.internal only: no rule can name it.");
    }
    if psl::suffix_str(&base).is_none_or(|suffix| suffix == base) {
        return Err(
            "This is a public suffix (like com, co.uk or github.io): name a host or domain under it.",
        );
    }
    if allow && (is_risky(&base) || (wildcard && is_risky(&format!("x.{base}")))) {
        return Err("This host always asks: it cannot be allowed always.");
    }
    Ok(rule)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RuleBody {
    host: String,
    #[serde(default = "https_port")]
    port: i64,
    /// `allow` or `deny`.
    action: String,
}

const fn https_port() -> i64 {
    443
}

async fn add_rule(
    State(state): State<AgentState>,
    Path((workspace_id, agent)): Path<(String, String)>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
    Json(body): Json<RuleBody>,
) -> Result<Response, ApiError> {
    keys(&state, &uri, &request_id)?;
    let session = authenticate_session(
        &state.identity,
        state.cookie_mode,
        &headers,
        uri.path(),
        request_id.as_ref(),
    )
    .await?;
    let workspace_id: Id = workspace_id
        .parse()
        .map_err(|_| manage_refused(&ManageRefused::NotFound, &uri, &request_id))?;
    let invalid = |detail| {
        error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "validation_failed",
            detail,
            &uri,
            &request_id,
        )
    };
    let allow = match body.action.as_str() {
        "allow" => true,
        "deny" => false,
        _ => return Err(invalid("The action is allow or deny.")),
    };
    if !(1..=65_535).contains(&body.port) {
        return Err(invalid("The port must be 1 to 65535."));
    }
    let own_host = state
        .public_origin
        .as_deref()
        .and_then(|origin| url::Url::parse(origin).ok())
        .and_then(|origin| origin.host_str().map(str::to_owned));
    // Check the rule only for people who may make one: a member learns nothing from the answer.
    let checked = check_rule_host(&body.host, allow, own_host.as_deref());
    let user = session.user.id;
    let audit_request = request_id_value(request_id.as_ref()).to_owned();
    let result: Result<Result<Result<Value, &'static str>, ManageRefused>, sqlx::Error> = async {
        let now = TimestampMillis::now();
        let mut tx = state.database().immediate_transaction().await?;
        let agent_user = match managed_agent(&mut tx, workspace_id, user, &agent).await? {
            Ok(agent_user) => agent_user,
            Err(refused) => return Ok(Err(refused)),
        };
        let host = match checked {
            Ok(host) => host,
            Err(detail) => return Ok(Ok(Err(detail))),
        };
        sqlx::query(
            "INSERT INTO agent_egress_rules (id, workspace_id, agent_user_id, host, port, action, created_by, \
             created_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8) ON CONFLICT (agent_user_id, host, port) \
             DO UPDATE SET action = excluded.action, created_by = excluded.created_by, created_at = excluded.created_at",
        )
        .bind(Id::new_v7().to_string())
        .bind(workspace_id.to_string())
        .bind(&agent_user)
        .bind(&host)
        .bind(body.port)
        .bind(&body.action)
        .bind(user.to_string())
        .bind(now.as_millis())
        .execute(&mut *tx)
        .await?;
        audit::record(
            &mut tx,
            workspace_id,
            Some(user),
            "agent_egress.rule_create",
            AuditOutcome::Success,
            "agent",
            agent_user.parse().ok(),
            &audit_request,
            json!({ "agent_user_id": agent_user, "host": host, "port": body.port, "rule": body.action }),
            now,
        )
        .await?;
        let view = sandbox_view(&mut tx, &agent_user, user).await?;
        tx.commit().await?;
        Ok(Ok(Ok(view)))
    }
    .await;
    match result {
        Ok(Ok(Ok(view))) => Ok((StatusCode::CREATED, Json(view)).into_response()),
        Ok(Ok(Err(detail))) => Err(invalid(detail)),
        Ok(Err(refused)) => Err(manage_refused(&refused, &uri, &request_id)),
        Err(failure) => Err(unavailable(&failure, &uri, &request_id)),
    }
}

async fn delete_rule(
    State(state): State<AgentState>,
    Path((workspace_id, agent, rule)): Path<(String, String, String)>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
) -> Result<Response, ApiError> {
    keys(&state, &uri, &request_id)?;
    let session = authenticate_session(
        &state.identity,
        state.cookie_mode,
        &headers,
        uri.path(),
        request_id.as_ref(),
    )
    .await?;
    let workspace_id: Id = workspace_id
        .parse()
        .map_err(|_| manage_refused(&ManageRefused::NotFound, &uri, &request_id))?;
    let user = session.user.id;
    let audit_request = request_id_value(request_id.as_ref()).to_owned();
    let result: Result<Result<Value, ManageRefused>, sqlx::Error> = async {
        let now = TimestampMillis::now();
        let mut tx = state.database().immediate_transaction().await?;
        let agent_user = match managed_agent(&mut tx, workspace_id, user, &agent).await? {
            Ok(agent_user) => agent_user,
            Err(refused) => return Ok(Err(refused)),
        };
        let Some(removed) = sqlx::query(
            "DELETE FROM agent_egress_rules WHERE id = $1 AND agent_user_id = $2 RETURNING host, port, action",
        )
        .bind(&rule)
        .bind(&agent_user)
        .fetch_optional(&mut *tx)
        .await?
        else {
            return Ok(Err(ManageRefused::NotFound));
        };
        audit::record(
            &mut tx,
            workspace_id,
            Some(user),
            "agent_egress.rule_delete",
            AuditOutcome::Success,
            "agent",
            agent_user.parse().ok(),
            &audit_request,
            json!({
                "agent_user_id": agent_user,
                "host": removed.get::<String, _>("host"),
                "port": removed.get::<i64, _>("port"),
                "rule": removed.get::<String, _>("action"),
            }),
            now,
        )
        .await?;
        let view = sandbox_view(&mut tx, &agent_user, user).await?;
        tx.commit().await?;
        Ok(Ok(view))
    }
    .await;
    match result {
        Ok(Ok(view)) => Ok(Json(view).into_response()),
        Ok(Err(refused)) => Err(manage_refused(&refused, &uri, &request_id)),
        Err(failure) => Err(unavailable(&failure, &uri, &request_id)),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Answer, AnswerRefused, SandboxKeys, check_rule_host, is_risky, may_answer, normalize_host,
        rule_matches, shown_status, viewer_may,
    };

    #[test]
    fn only_the_driver_allows_once_and_only_admins_make_rules() {
        let host = "api.example.com";
        // (answer, always, driver, admin) → result
        let cases = [
            (Answer::Once, false, true, false, Ok(())),
            (
                Answer::Once,
                false,
                false,
                true,
                Err(AnswerRefused::DriverOnly),
            ),
            (Answer::Always, false, false, true, Ok(())),
            (
                Answer::Always,
                false,
                true,
                false,
                Err(AnswerRefused::AdminOnly),
            ),
            (Answer::Deny, false, true, false, Ok(())),
            (Answer::Deny, false, false, true, Ok(())),
            (
                Answer::Deny,
                false,
                false,
                false,
                Err(AnswerRefused::DriverOrAdmin),
            ),
            (
                Answer::Deny,
                true,
                true,
                false,
                Err(AnswerRefused::AdminOnly),
            ),
            (Answer::Deny, true, false, true, Ok(())),
        ];
        for (answer, always, driver, admin, expected) in cases {
            assert_eq!(
                may_answer(answer, always, driver, admin, host),
                expected,
                "{answer:?} always={always} driver={driver} admin={admin}"
            );
        }
        // A risky host never gets an allow rule, not even from an admin who drives; once is fine.
        assert_eq!(
            may_answer(Answer::Always, false, true, true, "x.ngrok.io"),
            Err(AnswerRefused::Risky)
        );
        assert_eq!(
            may_answer(Answer::Once, false, true, true, "x.ngrok.io"),
            Ok(())
        );
    }

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

    #[test]
    fn rules_refuse_ips_public_suffix_wildcards_orbit_and_risky_allows() {
        let own = Some("orbit.example.com");
        let ok = |raw: &str, allow: bool| check_rule_host(raw, allow, own);
        assert_eq!(
            ok(" API.Example.org. ", true).as_deref(),
            Ok("api.example.org")
        );
        assert_eq!(ok("*.example.org", true).as_deref(), Ok("*.example.org"));
        assert_eq!(ok("*.bücher.de", true).as_deref(), Ok("*.xn--bcher-kva.de"));
        assert_eq!(
            ok("*.user.github.io", true).as_deref(),
            Ok("*.user.github.io")
        );
        // Public suffixes (ICANN and private parts of the list), and wildcards on them.
        for refused in [
            "*.com",
            "*.co.uk",
            "*.github.io",
            "*.s3.amazonaws.com",
            "co.uk",
            "github.io",
        ] {
            assert!(ok(refused, false).is_err(), "{refused}");
        }
        // No `*` alone or inside, no IPs, no Orbit (also under a wildcard).
        for refused in [
            "*",
            "*.",
            "a.*.example.org",
            "api*.example.org",
            "1.2.3.4",
            "[::1]",
            "localhost",
        ] {
            assert!(ok(refused, false).is_err(), "{refused}");
        }
        for refused in [
            "orbit.internal",
            "*.internal",
            "orbit.example.com",
            "*.example.com",
        ] {
            assert!(ok(refused, false).is_err(), "{refused}");
        }
        // A risky host never gets an allow rule (also by a wildcard over it), but may be denied.
        assert!(ok("webhook.site", true).is_err());
        assert!(ok("*.ngrok-free.app", true).is_err());
        assert!(ok("x.webhook.site", true).is_err());
        assert_eq!(ok("webhook.site", false).as_deref(), Ok("webhook.site"));
    }

    #[test]
    fn a_card_shows_each_viewer_only_the_answers_they_may_give() {
        // (allow once, allow always, deny)
        assert_eq!(
            viewer_may(true, false, "api.example.com"),
            (true, false, true)
        );
        assert_eq!(
            viewer_may(false, true, "api.example.com"),
            (false, true, true)
        );
        assert_eq!(
            viewer_may(true, true, "api.example.com"),
            (true, true, true)
        );
        assert_eq!(
            viewer_may(false, false, "api.example.com"),
            (false, false, false)
        );
        assert_eq!(viewer_may(true, true, "webhook.site"), (true, false, true));
        assert_eq!(shown_status("pending", Some(10), 10), "expired");
        assert_eq!(shown_status("pending", Some(11), 10), "pending");
        assert_eq!(shown_status("allowed", Some(5), 10), "allowed");
    }
}
