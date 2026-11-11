//! Short sessions for the agent to act as one person in one workspace. They are normal `sessions`
//! rows, so the whole API judges them like the person's browser; `agent_sessions` marks them for
//! the scope layer (`super::scope`).

use orbit_platform::{Database, Flag, Id, Row, TimestampMillis, generate_opaque_token};
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::scope::TOKEN_PREFIX;

/// An agent session lives this long; the sidecar asks for a new one before it ends.
pub(crate) const LIFETIME_MS: i64 = 15 * 60 * 1_000;

#[derive(Debug, Serialize)]
pub struct IssuedAgentSession {
    pub token: String,
    pub expires_at: i64,
    /// Orbit refuses this session's writes (a test run).
    pub dry_run: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum SessionRefused {
    #[error("the person is not an active member of the workspace")]
    NotMember,
    #[error("the person drives no agent run in the workspace")]
    NoRun,
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

/// A new agent session for `user_id` in `workspace_id`. Only for an active member who drives a
/// running or waiting agent thread there (`thread_key`: that thread).
///
/// The session is a **dry run** (`agent_sessions.dry_run`, see `super::scope`) when its thread is a
/// routine run that may not write (`agent_routine_runs.writes = 'dry'`: a test run, or a script
/// changed since its approval). Without `thread_key` (an older sidecar) it is one whenever the
/// person drives such a run, so a missing thread never turns writes on.
pub(crate) async fn issue(
    database: &Database,
    user_id: Id,
    workspace_id: Id,
    agent: Option<Id>,
    thread_key: Option<Id>,
) -> Result<IssuedAgentSession, SessionRefused> {
    let now = TimestampMillis::now().as_millis();
    let mut tx = database.immediate_transaction().await?;
    let member = sqlx::query_scalar::<_, String>(
        "SELECT memberships.id FROM memberships \
         JOIN users ON users.id = memberships.user_id \
         JOIN workspaces ON workspaces.id = memberships.workspace_id \
         WHERE memberships.user_id = $1 AND memberships.workspace_id = $2 \
         AND memberships.role <> 'guest' \
         AND users.suspended_at IS NULL AND workspaces.deleted_at IS NULL",
    )
    .bind(user_id.to_string())
    .bind(workspace_id.to_string())
    .fetch_optional(&mut *tx)
    .await?;
    if member.is_none() {
        return Err(SessionRefused::NotMember);
    }
    if let Some(agent) = agent
        && super::profiles::live_agent(&mut tx, workspace_id, agent)
            .await?
            .is_none()
    {
        return Err(SessionRefused::NoRun);
    }
    let driving = sqlx::query(
        "SELECT t.thread_key, EXISTS (SELECT 1 FROM agent_routine_runs r \
         WHERE r.thread_key = t.thread_key AND r.writes = 'dry') AS dry \
         FROM agent_threads t WHERE t.workspace_id = $1 AND t.driver_user_id = $2 \
         AND t.status IN ('running', 'waiting') AND t.agent_user_id IS NOT DISTINCT FROM $3 \
         AND ($4 = '' OR t.thread_key = $4)",
    )
    .bind(workspace_id.to_string())
    .bind(user_id.to_string())
    .bind(agent.map(|id| id.to_string()))
    .bind(thread_key.map(|id| id.to_string()).unwrap_or_default())
    .fetch_all(&mut *tx)
    .await?;
    if driving.is_empty() {
        return Err(SessionRefused::NoRun);
    }
    let dry_run = driving.iter().any(|row| row.get::<Flag, _>("dry").0);
    // Ended agent sessions go here; the session rows go with them (and expire anyway).
    sqlx::query(
        "DELETE FROM sessions WHERE id IN (SELECT session_id FROM agent_sessions WHERE expires_at < $1)",
    )
    .bind(now)
    .execute(&mut *tx)
    .await?;
    let id = Id::new_v7();
    let token = format!("{TOKEN_PREFIX}{}", generate_opaque_token());
    let hash: [u8; 32] = Sha256::digest(token.as_bytes()).into();
    let expires_at = now + LIFETIME_MS;
    sqlx::query(
        "INSERT INTO sessions (id, token_hash, user_id, created_at, last_activity_at, \
         idle_expires_at, absolute_expires_at) VALUES ($1, $2, $3, $4, $4, $5, $5)",
    )
    .bind(id.to_string())
    .bind(hash.to_vec())
    .bind(user_id.to_string())
    .bind(now)
    .bind(expires_at)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO agent_sessions (session_id, user_id, workspace_id, expires_at, agent_user_id, dry_run) \
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(id.to_string())
    .bind(user_id.to_string())
    .bind(workspace_id.to_string())
    .bind(expires_at)
    .bind(agent.map(|id| id.to_string()))
    .bind(i64::from(dry_run))
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(IssuedAgentSession {
        token,
        expires_at,
        dry_run,
    })
}

/// Which agent an agent session belongs to: `None` for a person's session, `Some(None)` for the
/// default agent, `Some(Some(bot))` for a custom agent.
pub(crate) async fn agent_of(
    database: &Database,
    session_id: Id,
) -> Result<Option<Option<Id>>, sqlx::Error> {
    let row = sqlx::query("SELECT agent_user_id FROM agent_sessions WHERE session_id = $1")
        .bind(session_id.to_string())
        .fetch_optional(database.pool())
        .await?;
    Ok(row.map(|row| {
        row.get::<Option<String>, _>("agent_user_id")
            .and_then(|id| id.parse().ok())
    }))
}
