//! Routines and scheduled messages in the database, with who may see and change them. Every
//! write checks the caller in its own transaction. Guests may use none of this: they cannot use
//! the agent (step 2), and send later goes with it.

use orbit_domain::{Actor, Permission};
use orbit_platform::{Database, Flag, Id, Row, TimestampMillis};
use serde::{Deserialize, Serialize};
// The SELECT prefixes are constants: no input reaches the SQL text.
use sqlx::{AnyConnection, AssertSqlSafe};

use super::schedule::{RoutineSchedule, parse_time, time_zone};
use crate::agent::inputs::bot_user_id;
use crate::chat_routes::ChatState;
use crate::repositories::membership;

/// Routines one person may have in one workspace, active or paused.
pub const MAX_ROUTINES_PER_USER: i64 = 25;
/// Scheduled messages and reminders one person may have waiting in one workspace.
pub const MAX_SCHEDULED_PER_USER: i64 = 100;
/// The latest time a message or reminder can be scheduled for: one year ahead.
pub const MAX_AHEAD_MS: i64 = 366 * 24 * 60 * 60 * 1000;
pub const NAME_MAX_CHARS: usize = 80;
pub const PROMPT_MAX_CHARS: usize = 4000;
pub const BODY_MAX_CHARS: usize = 4000;

#[derive(Debug, thiserror::Error)]
pub enum ScheduledError {
    #[error("not found")]
    NotFound,
    #[error("{0}")]
    Forbidden(&'static str),
    #[error("{0}")]
    Invalid(&'static str),
    #[error("{0}")]
    Limit(&'static str),
    #[error("the agent is off")]
    AgentOff,
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

const GUESTS: &str = "Guests cannot use routines, reminders or send later.";

/// The caller as a full member (not a guest) of the workspace.
async fn member(
    conn: &mut AnyConnection,
    workspace_id: Id,
    user_id: Id,
) -> Result<Actor, ScheduledError> {
    if let Some(actor) = membership::actor(&mut *conn, workspace_id, user_id).await? {
        return Ok(actor);
    }
    if membership::chat_actor(&mut *conn, workspace_id, user_id)
        .await?
        .is_some()
    {
        return Err(ScheduledError::Forbidden(GUESTS));
    }
    Err(ScheduledError::NotFound)
}

/// Why a person cannot post in a conversation (for a routine: with the agent).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TargetProblem {
    Gone,
    Archived,
    NotMember,
    AgentCannotPost,
}

impl TargetProblem {
    /// For the person who saves a routine or a message.
    const fn detail(self) -> &'static str {
        match self {
            Self::Gone => "The conversation no longer exists.",
            Self::Archived => "The channel is archived.",
            Self::NotMember => "You are not a member of this conversation.",
            Self::AgentCannotPost => {
                "The agent cannot post in this direct message: pick a channel or your DM with it."
            }
        }
    }

    /// Why a routine paused.
    const fn routine_reason(self) -> &'static str {
        match self {
            Self::Gone => "Paused: the conversation it posts to was deleted.",
            Self::Archived => "Paused: the channel it posts to is archived.",
            Self::NotMember => "Paused: its creator is no longer in the conversation it posts to.",
            Self::AgentCannotPost => {
                "Paused: the agent cannot post in the conversation it posts to."
            }
        }
    }

    /// Why a scheduled message was not sent.
    const fn message_reason(self) -> &'static str {
        match self {
            Self::Gone => "Not sent: the conversation was deleted.",
            Self::Archived => "Not sent: the channel is archived.",
            Self::NotMember | Self::AgentCannotPost => {
                "Not sent: you are no longer a member of this conversation."
            }
        }
    }
}

/// Why `user_id` cannot post in `conversation_id` (with `agent`, when given) now, if they cannot.
/// Checked when a routine or message is saved and again when it runs.
async fn target_problem(
    conn: &mut AnyConnection,
    workspace_id: Id,
    user_id: Id,
    conversation_id: Id,
    agent: Option<Id>,
) -> Result<Option<TargetProblem>, sqlx::Error> {
    let Some(row) = sqlx::query(
        "SELECT c.kind, c.archived_at, \
         EXISTS (SELECT 1 FROM chat_members m WHERE m.conversation_id = c.id AND m.user_id = $3) AS member, \
         EXISTS (SELECT 1 FROM chat_members m WHERE m.conversation_id = c.id AND m.user_id = $4) AS agent_member \
         FROM chat_conversations c WHERE c.id = $1 AND c.workspace_id = $2",
    )
    .bind(conversation_id.to_string())
    .bind(workspace_id.to_string())
    .bind(user_id.to_string())
    .bind(agent.map(|id| id.to_string()).unwrap_or_default())
    .fetch_optional(&mut *conn)
    .await?
    else {
        return Ok(Some(TargetProblem::Gone));
    };
    if row.get::<Option<i64>, _>("archived_at").is_some() {
        return Ok(Some(TargetProblem::Archived));
    }
    if !row.get::<Flag, _>("member").0 {
        return Ok(Some(TargetProblem::NotMember));
    }
    // A DM keeps the people it was opened with: the agent posts only in a DM it is in.
    if agent.is_some()
        && row.get::<String, _>("kind") == "dm"
        && !row.get::<Flag, _>("agent_member").0
    {
        return Ok(Some(TargetProblem::AgentCannotPost));
    }
    Ok(None)
}

fn clean_text(
    text: &str,
    max_chars: usize,
    empty: &'static str,
    long: &'static str,
) -> Result<String, ScheduledError> {
    let text = text.trim();
    if text.is_empty() {
        return Err(ScheduledError::Invalid(empty));
    }
    if text.chars().count() > max_chars {
        return Err(ScheduledError::Invalid(long));
    }
    Ok(text.to_owned())
}

fn clean_name(name: &str) -> Result<String, ScheduledError> {
    let line: String = name
        .chars()
        .map(|char| if char.is_control() { ' ' } else { char })
        .collect();
    clean_text(
        &line,
        NAME_MAX_CHARS,
        "Give the routine a name.",
        "The name can have at most 80 characters.",
    )
}

fn clean_prompt(prompt: &str) -> Result<String, ScheduledError> {
    clean_text(
        prompt,
        PROMPT_MAX_CHARS,
        "Say what the agent should do.",
        "The instruction can have at most 4,000 characters.",
    )
}

async fn profile_zone(conn: &mut AnyConnection, user_id: Id) -> Result<String, sqlx::Error> {
    let zone: Option<String> = sqlx::query_scalar("SELECT timezone FROM users WHERE id = $1")
        .bind(user_id.to_string())
        .fetch_optional(&mut *conn)
        .await?
        .flatten();
    Ok(zone
        .filter(|zone| time_zone(zone).is_ok())
        .unwrap_or_else(|| "UTC".to_owned()))
}

fn checked_schedule(
    expression: &str,
    zone: &str,
    now: i64,
) -> Result<(String, RoutineSchedule), ScheduledError> {
    let expression = expression.split_whitespace().collect::<Vec<_>>().join(" ");
    let schedule = RoutineSchedule::check(&expression, zone, now)
        .map_err(|error| ScheduledError::Invalid(error.detail()))?;
    Ok((expression, schedule))
}

// ---------------------------------------------------------------------------------------------
// Routines

#[derive(Clone, Debug, Serialize)]
pub struct RoutineRecord {
    pub id: String,
    pub workspace_id: String,
    /// The agent that runs it (its bot user).
    pub agent_user_id: String,
    /// The agent's name, as people mention it.
    pub agent_name: String,
    pub creator_id: String,
    pub creator_name: String,
    /// Where it posts; `null` when that conversation was deleted.
    pub conversation_id: Option<String>,
    pub conversation_name: Option<String>,
    pub conversation_kind: Option<String>,
    pub name: String,
    pub prompt: String,
    /// 5 fields: minute hour day month weekday.
    pub schedule: String,
    pub time_zone: String,
    /// `active` or `paused`.
    pub status: String,
    /// Why Orbit paused it; `null` when a person did or it runs.
    pub paused_reason: Option<String>,
    pub next_run_at: Option<i64>,
    pub last_run_at: Option<i64>,
    /// `started` (the agent got the run) or `skipped` (see `last_error`).
    pub last_run_status: Option<String>,
    pub last_run_message_id: Option<String>,
    pub last_error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    /// The caller may change the schedule, the instruction and the target, and resume it.
    pub can_edit: bool,
    /// The caller may pause and delete it.
    pub can_stop: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewRoutine {
    pub name: String,
    pub prompt: String,
    pub schedule: String,
    /// Default: the creator's profile time zone, else UTC.
    #[serde(default)]
    pub time_zone: Option<String>,
    /// Default: the creator's DM with the agent.
    #[serde(default)]
    pub conversation_id: Option<Id>,
    /// The agent that runs it; default: the default agent. An agent's own session always makes
    /// routines for itself.
    #[serde(default)]
    pub agent_user_id: Option<Id>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoutineChange {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub schedule: Option<String>,
    #[serde(default)]
    pub time_zone: Option<String>,
    #[serde(default)]
    pub conversation_id: Option<Id>,
    /// Another agent runs it (the default one or a custom agent of the workspace).
    #[serde(default)]
    pub agent_user_id: Option<Id>,
    /// `active` or `paused`.
    #[serde(default)]
    pub status: Option<String>,
}

const ROUTINE_SELECT: &str = "SELECT r.id, r.workspace_id, r.agent_user_id, ag.display_name AS agent_name, \
     r.creator_id, u.display_name AS creator_name, \
     r.conversation_id, c.name AS conversation_name, c.kind AS conversation_kind, r.name, r.prompt, r.schedule, \
     r.time_zone, r.status, r.paused_reason, r.next_run_at, r.last_run_at, r.last_run_status, \
     r.last_run_message_id, r.last_error, r.created_at, r.updated_at \
     FROM agent_routines r JOIN users u ON u.id = r.creator_id JOIN users ag ON ag.id = r.agent_user_id \
     LEFT JOIN chat_conversations c ON c.id = r.conversation_id";

fn routine_of(row: &sqlx::any::AnyRow, actor: Actor) -> Result<RoutineRecord, ScheduledError> {
    let creator: String = row.get("creator_id");
    let creator_id: Id = creator.parse().map_err(|_| ScheduledError::NotFound)?;
    Ok(RoutineRecord {
        id: row.get("id"),
        workspace_id: row.get("workspace_id"),
        agent_user_id: row.get("agent_user_id"),
        agent_name: row.get("agent_name"),
        creator_id: creator,
        creator_name: row.get("creator_name"),
        conversation_id: row.get("conversation_id"),
        conversation_name: row.get("conversation_name"),
        conversation_kind: row.get("conversation_kind"),
        name: row.get("name"),
        prompt: row.get("prompt"),
        schedule: row.get("schedule"),
        time_zone: row.get("time_zone"),
        status: row.get("status"),
        paused_reason: row.get("paused_reason"),
        next_run_at: row.get("next_run_at"),
        last_run_at: row.get("last_run_at"),
        last_run_status: row.get("last_run_status"),
        last_run_message_id: row.get("last_run_message_id"),
        last_error: row.get("last_error"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
        can_edit: actor.can_edit_routine(creator_id),
        can_stop: actor.can_stop_routine(creator_id),
    })
}

async fn load_routine(
    conn: &mut AnyConnection,
    workspace_id: Id,
    routine_id: Id,
    actor: Actor,
) -> Result<RoutineRecord, ScheduledError> {
    let row = sqlx::query(AssertSqlSafe(format!(
        "{ROUTINE_SELECT} WHERE r.id = $1 AND r.workspace_id = $2"
    )))
    .bind(routine_id.to_string())
    .bind(workspace_id.to_string())
    .fetch_optional(&mut *conn)
    .await?
    .ok_or(ScheduledError::NotFound)?;
    let routine = routine_of(&row, actor)?;
    // Someone else's routine is not there for a member who cannot manage routines.
    if !routine.can_stop {
        return Err(ScheduledError::NotFound);
    }
    Ok(routine)
}

/// The creator's DM with the agent, opened (and announced to both) when it does not exist yet.
async fn agent_dm(
    chat: &ChatState,
    workspace_id: Id,
    user_id: Id,
    agent: Id,
) -> Result<Id, ScheduledError> {
    let (written, _) = chat
        .publish(
            workspace_id,
            user_id,
            chat.chat.open_dm(workspace_id, user_id, vec![agent]),
        )
        .await
        .map_err(|_| ScheduledError::Invalid("Your DM with the agent could not be opened."))?;
    Ok(written.value.id)
}

/// The agent of a routine: the default agent, or a live custom agent of the workspace.
async fn routine_agent(
    conn: &mut AnyConnection,
    workspace_id: Id,
    wanted: Option<Id>,
) -> Result<Id, ScheduledError> {
    let donna = bot_user_id(conn).await?;
    match wanted {
        None => donna.ok_or(ScheduledError::AgentOff),
        Some(agent) if Some(agent) == donna => Ok(agent),
        Some(agent) => crate::agent::profiles::live_agent(conn, workspace_id, agent)
            .await?
            .map(|profile| profile.user_id)
            .ok_or(ScheduledError::Invalid("Pick an agent of this workspace.")),
    }
}

/// Refuses a custom agent's routine in a channel that is not one of its channels.
async fn agent_target(
    conn: &mut AnyConnection,
    workspace_id: Id,
    agent: Id,
    conversation: Id,
) -> Result<(), ScheduledError> {
    if let Some(profile) = crate::agent::profiles::live_agent(conn, workspace_id, agent).await?
        && !crate::agent::profiles::allowed_in(conn, &profile, conversation).await?
    {
        return Err(ScheduledError::Invalid(
            "This agent may not post there: pick one of its channels or your DM with it.",
        ));
    }
    Ok(())
}

pub struct Store {
    database: Database,
}

impl Store {
    #[must_use]
    pub const fn new(database: Database) -> Self {
        Self { database }
    }

    /// Own routines; every routine of the workspace for those who manage routines.
    pub async fn list_routines(
        &self,
        workspace_id: Id,
        user_id: Id,
    ) -> Result<Vec<RoutineRecord>, ScheduledError> {
        let mut conn = self.database.pool().acquire().await?;
        let actor = member(&mut conn, workspace_id, user_id).await?;
        let all = actor.can(Permission::RoutinesManage);
        let rows = sqlx::query(AssertSqlSafe(format!(
            "{ROUTINE_SELECT} WHERE r.workspace_id = $1 AND ($2 = 1 OR r.creator_id = $3) ORDER BY r.created_at, r.id"
        )))
        .bind(workspace_id.to_string())
        .bind(i64::from(all))
        .bind(user_id.to_string())
        .fetch_all(&mut *conn)
        .await?;
        rows.iter().map(|row| routine_of(row, actor)).collect()
    }

    pub async fn create_routine(
        &self,
        chat: &ChatState,
        workspace_id: Id,
        user_id: Id,
        input: NewRoutine,
        session_agent: Option<Option<Id>>,
        now: TimestampMillis,
    ) -> Result<RoutineRecord, ScheduledError> {
        let name = clean_name(&input.name)?;
        let prompt = clean_prompt(&input.prompt)?;
        let agent = {
            let mut conn = self.database.pool().acquire().await?;
            member(&mut conn, workspace_id, user_id).await?;
            let wanted = match session_agent {
                Some(own) => own,
                None => input.agent_user_id,
            };
            routine_agent(&mut conn, workspace_id, wanted).await?
        };
        let conversation_id = match input.conversation_id {
            Some(id) => id,
            None => agent_dm(chat, workspace_id, user_id, agent).await?,
        };
        let mut tx = self.database.immediate_transaction().await?;
        let actor = member(&mut tx, workspace_id, user_id).await?;
        let zone = match input.time_zone.filter(|zone| !zone.trim().is_empty()) {
            Some(zone) => zone.trim().to_owned(),
            None => profile_zone(&mut tx, user_id).await?,
        };
        let (expression, schedule) = checked_schedule(&input.schedule, &zone, now.as_millis())?;
        if let Some(problem) =
            target_problem(&mut tx, workspace_id, user_id, conversation_id, Some(agent)).await?
        {
            return Err(ScheduledError::Invalid(problem.detail()));
        }
        agent_target(&mut tx, workspace_id, agent, conversation_id).await?;
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM agent_routines WHERE workspace_id = $1 AND creator_id = $2",
        )
        .bind(workspace_id.to_string())
        .bind(user_id.to_string())
        .fetch_one(&mut *tx)
        .await?;
        if count >= MAX_ROUTINES_PER_USER {
            return Err(ScheduledError::Limit(
                "You have 25 routines in this workspace. Delete one first.",
            ));
        }
        let id = Id::new_v7();
        sqlx::query(
            "INSERT INTO agent_routines (id, workspace_id, agent_user_id, creator_id, conversation_id, name, prompt, \
             schedule, time_zone, status, next_run_at, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 'active', $10, $11, $11)",
        )
        .bind(id.to_string())
        .bind(workspace_id.to_string())
        .bind(agent.to_string())
        .bind(user_id.to_string())
        .bind(conversation_id.to_string())
        .bind(&name)
        .bind(&prompt)
        .bind(&expression)
        .bind(&zone)
        .bind(schedule.next_after(now.as_millis()))
        .bind(now.as_millis())
        .execute(&mut *tx)
        .await?;
        let routine = load_routine(&mut tx, workspace_id, id, actor).await?;
        tx.commit().await?;
        Ok(routine)
    }

    pub async fn update_routine(
        &self,
        workspace_id: Id,
        user_id: Id,
        routine_id: Id,
        change: RoutineChange,
        now: TimestampMillis,
    ) -> Result<RoutineRecord, ScheduledError> {
        let mut tx = self.database.immediate_transaction().await?;
        let actor = member(&mut tx, workspace_id, user_id).await?;
        let current = load_routine(&mut tx, workspace_id, routine_id, actor).await?;
        let edits = change.name.is_some()
            || change.prompt.is_some()
            || change.schedule.is_some()
            || change.time_zone.is_some()
            || change.conversation_id.is_some()
            || change.agent_user_id.is_some();
        let pause = match change.status.as_deref() {
            None => None,
            Some("paused") => Some(true),
            Some("active") => Some(false),
            Some(_) => {
                return Err(ScheduledError::Invalid(
                    "The status must be active or paused.",
                ));
            }
        };
        // Only the creator edits or resumes: the routine runs with the creator's permissions.
        if (edits || pause == Some(false)) && !current.can_edit {
            return Err(ScheduledError::Forbidden(
                "Only the person who made this routine can change or resume it.",
            ));
        }
        let creator: Id = current
            .creator_id
            .parse()
            .map_err(|_| ScheduledError::NotFound)?;
        let name = change
            .name
            .as_deref()
            .map(clean_name)
            .transpose()?
            .unwrap_or(current.name);
        let prompt = change
            .prompt
            .as_deref()
            .map(clean_prompt)
            .transpose()?
            .unwrap_or(current.prompt);
        let zone = change
            .time_zone
            .map(|zone| zone.trim().to_owned())
            .unwrap_or(current.time_zone);
        let (expression, schedule) = checked_schedule(
            change.schedule.as_deref().unwrap_or(&current.schedule),
            &zone,
            now.as_millis(),
        )?;
        let conversation = change
            .conversation_id
            .map(|id| id.to_string())
            .or(current.conversation_id);
        let active = match pause {
            Some(paused) => !paused,
            None => current.status == "active",
        };
        let agent: Id = match change.agent_user_id {
            Some(wanted) => routine_agent(&mut tx, workspace_id, Some(wanted)).await?,
            None => current
                .agent_user_id
                .parse()
                .map_err(|_| ScheduledError::NotFound)?,
        };
        if active {
            let Some(conversation) = conversation.as_deref().and_then(|id| id.parse().ok()) else {
                return Err(ScheduledError::Invalid("Pick where the routine posts."));
            };
            if let Some(problem) =
                target_problem(&mut tx, workspace_id, creator, conversation, Some(agent)).await?
            {
                return Err(ScheduledError::Invalid(problem.detail()));
            }
            agent_target(&mut tx, workspace_id, agent, conversation).await?;
        }
        // A new schedule, a resume or an edit of a waiting run starts counting from now.
        let next = if active {
            schedule.next_after(now.as_millis())
        } else {
            None
        };
        let status = if active { "active" } else { "paused" };
        sqlx::query(
            "UPDATE agent_routines SET name = $1, prompt = $2, schedule = $3, time_zone = $4, conversation_id = $5, \
             status = $6, paused_reason = NULL, next_run_at = $7, updated_at = $8, agent_user_id = $10 WHERE id = $9",
        )
        .bind(&name)
        .bind(&prompt)
        .bind(&expression)
        .bind(&zone)
        .bind(conversation)
        .bind(status)
        .bind(next)
        .bind(now.as_millis())
        .bind(routine_id.to_string())
        .bind(agent.to_string())
        .execute(&mut *tx)
        .await?;
        let routine = load_routine(&mut tx, workspace_id, routine_id, actor).await?;
        tx.commit().await?;
        Ok(routine)
    }

    pub async fn delete_routine(
        &self,
        workspace_id: Id,
        user_id: Id,
        routine_id: Id,
    ) -> Result<(), ScheduledError> {
        let mut tx = self.database.immediate_transaction().await?;
        let actor = member(&mut tx, workspace_id, user_id).await?;
        load_routine(&mut tx, workspace_id, routine_id, actor).await?;
        // A run that the agent has not taken yet goes with it.
        sqlx::query("DELETE FROM agent_inputs WHERE routine_id = $1 AND acked_at IS NULL")
            .bind(routine_id.to_string())
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM agent_routines WHERE id = $1")
            .bind(routine_id.to_string())
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    // -----------------------------------------------------------------------------------------
    // Scheduled messages and reminders

    pub async fn list_scheduled(
        &self,
        workspace_id: Id,
        user_id: Id,
    ) -> Result<Vec<ScheduledRecord>, ScheduledError> {
        let mut conn = self.database.pool().acquire().await?;
        member(&mut conn, workspace_id, user_id).await?;
        let rows = sqlx::query(AssertSqlSafe(format!(
            "{SCHEDULED_SELECT} WHERE s.workspace_id = $1 AND s.author_id = $2 ORDER BY s.send_at, s.id"
        )))
        .bind(workspace_id.to_string())
        .bind(user_id.to_string())
        .fetch_all(&mut *conn)
        .await?;
        Ok(rows.iter().map(scheduled_of).collect())
    }

    pub async fn create_scheduled(
        &self,
        workspace_id: Id,
        user_id: Id,
        input: NewScheduled,
        now: TimestampMillis,
    ) -> Result<ScheduledRecord, ScheduledError> {
        let body = clean_text(
            &input.body,
            BODY_MAX_CHARS,
            "Write the message.",
            "The message can have at most 4,000 characters.",
        )?;
        let mut tx = self.database.immediate_transaction().await?;
        member(&mut tx, workspace_id, user_id).await?;
        let zone = match input.time_zone.filter(|zone| !zone.trim().is_empty()) {
            Some(zone) => zone,
            None => profile_zone(&mut tx, user_id).await?,
        };
        let send_at = checked_send_at(&input.send_at, &zone, now)?;
        let conversation = match input.kind.as_str() {
            "message" => {
                let conversation = input.conversation_id.ok_or(ScheduledError::Invalid(
                    "Pick the conversation for the message.",
                ))?;
                if let Some(problem) =
                    target_problem(&mut tx, workspace_id, user_id, conversation, None).await?
                {
                    return Err(ScheduledError::Invalid(problem.detail()));
                }
                Some(conversation)
            }
            "reminder" => {
                if input.conversation_id.is_some() {
                    return Err(ScheduledError::Invalid(
                        "A reminder always comes in your DM with the agent.",
                    ));
                }
                bot_user_id(&mut tx)
                    .await?
                    .ok_or(ScheduledError::AgentOff)?;
                None
            }
            _ => {
                return Err(ScheduledError::Invalid(
                    "The kind must be message or reminder.",
                ));
            }
        };
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM scheduled_messages WHERE workspace_id = $1 AND author_id = $2",
        )
        .bind(workspace_id.to_string())
        .bind(user_id.to_string())
        .fetch_one(&mut *tx)
        .await?;
        if count >= MAX_SCHEDULED_PER_USER {
            return Err(ScheduledError::Limit(
                "You have 100 scheduled messages and reminders here. Delete some first.",
            ));
        }
        let id = Id::new_v7();
        sqlx::query(
            "INSERT INTO scheduled_messages (id, workspace_id, author_id, kind, conversation_id, body, send_at, \
             status, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7, 'scheduled', $8, $8)",
        )
        .bind(id.to_string())
        .bind(workspace_id.to_string())
        .bind(user_id.to_string())
        .bind(&input.kind)
        .bind(conversation.map(|id| id.to_string()))
        .bind(&body)
        .bind(send_at)
        .bind(now.as_millis())
        .execute(&mut *tx)
        .await?;
        let record = load_scheduled(&mut tx, workspace_id, user_id, id).await?;
        tx.commit().await?;
        Ok(record)
    }

    /// A new text or time. A failed message is scheduled again with it.
    pub async fn update_scheduled(
        &self,
        workspace_id: Id,
        user_id: Id,
        id: Id,
        change: ScheduledChange,
        now: TimestampMillis,
    ) -> Result<ScheduledRecord, ScheduledError> {
        let mut tx = self.database.immediate_transaction().await?;
        member(&mut tx, workspace_id, user_id).await?;
        let current = load_scheduled(&mut tx, workspace_id, user_id, id).await?;
        let body = match change.body {
            Some(body) => clean_text(
                &body,
                BODY_MAX_CHARS,
                "Write the message.",
                "The message can have at most 4,000 characters.",
            )?,
            None => current.body,
        };
        let send_at = match change.send_at {
            Some(send_at) => {
                let zone = match change.time_zone {
                    Some(zone) => zone,
                    None => profile_zone(&mut tx, user_id).await?,
                };
                checked_send_at(&send_at, &zone, now)?
            }
            None if current.status == "failed" => {
                return Err(ScheduledError::Invalid("Pick a new time for the message."));
            }
            None => current.send_at,
        };
        if let Some(conversation) = current
            .conversation_id
            .as_deref()
            .and_then(|id| id.parse().ok())
            && let Some(problem) =
                target_problem(&mut tx, workspace_id, user_id, conversation, None).await?
        {
            return Err(ScheduledError::Invalid(problem.detail()));
        }
        sqlx::query(
            "UPDATE scheduled_messages SET body = $1, send_at = $2, status = 'scheduled', error = NULL, updated_at = $3 \
             WHERE id = $4",
        )
        .bind(&body)
        .bind(send_at)
        .bind(now.as_millis())
        .bind(id.to_string())
        .execute(&mut *tx)
        .await?;
        let record = load_scheduled(&mut tx, workspace_id, user_id, id).await?;
        tx.commit().await?;
        Ok(record)
    }

    pub async fn delete_scheduled(
        &self,
        workspace_id: Id,
        user_id: Id,
        id: Id,
    ) -> Result<(), ScheduledError> {
        let mut tx = self.database.immediate_transaction().await?;
        member(&mut tx, workspace_id, user_id).await?;
        load_scheduled(&mut tx, workspace_id, user_id, id).await?;
        sqlx::query("DELETE FROM scheduled_messages WHERE id = $1")
            .bind(id.to_string())
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }
}

fn checked_send_at(text: &str, zone: &str, now: TimestampMillis) -> Result<i64, ScheduledError> {
    let send_at = parse_time(text, zone).map_err(ScheduledError::Invalid)?;
    // A minute of grace for a time picked "now" in a slow browser.
    if send_at < now.as_millis() - 60_000 {
        return Err(ScheduledError::Invalid("Pick a time in the future."));
    }
    if send_at > now.as_millis() + MAX_AHEAD_MS {
        return Err(ScheduledError::Invalid("Pick a time within one year."));
    }
    Ok(send_at)
}

#[derive(Clone, Debug, Serialize)]
pub struct ScheduledRecord {
    pub id: String,
    pub workspace_id: String,
    /// `message` (posted as the author) or `reminder` (posted by the agent in the author's DM with it).
    pub kind: String,
    pub conversation_id: Option<String>,
    pub conversation_name: Option<String>,
    pub conversation_kind: Option<String>,
    pub body: String,
    pub send_at: i64,
    /// `scheduled` or `failed` (see `error`). A sent message has no row any more.
    pub status: String,
    pub error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewScheduled {
    /// `message` or `reminder`.
    pub kind: String,
    /// For a message: where it is posted.
    #[serde(default)]
    pub conversation_id: Option<Id>,
    pub body: String,
    /// RFC 3339 with an offset, or a local time in `time_zone`.
    pub send_at: String,
    /// Default: the author's profile time zone, else UTC.
    #[serde(default)]
    pub time_zone: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScheduledChange {
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub send_at: Option<String>,
    #[serde(default)]
    pub time_zone: Option<String>,
}

const SCHEDULED_SELECT: &str = "SELECT s.id, s.workspace_id, s.kind, s.conversation_id, c.name AS conversation_name, \
     c.kind AS conversation_kind, s.body, s.send_at, s.status, s.error, s.created_at, s.updated_at \
     FROM scheduled_messages s LEFT JOIN chat_conversations c ON c.id = s.conversation_id";

fn scheduled_of(row: &sqlx::any::AnyRow) -> ScheduledRecord {
    ScheduledRecord {
        id: row.get("id"),
        workspace_id: row.get("workspace_id"),
        kind: row.get("kind"),
        conversation_id: row.get("conversation_id"),
        conversation_name: row.get("conversation_name"),
        conversation_kind: row.get("conversation_kind"),
        body: row.get("body"),
        send_at: row.get("send_at"),
        status: row.get("status"),
        error: row.get("error"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

/// Only the author sees a scheduled message, so for anyone else it does not exist.
async fn load_scheduled(
    conn: &mut AnyConnection,
    workspace_id: Id,
    user_id: Id,
    id: Id,
) -> Result<ScheduledRecord, ScheduledError> {
    let row = sqlx::query(AssertSqlSafe(format!(
        "{SCHEDULED_SELECT} WHERE s.id = $1 AND s.workspace_id = $2 AND s.author_id = $3"
    )))
    .bind(id.to_string())
    .bind(workspace_id.to_string())
    .bind(user_id.to_string())
    .fetch_optional(&mut *conn)
    .await?
    .ok_or(ScheduledError::NotFound)?;
    Ok(scheduled_of(&row))
}

// ---------------------------------------------------------------------------------------------
// For the runner

/// Why a routine of `creator` cannot run now: they left, became a guest or were suspended, or
/// lost the conversation. `None`: it can run.
pub(super) async fn routine_problem(
    conn: &mut AnyConnection,
    workspace_id: Id,
    creator: Id,
    agent: Id,
    conversation: Option<Id>,
) -> Result<Option<&'static str>, sqlx::Error> {
    let role: Option<String> = sqlx::query_scalar(
        "SELECT m.role FROM memberships m JOIN users u ON u.id = m.user_id \
         WHERE m.workspace_id = $1 AND m.user_id = $2 AND u.suspended_at IS NULL",
    )
    .bind(workspace_id.to_string())
    .bind(creator.to_string())
    .fetch_optional(&mut *conn)
    .await?;
    match role.as_deref() {
        None => {
            return Ok(Some(
                "Paused: its creator is no longer an active member of this workspace.",
            ));
        }
        Some("guest") => {
            return Ok(Some(
                "Paused: its creator is a guest now, and guests cannot use the agent.",
            ));
        }
        Some(_) => {}
    }
    let agent_member: bool = sqlx::query_scalar::<_, Flag>(
        "SELECT EXISTS (SELECT 1 FROM memberships WHERE workspace_id = $1 AND user_id = $2)",
    )
    .bind(workspace_id.to_string())
    .bind(agent.to_string())
    .fetch_one(&mut *conn)
    .await?
    .0;
    if !agent_member {
        return Ok(Some("Paused: the agent is no longer in this workspace."));
    }
    let Some(conversation) = conversation else {
        return Ok(Some("Paused: the conversation it posts to was deleted."));
    };
    let custom = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT deleted_at FROM agent_profiles WHERE user_id = $1",
    )
    .bind(agent.to_string())
    .fetch_optional(&mut *conn)
    .await?;
    match custom {
        Some(Some(_)) => return Ok(Some("Paused: its agent was deleted.")),
        Some(None) => {
            if let Some(profile) =
                crate::agent::profiles::live_agent(conn, workspace_id, agent).await?
                && !crate::agent::profiles::allowed_in(conn, &profile, conversation).await?
            {
                return Ok(Some(
                    "Paused: its agent may no longer post in the channel it posts to.",
                ));
            }
        }
        None => {}
    }
    Ok(
        target_problem(conn, workspace_id, creator, conversation, Some(agent))
            .await?
            .map(TargetProblem::routine_reason),
    )
}

/// Why `author` cannot post in `conversation` now (for a scheduled message), if they cannot.
pub(super) async fn message_problem(
    conn: &mut AnyConnection,
    workspace_id: Id,
    author: Id,
    conversation: Option<Id>,
) -> Result<Option<&'static str>, ScheduledError> {
    match member(conn, workspace_id, author).await {
        Ok(_) => {}
        Err(ScheduledError::Forbidden(_)) => {
            return Ok(Some("Not sent: guests cannot schedule messages."));
        }
        Err(ScheduledError::NotFound) => {
            return Ok(Some(
                "Not sent: you are no longer a member of this workspace.",
            ));
        }
        Err(error) => return Err(error),
    }
    let Some(conversation) = conversation else {
        return Ok(None);
    };
    Ok(
        target_problem(conn, workspace_id, author, conversation, None)
            .await?
            .map(TargetProblem::message_reason),
    )
}
