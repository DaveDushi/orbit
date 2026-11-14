//! Agent runs on tasks. A person who assigns a task to an agent, mentions an agent in a comment on
//! a task, or comments on a task an agent is assigned to queues a run in the transaction of that
//! write ([`assigned`], [`commented`]). The **driver** of the run is that person: Orbit queues it
//! as their agent input in their DM with the agent, so the run has only their permissions, uses
//! their ChatGPT connection, and asks them for answers and approvals in that DM thread. The agent
//! writes its progress and result into one comment on the task, as itself ([`write_comment`]).
//!
//! - **No loops.** Writes by an agent (its comments), through an agent session (an agent acting as
//!   its driver) and by service-account tokens start nothing. Guests see no task, so they never
//!   start or drive a run.
//! - **One run per task.** A partial unique index allows one `running` run per task. Other triggers
//!   queue; a trigger by the same person for the same agent while their run is queued joins it.
//!   The sidecar's poll starts the oldest queued run of a task once nothing runs on it
//!   ([`start_queued`]), after checking again that the driver and the agent still see the task.
//! - **Stop.** Unassigning the agent or deleting the task cancels its queued and running runs
//!   ([`cancel`]): the input is acknowledged, the thread idle and the driver's agent sessions with
//!   that agent deleted, so Orbit refuses the run's next call at once; the sidecar gets the run in
//!   `stops` and aborts it ([`stopping`], [`stopped`]).
//!
//! Design and reasons: `.ai/agent-design.md`, "Task runs".

use orbit_platform::{Database, Flag, Id, Row, TimestampMillis};
use serde::Serialize;
use sqlx::AnyConnection;

use super::AgentState;
use super::inputs::ThreadStatus;
use crate::push::PushService;
use crate::repositories::chat::{ChatError, SendInput};
use crate::repositories::project_access::user_sees_task;
use crate::repositories::tasks::{CommentContent, TaskError, TaskRepository};

/// What queued a run.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Cause {
    /// The driver assigned the task to the agent.
    Assigned,
    /// The driver mentioned the agent in a comment on the task.
    Mentioned,
    /// The driver commented on a task the agent is assigned to.
    Commented,
}

impl Cause {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Assigned => "assigned",
            Self::Mentioned => "mentioned",
            Self::Commented => "commented",
        }
    }
}

/// A claimed run whose header and input were not written (a crash in between) is posted again
/// after this long.
const POST_RETRY_MS: i64 = 30_000;
/// The longest comment the agent writes, in characters.
const COMMENT_MAX_CHARS: usize = 60_000;

/// Whether `user_id` is an agent's bot user: the default agent's, or a custom agent's (deleted
/// ones too).
async fn is_agent(conn: &mut AnyConnection, user_id: Id) -> Result<bool, sqlx::Error> {
    if super::inputs::bot_user_id(conn).await? == Some(user_id) {
        return Ok(true);
    }
    Ok(sqlx::query_scalar::<_, Flag>(
        "SELECT EXISTS (SELECT 1 FROM agent_profiles WHERE user_id = $1)",
    )
    .bind(user_id.to_string())
    .fetch_one(&mut *conn)
    .await?
    .0)
}

/// Whether `agent` is a live agent of the workspace (the default agent, or a custom agent that is
/// not deleted).
async fn is_live_agent(
    conn: &mut AnyConnection,
    workspace_id: Id,
    agent: Id,
) -> Result<bool, sqlx::Error> {
    Ok(live_agents(conn, workspace_id).await?.contains(&agent))
}

/// The workspace's live agents: the default agent's bot and the custom agents not deleted.
async fn live_agents(conn: &mut AnyConnection, workspace_id: Id) -> Result<Vec<Id>, sqlx::Error> {
    let donna = super::inputs::bot_user_id(conn).await?;
    let custom = super::profiles::workspace_agents(conn, workspace_id).await?;
    Ok(donna
        .into_iter()
        .chain(
            custom
                .into_iter()
                .filter(|(_, deleted)| !deleted)
                .map(|(id, _)| id),
        )
        .collect())
}

/// The agents among `candidates` (of `live`) that see the task (a private project's agent must be
/// on its member list), each once, in order.
async fn agents_among(
    conn: &mut AnyConnection,
    live: &[Id],
    task_id: Id,
    candidates: &[Id],
) -> Result<Vec<Id>, sqlx::Error> {
    let mut agents = Vec::new();
    for id in candidates {
        if !agents.contains(id)
            && live.contains(id)
            && user_sees_task(&mut *conn, task_id, *id).await?
        {
            agents.push(*id);
        }
    }
    Ok(agents)
}

/// Whether `actor` may start a run on the task and drive it: a person, not an agent, who sees the
/// task (every member who sees a task may edit it; guests see none). `automated`: the write came
/// through an agent session or a service-account token.
async fn may_drive(
    conn: &mut AnyConnection,
    task_id: Id,
    actor: Id,
    automated: bool,
) -> Result<bool, sqlx::Error> {
    if automated || is_agent(conn, actor).await? {
        return Ok(false);
    }
    user_sees_task(&mut *conn, task_id, actor).await
}

/// Queues a run of `agent` on the task for `driver`. A queued run of the same agent and driver
/// takes the newer comment instead (it reads the task when it starts, so nothing is lost).
#[allow(clippy::too_many_arguments)]
async fn queue(
    conn: &mut AnyConnection,
    workspace_id: Id,
    task_id: Id,
    agent: Id,
    driver: Id,
    cause: Cause,
    comment_id: Option<Id>,
    now: TimestampMillis,
) -> Result<(), sqlx::Error> {
    let joined = sqlx::query_scalar::<_, String>(
        "SELECT id FROM agent_task_runs WHERE task_id = $1 AND agent_user_id = $2 \
         AND driver_user_id = $3 AND status = 'queued'",
    )
    .bind(task_id.to_string())
    .bind(agent.to_string())
    .bind(driver.to_string())
    .fetch_optional(&mut *conn)
    .await?;
    if let Some(id) = joined {
        if let Some(comment_id) = comment_id {
            sqlx::query("UPDATE agent_task_runs SET cause = $1, comment_id = $2 WHERE id = $3")
                .bind(cause.as_str())
                .bind(comment_id.to_string())
                .bind(id)
                .execute(&mut *conn)
                .await?;
        }
        return Ok(());
    }
    sqlx::query(
        "INSERT INTO agent_task_runs (id, workspace_id, task_id, agent_user_id, driver_user_id, \
         cause, comment_id, status, created_at) VALUES ($1, $2, $3, $4, $5, $6, $7, 'queued', $8)",
    )
    .bind(Id::new_v7().to_string())
    .bind(workspace_id.to_string())
    .bind(task_id.to_string())
    .bind(agent.to_string())
    .bind(driver.to_string())
    .bind(cause.as_str())
    .bind(comment_id.map(|id| id.to_string()))
    .bind(now.as_millis())
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// In the transaction that changes a task's assignees: removed agents' runs on the task stop;
/// each added live agent that sees the task gets a run driven by `actor`, when `actor` may drive
/// one (`automated`: see [`may_drive`]).
#[allow(clippy::too_many_arguments)]
pub(crate) async fn assigned(
    conn: &mut AnyConnection,
    workspace_id: Id,
    task_id: Id,
    actor: Id,
    added: &[Id],
    removed: &[Id],
    automated: bool,
    now: TimestampMillis,
) -> Result<(), sqlx::Error> {
    if !removed.is_empty() {
        cancel(
            conn,
            task_id,
            Some(removed),
            "I was unassigned from this task",
            now,
        )
        .await?;
    }
    if added.is_empty() {
        return Ok(());
    }
    let live = live_agents(conn, workspace_id).await?;
    if !added.iter().any(|id| live.contains(id)) {
        return Ok(());
    }
    let agents = agents_among(conn, &live, task_id, added).await?;
    if agents.is_empty() || !may_drive(conn, task_id, actor, automated).await? {
        return Ok(());
    }
    for agent in agents {
        queue(
            conn,
            workspace_id,
            task_id,
            agent,
            actor,
            Cause::Assigned,
            None,
            now,
        )
        .await?;
    }
    Ok(())
}

/// In the transaction that writes a comment: the agents it mentions, and the agents assigned to
/// the task, get a run driven by its author, when the author may drive one.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn commented(
    conn: &mut AnyConnection,
    workspace_id: Id,
    task_id: Id,
    author: Id,
    comment_id: Id,
    mentioned: &[Id],
    automated: bool,
    now: TimestampMillis,
) -> Result<(), sqlx::Error> {
    if automated {
        return Ok(());
    }
    let live = live_agents(conn, workspace_id).await?;
    let assignees: Vec<Id> =
        sqlx::query_scalar::<_, String>("SELECT user_id FROM task_assignees WHERE task_id = $1")
            .bind(task_id.to_string())
            .fetch_all(&mut *conn)
            .await?
            .iter()
            .filter_map(|id| id.parse().ok())
            .collect();
    // Most comments involve no agent: nothing more to read then.
    if !mentioned
        .iter()
        .chain(&assignees)
        .any(|id| live.contains(id))
        || !may_drive(conn, task_id, author, automated).await?
    {
        return Ok(());
    }
    let mentioned = agents_among(conn, &live, task_id, mentioned).await?;
    let assigned = agents_among(conn, &live, task_id, &assignees).await?;
    for agent in mentioned.iter().chain(&assigned) {
        let cause = if mentioned.contains(agent) {
            Cause::Mentioned
        } else {
            Cause::Commented
        };
        // An agent both mentioned and assigned comes twice; the queued run takes it once.
        queue(
            conn,
            workspace_id,
            task_id,
            *agent,
            author,
            cause,
            Some(comment_id),
            now,
        )
        .await?;
    }
    Ok(())
}

/// Cancels the queued and running runs on the task (of `agents` only, when given). A running
/// run's input is acknowledged (the sidecar never takes it, if it had not yet), its thread is
/// idle and its driver's agent sessions with that agent are deleted, so Orbit refuses its next
/// call; the sidecar aborts it on its next poll ([`stopping`]).
pub(crate) async fn cancel(
    conn: &mut AnyConnection,
    task_id: Id,
    agents: Option<&[Id]>,
    reason: &str,
    now: TimestampMillis,
) -> Result<(), sqlx::Error> {
    let rows = sqlx::query(
        "SELECT id, workspace_id, agent_user_id, driver_user_id, status, thread_key \
         FROM agent_task_runs WHERE task_id = $1 AND status IN ('queued', 'running')",
    )
    .bind(task_id.to_string())
    .fetch_all(&mut *conn)
    .await?;
    let donna = super::inputs::bot_user_id(conn).await?;
    for row in rows {
        let agent: String = row.get("agent_user_id");
        if agents.is_some_and(|agents| !agents.iter().any(|id| id.to_string() == agent)) {
            continue;
        }
        let id: String = row.get("id");
        sqlx::query(
            "UPDATE agent_task_runs SET status = 'cancelled', reason = $1, finished_at = $2 \
             WHERE id = $3",
        )
        .bind(reason)
        .bind(now.as_millis())
        .bind(&id)
        .execute(&mut *conn)
        .await?;
        if row.get::<String, _>("status") != "running" {
            continue;
        }
        sqlx::query(
            "UPDATE agent_inputs SET acked_at = $1 WHERE message_id = $2 AND acked_at IS NULL",
        )
        .bind(now.as_millis())
        .bind(&id)
        .execute(&mut *conn)
        .await?;
        if let Some(thread_key) = row.get::<Option<String>, _>("thread_key") {
            sqlx::query(
                "UPDATE agent_threads SET status = 'idle', waiting_for_user_id = NULL, updated_at = $1 \
                 WHERE thread_key = $2",
            )
            .bind(now.as_millis())
            .bind(thread_key)
            .execute(&mut *conn)
            .await?;
        }
        // The default agent's sessions have no agent. Other runs of the driver with this agent
        // get a new session on their next call (the sidecar retries a 401 once).
        let custom =
            (donna.map(|id| id.to_string()).as_deref() != Some(agent.as_str())).then_some(agent);
        sqlx::query(
            "DELETE FROM sessions WHERE id IN (SELECT session_id FROM agent_sessions \
             WHERE user_id = $1 AND workspace_id = $2 AND agent_user_id IS NOT DISTINCT FROM $3)",
        )
        .bind(row.get::<String, _>("driver_user_id"))
        .bind(row.get::<String, _>("workspace_id"))
        .bind(custom)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

/// Cancels the runs on every task of a project that goes to the trash.
pub(crate) async fn cancel_project(
    conn: &mut AnyConnection,
    project_id: Id,
    now: TimestampMillis,
) -> Result<(), sqlx::Error> {
    let tasks = sqlx::query_scalar::<_, String>(
        "SELECT DISTINCT r.task_id FROM agent_task_runs r JOIN tasks t ON t.id = r.task_id \
         WHERE t.project_id = $1 AND r.status IN ('queued', 'running')",
    )
    .bind(project_id.to_string())
    .fetch_all(&mut *conn)
    .await?;
    for task in tasks.iter().filter_map(|id| id.parse::<Id>().ok()) {
        cancel(conn, task, None, "the task's project was deleted", now).await?;
    }
    Ok(())
}

/// What a reported thread state means for its task run: a running run whose thread is no longer
/// running is done, so the next queued run of its task can start; a cancelled run's thread stays
/// idle, so its driver's sessions stay refused. Returns the state to store.
pub(crate) async fn thread_reported(
    database: &Database,
    thread_key: Id,
    status: ThreadStatus,
) -> Result<ThreadStatus, sqlx::Error> {
    let run = sqlx::query(
        "SELECT id, status FROM agent_task_runs WHERE thread_key = $1 ORDER BY created_at DESC LIMIT 1",
    )
    .bind(thread_key.to_string())
    .fetch_optional(database.pool())
    .await?;
    let Some(run) = run else {
        return Ok(status);
    };
    match (run.get::<String, _>("status").as_str(), status) {
        ("cancelled", _) => Ok(ThreadStatus::Idle),
        ("running", ThreadStatus::Idle | ThreadStatus::Waiting) => {
            sqlx::query(
                "UPDATE agent_task_runs SET status = 'done', finished_at = $1 \
                 WHERE id = $2 AND status = 'running'",
            )
            .bind(TimestampMillis::now().as_millis())
            .bind(run.get::<String, _>("id"))
            .execute(database.pool())
            .await?;
            Ok(status)
        }
        _ => Ok(status),
    }
}

/// A run the sidecar must stop: cancelled after it started.
#[derive(Clone, Debug, Serialize)]
pub struct Stop {
    pub run_id: String,
    pub thread_key: String,
    pub reason: String,
}

/// The cancelled runs that started and that the sidecar has not stopped yet.
pub(crate) async fn stopping(database: &Database) -> Result<Vec<Stop>, sqlx::Error> {
    Ok(sqlx::query(
        "SELECT id, thread_key, reason FROM agent_task_runs WHERE status = 'cancelled' \
         AND stopped_at IS NULL AND thread_key IS NOT NULL ORDER BY id LIMIT 50",
    )
    .fetch_all(database.pool())
    .await?
    .iter()
    .map(|row| Stop {
        run_id: row.get("id"),
        thread_key: row.get("thread_key"),
        reason: row
            .get::<Option<String>, _>("reason")
            .unwrap_or_else(|| "the run was cancelled".to_owned()),
    })
    .collect())
}

/// The sidecar stopped the run (or never had it).
pub(crate) async fn stopped(database: &Database, run_id: Id) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE agent_task_runs SET stopped_at = $1 WHERE id = $2 AND stopped_at IS NULL")
        .bind(TimestampMillis::now().as_millis())
        .bind(run_id.to_string())
        .execute(database.pool())
        .await?;
    Ok(())
}

/// The task of a run, as the sidecar needs it (`inputs::PendingInput::task`).
#[derive(Clone, Debug, Serialize)]
pub struct TaskInput {
    pub run_id: String,
    pub task_id: String,
    /// `ENG-12`.
    pub identifier: String,
    pub title: String,
    /// `assigned`, `mentioned` or `commented`.
    pub cause: String,
    /// The comment that started the run; its text is the input's `body`.
    pub comment_id: Option<String>,
}

/// Starts the queued runs whose task has nothing running, oldest first, and posts again a claimed
/// run whose posting a crash cut off. Called on each poll of the sidecar, so a run starts as soon
/// as the agent service asks for work (and waits while it is off). Every server may call it: the
/// claim is a compare-and-set in an immediate transaction, and posting is idempotent (the run's
/// id is the header's nonce and the input's id).
pub(crate) async fn start_queued(state: &AgentState) -> Result<usize, sqlx::Error> {
    let now = TimestampMillis::now();
    let ids = sqlx::query_scalar::<_, String>(
        "SELECT q.id FROM agent_task_runs q WHERE (q.status = 'queued' AND NOT EXISTS \
         (SELECT 1 FROM agent_task_runs o WHERE o.task_id = q.task_id AND o.status = 'running')) \
         OR (q.status = 'running' AND q.thread_key IS NULL AND q.started_at < $1) \
         ORDER BY q.created_at, q.id LIMIT 20",
    )
    .bind(now.as_millis() - POST_RETRY_MS)
    .fetch_all(state.database().pool())
    .await?;
    let mut started = 0;
    for id in ids.iter().filter_map(|id| id.parse::<Id>().ok()) {
        if start(state, id, now).await? {
            started += 1;
        }
    }
    Ok(started)
}

/// Why a queued run cannot start any more, or `None`.
async fn problem(
    conn: &mut AnyConnection,
    row: &sqlx::any::AnyRow,
) -> Result<Option<&'static str>, sqlx::Error> {
    let (Ok(workspace_id), Ok(task_id), Ok(agent), Ok(driver)) = (
        row.get::<String, _>("workspace_id").parse::<Id>(),
        row.get::<String, _>("task_id").parse::<Id>(),
        row.get::<String, _>("agent_user_id").parse::<Id>(),
        row.get::<String, _>("driver_user_id").parse::<Id>(),
    ) else {
        return Ok(Some("the run is not valid"));
    };
    if row.get::<Option<i64>, _>("task_deleted_at").is_some()
        || row.get::<Option<i64>, _>("project_deleted_at").is_some()
    {
        return Ok(Some("the task was deleted"));
    }
    if !is_live_agent(conn, workspace_id, agent).await? {
        return Ok(Some("the agent was deleted"));
    }
    if !user_sees_task(&mut *conn, task_id, agent).await? {
        return Ok(Some("the agent cannot see the task"));
    }
    if row.get::<String, _>("cause") != Cause::Mentioned.as_str() {
        let assigned = sqlx::query_scalar::<_, Flag>(
            "SELECT EXISTS (SELECT 1 FROM task_assignees WHERE task_id = $1 AND user_id = $2)",
        )
        .bind(task_id.to_string())
        .bind(agent.to_string())
        .fetch_one(&mut *conn)
        .await?
        .0;
        if !assigned {
            return Ok(Some("I was unassigned from this task"));
        }
    }
    if !user_sees_task(&mut *conn, task_id, driver).await? {
        return Ok(Some(
            "the person who started it cannot see the task any more",
        ));
    }
    Ok(None)
}

/// Claims one run and posts it; `false` when it did not start (taken, cancelled, or a refusal).
async fn start(state: &AgentState, id: Id, now: TimestampMillis) -> Result<bool, sqlx::Error> {
    let database = state.database();
    let mut tx = database.immediate_transaction().await?;
    let Some(row) = sqlx::query(
        "SELECT r.workspace_id, r.task_id, r.agent_user_id, r.driver_user_id, r.cause, r.status, \
         r.thread_key, t.title, t.number, t.deleted_at AS task_deleted_at, p.project_key AS project_key, \
         p.deleted_at AS project_deleted_at FROM agent_task_runs r \
         JOIN tasks t ON t.id = r.task_id JOIN projects p ON p.id = t.project_id WHERE r.id = $1",
    )
    .bind(id.to_string())
    .fetch_optional(&mut *tx)
    .await?
    else {
        return Ok(false);
    };
    match (
        row.get::<String, _>("status").as_str(),
        row.get::<Option<String>, _>("thread_key"),
    ) {
        ("queued", _) => {
            if let Some(reason) = problem(&mut tx, &row).await? {
                sqlx::query(
                    "UPDATE agent_task_runs SET status = 'cancelled', reason = $1, finished_at = $2 WHERE id = $3",
                )
                .bind(reason)
                .bind(now.as_millis())
                .bind(id.to_string())
                .execute(&mut *tx)
                .await?;
                tx.commit().await?;
                return Ok(false);
            }
            let claimed = sqlx::query(
                "UPDATE agent_task_runs SET status = 'running', started_at = $1 WHERE id = $2 \
                 AND status = 'queued' AND NOT EXISTS (SELECT 1 FROM agent_task_runs o \
                 WHERE o.task_id = $3 AND o.status = 'running')",
            )
            .bind(now.as_millis())
            .bind(id.to_string())
            .bind(row.get::<String, _>("task_id"))
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
            if claimed.rows_affected() == 0 {
                return Ok(false);
            }
        }
        // Claimed earlier, but its posting did not finish.
        ("running", None) => tx.commit().await?,
        _ => return Ok(false),
    }
    let ids = (
        row.get::<String, _>("workspace_id").parse::<Id>(),
        row.get::<String, _>("agent_user_id").parse::<Id>(),
        row.get::<String, _>("driver_user_id").parse::<Id>(),
    );
    let (Ok(workspace_id), Ok(agent), Ok(driver)) = ids else {
        return Ok(false);
    };
    let identifier = format!(
        "{}-{}",
        row.get::<String, _>("project_key"),
        row.get::<i64, _>("number")
    );
    let title: String = row.get("title");
    let task_link = format!(
        "[{identifier} {}]({}/tasks/{})",
        title.replace(['[', ']', '\n'], " ").trim(),
        state.public_origin.as_deref().unwrap_or(""),
        row.get::<String, _>("task_id")
    );
    let header = match row.get::<String, _>("cause").as_str() {
        "assigned" => format!("📋 You assigned me {task_link}."),
        "mentioned" => format!("📋 You mentioned me on {task_link}."),
        _ => format!("📋 You commented on {task_link}, which is assigned to me."),
    } + " I'm on it: my progress and result go to the task's comments. If I need your answer or your yes, I ask here in this thread.";
    let Some(header) = post_header(state, workspace_id, agent, driver, header, id).await? else {
        let mut conn = database.pool().acquire().await?;
        cancel_one(
            &mut conn,
            id,
            "I could not write to you in our direct message",
            now,
        )
        .await?;
        return Ok(false);
    };
    // A custom agent's thread has its own key, as in chat (`inputs::agent_thread_key`).
    let custom = {
        let mut conn = database.pool().acquire().await?;
        super::profiles::live_agent(&mut conn, workspace_id, agent)
            .await?
            .map(|profile| profile.user_id)
    };
    let thread_key = super::inputs::agent_thread_key(custom, header.0);
    let mut tx = database.immediate_transaction().await?;
    sqlx::query("UPDATE agent_task_runs SET conversation_id = $1, thread_key = $2 WHERE id = $3")
        .bind(header.1.to_string())
        .bind(thread_key.to_string())
        .bind(id.to_string())
        .execute(&mut *tx)
        .await?;
    // Only while it still runs: a run cancelled since its claim gets no input.
    sqlx::query(
        "INSERT INTO agent_inputs (message_id, workspace_id, conversation_id, thread_key, root_message_id, \
         author_id, kind, created_at, agent_user_id) \
         SELECT $1, $2, $3, $4, $5, $6, 'instruction', $7, $8 WHERE EXISTS \
         (SELECT 1 FROM agent_task_runs WHERE id = $1 AND status = 'running') \
         ON CONFLICT (message_id) DO NOTHING",
    )
    .bind(id.to_string())
    .bind(workspace_id.to_string())
    .bind(header.1.to_string())
    .bind(thread_key.to_string())
    .bind(header.0.to_string())
    .bind(driver.to_string())
    .bind(now.as_millis())
    .bind(custom.map(|id| id.to_string()))
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(true)
}

async fn cancel_one(
    conn: &mut AnyConnection,
    id: Id,
    reason: &str,
    now: TimestampMillis,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE agent_task_runs SET status = 'cancelled', reason = $1, finished_at = $2 \
         WHERE id = $3 AND status IN ('queued', 'running')",
    )
    .bind(reason)
    .bind(now.as_millis())
    .bind(id.to_string())
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// Posts the run's header as the agent in the driver's DM with it, with the nonce
/// `agent:<run id>` that the sidecar's progress message uses, so it finds this message and the
/// run's questions and approvals go to the thread under it. Returns `(message, conversation)`;
/// `None`: the DM refuses it.
async fn post_header(
    state: &AgentState,
    workspace_id: Id,
    agent: Id,
    driver: Id,
    body: String,
    run: Id,
) -> Result<Option<(Id, Id)>, sqlx::Error> {
    let chat = &state.chat;
    let dm = match chat
        .publish(
            workspace_id,
            driver,
            chat.chat.open_dm(workspace_id, driver, vec![agent]),
        )
        .await
    {
        Ok((written, _)) => written.value.id,
        Err(ChatError::Unavailable(error)) => return Err(error),
        Err(error) => {
            tracing::warn!(%error, "agent task run DM");
            return Ok(None);
        }
    };
    let input = SendInput {
        conversation_id: dm,
        thread_root_id: None,
        reply_to_id: None,
        body,
        sticker_id: None,
        file_ids: Vec::new(),
        online: chat.hub.online(workspace_id),
        also_in_channel: false,
        nonce: format!("agent:{run}"),
        page_context: None,
    };
    match chat
        .publish(
            workspace_id,
            agent,
            chat.chat.send_message(workspace_id, agent, input),
        )
        .await
    {
        Ok((written, _)) => {
            PushService::of(state.database()).chat_message(&written.value);
            Ok(Some((written.value.id, dm)))
        }
        Err(ChatError::Unavailable(error)) => Err(error),
        Err(error) => {
            tracing::warn!(%error, "agent task run header");
            Ok(None)
        }
    }
}

/// Why the agent's comment was not written.
#[derive(Debug, thiserror::Error)]
pub(crate) enum CommentRefused {
    #[error("no such run")]
    NotFound,
    #[error("the run has not started")]
    NotStarted,
    #[error(transparent)]
    Task(#[from] TaskError),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

/// Writes the run's comment on its task as the agent: the first call creates it, later calls
/// replace its text (progress, then the result). A follow-up run in the DM thread (after an
/// answer or a yes) writes its result here too. The task must still exist and the agent see it.
pub(crate) async fn write_comment(
    database: &Database,
    run_id: Id,
    body: &str,
) -> Result<Id, CommentRefused> {
    let row = sqlx::query(
        "SELECT workspace_id, task_id, agent_user_id, status, result_comment_id \
         FROM agent_task_runs WHERE id = $1",
    )
    .bind(run_id.to_string())
    .fetch_optional(database.pool())
    .await?
    .ok_or(CommentRefused::NotFound)?;
    if row.get::<String, _>("status") == "queued" {
        return Err(CommentRefused::NotStarted);
    }
    let parse = |column: &str| {
        row.get::<String, _>(column)
            .parse::<Id>()
            .map_err(|_| CommentRefused::NotFound)
    };
    let (workspace_id, task_id, agent) = (
        parse("workspace_id")?,
        parse("task_id")?,
        parse("agent_user_id")?,
    );
    let body: String = body.chars().take(COMMENT_MAX_CHARS).collect();
    let body = if body.trim().is_empty() {
        "…".to_owned()
    } else {
        body
    };
    let content = || CommentContent::Markdown {
        body: body.clone(),
        mentioned_user_ids: Vec::new(),
    };
    let tasks = TaskRepository::new(database.clone());
    let request_id = format!("agent-task-run:{run_id}");
    let now = TimestampMillis::now();
    if let Some(comment_id) = row
        .get::<Option<String>, _>("result_comment_id")
        .and_then(|id| id.parse::<Id>().ok())
    {
        let version =
            sqlx::query_scalar::<_, i64>("SELECT version FROM task_comments WHERE id = $1")
                .bind(comment_id.to_string())
                .fetch_optional(database.pool())
                .await?;
        // Deleted by someone: the agent writes a new one.
        if let Some(version) = version {
            tasks
                .update_comment(
                    workspace_id,
                    task_id,
                    comment_id,
                    agent,
                    content(),
                    u64::try_from(version).unwrap_or_default(),
                    &request_id,
                    now,
                )
                .await?;
            return Ok(comment_id);
        }
    }
    let comment = tasks
        .create_comment(
            workspace_id,
            task_id,
            agent,
            None,
            content(),
            &request_id,
            now,
        )
        .await?;
    sqlx::query("UPDATE agent_task_runs SET result_comment_id = $1 WHERE id = $2")
        .bind(comment.id.to_string())
        .bind(run_id.to_string())
        .execute(database.pool())
        .await?;
    Ok(comment.id)
}
