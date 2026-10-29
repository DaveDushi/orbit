//! Short sessions for the agent to act as one person in one workspace. They are normal `sessions`
//! rows, so the whole API judges them like the person's browser; `agent_sessions` marks them for
//! the scope layer (`super::scope`).

use orbit_platform::{Database, Id, TimestampMillis, generate_opaque_token};
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::scope::TOKEN_PREFIX;

/// An agent session lives this long; the sidecar asks for a new one before it ends.
pub(crate) const LIFETIME_MS: i64 = 15 * 60 * 1_000;

#[derive(Debug, Serialize)]
pub struct IssuedAgentSession {
    pub token: String,
    pub expires_at: i64,
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
/// running or waiting agent thread there.
pub(crate) async fn issue(
    database: &Database,
    user_id: Id,
    workspace_id: Id,
) -> Result<IssuedAgentSession, SessionRefused> {
    let now = TimestampMillis::now().as_millis();
    let mut tx = database.immediate_transaction().await?;
    let member = sqlx::query_scalar::<_, String>(
        "SELECT memberships.id FROM memberships \
         JOIN users ON users.id = memberships.user_id \
         JOIN workspaces ON workspaces.id = memberships.workspace_id \
         WHERE memberships.user_id = $1 AND memberships.workspace_id = $2 \
         AND users.suspended_at IS NULL AND workspaces.deleted_at IS NULL",
    )
    .bind(user_id.to_string())
    .bind(workspace_id.to_string())
    .fetch_optional(&mut *tx)
    .await?;
    if member.is_none() {
        return Err(SessionRefused::NotMember);
    }
    let driving = sqlx::query_scalar::<_, String>(
        "SELECT thread_key FROM agent_threads WHERE workspace_id = $1 AND driver_user_id = $2 \
         AND status IN ('running', 'waiting') LIMIT 1",
    )
    .bind(workspace_id.to_string())
    .bind(user_id.to_string())
    .fetch_optional(&mut *tx)
    .await?;
    if driving.is_none() {
        return Err(SessionRefused::NoRun);
    }
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
        "INSERT INTO agent_sessions (session_id, user_id, workspace_id, expires_at) VALUES ($1, $2, $3, $4)",
    )
    .bind(id.to_string())
    .bind(user_id.to_string())
    .bind(workspace_id.to_string())
    .bind(expires_at)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(IssuedAgentSession { token, expires_at })
}
