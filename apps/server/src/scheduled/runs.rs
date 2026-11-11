//! The history of a routine's runs (`agent_routine_runs`, the last [`RUNS_KEPT`] per routine): scheduled
//! runs and test runs, with what each may write.
//!
//! - `normal`: a run of the routine's instruction, with the same write approvals as chat.
//! - `script`: a run of the routine's saved script whose exact code (its hash) the creator approved. Its
//!   approval-gated writes (connected services, Drive trash and links) need no new yes
//!   ([`approved_script_run`], used by `agent::integrations::approve`).
//! - `dry`: a test run, or a script changed since its approval. The run's agent sessions are dry runs
//!   (`agent::sessions::issue`): Orbit refuses every write (`agent::scope`), and the agent tool routes
//!   answer writes with what they would do. A test run posts only in the creator's DM with the agent.
//!
//! The sidecar reports the end of a run ([`finish`]): its status, a summary of its answer, its error, and
//! the tool calls it made (for a script, the script's calls). A script's `store()` values are kept for
//! the next run, but never from a dry run.

use orbit_platform::{Database, Id, Row, TimestampMillis};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::AnyConnection;

use super::store::{ScheduledError, Store, member};

/// Runs kept per routine; older ones are deleted when a new one starts.
pub const RUNS_KEPT: i64 = 20;
/// A test run that has not ended after this long no longer blocks the next one.
pub const TEST_RUN_STALE_MS: i64 = 15 * 60 * 1000;
pub const OUTPUT_MAX_CHARS: usize = 4000;
pub const ERROR_MAX_CHARS: usize = 1000;
/// Tool calls kept per run, and the largest `store()` state (JSON) kept per routine.
pub const TOOL_CALLS_KEPT: usize = 100;
pub const STATE_MAX_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, Serialize)]
pub struct RunRecord {
    pub id: String,
    pub routine_id: String,
    /// A test run (in the creator's DM, nothing written).
    pub test: bool,
    /// `started`, `skipped`, `done` or `failed`.
    pub status: String,
    /// `normal`, `script` or `dry`.
    pub writes: String,
    pub conversation_id: Option<String>,
    /// The run's first message, where its answer is.
    pub message_id: Option<String>,
    pub script_name: Option<String>,
    pub script_hash: Option<String>,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub output: Option<String>,
    pub error: Option<String>,
    /// `[{ name, args, status, would_do? }]`, in the order they were made.
    pub tool_calls: Value,
}

/// What a new run starts with.
pub(super) struct NewRun<'a> {
    pub id: Id,
    pub routine_id: Id,
    pub workspace_id: Id,
    pub test: bool,
    pub status: &'a str,
    pub writes: &'a str,
    pub script: Option<&'a RunScript>,
    pub error: Option<&'a str>,
    pub now: i64,
}

/// The routine's script as one run uses it: the code is kept with the run, so the run (and a rerun
/// after a restart) runs exactly what was approved, whatever changes meanwhile.
#[derive(Clone, Debug)]
pub(super) struct RunScript {
    pub id: String,
    pub name: String,
    pub code: String,
    pub hash: String,
    /// The creator approved this exact code for the routine.
    pub approved: bool,
}

/// The routine's script now, if it has one.
pub(super) async fn routine_script(
    conn: &mut AnyConnection,
    routine_id: Id,
) -> Result<Option<RunScript>, sqlx::Error> {
    let row = sqlx::query(
        "SELECT s.id, s.name, s.code, s.code_hash, r.script_approved_hash FROM agent_routines r \
         JOIN agent_scripts s ON s.id = r.script_id WHERE r.id = $1",
    )
    .bind(routine_id.to_string())
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row.map(|row| {
        let hash: String = row.get("code_hash");
        RunScript {
            id: row.get("id"),
            name: row.get("name"),
            code: row.get("code"),
            approved: row
                .get::<Option<String>, _>("script_approved_hash")
                .as_deref()
                == Some(hash.as_str()),
            hash,
        }
    }))
}

/// What a scheduled run may write: its script's approval decides.
pub(super) fn writes_of(script: Option<&RunScript>) -> &'static str {
    match script {
        None => "normal",
        Some(script) if script.approved => "script",
        Some(_) => "dry",
    }
}

pub(super) async fn insert_run(
    conn: &mut AnyConnection,
    run: NewRun<'_>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO agent_routine_runs (id, routine_id, workspace_id, test, status, writes, script_id, \
         script_name, script_hash, script_code, started_at, finished_at, error) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)",
    )
    .bind(run.id.to_string())
    .bind(run.routine_id.to_string())
    .bind(run.workspace_id.to_string())
    .bind(i64::from(run.test))
    .bind(run.status)
    .bind(run.writes)
    .bind(run.script.map(|script| script.id.clone()))
    .bind(run.script.map(|script| script.name.clone()))
    .bind(run.script.map(|script| script.hash.clone()))
    .bind(run.script.map(|script| script.code.clone()))
    .bind(run.now)
    .bind((run.status == "skipped").then_some(run.now))
    .bind(run.error)
    .execute(&mut *conn)
    .await?;
    // Only the newest runs stay.
    sqlx::query(
        "DELETE FROM agent_routine_runs WHERE routine_id = $1 AND id NOT IN (\
         SELECT id FROM agent_routine_runs WHERE routine_id = $1 ORDER BY started_at DESC, id DESC LIMIT $2)",
    )
    .bind(run.routine_id.to_string())
    .bind(RUNS_KEPT)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// The run of a routine's approved script that this thread is (its first message is the thread's
/// root), while it runs and when `user` is its routine's creator: its approval-gated writes need no
/// new yes.
pub async fn approved_script_run(
    database: &Database,
    conversation_id: &str,
    root_message_id: Option<&str>,
    user: Id,
) -> Result<Option<String>, sqlx::Error> {
    let Some(root) = root_message_id else {
        return Ok(None);
    };
    sqlx::query_scalar(
        "SELECT r.id FROM agent_routine_runs r JOIN agent_routines a ON a.id = r.routine_id \
         WHERE r.conversation_id = $1 AND r.message_id = $2 AND r.writes = 'script' AND r.test = 0 \
         AND r.status = 'started' AND a.creator_id = $3",
    )
    .bind(conversation_id)
    .bind(root)
    .bind(user.to_string())
    .fetch_optional(database.pool())
    .await
}

const RUN_SELECT: &str = "SELECT id, routine_id, test, status, writes, conversation_id, message_id, script_name, \
     script_hash, started_at, finished_at, output, error, tool_calls FROM agent_routine_runs";

fn run_of(row: &sqlx::any::AnyRow) -> RunRecord {
    RunRecord {
        id: row.get("id"),
        routine_id: row.get("routine_id"),
        test: row.get("test"),
        status: row.get("status"),
        writes: row.get("writes"),
        conversation_id: row.get("conversation_id"),
        message_id: row.get("message_id"),
        script_name: row.get("script_name"),
        script_hash: row.get("script_hash"),
        started_at: row.get("started_at"),
        finished_at: row.get("finished_at"),
        output: row.get("output"),
        error: row.get("error"),
        tool_calls: row
            .get::<Option<String>, _>("tool_calls")
            .and_then(|calls| serde_json::from_str(&calls).ok())
            .unwrap_or_else(|| Value::Array(Vec::new())),
    }
}

pub(super) async fn load_run(
    conn: &mut AnyConnection,
    run_id: Id,
) -> Result<RunRecord, ScheduledError> {
    let row = sqlx::query(sqlx::AssertSqlSafe(format!("{RUN_SELECT} WHERE id = $1")))
        .bind(run_id.to_string())
        .fetch_optional(&mut *conn)
        .await?
        .ok_or(ScheduledError::NotFound)?;
    Ok(run_of(&row))
}

impl Store {
    /// The routine's last runs, newest first, for whoever may see the routine.
    pub async fn list_runs(
        &self,
        workspace_id: Id,
        user_id: Id,
        routine_id: Id,
    ) -> Result<Vec<RunRecord>, ScheduledError> {
        let mut conn = self.database.pool().acquire().await?;
        let actor = member(&mut conn, workspace_id, user_id).await?;
        super::store::load_routine(&mut conn, workspace_id, routine_id, actor).await?;
        let rows = sqlx::query(sqlx::AssertSqlSafe(format!(
            "{RUN_SELECT} WHERE routine_id = $1 ORDER BY started_at DESC, id DESC LIMIT $2"
        )))
        .bind(routine_id.to_string())
        .bind(RUNS_KEPT)
        .fetch_all(&mut *conn)
        .await?;
        Ok(rows.iter().map(run_of).collect())
    }
}

/// One tool call of a run, as the sidecar reports it.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ToolCall {
    pub name: String,
    /// The arguments as compact JSON, cut for display.
    #[serde(default)]
    pub args: String,
    /// `ok`, `error` or `dry_run`.
    pub status: String,
    /// For a write a dry run did not make: what it would have done.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub would_do: Option<String>,
}

/// The end of a run, from the sidecar.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunReport {
    /// `done` or `failed`.
    pub status: String,
    #[serde(default)]
    pub output: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub tool_calls: Vec<ToolCall>,
    /// The script's `store()` values after the run, kept for the next run of the routine.
    #[serde(default)]
    pub state: Option<Value>,
}

fn cut(text: Option<String>, max: usize) -> Option<String> {
    text.map(|text| {
        if text.chars().count() > max {
            let mut short: String = text.chars().take(max - 1).collect();
            short.push('…');
            short
        } else {
            text
        }
    })
}

fn cut_call(mut call: ToolCall) -> ToolCall {
    call.name = cut(Some(call.name), 120).unwrap_or_default();
    call.args = cut(Some(call.args), 300).unwrap_or_default();
    call.status = cut(Some(call.status), 20).unwrap_or_default();
    call.would_do = cut(call.would_do, 600);
    call
}

/// Records the end of a run once: a second report of the same run changes nothing. `false` when the
/// run is unknown or ended already.
pub async fn finish(
    database: &Database,
    run_id: Id,
    report: RunReport,
    now: TimestampMillis,
) -> Result<bool, ScheduledError> {
    let status = match report.status.as_str() {
        "done" | "failed" => report.status.as_str(),
        _ => {
            return Err(ScheduledError::Invalid(
                "The status must be done or failed.",
            ));
        }
    };
    let calls: Vec<ToolCall> = report
        .tool_calls
        .into_iter()
        .take(TOOL_CALLS_KEPT)
        .map(cut_call)
        .collect();
    let calls = serde_json::to_string(&calls).unwrap_or_else(|_| "[]".to_owned());
    let state = report
        .state
        .filter(Value::is_object)
        .map(|state| state.to_string())
        .filter(|state| state.len() <= STATE_MAX_BYTES);
    let mut tx = database.immediate_transaction().await?;
    let Some(row) = sqlx::query(
        "SELECT routine_id, test, writes FROM agent_routine_runs WHERE id = $1 AND status = 'started'",
    )
    .bind(run_id.to_string())
    .fetch_optional(&mut *tx)
    .await?
    else {
        return Ok(false);
    };
    sqlx::query(
        "UPDATE agent_routine_runs SET status = $1, finished_at = $2, output = $3, error = $4, tool_calls = $5 \
         WHERE id = $6",
    )
    .bind(status)
    .bind(now.as_millis())
    .bind(cut(report.output, OUTPUT_MAX_CHARS))
    .bind(cut(report.error, ERROR_MAX_CHARS))
    .bind(calls)
    .bind(run_id.to_string())
    .execute(&mut *tx)
    .await?;
    // A dry run keeps nothing, not even the script's own state.
    let test: bool = row.get("test");
    if !test
        && row.get::<String, _>("writes") == "script"
        && let Some(state) = state
    {
        sqlx::query("UPDATE agent_routines SET script_state = $1 WHERE id = $2")
            .bind(state)
            .bind(row.get::<String, _>("routine_id"))
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(true)
}
