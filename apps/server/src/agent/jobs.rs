//! Jobs of the team agent: one approval for a whole piece of work (an import of 2,000 customers)
//! instead of one "yes" per write. Design: `.ai/agent-design.md`, "Jobs".
//!
//! 1. The agent plans the job (`POST …/agent-tools/jobs`): what it does in plain words, the service
//!    and the one tool it writes with (for an HTTP API: the method, the path and, for GraphQL, the
//!    mutations it may run), the most calls, the field mapping, the columns left out, a few sample
//!    records, the source file, and the script (JavaScript for code mode). Orbit keeps it, and the
//!    driver gets one question built from it here, never from the model's own text.
//! 2. After the driver's "yes" in the thread ([`approve`], bound to the hash of the whole plan), the
//!    agent starts it (`…/jobs/{id}/start`): Orbit gives back the stored script, and the job runs
//!    for at most [`JOB_LIFETIME_MS`].
//! 3. Each write of the script names the job and the hash of the code it runs ([`JobRef`]).
//!    [`approve_in_job`] lets it run without a new "yes" only inside the job: the same driver and
//!    thread, the same integration (or Orbit Drive) and tool, the same method and path, only the
//!    approved GraphQL mutations, the approved code, below the most calls, before the end. A call
//!    outside the job asks again; a call of a job that ended, ran out, expired, was stopped (the
//!    driver wrote "stop") or whose script changed is refused. Data the script reads (a CSV) cannot
//!    change any of this: Orbit checks every call itself.
//! 4. The agent ends the job (`…/jobs/{id}/finish`). The audit log keeps the approval, each call
//!    (`agent_integration.call` with `job_id`, Drive's approved actions) and the end.

use axum::extract::{Extension, Path, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::routing::post;
use axum::{Json, Router};
use orbit_platform::{Database, Id, RequestId, Row, TimestampMillis};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::AgentState;
use super::graphql;
use super::integrations::{
    APPROVAL_LIFETIME_MS, Approval, Approves, SEPARATOR, approve, canonical, driver_access, hash,
    invalid, load_one, problem, record, require_running, unavailable,
};
use crate::audit::AuditOutcome;
use crate::scheduled::scripts::SCRIPT_MAX_CHARS;
use crate::task_routes::ApiError;

type RequestIdExtension = Option<Extension<RequestId>>;

/// How long a started job may run.
pub(crate) const JOB_LIFETIME_MS: i64 = 60 * 60 * 1_000;
/// The most approved calls of one job.
pub(crate) const MAX_JOB_CALLS: i64 = 1_000;
/// The approval's tool name: an approval of a job is never one of a single call.
const JOB_TOOL: &str = "job";
/// Orbit Drive's writes that need a "yes", and so can be a job.
const DRIVE_JOB_TOOLS: [&str; 2] = ["drive_trash", "drive_share"];

pub fn router(state: AgentState) -> Router {
    let jobs = "/api/v1/workspaces/{workspace_id}/agent-tools/jobs";
    Router::new()
        .route(jobs, post(plan))
        .route(&format!("{jobs}/{{job_id}}/start"), post(start))
        .route(&format!("{jobs}/{{job_id}}/finish"), post(finish))
        .with_state(state)
}

/// What a write of a job's script sends with its call: the job, and the hash of the code that runs.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct JobRef {
    pub(crate) id: String,
    pub(crate) script_hash: String,
}

/// One write, as a job is checked against it.
pub(crate) struct JobCall<'a> {
    /// The integration's tool (`api_request`, an MCP tool) or the Drive tool.
    pub(crate) tool: &'a str,
    pub(crate) method: Option<&'a str>,
    pub(crate) path: Option<&'a str>,
    /// The URL has a query string, which no job allows.
    pub(crate) query: bool,
    pub(crate) body: Option<&'a Value>,
}

/// What a job says about one write.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum JobUse {
    /// Inside the job: it runs, and counts.
    Granted(String),
    /// Not this job's (another driver, thread, service, tool, endpoint or mutation): ask as always.
    Outside,
    /// The job ended or may not go on: refuse, and say why.
    Ended(String),
}

struct JobRow {
    id: String,
    user_id: String,
    conversation_id: String,
    root_message_id: Option<String>,
    integration_id: Option<String>,
    workspace_id: String,
    tool: String,
    method: Option<String>,
    path: Option<String>,
    mutations: Vec<String>,
    max_calls: i64,
    calls: i64,
    plan: Value,
    code: String,
    script_hash: String,
    plan_hash: String,
    approved_at: Option<i64>,
    expires_at: i64,
    ended_at: Option<i64>,
    end_reason: Option<String>,
}

const JOB_COLUMNS: &str = "id, user_id, conversation_id, root_message_id, integration_id, workspace_id, tool, \
     method, path, mutations, max_calls, calls, plan, code, script_hash, plan_hash, approved_at, expires_at, \
     ended_at, end_reason";

impl JobRow {
    fn from_row(row: &sqlx::any::AnyRow) -> Self {
        Self {
            id: row.get("id"),
            user_id: row.get("user_id"),
            conversation_id: row.get("conversation_id"),
            root_message_id: row.get("root_message_id"),
            integration_id: row.get("integration_id"),
            workspace_id: row.get("workspace_id"),
            tool: row.get("tool"),
            method: row.get("method"),
            path: row.get("path"),
            mutations: row
                .get::<Option<String>, _>("mutations")
                .and_then(|text| serde_json::from_str(&text).ok())
                .unwrap_or_default(),
            max_calls: row.get("max_calls"),
            calls: row.get("calls"),
            plan: row
                .get::<Option<String>, _>("plan")
                .and_then(|text| serde_json::from_str(&text).ok())
                .unwrap_or(Value::Null),
            code: row.get("code"),
            script_hash: row.get("script_hash"),
            plan_hash: row.get("plan_hash"),
            approved_at: row.get("approved_at"),
            expires_at: row.get("expires_at"),
            ended_at: row.get("ended_at"),
            end_reason: row.get("end_reason"),
        }
    }

    fn subject<'a>(&'a self) -> Approves<'a> {
        match &self.integration_id {
            Some(id) => Approves::Integration(id),
            None => Approves::Drive(&self.workspace_id),
        }
    }

    fn thread_is(
        &self,
        user: Id,
        (conversation_id, root_message_id): (&str, Option<&str>),
    ) -> bool {
        self.user_id == user.to_string()
            && self.conversation_id == conversation_id
            && self.root_message_id.as_deref() == root_message_id
    }

    /// Whether `call` of `subject` is this job's kind of write.
    fn covers(&self, subject: Approves<'_>, call: &JobCall<'_>) -> bool {
        let same_subject = match (subject, &self.integration_id) {
            (Approves::Integration(id), Some(own)) => id == own,
            (Approves::Drive(workspace), None) => workspace == self.workspace_id,
            _ => false,
        };
        same_subject
            && call.tool == self.tool
            && call.method == self.method.as_deref()
            && call.path == self.path.as_deref()
            && !call.query
            && body_allowed(&self.mutations, call.body)
    }
}

/// A body is the job's when it is GraphQL whose root fields are all approved mutations, or, for a
/// job without mutations, not GraphQL at all. Fails closed: a GraphQL body that is not one clean
/// mutation never runs in a job.
fn body_allowed(mutations: &[String], body: Option<&Value>) -> bool {
    let graphql_body = body.filter(|body| body.get("query").is_some_and(Value::is_string));
    match graphql_body {
        None => mutations.is_empty(),
        Some(body) => {
            !mutations.is_empty()
                && graphql::mutation_fields(body)
                    .is_some_and(|fields| fields.iter().all(|field| mutations.contains(field)))
        }
    }
}

/// Whether `body` stops a job: its first word after any mentions is stop or cancel.
pub(crate) fn is_stop(body: &str) -> bool {
    let mut text = body.trim();
    while let Some(rest) = text.strip_prefix("<@") {
        text = rest
            .split_once('>')
            .map_or("", |(_, after)| after)
            .trim_start();
    }
    let word: String = text
        .chars()
        .take_while(|character| character.is_alphabetic())
        .collect::<String>()
        .to_lowercase();
    matches!(word.as_str(), "stop" | "cancel")
}

async fn load_job(database: &Database, id: &str) -> Result<Option<JobRow>, sqlx::Error> {
    let row = sqlx::query(sqlx::AssertSqlSafe(format!(
        "SELECT {JOB_COLUMNS} FROM agent_jobs WHERE id = $1"
    )))
    .bind(id)
    .fetch_optional(database.pool())
    .await?;
    Ok(row.as_ref().map(JobRow::from_row))
}

/// Ends the job once; a job that already ended keeps its first reason.
async fn end_job(database: &Database, id: &str, reason: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE agent_jobs SET ended_at = $1, end_reason = $2 WHERE id = $3 AND ended_at IS NULL",
    )
    .bind(TimestampMillis::now().as_millis())
    .bind(reason)
    .bind(id)
    .execute(database.pool())
    .await?;
    Ok(())
}

/// The driver wrote "stop" in the thread since the job started.
async fn stopped(database: &Database, job: &JobRow, since: i64) -> Result<bool, sqlx::Error> {
    let replies = sqlx::query_scalar::<_, String>(
        "SELECT body FROM chat_messages WHERE conversation_id = $1 \
         AND thread_root_id IS NOT DISTINCT FROM $2 AND author_id = $3 \
         AND kind = 'message' AND deleted_at IS NULL AND created_at >= $4 \
         ORDER BY created_at LIMIT 200",
    )
    .bind(&job.conversation_id)
    .bind(job.root_message_id.as_deref())
    .bind(&job.user_id)
    .bind(since)
    .fetch_all(database.pool())
    .await?;
    Ok(replies.iter().any(|body| is_stop(body)))
}

/// Whether the write `call` runs inside the job `job` names (see the module's comment).
pub(crate) async fn use_job(
    database: &Database,
    job: &JobRef,
    subject: Approves<'_>,
    user: Id,
    thread: (&str, Option<&str>),
    call: &JobCall<'_>,
) -> Result<JobUse, sqlx::Error> {
    let Some(row) = load_job(database, &job.id).await? else {
        return Ok(JobUse::Outside);
    };
    if !row.thread_is(user, thread) || !row.covers(subject, call) {
        return Ok(JobUse::Outside);
    }
    let Some(approved_at) = row.approved_at else {
        return Ok(JobUse::Outside);
    };
    if job.script_hash != row.script_hash {
        return Ok(JobUse::Ended(
            "This is not the script the driver approved for this job: it changed. Nothing was done."
                .to_owned(),
        ));
    }
    let now = TimestampMillis::now().as_millis();
    if row.ended_at.is_some() {
        return Ok(JobUse::Ended(format!(
            "This job has ended ({}): plan a new one for the rest.",
            row.end_reason.as_deref().unwrap_or("done")
        )));
    }
    if row.expires_at <= now {
        end_job(database, &row.id, "failed").await?;
        return Ok(JobUse::Ended(
            "This job ran out of time (60 minutes after its approval): plan a new one for the rest."
                .to_owned(),
        ));
    }
    if stopped(database, &row, approved_at).await? {
        end_job(database, &row.id, "stopped").await?;
        return Ok(JobUse::Ended(
            "The driver stopped this job (they wrote stop). Nothing more runs.".to_owned(),
        ));
    }
    // One statement: two calls at once cannot both take the last one.
    let counted = sqlx::query(
        "UPDATE agent_jobs SET calls = calls + 1 WHERE id = $1 AND calls < max_calls \
         AND ended_at IS NULL AND expires_at > $2",
    )
    .bind(&row.id)
    .bind(now)
    .execute(database.pool())
    .await?
    .rows_affected();
    if counted != 1 {
        return Ok(JobUse::Ended(format!(
            "This job used all of its {} approved calls: plan a new one for the rest.",
            row.max_calls
        )));
    }
    Ok(JobUse::Granted(format!("job:{}", row.id)))
}

/// [`approve`], except that a write inside an approved job (`job`) runs without a new "yes".
/// `Err(detail)`: refused, because the job it names may not go on.
pub(crate) async fn approve_in_job(
    database: &Database,
    job: Option<&JobRef>,
    subject: Approves<'_>,
    user: Id,
    thread: (&str, Option<&str>),
    call: JobCall<'_>,
    arguments_hash: &str,
) -> Result<Result<Approval, String>, sqlx::Error> {
    if let Some(job) = job {
        match use_job(database, job, subject, user, thread, &call).await? {
            JobUse::Granted(id) => return Ok(Ok(Approval::Granted { id })),
            JobUse::Ended(detail) => return Ok(Err(detail)),
            JobUse::Outside => {}
        }
    }
    approve(database, subject, user, thread, call.tool, arguments_hash)
        .await
        .map(Ok)
}

// ---------------------------------------------------------------------------------------------
// Routes.

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Mapping {
    source: String,
    target: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Source {
    /// `chat` (a file sent in chat) or `drive`.
    kind: String,
    file_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sheet: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PlanBody {
    conversation_id: String,
    root_message_id: Option<String>,
    title: String,
    summary: String,
    /// `<integration>__<tool>`, or `drive_trash` / `drive_share`.
    tool: String,
    method: Option<String>,
    path: Option<String>,
    #[serde(default)]
    mutations: Vec<String>,
    max_calls: i64,
    records: Option<i64>,
    #[serde(default)]
    mapping: Vec<Mapping>,
    #[serde(default)]
    left_out: Vec<String>,
    #[serde(default)]
    samples: Vec<String>,
    source: Option<Source>,
    code: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ThreadBody {
    conversation_id: String,
    root_message_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FinishBody {
    /// `done` or `failed`.
    status: String,
    /// What the script reported last: records done of the total.
    done: Option<i64>,
    total: Option<i64>,
    error: Option<String>,
}

/// Text from the model in Orbit's question: no mention or channel token can ping anyone, and it
/// stays short.
fn plain(text: &str, max: usize) -> String {
    let text: String = text
        .chars()
        .map(|character| match character {
            '<' => '‹',
            '>' => '›',
            '`' => '\'',
            other => other,
        })
        .collect();
    let text = text.trim();
    if text.chars().count() > max {
        format!("{}…", text.chars().take(max).collect::<String>())
    } else {
        text.to_owned()
    }
}

/// `2224` → `2,224`.
fn grouped(number: i64) -> String {
    let digits = number.unsigned_abs().to_string();
    let mut out = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            out.push(',');
        }
        out.push(digit);
    }
    if number < 0 { format!("-{out}") } else { out }
}

fn check_plan(body: &PlanBody) -> Result<(), &'static str> {
    let chars = |text: &str| text.trim().chars().count();
    if !(1..=150).contains(&chars(&body.title)) {
        return Err("Give a title of 1 to 150 characters.");
    }
    if !(1..=2_000).contains(&chars(&body.summary)) {
        return Err("Give a summary of 1 to 2,000 characters.");
    }
    if !(1..=MAX_JOB_CALLS).contains(&body.max_calls) {
        return Err("max_calls must be 1 to 1,000.");
    }
    if body
        .records
        .is_some_and(|records| !(0..=1_000_000).contains(&records))
    {
        return Err("records must be 0 to 1,000,000.");
    }
    if body.mapping.len() > 100
        || body
            .mapping
            .iter()
            .any(|pair| chars(&pair.source) > 200 || chars(&pair.target) > 200)
    {
        return Err("At most 100 mapping lines of 200 characters each.");
    }
    if body.left_out.len() > 100 || body.left_out.iter().any(|item| chars(item) > 300) {
        return Err("At most 100 left_out lines of 300 characters each.");
    }
    if body.samples.len() > 3 || body.samples.iter().any(|item| chars(item) > 500) {
        return Err("At most 3 samples of 500 characters each.");
    }
    if body.mutations.len() > 20
        || body.mutations.iter().any(|name| {
            name.is_empty()
                || name.len() > 100
                || !name
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || character == '_')
        })
    {
        return Err("mutations are at most 20 GraphQL field names.");
    }
    if body.code.trim().is_empty() || body.code.chars().count() > SCRIPT_MAX_CHARS {
        return Err("Give the script: 1 to 20,000 characters of JavaScript.");
    }
    if let Some(source) = &body.source
        && (!matches!(source.kind.as_str(), "chat" | "drive")
            || source.file_id.parse::<Id>().is_err())
    {
        return Err("source is {kind: chat or drive, file_id, sheet?}.");
    }
    Ok(())
}

/// The question the driver answers: Orbit's own words around the plan's facts, the service named
/// as Orbit knows it.
fn question(body: &PlanBody, service: &str) -> String {
    let mut lines = vec![
        format!("**{}**", plain(&body.title, 150)),
        plain(&body.summary, 2_000),
        String::new(),
    ];
    let endpoint = match (&body.method, &body.path) {
        (Some(method), Some(path)) => {
            format!(", {} {}", method.to_ascii_uppercase(), plain(path, 300))
        }
        _ => String::new(),
    };
    let only = if body.mutations.is_empty() {
        String::new()
    } else {
        format!(", only the GraphQL mutations {}", body.mutations.join(", "))
    };
    lines.push(format!("- **Service:** {service}{endpoint}{only}"));
    let size = body.records.map_or_else(String::new, |records| {
        format!("{} records, ", grouped(records))
    });
    lines.push(format!(
        "- **Size:** {size}at most {} calls, for at most 60 minutes",
        grouped(body.max_calls)
    ));
    if !body.mapping.is_empty() {
        lines.push("- **Field mapping:**".to_owned());
        for pair in &body.mapping {
            lines.push(format!(
                "  - {} → {}",
                plain(&pair.source, 200),
                plain(&pair.target, 200)
            ));
        }
    }
    if !body.mapping.is_empty() || !body.left_out.is_empty() {
        lines.push(if body.left_out.is_empty() {
            "- **Left out:** nothing".to_owned()
        } else {
            format!(
                "- **Left out:** {}",
                body.left_out
                    .iter()
                    .map(|item| plain(item, 300))
                    .collect::<Vec<_>>()
                    .join("; ")
            )
        });
    }
    if !body.samples.is_empty() {
        lines.push("- **Samples:**".to_owned());
        for sample in &body.samples {
            lines.push(format!("  - {}", plain(sample, 500)));
        }
    }
    lines.join("\n")
}

/// The raw job for "Details": the exact scope Orbit enforces and the script, as stored.
fn details(row: &JobRow, service: &str) -> String {
    let scope = json!({
        "service": service,
        "tool": row.tool,
        "method": row.method,
        "path": row.path,
        "mutations": row.mutations,
        "max_calls": row.max_calls,
        "source": row.plan.get("source"),
    });
    format!(
        "Scope: {}\nScript (sha256 {}):\n{}",
        serde_json::to_string_pretty(&scope).unwrap_or_default(),
        &row.script_hash[..12],
        row.code
    )
}

/// What the agent plans a job for: the integration (`None`: Orbit Drive), its tool, and the
/// service's name for the question.
async fn job_target(
    state: &AgentState,
    (workspace_id, role): (Id, orbit_domain::WorkspaceRole),
    access: &super::profiles::IntegrationAccess,
    custom: Option<&super::profiles::AgentTools>,
    body: &PlanBody,
    (uri, request_id): (&Uri, &RequestIdExtension),
) -> Result<Result<(Option<String>, String, String), &'static str>, ApiError> {
    let has = |tool: &str| custom.is_none_or(|tools| tools.0.iter().any(|own| own == tool));
    if DRIVE_JOB_TOOLS.contains(&body.tool.as_str()) {
        if !has(&body.tool) {
            return Ok(Err("This agent's tools do not include this Drive action."));
        }
        if body.method.is_some() || body.path.is_some() || !body.mutations.is_empty() {
            return Ok(Err("A Drive job has no method, path or mutations."));
        }
        return Ok(Ok((None, body.tool.clone(), "Orbit Drive".to_owned())));
    }
    let Some((slug, tool)) = body.tool.split_once(SEPARATOR) else {
        return Ok(Err(
            "tool is an integration tool (<integration>__<tool>), drive_trash or drive_share.",
        ));
    };
    if !has("use_integration") {
        return Ok(Err("This agent cannot use connected services."));
    }
    let Some(integration) = load_one(state.database(), workspace_id, "slug", slug)
        .await
        .map_err(unavailable(uri, request_id))?
    else {
        return Ok(Err("There is no such integration."));
    };
    if !integration.allows(role) || !access.allows(&integration.id) {
        return Ok(Err(
            "The driver or this agent may not use this integration.",
        ));
    }
    if let Err(detail) = integration.job_scope(
        tool,
        body.method.as_deref(),
        body.path.as_deref(),
        &body.mutations,
    ) {
        return Ok(Err(detail));
    }
    Ok(Ok((
        Some(integration.id.clone()),
        tool.to_owned(),
        integration.name.clone(),
    )))
}

fn refused(detail: &str) -> Json<Value> {
    Json(json!({ "status": "refused", "detail": detail }))
}

// An axum handler: each extractor is one argument.
#[allow(clippy::too_many_arguments)]
async fn plan(
    State(state): State<AgentState>,
    Path(workspace_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
    custom: Option<Extension<super::profiles::AgentTools>>,
    Json(body): Json<PlanBody>,
) -> Result<Json<Value>, ApiError> {
    let (user, workspace_id, role, access) =
        driver_access(&state, &headers, &workspace_id, &uri, &request_id).await?;
    let database = state.database();
    let thread = (
        body.conversation_id.as_str(),
        body.root_message_id.as_deref(),
    );
    require_running(database, (workspace_id, user), thread, &uri, &request_id).await?;
    if let Err(detail) = check_plan(&body) {
        return Ok(refused(detail));
    }
    let (integration_id, tool, service) = match job_target(
        &state,
        (workspace_id, role),
        &access,
        custom.as_ref().map(|Extension(tools)| tools),
        &body,
        (&uri, &request_id),
    )
    .await?
    {
        Ok(target) => target,
        Err(detail) => return Ok(refused(detail)),
    };
    let method = body.method.as_deref().map(str::to_ascii_uppercase);
    let plan = json!({
        "title": body.title.trim(),
        "summary": body.summary.trim(),
        "records": body.records,
        "mapping": body.mapping,
        "left_out": body.left_out,
        "samples": body.samples,
        "source": body.source,
        "question": question(&body, &service),
    });
    let script_hash = hash(&body.code);
    // The approval is bound to all of it: the plan the driver reads, the scope and the code.
    let plan_hash = hash(&canonical(&json!({
        "plan": plan,
        "integration_id": integration_id,
        "tool": tool,
        "method": method,
        "path": body.path,
        "mutations": body.mutations,
        "max_calls": body.max_calls,
        "script_hash": script_hash,
    })));
    let now = TimestampMillis::now().as_millis();
    let workspace = workspace_id.to_string();
    // The same plan again (the question is still open) is the same job.
    let open = sqlx::query_scalar::<_, String>(
        "SELECT id FROM agent_jobs WHERE workspace_id = $1 AND user_id = $2 AND conversation_id = $3 \
         AND root_message_id IS NOT DISTINCT FROM $4 AND plan_hash = $5 AND ended_at IS NULL \
         AND expires_at > $6 ORDER BY created_at DESC LIMIT 1",
    )
    .bind(&workspace)
    .bind(user.to_string())
    .bind(thread.0)
    .bind(thread.1)
    .bind(&plan_hash)
    .bind(now)
    .fetch_optional(database.pool())
    .await
    .map_err(unavailable(&uri, &request_id))?;
    let job_id = match open {
        Some(id) => id,
        None => {
            let id = Id::new_v7().to_string();
            sqlx::query(
                "INSERT INTO agent_jobs (id, workspace_id, user_id, conversation_id, root_message_id, \
                 integration_id, tool, method, path, mutations, max_calls, plan, code, script_hash, \
                 plan_hash, created_at, expires_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17)",
            )
            .bind(&id)
            .bind(&workspace)
            .bind(user.to_string())
            .bind(thread.0)
            .bind(thread.1)
            .bind(integration_id.as_deref())
            .bind(&tool)
            .bind(method.as_deref())
            .bind(body.path.as_deref())
            .bind((!body.mutations.is_empty()).then(|| json!(body.mutations).to_string()))
            .bind(body.max_calls)
            .bind(plan.to_string())
            .bind(&body.code)
            .bind(&script_hash)
            .bind(&plan_hash)
            .bind(now)
            .bind(now + APPROVAL_LIFETIME_MS)
            .execute(database.pool())
            .await
            .map_err(unavailable(&uri, &request_id))?;
            id
        }
    };
    let row = load_job(database, &job_id)
        .await
        .map_err(unavailable(&uri, &request_id))?
        .ok_or_else(|| unavailable(&uri, &request_id)(sqlx::Error::RowNotFound))?;
    match ask(&state, &row, &request_id).await {
        Ok(Some(answer)) => Ok(Json(answer)),
        Ok(None) => Ok(Json(json!({
            "status": "approved",
            "job_id": row.id,
            "detail": "The driver approved this job already: call run_job with its job_id.",
        }))),
        Err(error) => Err(unavailable(&uri, &request_id)(error)),
    }
}

/// Asks for the job's approval: `Some(approval_required)`, or `None` when the driver said yes
/// (the job is then started, once).
async fn ask(
    state: &AgentState,
    row: &JobRow,
    request_id: &RequestIdExtension,
) -> Result<Option<Value>, sqlx::Error> {
    let database = state.database();
    if row.approved_at.is_some() {
        return Ok(None);
    }
    let user: Id = row.user_id.parse().map_err(|_| sqlx::Error::RowNotFound)?;
    let approval = approve(
        database,
        row.subject(),
        user,
        (&row.conversation_id, row.root_message_id.as_deref()),
        JOB_TOOL,
        &row.plan_hash,
    )
    .await?;
    let service = match &row.integration_id {
        Some(id) => {
            let workspace: Id = row
                .workspace_id
                .parse()
                .map_err(|_| sqlx::Error::RowNotFound)?;
            load_one(database, workspace, "id", id)
                .await?
                .map_or_else(|| "a connected service".to_owned(), |found| found.name)
        }
        None => "Orbit Drive".to_owned(),
    };
    match approval {
        Approval::Needed { id, expires_at } => Ok(Some(json!({
            "status": "approval_required",
            "approval_id": id,
            "job_id": row.id,
            "max_calls": row.max_calls,
            "action": row.plan.get("question").and_then(Value::as_str).unwrap_or_default(),
            "details": details(row, &service),
            "expires_at": expires_at,
        }))),
        Approval::Granted { id } => {
            let now = TimestampMillis::now().as_millis();
            sqlx::query(
                "UPDATE agent_jobs SET approved_at = $1, expires_at = $2 \
                 WHERE id = $3 AND approved_at IS NULL",
            )
            .bind(now)
            .bind(now + JOB_LIFETIME_MS)
            .bind(&row.id)
            .execute(database.pool())
            .await?;
            let workspace: Id = row
                .workspace_id
                .parse()
                .map_err(|_| sqlx::Error::RowNotFound)?;
            record(
                database,
                workspace,
                user,
                "agent_job.approved",
                AuditOutcome::Success,
                row.integration_id.as_deref().unwrap_or_default(),
                request_id,
                json!({
                    "job_id": row.id,
                    "approval_id": id,
                    "service": service,
                    "tool": row.tool,
                    "method": row.method,
                    "path": row.path,
                    "mutations": row.mutations,
                    "max_calls": row.max_calls,
                    "script_hash": row.script_hash,
                    "conversation_id": row.conversation_id,
                    "root_message_id": row.root_message_id,
                }),
            )
            .await;
            Ok(None)
        }
    }
}

fn job_not_found(uri: &Uri, request_id: &RequestIdExtension) -> ApiError {
    problem(
        StatusCode::NOT_FOUND,
        "agent_job_not_found",
        "There is no such job of the driver in this thread.",
        uri,
        request_id,
    )
}

/// Starts an approved job and gives its stored script back; before the driver's yes, the question
/// again.
async fn start(
    State(state): State<AgentState>,
    Path((workspace_id, job_id)): Path<(String, String)>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
    Json(body): Json<ThreadBody>,
) -> Result<Json<Value>, ApiError> {
    let (user, workspace_id, _, _) =
        driver_access(&state, &headers, &workspace_id, &uri, &request_id).await?;
    let database = state.database();
    let thread = (
        body.conversation_id.as_str(),
        body.root_message_id.as_deref(),
    );
    require_running(database, (workspace_id, user), thread, &uri, &request_id).await?;
    let row = load_job(database, &job_id)
        .await
        .map_err(unavailable(&uri, &request_id))?
        .filter(|row| row.workspace_id == workspace_id.to_string() && row.thread_is(user, thread))
        .ok_or_else(|| job_not_found(&uri, &request_id))?;
    if let Some(reason) = &row.end_reason {
        return Ok(refused(&format!(
            "This job has ended ({reason}). Plan a new job for what is left."
        )));
    }
    if let Some(answer) = ask(&state, &row, &request_id)
        .await
        .map_err(unavailable(&uri, &request_id))?
    {
        return Ok(Json(answer));
    }
    let row = load_job(database, &job_id)
        .await
        .map_err(unavailable(&uri, &request_id))?
        .ok_or_else(|| job_not_found(&uri, &request_id))?;
    if row.expires_at <= TimestampMillis::now().as_millis() {
        end_job(database, &row.id, "failed")
            .await
            .map_err(unavailable(&uri, &request_id))?;
        return Ok(refused(
            "This job ran out of time (60 minutes after its approval). Plan a new job for what is left.",
        ));
    }
    Ok(Json(json!({
        "status": "ok",
        "job": {
            "id": row.id,
            "title": row.plan.get("title"),
            "code": row.code,
            "script_hash": row.script_hash,
            "source": row.plan.get("source"),
            "records": row.plan.get("records"),
            "max_calls": row.max_calls,
            "calls": row.calls,
            "expires_at": row.expires_at,
        },
    })))
}

/// The agent ends a job: no call of it runs after this.
async fn finish(
    State(state): State<AgentState>,
    Path((workspace_id, job_id)): Path<(String, String)>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
    Json(body): Json<FinishBody>,
) -> Result<Json<Value>, ApiError> {
    let (user, workspace_id, _, _) =
        driver_access(&state, &headers, &workspace_id, &uri, &request_id).await?;
    let database = state.database();
    let row = load_job(database, &job_id)
        .await
        .map_err(unavailable(&uri, &request_id))?
        .filter(|row| {
            row.workspace_id == workspace_id.to_string() && row.user_id == user.to_string()
        })
        .ok_or_else(|| job_not_found(&uri, &request_id))?;
    if !matches!(body.status.as_str(), "done" | "failed") {
        return Err(invalid("status is done or failed.", &uri, &request_id));
    }
    end_job(database, &row.id, &body.status)
        .await
        .map_err(unavailable(&uri, &request_id))?;
    let ended = load_job(database, &job_id)
        .await
        .map_err(unavailable(&uri, &request_id))?
        .unwrap_or(row);
    record(
        database,
        workspace_id,
        user,
        "agent_job.finished",
        if ended.end_reason.as_deref() == Some("done") {
            AuditOutcome::Success
        } else {
            AuditOutcome::Failure
        },
        ended.integration_id.as_deref().unwrap_or_default(),
        &request_id,
        json!({
            "job_id": ended.id,
            "end_reason": ended.end_reason,
            "calls": ended.calls,
            "max_calls": ended.max_calls,
            "done": body.done,
            "total": body.total,
            "error": body.error.map(|error| error.chars().take(300).collect::<String>()),
        }),
    )
    .await;
    Ok(Json(json!({
        "status": "ok",
        "end_reason": ended.end_reason,
        "calls": ended.calls,
    })))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{body_allowed, grouped, is_stop, plain};

    #[test]
    fn stop_or_cancel_first_stops_a_job() {
        for body in ["stop", "Stop!", "<@u1> stop it", "cancel", "STOP please"] {
            assert!(is_stop(body), "{body}");
        }
        for body in ["yes", "don't stop", "stopped?", "nonstop", ""] {
            assert!(!is_stop(body), "{body}");
        }
    }

    #[test]
    fn only_approved_mutations_or_plain_bodies_fit_a_job() {
        let create = vec!["customerCreate".to_owned()];
        let mutation =
            json!({ "query": "mutation { a: customerCreate(input: {}) { customer { id } } }" });
        let delete = json!({ "query": "mutation { a: customerCreate(input: {}) { customer { id } } b: customerDelete(input: {}) { deletedCustomerId } }" });
        assert!(body_allowed(&create, Some(&mutation)));
        assert!(!body_allowed(&create, Some(&delete)));
        assert!(!body_allowed(&[], Some(&mutation)));
        assert!(body_allowed(
            &[],
            Some(&json!({ "customer": { "email": "a@b.c" } }))
        ));
        assert!(body_allowed(&[], None));
        assert!(!body_allowed(&create, Some(&json!({ "customer": {} }))));
        assert!(!body_allowed(
            &create,
            Some(&json!({ "query": "query { shop { name } }" }))
        ));
    }

    #[test]
    fn model_text_cannot_mention_anyone() {
        assert_eq!(
            plain("hi <@u1> and <!channel> `x`", 100),
            "hi ‹@u1› and ‹!channel› 'x'"
        );
        assert_eq!(plain("abcdef", 3), "abc…");
        assert_eq!(grouped(2224), "2,224");
        assert_eq!(grouped(1_000_000), "1,000,000");
        assert_eq!(grouped(12), "12");
    }
}
