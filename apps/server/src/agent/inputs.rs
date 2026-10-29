//! Chat messages for the agent. [`record`] runs in the transaction that sends a message, so a
//! message that is for the agent is never lost; the sidecar reads the queue and acknowledges each
//! row after Pi stored it.

use orbit_platform::{Database, Flag, Id, Row, TimestampMillis};
use serde::{Deserialize, Serialize};
use sqlx::AnyConnection;

use crate::repositories::chat::MessageRecord;

/// What a message means to the agent. See "Which user's ChatGPT account pays" in
/// `.ai/agent-design.md`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InputKind {
    /// It mentions the agent (or is in a DM with it): its author starts a run.
    Instruction,
    /// A reply in a thread where the agent waits for an answer: it continues the waiting run.
    Answer,
    /// A reply from the person whose run is going: it joins the run.
    Steer,
}

impl InputKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Instruction => "instruction",
            Self::Answer => "answer",
            Self::Steer => "steer",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThreadStatus {
    Idle,
    Running,
    Waiting,
}

impl ThreadStatus {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Running => "running",
            Self::Waiting => "waiting",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "idle" => Some(Self::Idle),
            "running" => Some(Self::Running),
            "waiting" => Some(Self::Waiting),
            _ => None,
        }
    }
}

/// The agent's state in a thread, as far as a new message needs it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ThreadState {
    pub status: ThreadStatus,
    pub driver: Option<Id>,
}

/// Whether a message by `author` is for the agent, and as what.
#[must_use]
pub fn classify(addressed: bool, thread: Option<ThreadState>, author: Id) -> Option<InputKind> {
    if addressed {
        return Some(InputKind::Instruction);
    }
    match thread? {
        ThreadState {
            status: ThreadStatus::Waiting,
            ..
        } => Some(InputKind::Answer),
        ThreadState {
            status: ThreadStatus::Running,
            driver: Some(driver),
        } if driver == author => Some(InputKind::Steer),
        _ => None,
    }
}

/// Where the agent answers a message: the thread it is in, the main list of a DM with the agent,
/// or a new thread under the message. Returns `(thread_key, root_message_id)`.
#[must_use]
pub fn thread_of(message: &MessageRecord, conversation_id: Id, agent_dm: bool) -> (Id, Option<Id>) {
    match message.thread_root_id {
        Some(root) => (root, Some(root)),
        None if agent_dm => (conversation_id, None),
        None => (message.id, Some(message.id)),
    }
}

pub(crate) async fn bot_user_id(conn: &mut AnyConnection) -> Result<Option<Id>, sqlx::Error> {
    let id = sqlx::query_scalar::<_, String>("SELECT user_id FROM agent_bot WHERE id = 1")
        .fetch_optional(&mut *conn)
        .await?;
    Ok(id.and_then(|id| id.parse().ok()))
}

/// Queues `message` for the agent when it is for the agent. Called by `send_message` in its
/// transaction, after the message row is written. Never fails for a reason of its own: on
/// Postgres an error would end the whole transaction and lose the message.
pub(crate) async fn record(
    conn: &mut AnyConnection,
    workspace_id: Id,
    conversation_id: Id,
    is_dm: bool,
    message: &MessageRecord,
) -> Result<(), sqlx::Error> {
    let Some(bot) = bot_user_id(conn).await? else {
        return Ok(());
    };
    if message.author_id == bot {
        return Ok(());
    }
    let mentioned = message.mentions.user_ids.contains(&bot);
    if !mentioned && !is_dm && message.thread_root_id.is_none() {
        return Ok(());
    }
    let bot_member = sqlx::query_scalar::<_, Flag>(
        "SELECT EXISTS (SELECT 1 FROM chat_members WHERE conversation_id = $1 AND user_id = $2)",
    )
    .bind(conversation_id.to_string())
    .bind(bot.to_string())
    .fetch_one(&mut *conn)
    .await?
    .0;
    // A DM keeps the people it was opened with: the agent cannot be added to one.
    if is_dm && !bot_member {
        return Ok(());
    }
    let (thread_key, root) = thread_of(message, conversation_id, is_dm);
    let thread =
        sqlx::query("SELECT status, driver_user_id FROM agent_threads WHERE thread_key = $1")
            .bind(thread_key.to_string())
            .fetch_optional(&mut *conn)
            .await?
            .and_then(|row| {
                Some(ThreadState {
                    status: ThreadStatus::parse(&row.get::<String, _>("status"))?,
                    driver: row
                        .get::<Option<String>, _>("driver_user_id")
                        .and_then(|id| id.parse().ok()),
                })
            });
    let Some(kind) = classify(mentioned || is_dm, thread, message.author_id) else {
        return Ok(());
    };
    if !bot_member {
        // Mentioned in a private channel: a member brought the agent in. It joins with
        // everything read, the way a new member of a public channel does.
        sqlx::query(
            "INSERT INTO chat_members (conversation_id, user_id, workspace_id, notify, \
             last_read_message_id, read_count, joined_at) \
             SELECT c.id, $2, c.workspace_id, 'mentions', $3, c.message_count, $4 \
             FROM chat_conversations c \
             WHERE c.id = $1 AND EXISTS (SELECT 1 FROM memberships m \
                 WHERE m.workspace_id = c.workspace_id AND m.user_id = $2) \
             ON CONFLICT (conversation_id, user_id) DO NOTHING",
        )
        .bind(conversation_id.to_string())
        .bind(bot.to_string())
        .bind(message.id.to_string())
        .bind(message.created_at.as_millis())
        .execute(&mut *conn)
        .await?;
    }
    sqlx::query(
        "INSERT INTO agent_inputs (message_id, workspace_id, conversation_id, thread_key, \
         root_message_id, author_id, kind, created_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
         ON CONFLICT (message_id) DO NOTHING",
    )
    .bind(message.id.to_string())
    .bind(workspace_id.to_string())
    .bind(conversation_id.to_string())
    .bind(thread_key.to_string())
    .bind(root.map(|id| id.to_string()))
    .bind(message.author_id.to_string())
    .bind(kind.as_str())
    .bind(message.created_at.as_millis())
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// One queued message, with what the sidecar needs to read it.
#[derive(Clone, Debug, Serialize)]
pub struct PendingInput {
    pub message_id: String,
    pub workspace_id: String,
    pub conversation_id: String,
    pub conversation_kind: String,
    pub conversation_name: String,
    pub thread_key: String,
    pub root_message_id: Option<String>,
    pub author_id: String,
    pub author_name: String,
    pub kind: String,
    pub body: String,
    pub created_at: i64,
}

/// The oldest unacknowledged inputs, in message order.
pub(crate) async fn pending(
    database: &Database,
    limit: i64,
) -> Result<Vec<PendingInput>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT i.message_id, i.workspace_id, i.conversation_id, i.thread_key, i.root_message_id, \
         i.author_id, i.kind, i.created_at, u.display_name, m.body, c.kind AS conversation_kind, \
         c.name AS conversation_name \
         FROM agent_inputs i \
         JOIN chat_messages m ON m.id = i.message_id \
         JOIN chat_conversations c ON c.id = i.conversation_id \
         JOIN users u ON u.id = i.author_id \
         WHERE i.acked_at IS NULL ORDER BY i.message_id LIMIT $1",
    )
    .bind(limit)
    .fetch_all(database.pool())
    .await?;
    Ok(rows
        .iter()
        .map(|row| PendingInput {
            message_id: row.get("message_id"),
            workspace_id: row.get("workspace_id"),
            conversation_id: row.get("conversation_id"),
            conversation_kind: row.get("conversation_kind"),
            conversation_name: row.get("conversation_name"),
            thread_key: row.get("thread_key"),
            root_message_id: row.get("root_message_id"),
            author_id: row.get("author_id"),
            author_name: row.get("display_name"),
            kind: row.get("kind"),
            body: row.get("body"),
            created_at: row.get("created_at"),
        })
        .collect())
}

pub(crate) async fn acknowledge(database: &Database, message_id: Id) -> Result<bool, sqlx::Error> {
    let done = sqlx::query(
        "UPDATE agent_inputs SET acked_at = $1 WHERE message_id = $2 AND acked_at IS NULL",
    )
    .bind(TimestampMillis::now().as_millis())
    .bind(message_id.to_string())
    .execute(database.pool())
    .await?;
    Ok(done.rows_affected() > 0)
}

/// The sidecar's report of a thread's state.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThreadUpdate {
    pub workspace_id: Id,
    pub conversation_id: Id,
    pub root_message_id: Option<Id>,
    pub status: ThreadStatus,
    pub driver_user_id: Option<Id>,
    pub waiting_for_user_id: Option<Id>,
}

pub(crate) async fn update_thread(
    database: &Database,
    thread_key: Id,
    update: &ThreadUpdate,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO agent_threads (thread_key, workspace_id, conversation_id, root_message_id, \
         status, driver_user_id, waiting_for_user_id, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
         ON CONFLICT (thread_key) DO UPDATE SET status = excluded.status, \
         driver_user_id = excluded.driver_user_id, \
         waiting_for_user_id = excluded.waiting_for_user_id, updated_at = excluded.updated_at",
    )
    .bind(thread_key.to_string())
    .bind(update.workspace_id.to_string())
    .bind(update.conversation_id.to_string())
    .bind(update.root_message_id.map(|id| id.to_string()))
    .bind(update.status.as_str())
    .bind(update.driver_user_id.map(|id| id.to_string()))
    .bind(update.waiting_for_user_id.map(|id| id.to_string()))
    .bind(TimestampMillis::now().as_millis())
    .execute(database.pool())
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use orbit_platform::Id;

    use super::{InputKind, ThreadState, ThreadStatus, classify};

    #[test]
    fn a_mention_is_always_an_instruction() {
        let author = Id::new_v7();
        for thread in [
            None,
            Some(ThreadState {
                status: ThreadStatus::Idle,
                driver: None,
            }),
            Some(ThreadState {
                status: ThreadStatus::Waiting,
                driver: Some(Id::new_v7()),
            }),
            Some(ThreadState {
                status: ThreadStatus::Running,
                driver: Some(Id::new_v7()),
            }),
        ] {
            assert_eq!(classify(true, thread, author), Some(InputKind::Instruction));
        }
    }

    #[test]
    fn plain_replies_count_only_while_the_agent_waits_or_from_the_driver() {
        let driver = Id::new_v7();
        let teammate = Id::new_v7();
        let waiting = Some(ThreadState {
            status: ThreadStatus::Waiting,
            driver: Some(driver),
        });
        let running = Some(ThreadState {
            status: ThreadStatus::Running,
            driver: Some(driver),
        });
        let idle = Some(ThreadState {
            status: ThreadStatus::Idle,
            driver: Some(driver),
        });

        assert_eq!(classify(false, waiting, teammate), Some(InputKind::Answer));
        assert_eq!(classify(false, waiting, driver), Some(InputKind::Answer));
        assert_eq!(classify(false, running, driver), Some(InputKind::Steer));
        // Another person cannot steer someone else's run without mentioning the agent.
        assert_eq!(classify(false, running, teammate), None);
        assert_eq!(classify(false, idle, driver), None);
        assert_eq!(classify(false, None, driver), None);
    }
}
