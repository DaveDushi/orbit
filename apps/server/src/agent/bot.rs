//! The agent's chat identity: one bot user with a membership in every workspace. It has no
//! password that any sign-in accepts and no sessions; Orbit writes its messages only for the
//! sidecar (`routes::post_message`). Its role does not matter: the agent's tools never act as the
//! bot, only as the person who drives a run.

use orbit_platform::{Database, Id, PasswordService, TimestampMillis, generate_opaque_token};

use crate::repositories::chat::join_public_channels;

const EMAIL: &str = "agent@orbit.invalid";
const NAME: &str = "Orbit Agent";

/// The bot user, made on first use.
pub(crate) async fn ensure(database: &Database) -> Result<Id, sqlx::Error> {
    let mut tx = database.immediate_transaction().await?;
    if let Some(id) = super::inputs::bot_user_id(&mut tx).await? {
        tx.commit().await?;
        return Ok(id);
    }
    let id = Id::new_v7();
    let now = TimestampMillis::now().as_millis();
    // The hash of a random secret that is thrown away: no password signs in as the bot.
    let password_hash = PasswordService::default()
        .hash(&generate_opaque_token())
        .map_err(|error| sqlx::Error::Protocol(format!("agent bot password: {error}")))?;
    sqlx::query(
        "INSERT INTO users (id, email, normalized_email, display_name, password_hash, created_at, updated_at) \
         VALUES ($1, $2, $2, $3, $4, $5, $5)",
    )
    .bind(id.to_string())
    .bind(EMAIL)
    .bind(NAME)
    .bind(password_hash)
    .bind(now)
    .execute(&mut *tx)
    .await?;
    sqlx::query("INSERT INTO agent_bot (id, user_id) VALUES (1, $1)")
        .bind(id.to_string())
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(id)
}

/// Adds the bot to every live workspace it is not in yet, and to their public channels.
/// Removing the bot from a workspace keeps it out only until this runs again, so an admin turns
/// the agent off for the installation, not per workspace.
pub(crate) async fn join_workspaces(database: &Database, bot: Id) -> Result<usize, sqlx::Error> {
    let missing = sqlx::query_scalar::<_, String>(
        "SELECT id FROM workspaces WHERE deleted_at IS NULL AND NOT EXISTS \
         (SELECT 1 FROM memberships WHERE memberships.workspace_id = workspaces.id \
          AND memberships.user_id = $1)",
    )
    .bind(bot.to_string())
    .fetch_all(database.pool())
    .await?;
    for workspace_id in &missing {
        let Ok(workspace_id) = workspace_id.parse::<Id>() else {
            continue;
        };
        let now = TimestampMillis::now();
        let mut tx = database.immediate_transaction().await?;
        let inserted = sqlx::query(
            "INSERT INTO memberships (id, workspace_id, user_id, role, version, created_at, updated_at) \
             VALUES ($1, $2, $3, 'member', 0, $4, $4) ON CONFLICT (workspace_id, user_id) DO NOTHING",
        )
        .bind(Id::new_v7().to_string())
        .bind(workspace_id.to_string())
        .bind(bot.to_string())
        .bind(now.as_millis())
        .execute(&mut *tx)
        .await?;
        if inserted.rows_affected() > 0 {
            join_public_channels(&mut tx, workspace_id, bot, now).await?;
        }
        tx.commit().await?;
    }
    Ok(missing.len())
}
