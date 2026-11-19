//! What an external agent may reach through MCP: the ONE check that every tool family (chat,
//! tasks, docs) uses. An external agent acts as its bot user, a normal `member`, so the usual
//! access checks of each repository still apply; on top, its card limits it:
//!
//! - **Conversations:** a channel on its card's list (`agent_profile_channels`), or a DM or group
//!   DM when its card allows DMs. Never another channel, even a public one.
//! - **Projects:** the projects on its card that the bot user sees (`ApiTokenPrincipal::project_ids`
//!   is built from them, so every task tool already keeps to them); docs only in the teamspaces of
//!   those projects.
//!
//! An ID outside the card answers exactly like an unknown ID, so nothing tells that it exists.
//! Lists filter in SQL ([`AgentLimits::conversation_sql`]), not after the fetch.

use orbit_platform::{Flag, Id};
use sqlx::AnyConnection;

/// The card of the external agent a credential acts as (`ApiTokenPrincipal::agent`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentLimits {
    pub profile_id: Id,
    /// The agent's bot user, also the principal's `actor_id`.
    pub user_id: Id,
    pub allow_dms: bool,
    /// `chat:write` on its card: `send_message` and `set_status`.
    pub chat_write: bool,
}

impl AgentLimits {
    /// The SQL condition that the conversation `alias` (a `chat_conversations` row) is on the
    /// card. The IDs are parsed UUIDs, so they are safe as literals.
    #[must_use]
    pub fn conversation_sql(&self, alias: &str) -> String {
        let dms = if self.allow_dms { "1 = 1" } else { "1 = 0" };
        format!(
            "(({alias}.kind = 'dm' AND {dms}) OR {alias}.id IN (SELECT conversation_id FROM \
             agent_profile_channels WHERE agent_id = '{}'))",
            self.profile_id
        )
    }
}

/// Whether the agent may use the conversation. `None` (a person's credential) allows all.
pub async fn conversation_allowed(
    conn: &mut AnyConnection,
    limits: Option<&AgentLimits>,
    conversation_id: Id,
) -> Result<bool, sqlx::Error> {
    let Some(limits) = limits else {
        return Ok(true);
    };
    let Flag(allowed) = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT EXISTS (SELECT 1 FROM chat_conversations c WHERE c.id = $1 AND {})",
        limits.conversation_sql("c")
    )))
    .bind(conversation_id.to_string())
    .fetch_one(&mut *conn)
    .await?;
    Ok(allowed)
}

/// Whether the agent may use the conversation of this message (a thread root or any message).
pub async fn message_allowed(
    conn: &mut AnyConnection,
    limits: Option<&AgentLimits>,
    message_id: Id,
) -> Result<bool, sqlx::Error> {
    if limits.is_none() {
        return Ok(true);
    }
    let conversation: Option<String> =
        sqlx::query_scalar("SELECT conversation_id FROM chat_messages WHERE id = $1")
            .bind(message_id.to_string())
            .fetch_optional(&mut *conn)
            .await?;
    match conversation.and_then(|id| id.parse().ok()) {
        Some(conversation) => conversation_allowed(conn, limits, conversation).await,
        None => Ok(false),
    }
}

/// Whether a project is one the credential may use: `approved` is the principal's
/// `project_ids`, which for an agent are its card's projects that its bot user sees.
#[must_use]
pub fn project_allowed(approved: &[Id], project_id: Id) -> bool {
    approved.contains(&project_id)
}

/// Whether the agent may use a doc in this teamspace: one linked to a project on its card. Private
/// docs (`None`) and the docs of other teamspaces are out. `None` limits (a person) allow all.
pub async fn teamspace_allowed(
    conn: &mut AnyConnection,
    limits: Option<&AgentLimits>,
    approved: &[Id],
    teamspace_id: Option<Id>,
) -> Result<bool, sqlx::Error> {
    if limits.is_none() {
        return Ok(true);
    }
    let Some(teamspace_id) = teamspace_id else {
        return Ok(false);
    };
    let project: Option<Option<String>> =
        sqlx::query_scalar("SELECT project_id FROM teamspaces WHERE id = $1")
            .bind(teamspace_id.to_string())
            .fetch_optional(&mut *conn)
            .await?;
    Ok(project
        .flatten()
        .and_then(|id| id.parse().ok())
        .is_some_and(|project| project_allowed(approved, project)))
}

#[cfg(test)]
mod tests {
    use orbit_platform::Id;

    use super::{AgentLimits, project_allowed};

    #[test]
    fn the_conversation_condition_keeps_dms_only_when_allowed() {
        let mut limits = AgentLimits {
            profile_id: Id::new_v7(),
            user_id: Id::new_v7(),
            allow_dms: true,
            chat_write: false,
        };
        assert!(
            limits
                .conversation_sql("c")
                .contains("c.kind = 'dm' AND 1 = 1")
        );
        limits.allow_dms = false;
        let sql = limits.conversation_sql("c");
        assert!(sql.contains("c.kind = 'dm' AND 1 = 0"));
        assert!(sql.contains(&format!("agent_id = '{}'", limits.profile_id)));
    }

    #[test]
    fn only_approved_projects_are_allowed() {
        let (one, two) = (Id::new_v7(), Id::new_v7());
        assert!(project_allowed(&[one], one));
        assert!(!project_allowed(&[one], two));
        assert!(!project_allowed(&[], one));
    }
}
