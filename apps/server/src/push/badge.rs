//! The number on the iPhone app's icon: what the user has not read yet, in every workspace.
//!
//! - Chat: what the Chat tab's badge counts, conversation by conversation: every unread message
//!   when the user's level is `all`, else the mentions (with `@channel` and `@here` unless muted).
//! - Inbox: the unread notifications the Activity inbox lists (not for a guest, who has no
//!   inbox), but not its chat mentions: they are mentions in a channel's main list, which chat
//!   counts already.

use orbit_platform::{Database, Id};

use crate::repositories::chat::reachable_dm;
use crate::repositories::project_access::user_sees_task_sql;
use crate::repositories::tasks::notification_reachable_sql;

/// The badge of the user: unread chat plus unread inbox.
pub async fn count(database: &Database, user_id: Id) -> Result<i64, sqlx::Error> {
    let chat: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT CAST(COALESCE(SUM(CASE m.notify \
           WHEN 'all' THEN CASE WHEN c.message_count > m.read_count THEN c.message_count - m.read_count ELSE 0 END \
           WHEN 'muted' THEN m.mention_count \
           ELSE m.mention_count + m.broadcast_count END), 0) AS BIGINT) \
         FROM chat_members m JOIN chat_conversations c ON c.id = m.conversation_id \
         JOIN memberships ON memberships.workspace_id = m.workspace_id AND memberships.user_id = m.user_id \
         WHERE m.user_id = $1 AND c.archived_at IS NULL AND {}",
        reachable_dm()
    )))
    .bind(user_id.to_string())
    .fetch_one(database.pool())
    .await?;
    let inbox: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT COUNT(*) FROM notifications \
         JOIN memberships ON memberships.workspace_id = notifications.workspace_id \
           AND memberships.user_id = notifications.recipient_user_id AND memberships.role <> 'guest' \
         WHERE notifications.recipient_user_id = $1 AND notifications.read_at IS NULL \
         AND notifications.kind <> 'chat_mentioned' \
         AND {} AND (notifications.task_id IS NULL OR {})",
        notification_reachable_sql(),
        user_sees_task_sql("notifications.task_id", "notifications.recipient_user_id")
    )))
    .bind(user_id.to_string())
    .fetch_one(database.pool())
    .await?;
    Ok(chat + inbox)
}
