//! External agents, phase 2 (push): Orbit sends each input of an external agent with a Push URL
//! to that URL, signed exactly like chat webhook events (`chat_routes::interactions::post_signed`:
//! HMAC-SHA256 of `v0:{ts}:{body}` in `X-Orbit-Signature` and `X-Slack-Signature`, https on a
//! public address only, no redirects, 3 s). The body is the Slack `event_callback` of
//! `chat_routes::events`, with `event_id` = `Ev<input id>` for the receiver's dedup.
//!
//! - **Tries:** at once, then after 10 s, 60 s and 5 min. A 2xx sets `delivered_at` and
//!   acknowledges the input. After the last try the input stays for `wait_for_messages`, and the
//!   card shows "push failing". Each try is claimed with one `UPDATE`, so several servers on one
//!   database never send the same try twice.
//! - **DNS:** the push client keeps no connections, so each send resolves the host again through
//!   `PublicResolver` (a name that now points at a local address is refused).
//! - **Local receivers:** `ORBIT_AGENT_PUSH_ALLOW_LOCAL=1` lets the URL be http and local, for
//!   tests on the developer's machine. Only with `ORBIT_ENV=development` (a loopback-only
//!   server); in any other mode the server refuses to start with it.

use std::time::{Duration, Instant};

use axum::extract::{Path, State};
use axum::http::{HeaderMap, Uri};
use axum::routing::{get, post};
use axum::{Json, Router};
use orbit_domain::Permission;
use orbit_platform::{Id, Row, TimestampMillis, generate_opaque_token};
use serde::Serialize;
use serde_json::{Value, json};
use sqlx::AnyConnection;
use tokio_util::sync::CancellationToken;

use super::AgentState;
use super::external::{Event, PushDue, acknowledge, load_external, pending, principal_of};
use super::profiles::{RequestIdExtension, SaveError, invalid, manager, not_found, save_failure};
use crate::audit::{self, AuditOutcome};
use crate::repositories::membership;
use crate::task_routes::{ApiError, request_id_value};

pub const ALLOW_LOCAL_ENV: &str = "ORBIT_AGENT_PUSH_ALLOW_LOCAL";
/// Sends of one input at most: the first, then one after each delay.
pub const MAX_PUSHES: i64 = 4;
const RETRY_DELAYS_MILLIS: [i64; 3] = [10_000, 60_000, 300_000];
/// A claimed try that never ends (the server stopped) is due again after this long.
const LEASE_MILLIS: i64 = 30_000;
const TICK: Duration = Duration::from_secs(2);
/// Tries kept per agent for its card.
const MAX_DELIVERIES: i64 = 20;

/// Whether pushes may reach http and local URLs: `ORBIT_AGENT_PUSH_ALLOW_LOCAL`, development only.
pub fn allow_local_from_env(development: bool) -> Result<bool, String> {
    allow_local(std::env::var(ALLOW_LOCAL_ENV).ok().as_deref(), development)
}

fn allow_local(value: Option<&str>, development: bool) -> Result<bool, String> {
    match value.map(str::trim) {
        None | Some("" | "0") => Ok(false),
        Some("1") if development => Ok(true),
        Some("1") => Err(format!(
            "{ALLOW_LOCAL_ENV} lets external agents' pushes reach local addresses: it works only with ORBIT_ENV=development"
        )),
        Some(_) => Err(format!("{ALLOW_LOCAL_ENV} must be 1 or 0")),
    }
}

pub fn router(state: AgentState) -> Router {
    let base = "/api/v1/workspaces/{workspace_id}/agents/{agent_id}/push-secret";
    Router::new()
        .route(base, get(reveal_secret))
        .route(&format!("{base}/rotate"), post(rotate_secret))
        .with_state(state)
}

// ---------------------------------------------------------------------------------------------
// The card

/// The Push URL of a card's body: `None` leaves it, `Some(None)` turns push off (an empty value).
pub(super) fn checked_push_url(
    state: &AgentState,
    raw: Option<&str>,
) -> Result<Option<Option<String>>, &'static str> {
    let Some(raw) = raw.map(str::trim) else {
        return Ok(None);
    };
    if raw.is_empty() {
        return Ok(Some(None));
    }
    if state.app_key.is_none() {
        return Err("Push needs the server's app key (ORBIT_APP_KEY).");
    }
    crate::repositories::chat::clean_request_url(raw, state.push.allows_local())
        .map(|url| Some(Some(url)))
        .map_err(|_| "The Push URL must be an https address on a public host.")
}

/// Sets the Push URL in the card's write transaction. A new URL pushes only inputs from now on;
/// the first one makes the signing secret.
pub(super) async fn set_push_url(
    conn: &mut AnyConnection,
    state: &AgentState,
    profile_id: Id,
    url: Option<&str>,
    now: TimestampMillis,
) -> Result<(), SaveError> {
    sqlx::query(
        "UPDATE agent_profiles SET push_since = CASE WHEN push_url IS NOT DISTINCT FROM $1 \
         THEN push_since ELSE $2 END, push_url = $1 WHERE id = $3",
    )
    .bind(url)
    .bind(now.as_millis())
    .bind(profile_id.to_string())
    .execute(&mut *conn)
    .await?;
    if url.is_some()
        && let Some(key) = &state.app_key
    {
        let sealed = seal(key)?.1;
        sqlx::query(
            "UPDATE agent_profiles SET push_secret = $1 WHERE id = $2 AND push_secret IS NULL",
        )
        .bind(sealed)
        .bind(profile_id.to_string())
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

/// A new signing secret and its sealed form.
fn seal(key: &[u8; 32]) -> Result<(String, Vec<u8>), SaveError> {
    let secret = generate_opaque_token();
    let sealed = crate::secret_box::encrypt_secret(key, &secret)
        .map_err(|()| SaveError::Invalid("The signing secret could not be stored."))?;
    Ok((secret, sealed))
}

async fn reveal_secret(
    State(state): State<AgentState>,
    Path((workspace_id, agent_id)): Path<(String, String)>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
) -> Result<Json<Value>, ApiError> {
    signing_secret(
        state,
        workspace_id,
        agent_id,
        headers,
        uri,
        request_id,
        false,
    )
    .await
}

async fn rotate_secret(
    State(state): State<AgentState>,
    Path((workspace_id, agent_id)): Path<(String, String)>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
) -> Result<Json<Value>, ApiError> {
    signing_secret(
        state,
        workspace_id,
        agent_id,
        headers,
        uri,
        request_id,
        true,
    )
    .await
}

/// The signing secret of an external agent's pushes, for owners and admins. `rotate` (or no
/// secret yet) makes a new one; pushes are signed with it at once.
async fn signing_secret(
    state: AgentState,
    workspace_id: String,
    agent_id: String,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
    rotate: bool,
) -> Result<Json<Value>, ApiError> {
    let (workspace_id, actor) = manager(&state, &headers, &workspace_id, &uri, &request_id).await?;
    let agent_id: Id = agent_id.parse().map_err(|_| not_found(&uri, &request_id))?;
    let key = state.app_key.ok_or_else(|| {
        invalid(
            "Push needs the server's app key (ORBIT_APP_KEY).",
            &uri,
            &request_id,
        )
    })?;
    let request = request_id_value(request_id.as_ref()).to_owned();
    let database = state.database();
    let secret = async {
        let mut tx = database.immediate_transaction().await?;
        let allowed = membership::actor(&mut *tx, workspace_id, actor.user_id)
            .await?
            .is_some_and(|actor| actor.can(Permission::WorkspaceUpdate));
        if !allowed {
            return Err(SaveError::Forbidden);
        }
        // Another workspace's agent is not found.
        load_external(&mut tx, workspace_id, agent_id).await?;
        let stored: Option<Vec<u8>> =
            sqlx::query_scalar("SELECT push_secret FROM agent_profiles WHERE id = $1")
                .bind(agent_id.to_string())
                .fetch_one(&mut *tx)
                .await?;
        let readable = stored
            .filter(|_| !rotate)
            .map(|stored| crate::secret_box::decrypt_secret(&key, &stored));
        let secret = match readable {
            Some(Ok(secret)) => secret,
            Some(Err(())) => {
                return Err(SaveError::Invalid(
                    "The signing secret cannot be read with this server's app key. Rotate it.",
                ));
            }
            None => {
                let (secret, sealed) = seal(&key)?;
                sqlx::query("UPDATE agent_profiles SET push_secret = $1 WHERE id = $2")
                    .bind(sealed)
                    .bind(agent_id.to_string())
                    .execute(&mut *tx)
                    .await?;
                let now = TimestampMillis::now();
                audit::record(
                    &mut tx,
                    workspace_id,
                    Some(actor.user_id),
                    "agent_profile.push_secret_rotate",
                    AuditOutcome::Success,
                    "agent_profile",
                    Some(agent_id),
                    &request,
                    json!({}),
                    now,
                )
                .await?;
                secret
            }
        };
        tx.commit().await?;
        Ok::<_, SaveError>(secret)
    }
    .await
    .map_err(|error| save_failure(error, &uri, &request_id))?;
    Ok(Json(json!({ "signing_secret": secret })))
}

/// One push try on the card.
#[derive(Serialize)]
pub(super) struct DeliveryView {
    event_id: String,
    /// 1 to 4.
    attempt: i64,
    /// The receiver's HTTP status; `null` when it did not answer (`error` says why).
    status: Option<i64>,
    error: Option<String>,
    duration_ms: i64,
    created_at: i64,
}

impl DeliveryView {
    fn delivered(&self) -> bool {
        self.status
            .is_some_and(|status| (200..300).contains(&status))
    }
}

/// The newest tries of an agent, newest first.
pub(super) async fn deliveries(
    conn: &mut AnyConnection,
    profile_id: &str,
) -> Result<Vec<DeliveryView>, sqlx::Error> {
    Ok(sqlx::query(
        "SELECT event_id, attempt, status, error, duration_ms, created_at FROM agent_push_deliveries \
         WHERE agent_id = $1 ORDER BY created_at DESC, id DESC LIMIT $2",
    )
    .bind(profile_id)
    .bind(MAX_DELIVERIES)
    .fetch_all(&mut *conn)
    .await?
    .iter()
    .map(|row| DeliveryView {
        event_id: row.get("event_id"),
        attempt: row.get("attempt"),
        status: row.get("status"),
        error: row.get("error"),
        duration_ms: row.get("duration_ms"),
        created_at: row.get("created_at"),
    })
    .collect())
}

/// After the newest try: `ok`, `retrying`, or `failing` (it was the last try of its input).
pub(super) fn push_state(deliveries: &[DeliveryView]) -> Option<&'static str> {
    let newest = deliveries.first()?;
    Some(if newest.delivered() {
        "ok"
    } else if newest.attempt >= MAX_PUSHES {
        "failing"
    } else {
        "retrying"
    })
}

// ---------------------------------------------------------------------------------------------
// The loop

pub async fn run_service(state: AgentState, token: CancellationToken) -> Result<(), String> {
    let mut tick = tokio::time::interval(TICK);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            () = token.cancelled() => return Ok(()),
            _ = tick.tick() => {}
        }
        // Never fatal: a locked or busy database is tried again on the next round.
        if let Some(Err(error)) =
            crate::app::unless_panic("external agent push", run_round(&state)).await
        {
            tracing::warn!(%error, "external agent push");
        }
    }
}

/// Sends every due push once; the number delivered.
pub async fn run_round(state: &AgentState) -> Result<usize, sqlx::Error> {
    let Some(key) = state.app_key else {
        return Ok(0);
    };
    let rows = sqlx::query(
        "SELECT id, push_url, push_secret, push_since FROM agent_profiles WHERE kind = 'external' \
         AND deleted_at IS NULL AND push_url IS NOT NULL AND push_secret IS NOT NULL",
    )
    .fetch_all(state.database().pool())
    .await?;
    let agents = rows.iter().filter_map(|row| {
        let profile_id = row.get::<String, _>("id").parse::<Id>().ok()?;
        Some(push_agent(
            state,
            &key,
            profile_id,
            row.get("push_url"),
            row.get("push_secret"),
            row.get::<Option<i64>, _>("push_since").unwrap_or_default(),
        ))
    });
    let mut delivered = 0;
    for result in futures_util::future::join_all(agents).await {
        delivered += result?;
    }
    Ok(delivered)
}

/// Sends the due inputs of one agent, oldest first. A failed try ends this agent's round, so a
/// receiver that is down costs one timeout per round.
async fn push_agent(
    state: &AgentState,
    key: &[u8; 32],
    profile_id: Id,
    url: String,
    sealed: Vec<u8>,
    since: i64,
) -> Result<usize, sqlx::Error> {
    let database = state.database();
    let Some(principal) = principal_of(database, profile_id, profile_id).await? else {
        return Ok(0);
    };
    let Some(limits) = principal.agent.clone() else {
        return Ok(0);
    };
    // Task assignments and mentions become inputs here too (`wait_for_messages` does it for pull).
    if let Err(error) = super::task_runs::start_queued_for(state, limits.user_id).await {
        tracing::warn!(%error, "external agent task runs");
    }
    let Ok(secret) = crate::secret_box::decrypt_secret(key, &sealed) else {
        tracing::warn!(agent = %profile_id, "external agent push secret cannot be read with this app key");
        return Ok(0);
    };
    let now = TimestampMillis::now().as_millis();
    let due = PushDue {
        since,
        max_attempts: MAX_PUSHES,
        now,
    };
    let mut delivered = 0;
    for event in pending(database, &principal, &limits, Some(due)).await? {
        let message_id = event.event_id.trim_start_matches("Ev").to_owned();
        let attempt = event.push_attempts + 1;
        let claimed = sqlx::query(
            "UPDATE agent_inputs SET push_attempts = $1, push_next_at = $2 WHERE message_id = $3 \
             AND push_attempts = $4 AND acked_at IS NULL AND delivered_at IS NULL",
        )
        .bind(attempt)
        .bind(now + LEASE_MILLIS)
        .bind(&message_id)
        .bind(event.push_attempts)
        .execute(database.pool())
        .await?;
        if claimed.rows_affected() != 1 {
            continue;
        }
        let body = payload(principal.workspace_id, profile_id, &event).to_string();
        let started = Instant::now();
        let result = state
            .push
            .post_signed(&url, &secret, "application/json", body)
            .await;
        let duration_ms = i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX);
        let (status, error) = match result {
            Ok(status) => (Some(status), None),
            Err(reason) => (None, Some(reason)),
        };
        let ok = status.is_some_and(|status| (200..300).contains(&status));
        let done = TimestampMillis::now().as_millis();
        if ok {
            sqlx::query(
                "UPDATE agent_inputs SET delivered_at = $1, push_next_at = NULL WHERE message_id = $2",
            )
            .bind(done)
            .bind(&message_id)
            .execute(database.pool())
            .await?;
            if let Ok(id) = message_id.parse() {
                acknowledge(database, limits.user_id, id).await?;
            }
            delivered += 1;
        } else {
            // After the last try: no next one; the input stays for `wait_for_messages`.
            let next = usize::try_from(attempt - 1)
                .ok()
                .and_then(|index| RETRY_DELAYS_MILLIS.get(index))
                .map(|delay| done + delay);
            sqlx::query("UPDATE agent_inputs SET push_next_at = $1 WHERE message_id = $2")
                .bind(next)
                .bind(&message_id)
                .execute(database.pool())
                .await?;
        }
        record_delivery(
            database,
            profile_id,
            &event.event_id,
            attempt,
            status,
            error,
            duration_ms,
        )
        .await?;
        if !ok {
            break;
        }
    }
    Ok(delivered)
}

/// Keeps one try for the card, and drops the ones past the newest 20.
async fn record_delivery(
    database: &orbit_platform::Database,
    profile_id: Id,
    event_id: &str,
    attempt: i64,
    status: Option<u16>,
    error: Option<&str>,
    duration_ms: i64,
) -> Result<(), sqlx::Error> {
    let mut tx = database.immediate_transaction().await?;
    sqlx::query(
        "INSERT INTO agent_push_deliveries (id, agent_id, event_id, attempt, status, error, duration_ms, \
         created_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(Id::new_v7().to_string())
    .bind(profile_id.to_string())
    .bind(event_id)
    .bind(attempt)
    .bind(status.map(i64::from))
    .bind(error)
    .bind(duration_ms)
    .bind(TimestampMillis::now().as_millis())
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "DELETE FROM agent_push_deliveries WHERE agent_id = $1 AND id NOT IN \
         (SELECT id FROM agent_push_deliveries WHERE agent_id = $1 \
         ORDER BY created_at DESC, id DESC LIMIT $2)",
    )
    .bind(profile_id.to_string())
    .bind(MAX_DELIVERIES)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

/// The Slack Events API body of one input, as chat webhook events have it (`events.rs`):
/// `api_app_id` is the agent, `ts` the message, `thread_ts` where to answer.
fn payload(workspace_id: Id, agent_id: Id, event: &Event) -> Value {
    let message_id = event.event_id.trim_start_matches("Ev");
    let name = &event.author_name;
    let mut inner = json!({
        "type": event.kind,
        "channel": event.conversation_id,
        "channel_name": event.conversation_name,
        "channel_type": match event.conversation_type.as_str() {
            "im" => "im",
            "private" => "group",
            _ => "channel",
        },
        "user": event.author_id,
        "user_profile": {"name": name, "display_name": name, "real_name": name},
        "text": event.text,
        "ts": message_id,
        "event_ts": message_id,
    });
    if let Some(thread) = &event.thread_ts {
        inner["thread_ts"] = json!(thread);
    }
    if let Some(task) = &event.task {
        inner["task"] = task.clone();
    }
    json!({
        "type": "event_callback",
        "team_id": workspace_id.to_string(),
        "api_app_id": agent_id.to_string(),
        "event_id": event.event_id,
        "event_time": event.created_at / 1000,
        "event": inner,
    })
}

#[cfg(test)]
mod tests {
    use super::{DeliveryView, allow_local, push_state};

    #[test]
    fn local_pushes_need_development() {
        assert_eq!(allow_local(None, false), Ok(false));
        assert_eq!(allow_local(Some("0"), false), Ok(false));
        assert_eq!(allow_local(Some("1"), true), Ok(true));
        assert!(allow_local(Some("1"), false).is_err());
        assert!(allow_local(Some("yes"), true).is_err());
    }

    fn try_(attempt: i64, status: Option<i64>) -> DeliveryView {
        DeliveryView {
            event_id: "Ev1".into(),
            attempt,
            status,
            error: None,
            duration_ms: 1,
            created_at: 1,
        }
    }

    #[test]
    fn the_newest_try_sets_the_push_state() {
        assert_eq!(push_state(&[]), None);
        assert_eq!(push_state(&[try_(2, Some(204))]), Some("ok"));
        assert_eq!(push_state(&[try_(1, Some(500))]), Some("retrying"));
        assert_eq!(push_state(&[try_(3, Some(302))]), Some("retrying"));
        assert_eq!(
            push_state(&[try_(4, None), try_(1, Some(200))]),
            Some("failing")
        );
        assert_eq!(push_state(&[try_(1, Some(200)), try_(4, None)]), Some("ok"));
    }
}
