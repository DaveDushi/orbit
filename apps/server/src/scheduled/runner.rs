//! The loop that runs due routines and sends due messages and reminders. Every server runs it;
//! the database decides who does what, so nothing runs twice:
//!
//! - A routine run is claimed with a compare-and-set on `next_run_at`, which moves to the next run
//!   after *now*: a routine that was due while the server was down runs once, not once per missed
//!   time. The claim stores `pending_run_id`; posting the run uses it as the nonce of the agent's
//!   first message and as the id of the agent input, so a run that a restart interrupts is
//!   finished later with the same message and input, never twice.
//! - A message or reminder is posted with the nonce `scheduled:<id>`, so a second server (or a
//!   retry after a crash) gets the same message back instead of a new one; its row goes after.

use std::time::Duration;

use orbit_platform::{Id, Row, TimestampMillis};
use tokio_util::sync::CancellationToken;

use super::schedule::RoutineSchedule;
use super::store::{ScheduledError, message_problem, routine_problem};
use crate::agent::AgentState;
use crate::agent::inputs::bot_user_id;
use crate::push::PushService;
use crate::repositories::chat::{ChatError, MessageRecord, SendInput};

/// How often the loop looks for due work.
pub const TICK: Duration = Duration::from_secs(10);
/// Rows of each kind one round takes; the rest wait for the next round.
const BATCH: i64 = 50;
/// A claimed run whose posting did not finish is taken up again after this long.
const PENDING_RETRY_MS: i64 = 30_000;

/// What one round did, for tests.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Round {
    pub messages_sent: usize,
    pub messages_failed: usize,
    pub routines_started: usize,
    pub routines_skipped: usize,
    pub routines_paused: usize,
}

pub async fn run_service(state: AgentState, token: CancellationToken) -> Result<(), String> {
    let mut tick = tokio::time::interval(TICK);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            () = token.cancelled() => return Ok(()),
            _ = tick.tick() => {}
        }
        // Never fatal: a locked or busy database is tried again on the next round.
        if let Err(error) = run_due(&state, TimestampMillis::now()).await {
            tracing::warn!(%error, "scheduled routines and messages");
        }
    }
}

/// Runs everything due at `now`.
pub async fn run_due(state: &AgentState, now: TimestampMillis) -> Result<Round, sqlx::Error> {
    let mut round = Round::default();
    send_due_messages(state, now, &mut round).await?;
    // Without the agent, routines wait; each runs once when it is back.
    if state.config.is_some() {
        run_due_routines(state, now, &mut round).await?;
    }
    Ok(round)
}

// ---------------------------------------------------------------------------------------------
// Messages and reminders

async fn send_due_messages(
    state: &AgentState,
    now: TimestampMillis,
    round: &mut Round,
) -> Result<(), sqlx::Error> {
    let database = state.database();
    let rows = sqlx::query(
        "SELECT id, workspace_id, author_id, kind, conversation_id, body FROM scheduled_messages \
         WHERE status = 'scheduled' AND send_at <= $1 ORDER BY send_at, id LIMIT $2",
    )
    .bind(now.as_millis())
    .bind(BATCH)
    .fetch_all(database.pool())
    .await?;
    for row in rows {
        let (Ok(id), Ok(workspace_id), Ok(author)) = (
            row.get::<String, _>("id").parse::<Id>(),
            row.get::<String, _>("workspace_id").parse::<Id>(),
            row.get::<String, _>("author_id").parse::<Id>(),
        ) else {
            continue;
        };
        let conversation = row
            .get::<Option<String>, _>("conversation_id")
            .and_then(|id| id.parse().ok());
        let reminder = row.get::<String, _>("kind") == "reminder";
        let body: String = row.get("body");
        let problem = {
            let mut conn = database.pool().acquire().await?;
            match message_problem(&mut conn, workspace_id, author, conversation).await {
                Ok(problem) => problem,
                Err(super::store::ScheduledError::Database(error)) => return Err(error),
                Err(_) => Some("Not sent."),
            }
        };
        let sent = match problem {
            Some(problem) => Err(problem),
            None if reminder => send_reminder(state, workspace_id, author, id, &body).await?,
            None => match conversation {
                Some(conversation) => {
                    post(
                        state,
                        workspace_id,
                        author,
                        conversation,
                        None,
                        body,
                        format!("scheduled:{id}"),
                    )
                    .await?
                }
                None => Err("Not sent."),
            },
        };
        match sent {
            Ok(_) => {
                sqlx::query("DELETE FROM scheduled_messages WHERE id = $1")
                    .bind(id.to_string())
                    .execute(database.pool())
                    .await?;
                round.messages_sent += 1;
            }
            Err(error) => {
                sqlx::query(
                    "UPDATE scheduled_messages SET status = 'failed', error = $1, updated_at = $2 \
                     WHERE id = $3 AND status = 'scheduled'",
                )
                .bind(error)
                .bind(now.as_millis())
                .bind(id.to_string())
                .execute(database.pool())
                .await?;
                round.messages_failed += 1;
            }
        }
    }
    Ok(())
}

/// The reminder as a message of the agent in the author's DM with it, which notifies them.
async fn send_reminder(
    state: &AgentState,
    workspace_id: Id,
    author: Id,
    id: Id,
    body: &str,
) -> Result<Result<MessageRecord, &'static str>, sqlx::Error> {
    let bot = {
        let mut conn = state.database().pool().acquire().await?;
        bot_user_id(&mut conn).await?
    };
    let Some(bot) = bot else {
        return Ok(Err("Not sent: the agent is off."));
    };
    let chat = &state.chat;
    let dm = match chat
        .publish(
            workspace_id,
            bot,
            chat.chat.open_dm(workspace_id, bot, vec![author]),
        )
        .await
    {
        Ok((written, _)) => written.value.id,
        Err(ChatError::Unavailable(error)) => return Err(error),
        Err(_) => return Ok(Err("Not sent: the agent could not open your DM with it.")),
    };
    post(
        state,
        workspace_id,
        bot,
        dm,
        None,
        format!("⏰ Reminder: {body}"),
        format!("scheduled:{id}"),
    )
    .await
}

/// Posts through the normal chat send (live events, unread counts, mentions, the agent's queue)
/// and push. `Ok(Err(..))` is a refusal that will not change by trying again.
async fn post(
    state: &AgentState,
    workspace_id: Id,
    author: Id,
    conversation_id: Id,
    thread_root_id: Option<Id>,
    body: String,
    nonce: String,
) -> Result<Result<MessageRecord, &'static str>, sqlx::Error> {
    let chat = &state.chat;
    let input = SendInput {
        conversation_id,
        thread_root_id,
        reply_to_id: None,
        body,
        sticker_id: None,
        file_ids: Vec::new(),
        online: chat.hub.online(workspace_id),
        also_in_channel: false,
        nonce,
        page_context: None,
    };
    match chat
        .publish(
            workspace_id,
            author,
            chat.chat.send_message(workspace_id, author, input),
        )
        .await
    {
        Ok((written, _)) => {
            PushService::of(state.database()).chat_message(&written.value);
            Ok(Ok(written.value))
        }
        Err(ChatError::Unavailable(error)) => Err(error),
        Err(ChatError::TooLong) => Ok(Err("Not sent: the message is too long.")),
        Err(ChatError::Forbidden(reason)) => Ok(Err(reason)),
        Err(error) => {
            tracing::warn!(%error, "scheduled message");
            Ok(Err("Not sent: the conversation does not take it."))
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Routines

async fn run_due_routines(
    state: &AgentState,
    now: TimestampMillis,
    round: &mut Round,
) -> Result<(), sqlx::Error> {
    let rows = sqlx::query(
        "SELECT id, pending_run_id FROM agent_routines \
         WHERE (status = 'active' AND next_run_at <= $1) OR (pending_run_id IS NOT NULL AND last_run_at <= $2) \
         ORDER BY next_run_at, id LIMIT $3",
    )
    .bind(now.as_millis())
    .bind(now.as_millis() - PENDING_RETRY_MS)
    .bind(BATCH)
    .fetch_all(state.database().pool())
    .await?;
    for row in rows {
        let Ok(id) = row.get::<String, _>("id").parse::<Id>() else {
            continue;
        };
        let pending = row
            .get::<Option<String>, _>("pending_run_id")
            .and_then(|id| id.parse().ok());
        let run = match pending {
            Some(run) => Some(run),
            None => claim(state, id, now, round).await?,
        };
        if let Some(run) = run {
            start(state, id, run, now, round).await?;
        }
    }
    Ok(())
}

/// Takes one due run of the routine: pauses it when its creator lost access, skips it without a
/// ChatGPT connection, or claims it. `Some(run id)` when this caller won the claim.
async fn claim(
    state: &AgentState,
    id: Id,
    now: TimestampMillis,
    round: &mut Round,
) -> Result<Option<Id>, sqlx::Error> {
    let mut tx = state.database().immediate_transaction().await?;
    let Some(row) = sqlx::query(
        "SELECT workspace_id, agent_user_id, creator_id, conversation_id, schedule, time_zone, next_run_at \
         FROM agent_routines WHERE id = $1 AND status = 'active' AND next_run_at <= $2 AND pending_run_id IS NULL",
    )
    .bind(id.to_string())
    .bind(now.as_millis())
    .fetch_optional(&mut *tx)
    .await?
    else {
        // Another server took it, or it changed.
        return Ok(None);
    };
    let (Ok(workspace_id), Ok(agent), Ok(creator)) = (
        row.get::<String, _>("workspace_id").parse::<Id>(),
        row.get::<String, _>("agent_user_id").parse::<Id>(),
        row.get::<String, _>("creator_id").parse::<Id>(),
    ) else {
        return Ok(None);
    };
    let conversation = row
        .get::<Option<String>, _>("conversation_id")
        .and_then(|id| id.parse().ok());
    if let Some(reason) =
        routine_problem(&mut tx, workspace_id, creator, agent, conversation).await?
    {
        sqlx::query(
            "UPDATE agent_routines SET status = 'paused', paused_reason = $1, next_run_at = NULL, updated_at = $2 \
             WHERE id = $3",
        )
        .bind(reason)
        .bind(now.as_millis())
        .bind(id.to_string())
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        round.routines_paused += 1;
        return Ok(None);
    }
    // The next run counts from now: missed runs while the server was down run once.
    let next = RoutineSchedule::parse(
        &row.get::<String, _>("schedule"),
        &row.get::<String, _>("time_zone"),
    )
    .ok()
    .and_then(|schedule| schedule.next_after(now.as_millis()));
    let connected = sqlx::query_scalar::<_, orbit_platform::Flag>(
        "SELECT EXISTS (SELECT 1 FROM agent_credentials WHERE user_id = $1 AND invalid_at IS NULL)",
    )
    .bind(creator.to_string())
    .fetch_one(&mut *tx)
    .await?
    .0;
    let run = connected.then(Id::new_v7);
    let script = super::runs::routine_script(&mut tx, id).await?;
    super::runs::insert_run(
        &mut tx,
        super::runs::NewRun {
            id: run.unwrap_or_else(Id::new_v7),
            routine_id: id,
            workspace_id,
            test: false,
            status: if connected { "started" } else { "skipped" },
            writes: super::runs::writes_of(script.as_ref()),
            script: script.as_ref(),
            error: (!connected)
                .then_some("Skipped: connect your ChatGPT account in Settings → Agentic."),
            now: now.as_millis(),
        },
    )
    .await?;
    let (status, error) = if connected {
        ("started", None)
    } else {
        (
            "skipped",
            Some("Skipped: connect your ChatGPT account in Settings → Agentic."),
        )
    };
    // A routine whose schedule no longer runs (or cannot be read) pauses after this run.
    sqlx::query(
        "UPDATE agent_routines SET next_run_at = $1, status = $2, pending_run_id = $3, last_run_at = $4, \
         last_run_status = $5, last_error = $6, updated_at = $4 WHERE id = $7",
    )
    .bind(next)
    .bind(if next.is_some() { "active" } else { "paused" })
    .bind(run.map(|run| run.to_string()))
    .bind(now.as_millis())
    .bind(status)
    .bind(error)
    .bind(id.to_string())
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    if run.is_none() {
        round.routines_skipped += 1;
    }
    Ok(run)
}

/// Posts the run's first message as the agent and queues the run for it, as the creator. Safe to
/// call again for the same run: the nonce gives the same message, the input id the same input.
async fn start(
    state: &AgentState,
    id: Id,
    run: Id,
    now: TimestampMillis,
    round: &mut Round,
) -> Result<(), sqlx::Error> {
    let database = state.database();
    let Some(row) = sqlx::query(
        "SELECT workspace_id, agent_user_id, creator_id, conversation_id, name FROM agent_routines \
         WHERE id = $1 AND pending_run_id = $2",
    )
    .bind(id.to_string())
    .bind(run.to_string())
    .fetch_optional(database.pool())
    .await?
    else {
        return Ok(());
    };
    let (Ok(workspace_id), Ok(agent), Ok(creator), Some(conversation)) = (
        row.get::<String, _>("workspace_id").parse::<Id>(),
        row.get::<String, _>("agent_user_id").parse::<Id>(),
        row.get::<String, _>("creator_id").parse::<Id>(),
        row.get::<Option<String>, _>("conversation_id")
            .and_then(|id| id.parse::<Id>().ok()),
    ) else {
        return give_up(
            state,
            id,
            run,
            "Paused: the conversation it posts to was deleted.",
            now,
        )
        .await;
    };
    let name: String = row.get("name");
    let header = format!("⏳ Routine **{}** is starting…", name.replace('*', ""));
    let Some(header_id) = post_run(
        state,
        (workspace_id, agent, creator),
        (id, run),
        conversation,
        header,
        now,
    )
    .await?
    else {
        return give_up(
            state,
            id,
            run,
            "Paused: the agent cannot post in the conversation it posts to.",
            now,
        )
        .await;
    };
    sqlx::query(
        "UPDATE agent_routines SET pending_run_id = NULL, last_run_message_id = $1 WHERE id = $2 AND pending_run_id = $3",
    )
    .bind(header_id.to_string())
    .bind(id.to_string())
    .bind(run.to_string())
    .execute(database.pool())
    .await?;
    round.routines_started += 1;
    Ok(())
}

/// Posts a run's first message as the agent in `conversation` and queues the run for it, as the
/// creator, with the run's thread on its history row. Safe to call again for the same run: the
/// nonce gives the same message, the input id the same input. `None`: the agent cannot post there.
async fn post_run(
    state: &AgentState,
    (workspace_id, agent, creator): (Id, Id, Id),
    (routine_id, run): (Id, Id),
    conversation: Id,
    header: String,
    now: TimestampMillis,
) -> Result<Option<Id>, sqlx::Error> {
    let database = state.database();
    join_conversation(state, conversation, agent, now).await?;
    // The sidecar posts its progress with the nonce `agent:<input id>`: it finds this message and
    // writes the answer into it, so the answer is in the conversation, and questions and
    // approvals go to the thread under it.
    let Ok(header) = post(
        state,
        workspace_id,
        agent,
        conversation,
        None,
        header,
        format!("agent:{run}"),
    )
    .await?
    else {
        return Ok(None);
    };
    // A custom agent's run has its own thread key, as in chat (`inputs::agent_thread_key`).
    let custom = {
        let mut conn = database.pool().acquire().await?;
        crate::agent::profiles::live_agent(&mut conn, workspace_id, agent)
            .await?
            .map(|profile| profile.user_id)
    };
    let thread_key = crate::agent::inputs::agent_thread_key(custom, header.id);
    let mut tx = database.immediate_transaction().await?;
    // The thread first: the run's agent sessions read from it whether they may write.
    sqlx::query(
        "UPDATE agent_routine_runs SET conversation_id = $1, message_id = $2, thread_key = $3 WHERE id = $4",
    )
    .bind(conversation.to_string())
    .bind(header.id.to_string())
    .bind(thread_key.to_string())
    .bind(run.to_string())
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO agent_inputs (message_id, workspace_id, conversation_id, thread_key, root_message_id, \
         author_id, kind, created_at, routine_id, agent_user_id) \
         VALUES ($1, $2, $3, $4, $5, $6, 'instruction', $7, $8, $9) ON CONFLICT (message_id) DO NOTHING",
    )
    .bind(run.to_string())
    .bind(workspace_id.to_string())
    .bind(conversation.to_string())
    .bind(thread_key.to_string())
    .bind(header.id.to_string())
    .bind(creator.to_string())
    .bind(now.as_millis())
    .bind(routine_id.to_string())
    .bind(custom.map(|id| id.to_string()))
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Some(header.id))
}

async fn give_up(
    state: &AgentState,
    id: Id,
    run: Id,
    reason: &str,
    now: TimestampMillis,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE agent_routines SET status = 'paused', paused_reason = $1, next_run_at = NULL, pending_run_id = NULL, \
         last_run_status = 'skipped', last_error = $1, updated_at = $2 WHERE id = $3 AND pending_run_id = $4",
    )
    .bind(reason)
    .bind(now.as_millis())
    .bind(id.to_string())
    .bind(run.to_string())
    .execute(state.database().pool())
    .await?;
    sqlx::query(
        "UPDATE agent_routine_runs SET status = 'failed', error = $1, finished_at = $2 WHERE id = $3 AND status = 'started'",
    )
    .bind(reason)
    .bind(now.as_millis())
    .bind(run.to_string())
    .execute(state.database().pool())
    .await?;
    Ok(())
}

/// The agent joins a channel it is not in yet with everything read, as when a member mentions it
/// there (`agent::inputs::record`). A DM it is not in was refused when the routine was saved.
async fn join_conversation(
    state: &AgentState,
    conversation: Id,
    agent: Id,
    now: TimestampMillis,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO chat_members (conversation_id, user_id, workspace_id, notify, last_read_message_id, \
         read_count, joined_at) \
         SELECT c.id, $2, c.workspace_id, 'mentions', NULL, c.message_count, $3 FROM chat_conversations c \
         WHERE c.id = $1 AND c.kind <> 'dm' AND EXISTS (SELECT 1 FROM memberships m \
             WHERE m.workspace_id = c.workspace_id AND m.user_id = $2) \
         ON CONFLICT (conversation_id, user_id) DO NOTHING",
    )
    .bind(conversation.to_string())
    .bind(agent.to_string())
    .bind(now.as_millis())
    .execute(state.database().pool())
    .await?;
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Test runs

/// A test run of a routine, now, by its creator (`POST …/routines/{id}/test`, the agent's
/// `test_routine`). It runs as a normal run of the routine (its script or its instruction) as the
/// creator, but posts only in the creator's DM with the routine's agent, marked "Test run", and is a
/// dry run: Orbit makes none of its writes, and the answer says what it would do (`super::runs`).
pub async fn test_run(
    state: &AgentState,
    workspace_id: Id,
    user_id: Id,
    routine_id: Id,
    now: TimestampMillis,
) -> Result<super::runs::RunRecord, ScheduledError> {
    if state.config.is_none() {
        return Err(ScheduledError::AgentOff);
    }
    let database = state.database();
    let routine = {
        let mut conn = database.pool().acquire().await?;
        let actor = super::store::member(&mut conn, workspace_id, user_id).await?;
        super::store::load_routine(&mut conn, workspace_id, routine_id, actor).await?
    };
    if !routine.can_edit {
        return Err(ScheduledError::Forbidden(
            "Only the person who made this routine can test it: it runs as them.",
        ));
    }
    let agent: Id = routine
        .agent_user_id
        .parse()
        .map_err(|_| ScheduledError::NotFound)?;
    let dm = super::store::agent_dm(&state.chat, workspace_id, user_id, agent).await?;
    let run = Id::new_v7();
    {
        let mut tx = database.immediate_transaction().await?;
        if let Some(reason) =
            routine_problem(&mut tx, workspace_id, user_id, agent, Some(dm)).await?
        {
            return Err(ScheduledError::Invalid(reason));
        }
        let connected = sqlx::query_scalar::<_, orbit_platform::Flag>(
            "SELECT EXISTS (SELECT 1 FROM agent_credentials WHERE user_id = $1 AND invalid_at IS NULL)",
        )
        .bind(user_id.to_string())
        .fetch_one(&mut *tx)
        .await?
        .0;
        if !connected {
            return Err(ScheduledError::Invalid(
                "Connect your ChatGPT account in Settings → Agentic first.",
            ));
        }
        let going = sqlx::query_scalar::<_, orbit_platform::Flag>(
            "SELECT EXISTS (SELECT 1 FROM agent_routine_runs WHERE routine_id = $1 AND test = 1 \
             AND status = 'started' AND started_at > $2)",
        )
        .bind(routine_id.to_string())
        .bind(now.as_millis() - super::runs::TEST_RUN_STALE_MS)
        .fetch_one(&mut *tx)
        .await?
        .0;
        if going {
            return Err(ScheduledError::Limit(
                "A test run of this routine is still going. Wait for its result.",
            ));
        }
        let script = super::runs::routine_script(&mut tx, routine_id).await?;
        super::runs::insert_run(
            &mut tx,
            super::runs::NewRun {
                id: run,
                routine_id,
                workspace_id,
                test: true,
                status: "started",
                writes: "dry",
                script: script.as_ref(),
                error: None,
                now: now.as_millis(),
            },
        )
        .await?;
        tx.commit().await?;
    }
    let header = format!(
        "🧪 Test run of the routine **{}**: nothing is written or posted elsewhere…",
        routine.name.replace('*', "")
    );
    let posted = post_run(
        state,
        (workspace_id, agent, user_id),
        (routine_id, run),
        dm,
        header,
        now,
    )
    .await?;
    if posted.is_none() {
        sqlx::query(
            "UPDATE agent_routine_runs SET status = 'failed', error = $1, finished_at = $2 WHERE id = $3",
        )
        .bind("The agent could not post in your DM with it.")
        .bind(now.as_millis())
        .bind(run.to_string())
        .execute(database.pool())
        .await?;
    }
    let mut conn = database.pool().acquire().await?;
    super::runs::load_run(&mut conn, run).await
}
