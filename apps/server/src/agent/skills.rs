//! Agent Skills (<https://agentskills.io/specification>) for the workspace's agents. Owners and
//! admins (`workspace.update`) add a skill from GitHub or skills.sh, a `.zip`, or write one in
//! Settings → Agentic → Skills, and turn it on per agent (the default agent and each custom agent
//! have their own list). Members and guests reach none of these routes, and agent sessions never
//! do (`super::scope::DENIED`).
//!
//! The agent reads its skills through `/agent-tools/skills…`, only with an agent session: the list
//! (name and description, for the system prompt), one skill's `SKILL.md` body and file list, and
//! one file. Each answers only for skills turned on for the session's own agent. A skill is text:
//! it gives no permission. Every tool call of a run still acts as its driver, with the same
//! approvals. Scripts are stored, never run by Orbit: the sidecar copies them into its media
//! sandbox (no network, no host access) when it has one, else they cannot run.

use axum::extract::{DefaultBodyLimit, Extension, Multipart, Path, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine;
use orbit_domain::Permission;
use orbit_platform::{Id, RequestId, Row, TimestampMillis};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::AnyConnection;

use super::AgentState;
use super::skill_package::{
    self, MAX_ZIP_BYTES, SkillPackage, checked_body, checked_description, checked_name,
};
use crate::audit::{self, AuditOutcome};
use crate::repositories::membership;
use crate::task_routes::{ApiError, ApiJson, authenticate_session, request_id_value};

type RequestIdExtension = Option<Extension<RequestId>>;

/// Skills one workspace may have.
pub const MAX_SKILLS: i64 = 100;
/// The default agent in `agent_skill_agents.agent`; a custom agent is its bot user id.
pub const DEFAULT_AGENT: &str = "default";

pub fn router(state: AgentState) -> Router {
    let base = "/api/v1/workspaces/{workspace_id}/agent-skills";
    let tools = "/api/v1/workspaces/{workspace_id}/agent-tools/skills";
    Router::new()
        .route(base, get(list_skills).post(create_skill))
        .route(&format!("{base}/install"), post(install_skill))
        .route(
            &format!("{base}/upload"),
            post(upload_skill).layer(DefaultBodyLimit::max(MAX_ZIP_BYTES + 64 * 1024)),
        )
        .route(
            &format!("{base}/{{skill_id}}"),
            get(read_skill).patch(update_skill).delete(delete_skill),
        )
        .route(tools, get(agent_list))
        .route(&format!("{tools}/{{name}}"), get(agent_load))
        .route(
            &format!("{tools}/{{name}}/files/{{*path}}"),
            get(agent_file),
        )
        .with_state(state)
}

// ---------------------------------------------------------------------------------------------
// Errors and access

fn problem(
    status: StatusCode,
    code: &'static str,
    detail: &'static str,
    uri: &Uri,
    request_id: &RequestIdExtension,
) -> ApiError {
    let title = status.canonical_reason().unwrap_or("Error");
    ApiError::new(status, code, title, detail, uri.path(), request_id.as_ref())
}

fn unavailable(error: &sqlx::Error, uri: &Uri, request_id: &RequestIdExtension) -> ApiError {
    tracing::warn!(%error, "agent skills");
    problem(
        StatusCode::SERVICE_UNAVAILABLE,
        "unavailable",
        "Try again in a moment.",
        uri,
        request_id,
    )
}

fn invalid(detail: String, uri: &Uri, request_id: &RequestIdExtension) -> ApiError {
    problem(
        StatusCode::UNPROCESSABLE_ENTITY,
        "validation_failed",
        "The skill is not valid.",
        uri,
        request_id,
    )
    .with_detail(detail)
}

fn not_found(uri: &Uri, request_id: &RequestIdExtension) -> ApiError {
    problem(
        StatusCode::NOT_FOUND,
        "not_found",
        "Not found.",
        uri,
        request_id,
    )
}

fn forbidden(uri: &Uri, request_id: &RequestIdExtension) -> ApiError {
    problem(
        StatusCode::FORBIDDEN,
        "forbidden",
        "Only owners and admins can manage skills.",
        uri,
        request_id,
    )
}

/// The caller as an owner or admin of the workspace. Members and guests get 403, others 404.
async fn manager(
    state: &AgentState,
    headers: &HeaderMap,
    workspace_id: &str,
    uri: &Uri,
    request_id: &RequestIdExtension,
) -> Result<(Id, Id), ApiError> {
    let session = authenticate_session(
        &state.identity,
        state.cookie_mode,
        headers,
        uri.path(),
        request_id.as_ref(),
    )
    .await?;
    if state.config.is_none() {
        return Err(problem(
            StatusCode::NOT_FOUND,
            "agent_disabled",
            "The agent is not set up on this server.",
            uri,
            request_id,
        ));
    }
    let workspace_id: Id = workspace_id
        .parse()
        .map_err(|_| not_found(uri, request_id))?;
    let actor = membership::chat_actor(state.database().pool(), workspace_id, session.user.id)
        .await
        .map_err(|error| unavailable(&error, uri, request_id))?
        .ok_or_else(|| not_found(uri, request_id))?;
    if actor.is_guest() || !actor.can(Permission::WorkspaceUpdate) {
        return Err(forbidden(uri, request_id));
    }
    Ok((workspace_id, session.user.id))
}

/// The workspace and the agent (`default` or a custom agent's bot user) of an agent session.
/// People's sessions get 403: only the agent reads skills this way.
async fn session_agent(
    state: &AgentState,
    headers: &HeaderMap,
    workspace_id: &str,
    uri: &Uri,
    request_id: &RequestIdExtension,
) -> Result<(Id, String), ApiError> {
    let session = authenticate_session(
        &state.identity,
        state.cookie_mode,
        headers,
        uri.path(),
        request_id.as_ref(),
    )
    .await?;
    let workspace_id: Id = workspace_id
        .parse()
        .map_err(|_| not_found(uri, request_id))?;
    let database = state.database();
    let row =
        sqlx::query("SELECT workspace_id, agent_user_id FROM agent_sessions WHERE session_id = $1")
            .bind(session.id.to_string())
            .fetch_optional(database.pool())
            .await
            .map_err(|error| unavailable(&error, uri, request_id))?;
    let Some(row) =
        row.filter(|row| row.get::<String, _>("workspace_id") == workspace_id.to_string())
    else {
        return Err(problem(
            StatusCode::FORBIDDEN,
            "agent_session_required",
            "Only the team agent reads skills this way, in a run of its driver.",
            uri,
            request_id,
        ));
    };
    if membership::actor(database.pool(), workspace_id, session.user.id)
        .await
        .map_err(|error| unavailable(&error, uri, request_id))?
        .is_none()
    {
        return Err(not_found(uri, request_id));
    }
    let agent = match row.get::<Option<String>, _>("agent_user_id") {
        None => DEFAULT_AGENT.to_owned(),
        Some(agent) => {
            // A deleted agent has no skills.
            let mut conn = database
                .pool()
                .acquire()
                .await
                .map_err(|error| unavailable(&error, uri, request_id))?;
            let user: Id = agent.parse().map_err(|_| not_found(uri, request_id))?;
            if super::profiles::live_agent(&mut conn, workspace_id, user)
                .await
                .map_err(|error| unavailable(&error, uri, request_id))?
                .is_none()
            {
                return Err(not_found(uri, request_id));
            }
            agent
        }
    };
    Ok((workspace_id, agent))
}

// ---------------------------------------------------------------------------------------------
// Views

#[derive(Serialize)]
struct SkillFileView {
    path: String,
    size: i64,
}

#[derive(Serialize)]
struct SkillView {
    id: String,
    name: String,
    description: String,
    source: String,
    source_url: Option<String>,
    /// `default` and the bot user ids of the custom agents that use it.
    agents: Vec<String>,
    files: Vec<SkillFileView>,
    /// Files below `scripts/`: they run only in the agent's sandbox.
    has_scripts: bool,
    /// `SKILL.md` and every file.
    size_bytes: i64,
    created_at: i64,
    updated_at: i64,
    /// Only in the answer for one skill.
    #[serde(skip_serializing_if = "Option::is_none")]
    body: Option<String>,
}

#[derive(Serialize)]
struct AgentOption {
    /// `default` or the agent's bot user id.
    key: String,
    name: String,
}

async fn skill_views(
    conn: &mut AnyConnection,
    workspace_id: Id,
    only: Option<Id>,
) -> Result<Vec<SkillView>, sqlx::Error> {
    let mut query = orbit_platform::SqlBuilder::new(
        "SELECT id, name, description, body, source, source_url, created_at, updated_at \
         FROM agent_skills WHERE workspace_id = ",
    );
    query.push_bind(workspace_id.to_string());
    if let Some(only) = only {
        query.push(" AND id = ");
        query.push_bind(only.to_string());
    }
    query.push(" ORDER BY name");
    let rows = query.build().fetch_all(&mut *conn).await?;
    let mut views = Vec::with_capacity(rows.len());
    for row in rows {
        let id: String = row.get("id");
        let files: Vec<SkillFileView> = sqlx::query(
            "SELECT path, size_bytes FROM agent_skill_files WHERE skill_id = $1 ORDER BY path",
        )
        .bind(&id)
        .fetch_all(&mut *conn)
        .await?
        .iter()
        .map(|file| SkillFileView {
            path: file.get("path"),
            size: file.get("size_bytes"),
        })
        .collect();
        let agents = sqlx::query_scalar(
            "SELECT agent FROM agent_skill_agents WHERE skill_id = $1 ORDER BY agent",
        )
        .bind(&id)
        .fetch_all(&mut *conn)
        .await?;
        let body: String = row.get("body");
        views.push(SkillView {
            id,
            name: row.get("name"),
            description: row.get("description"),
            source: row.get("source"),
            source_url: row.get("source_url"),
            agents,
            has_scripts: files.iter().any(|file| file.path.starts_with("scripts/")),
            size_bytes: i64::try_from(body.len()).unwrap_or(i64::MAX)
                + files.iter().map(|file| file.size).sum::<i64>(),
            files,
            created_at: row.get("created_at"),
            updated_at: row.get("updated_at"),
            body: only.map(|_| body),
        });
    }
    Ok(views)
}

/// The agents a skill can be turned on for: the default agent, then the live custom agents.
async fn agent_options(
    conn: &mut AnyConnection,
    workspace_id: Id,
) -> Result<Vec<AgentOption>, sqlx::Error> {
    let mut options = vec![AgentOption {
        key: DEFAULT_AGENT.to_owned(),
        name: super::bot::name(conn).await?,
    }];
    let rows = sqlx::query(
        "SELECT p.user_id, u.display_name FROM agent_profiles p JOIN users u ON u.id = p.user_id \
         WHERE p.workspace_id = $1 AND p.deleted_at IS NULL ORDER BY lower(u.display_name), p.id",
    )
    .bind(workspace_id.to_string())
    .fetch_all(&mut *conn)
    .await?;
    options.extend(rows.iter().map(|row| AgentOption {
        key: row.get("user_id"),
        name: row.get("display_name"),
    }));
    Ok(options)
}

/// Whether the sidecar has its media sandbox, where skill scripts run: `None` when it is offline.
async fn sandbox(state: &AgentState) -> Option<bool> {
    let config = state.config.as_ref()?;
    let response = state
        .http
        .get(format!("{}/health", config.url))
        .header(super::routes::SECRET_HEADER, &config.secret)
        .send()
        .await
        .ok()?;
    let health: Value = response.json().await.ok()?;
    Some(health.get("sandbox").and_then(Value::as_bool) == Some(true))
}

async fn list_skills(
    State(state): State<AgentState>,
    Path(workspace_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
) -> Result<Json<Value>, ApiError> {
    let (workspace_id, _) = manager(&state, &headers, &workspace_id, &uri, &request_id).await?;
    let mut conn = state
        .database()
        .pool()
        .acquire()
        .await
        .map_err(|error| unavailable(&error, &uri, &request_id))?;
    let skills = skill_views(&mut conn, workspace_id, None)
        .await
        .map_err(|error| unavailable(&error, &uri, &request_id))?;
    let agents = agent_options(&mut conn, workspace_id)
        .await
        .map_err(|error| unavailable(&error, &uri, &request_id))?;
    drop(conn);
    Ok(Json(json!({
        "skills": skills,
        "agents": agents,
        "sandbox": sandbox(&state).await,
        "max_skills": MAX_SKILLS,
    })))
}

async fn read_skill(
    State(state): State<AgentState>,
    Path((workspace_id, skill_id)): Path<(String, String)>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
) -> Result<Json<Value>, ApiError> {
    let (workspace_id, _) = manager(&state, &headers, &workspace_id, &uri, &request_id).await?;
    let skill_id: Id = skill_id.parse().map_err(|_| not_found(&uri, &request_id))?;
    one(&state, workspace_id, skill_id, &uri, &request_id).await
}

async fn one(
    state: &AgentState,
    workspace_id: Id,
    skill_id: Id,
    uri: &Uri,
    request_id: &RequestIdExtension,
) -> Result<Json<Value>, ApiError> {
    let mut conn = state
        .database()
        .pool()
        .acquire()
        .await
        .map_err(|error| unavailable(&error, uri, request_id))?;
    let view = skill_views(&mut conn, workspace_id, Some(skill_id))
        .await
        .map_err(|error| unavailable(&error, uri, request_id))?
        .into_iter()
        .next()
        .ok_or_else(|| not_found(uri, request_id))?;
    Ok(Json(json!(view)))
}

// ---------------------------------------------------------------------------------------------
// Adding skills

enum SaveError {
    Invalid(String),
    Exists(String),
    Limit,
    NotFound,
    Forbidden,
    Database(sqlx::Error),
}

impl From<sqlx::Error> for SaveError {
    fn from(error: sqlx::Error) -> Self {
        Self::Database(error)
    }
}

fn save_failure(error: SaveError, uri: &Uri, request_id: &RequestIdExtension) -> ApiError {
    match error {
        SaveError::Invalid(detail) => invalid(detail, uri, request_id),
        SaveError::Exists(name) => problem(
            StatusCode::CONFLICT,
            "skill_exists",
            "A skill with this name exists.",
            uri,
            request_id,
        )
        .with_detail(format!(
            "A skill named {name} exists. Delete it first to add it again."
        )),
        SaveError::Limit => problem(
            StatusCode::CONFLICT,
            "limit_reached",
            "This workspace has 100 skills. Delete one first.",
            uri,
            request_id,
        ),
        SaveError::NotFound => not_found(uri, request_id),
        SaveError::Forbidden => forbidden(uri, request_id),
        SaveError::Database(error) => unavailable(&error, uri, request_id),
    }
}

/// Whether the caller is still an owner or admin, inside the write transaction.
async fn still_manager(
    conn: &mut AnyConnection,
    workspace_id: Id,
    user_id: Id,
) -> Result<(), SaveError> {
    let allowed = membership::actor(&mut *conn, workspace_id, user_id)
        .await?
        .is_some_and(|actor| actor.can(Permission::WorkspaceUpdate));
    if allowed {
        Ok(())
    } else {
        Err(SaveError::Forbidden)
    }
}

/// Stores a new skill, turned on for no agent yet.
async fn save_new(
    state: &AgentState,
    workspace_id: Id,
    actor: Id,
    package: SkillPackage,
    source: &str,
    source_url: Option<&str>,
    request: &str,
) -> Result<Id, SaveError> {
    let mut tx = state.database().immediate_transaction().await?;
    still_manager(&mut tx, workspace_id, actor).await?;
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM agent_skills WHERE workspace_id = $1")
            .bind(workspace_id.to_string())
            .fetch_one(&mut *tx)
            .await?;
    if count >= MAX_SKILLS {
        return Err(SaveError::Limit);
    }
    if name_used(&mut tx, workspace_id, &package.name, None).await? {
        return Err(SaveError::Exists(package.name));
    }
    let id = Id::new_v7();
    let now = TimestampMillis::now();
    sqlx::query(
        "INSERT INTO agent_skills (id, workspace_id, name, description, body, source, source_url, \
         created_by, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $9)",
    )
    .bind(id.to_string())
    .bind(workspace_id.to_string())
    .bind(&package.name)
    .bind(&package.description)
    .bind(&package.body)
    .bind(source)
    .bind(source_url)
    .bind(actor.to_string())
    .bind(now.as_millis())
    .execute(&mut *tx)
    .await?;
    for file in &package.files {
        sqlx::query(
            "INSERT INTO agent_skill_files (skill_id, path, content, size_bytes) VALUES ($1, $2, $3, $4)",
        )
        .bind(id.to_string())
        .bind(&file.path)
        .bind(file.content.clone())
        .bind(i64::try_from(file.content.len()).unwrap_or(i64::MAX))
        .execute(&mut *tx)
        .await?;
    }
    audit::record(
        &mut tx,
        workspace_id,
        Some(actor),
        "agent_skill.create",
        AuditOutcome::Success,
        "agent_skill",
        Some(id),
        request,
        json!({ "name": package.name, "source": source, "source_url": source_url, "files": package.files.len() }),
        now,
    )
    .await?;
    tx.commit().await?;
    Ok(id)
}

async fn name_used(
    conn: &mut AnyConnection,
    workspace_id: Id,
    name: &str,
    except: Option<Id>,
) -> Result<bool, sqlx::Error> {
    Ok(sqlx::query_scalar::<_, orbit_platform::Flag>(
        "SELECT EXISTS (SELECT 1 FROM agent_skills WHERE workspace_id = $1 AND name = $2 AND id <> $3)",
    )
    .bind(workspace_id.to_string())
    .bind(name)
    .bind(except.map(|id| id.to_string()).unwrap_or_default())
    .fetch_one(&mut *conn)
    .await?
    .0)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WrittenSkill {
    name: String,
    description: String,
    body: String,
}

/// A skill written in Settings: name, description and body, no files.
async fn create_skill(
    State(state): State<AgentState>,
    Path(workspace_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
    ApiJson(input): ApiJson<WrittenSkill>,
) -> Result<Json<Value>, ApiError> {
    let (workspace_id, actor) = manager(&state, &headers, &workspace_id, &uri, &request_id).await?;
    let package = (|| {
        Ok::<_, String>(SkillPackage {
            name: checked_name(&input.name)?,
            description: checked_description(&input.description)?,
            body: checked_body(&input.body)?,
            files: Vec::new(),
        })
    })()
    .map_err(|detail| invalid(detail, &uri, &request_id))?;
    let request = request_id_value(request_id.as_ref()).to_owned();
    let id = save_new(
        &state,
        workspace_id,
        actor,
        package,
        "manual",
        None,
        &request,
    )
    .await
    .map_err(|error| save_failure(error, &uri, &request_id))?;
    one(&state, workspace_id, id, &uri, &request_id).await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InstallBody {
    /// A GitHub folder, a skills.sh page, or `owner/repo@skill`.
    source: String,
}

/// Installs a skill from public GitHub (a skills.sh entry names a GitHub repository).
async fn install_skill(
    State(state): State<AgentState>,
    Path(workspace_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
    ApiJson(input): ApiJson<InstallBody>,
) -> Result<Json<Value>, ApiError> {
    let (workspace_id, actor) = manager(&state, &headers, &workspace_id, &uri, &request_id).await?;
    let source = skill_package::parse_source(&input.source)
        .map_err(|detail| invalid(detail, &uri, &request_id))?;
    let package = skill_package::fetch(&state.outbound, &state.skill_sources, &source)
        .await
        .map_err(|detail| {
            problem(
                StatusCode::BAD_GATEWAY,
                "skill_fetch_failed",
                "The skill could not be downloaded.",
                &uri,
                &request_id,
            )
            .with_detail(detail)
        })?;
    let request = request_id_value(request_id.as_ref()).to_owned();
    let id = save_new(
        &state,
        workspace_id,
        actor,
        package,
        source.kind,
        Some(&source.url),
        &request,
    )
    .await
    .map_err(|error| save_failure(error, &uri, &request_id))?;
    one(&state, workspace_id, id, &uri, &request_id).await
}

/// Adds a skill from an uploaded `.zip` (the multipart field `file`).
async fn upload_skill(
    State(state): State<AgentState>,
    Path(workspace_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
    mut multipart: Multipart,
) -> Result<Json<Value>, ApiError> {
    let (workspace_id, actor) = manager(&state, &headers, &workspace_id, &uri, &request_id).await?;
    let bad = || {
        problem(
            StatusCode::BAD_REQUEST,
            "invalid_multipart",
            "Send the .zip as the multipart field \"file\" (at most 4 MiB).",
            &uri,
            &request_id,
        )
    };
    let mut zip = None;
    while let Some(field) = multipart.next_field().await.map_err(|_| bad())? {
        if field.name() == Some("file") {
            zip = Some(field.bytes().await.map_err(|_| bad())?);
        }
    }
    let zip = zip.ok_or_else(bad)?;
    let package = tokio::task::spawn_blocking(move || skill_package::from_zip(&zip))
        .await
        .map_err(|_| bad())?
        .map_err(|detail| invalid(detail, &uri, &request_id))?;
    let request = request_id_value(request_id.as_ref()).to_owned();
    let id = save_new(&state, workspace_id, actor, package, "zip", None, &request)
        .await
        .map_err(|error| save_failure(error, &uri, &request_id))?;
    one(&state, workspace_id, id, &uri, &request_id).await
}

// ---------------------------------------------------------------------------------------------
// Changing and deleting

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SkillChange {
    name: Option<String>,
    description: Option<String>,
    body: Option<String>,
    /// Exactly the agents that use it: `default` and custom agents' bot user ids.
    agents: Option<Vec<String>>,
}

async fn update_skill(
    State(state): State<AgentState>,
    Path((workspace_id, skill_id)): Path<(String, String)>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
    ApiJson(input): ApiJson<SkillChange>,
) -> Result<Json<Value>, ApiError> {
    let (workspace_id, actor) = manager(&state, &headers, &workspace_id, &uri, &request_id).await?;
    let skill_id: Id = skill_id.parse().map_err(|_| not_found(&uri, &request_id))?;
    let checked = (|| {
        Ok::<_, String>((
            input.name.as_deref().map(checked_name).transpose()?,
            input
                .description
                .as_deref()
                .map(checked_description)
                .transpose()?,
            input.body.as_deref().map(checked_body).transpose()?,
        ))
    })()
    .map_err(|detail| invalid(detail, &uri, &request_id))?;
    let (name, description, body) = checked;
    let request = request_id_value(request_id.as_ref()).to_owned();
    let database = state.database();
    async {
        let mut tx = database.immediate_transaction().await?;
        still_manager(&mut tx, workspace_id, actor).await?;
        let exists = sqlx::query_scalar::<_, String>(
            "SELECT id FROM agent_skills WHERE id = $1 AND workspace_id = $2",
        )
        .bind(skill_id.to_string())
        .bind(workspace_id.to_string())
        .fetch_optional(&mut *tx)
        .await?;
        if exists.is_none() {
            return Err(SaveError::NotFound);
        }
        let now = TimestampMillis::now();
        if let Some(name) = &name {
            if name_used(&mut tx, workspace_id, name, Some(skill_id)).await? {
                return Err(SaveError::Exists(name.clone()));
            }
            sqlx::query("UPDATE agent_skills SET name = $1 WHERE id = $2")
                .bind(name)
                .bind(skill_id.to_string())
                .execute(&mut *tx)
                .await?;
        }
        if let Some(description) = &description {
            sqlx::query("UPDATE agent_skills SET description = $1 WHERE id = $2")
                .bind(description)
                .bind(skill_id.to_string())
                .execute(&mut *tx)
                .await?;
        }
        if let Some(body) = &body {
            sqlx::query("UPDATE agent_skills SET body = $1 WHERE id = $2")
                .bind(body)
                .bind(skill_id.to_string())
                .execute(&mut *tx)
                .await?;
        }
        if let Some(agents) = &input.agents {
            let known: Vec<String> = agent_options(&mut tx, workspace_id)
                .await?
                .into_iter()
                .map(|option| option.key)
                .collect();
            if agents.iter().any(|agent| !known.contains(agent)) {
                return Err(SaveError::Invalid(
                    "One of the agents is not an agent of this workspace.".to_owned(),
                ));
            }
            sqlx::query("DELETE FROM agent_skill_agents WHERE skill_id = $1")
                .bind(skill_id.to_string())
                .execute(&mut *tx)
                .await?;
            for agent in known.iter().filter(|key| agents.contains(key)) {
                sqlx::query("INSERT INTO agent_skill_agents (skill_id, agent) VALUES ($1, $2)")
                    .bind(skill_id.to_string())
                    .bind(agent)
                    .execute(&mut *tx)
                    .await?;
            }
        }
        sqlx::query("UPDATE agent_skills SET updated_at = $1 WHERE id = $2")
            .bind(now.as_millis())
            .bind(skill_id.to_string())
            .execute(&mut *tx)
            .await?;
        audit::record(
            &mut tx,
            workspace_id,
            Some(actor),
            "agent_skill.update",
            AuditOutcome::Success,
            "agent_skill",
            Some(skill_id),
            &request,
            json!({
                "name": name,
                "description": description.is_some(),
                "body": body.is_some(),
                "agents": input.agents,
            }),
            now,
        )
        .await?;
        tx.commit().await?;
        Ok::<_, SaveError>(())
    }
    .await
    .map_err(|error| save_failure(error, &uri, &request_id))?;
    one(&state, workspace_id, skill_id, &uri, &request_id).await
}

async fn delete_skill(
    State(state): State<AgentState>,
    Path((workspace_id, skill_id)): Path<(String, String)>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
) -> Result<StatusCode, ApiError> {
    let (workspace_id, actor) = manager(&state, &headers, &workspace_id, &uri, &request_id).await?;
    let skill_id: Id = skill_id.parse().map_err(|_| not_found(&uri, &request_id))?;
    let request = request_id_value(request_id.as_ref()).to_owned();
    let database = state.database();
    async {
        let mut tx = database.immediate_transaction().await?;
        still_manager(&mut tx, workspace_id, actor).await?;
        let name = sqlx::query_scalar::<_, String>(
            "SELECT name FROM agent_skills WHERE id = $1 AND workspace_id = $2",
        )
        .bind(skill_id.to_string())
        .bind(workspace_id.to_string())
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(SaveError::NotFound)?;
        sqlx::query("DELETE FROM agent_skills WHERE id = $1")
            .bind(skill_id.to_string())
            .execute(&mut *tx)
            .await?;
        let now = TimestampMillis::now();
        audit::record(
            &mut tx,
            workspace_id,
            Some(actor),
            "agent_skill.delete",
            AuditOutcome::Success,
            "agent_skill",
            Some(skill_id),
            &request,
            json!({ "name": name }),
            now,
        )
        .await?;
        tx.commit().await?;
        Ok::<_, SaveError>(())
    }
    .await
    .map_err(|error| save_failure(error, &uri, &request_id))?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------------------------
// The agent's routes

/// The skill `name` if it is turned on for `agent`.
async fn enabled_skill(
    state: &AgentState,
    workspace_id: Id,
    agent: &str,
    name: &str,
) -> Result<Option<(String, String, String, String)>, sqlx::Error> {
    let row = sqlx::query(
        "SELECT s.id, s.name, s.description, s.body FROM agent_skills s \
         JOIN agent_skill_agents a ON a.skill_id = s.id \
         WHERE s.workspace_id = $1 AND a.agent = $2 AND s.name = $3",
    )
    .bind(workspace_id.to_string())
    .bind(agent)
    .bind(name)
    .fetch_optional(state.database().pool())
    .await?;
    Ok(row.map(|row| {
        (
            row.get("id"),
            row.get("name"),
            row.get("description"),
            row.get("body"),
        )
    }))
}

/// The session agent's skills: name and description only, for its system prompt.
async fn agent_list(
    State(state): State<AgentState>,
    Path(workspace_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
) -> Result<Json<Value>, ApiError> {
    let (workspace_id, agent) =
        session_agent(&state, &headers, &workspace_id, &uri, &request_id).await?;
    let skills: Vec<Value> = sqlx::query(
        "SELECT s.name, s.description, (SELECT COUNT(*) FROM agent_skill_files f \
         WHERE f.skill_id = s.id AND f.path LIKE 'scripts/%') AS scripts \
         FROM agent_skills s JOIN agent_skill_agents a ON a.skill_id = s.id \
         WHERE s.workspace_id = $1 AND a.agent = $2 ORDER BY s.name",
    )
    .bind(workspace_id.to_string())
    .bind(&agent)
    .fetch_all(state.database().pool())
    .await
    .map_err(|error| unavailable(&error, &uri, &request_id))?
    .iter()
    .map(|row| {
        json!({
            "name": row.get::<String, _>("name"),
            "description": row.get::<String, _>("description"),
            "has_scripts": row.get::<i64, _>("scripts") > 0,
        })
    })
    .collect();
    Ok(Json(json!({ "skills": skills })))
}

/// One skill of the session's agent: its `SKILL.md` body and its files.
async fn agent_load(
    State(state): State<AgentState>,
    Path((workspace_id, name)): Path<(String, String)>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
) -> Result<Json<Value>, ApiError> {
    let (workspace_id, agent) =
        session_agent(&state, &headers, &workspace_id, &uri, &request_id).await?;
    let (id, name, description, body) = enabled_skill(&state, workspace_id, &agent, &name)
        .await
        .map_err(|error| unavailable(&error, &uri, &request_id))?
        .ok_or_else(|| skill_not_enabled(&uri, &request_id))?;
    let files: Vec<Value> = sqlx::query(
        "SELECT path, size_bytes FROM agent_skill_files WHERE skill_id = $1 ORDER BY path",
    )
    .bind(&id)
    .fetch_all(state.database().pool())
    .await
    .map_err(|error| unavailable(&error, &uri, &request_id))?
    .iter()
    .map(|row| json!({ "path": row.get::<String, _>("path"), "size": row.get::<i64, _>("size_bytes") }))
    .collect();
    Ok(Json(json!({
        "name": name,
        "description": description,
        "body": body,
        "files": files,
    })))
}

fn skill_not_enabled(uri: &Uri, request_id: &RequestIdExtension) -> ApiError {
    problem(
        StatusCode::NOT_FOUND,
        "skill_not_enabled",
        "This agent has no skill with this name.",
        uri,
        request_id,
    )
}

/// One file of a skill of the session's agent: UTF-8 text as `text`, other bytes as `base64`.
async fn agent_file(
    State(state): State<AgentState>,
    Path((workspace_id, name, path)): Path<(String, String, String)>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
) -> Result<Json<Value>, ApiError> {
    let (workspace_id, agent) =
        session_agent(&state, &headers, &workspace_id, &uri, &request_id).await?;
    let (id, ..) = enabled_skill(&state, workspace_id, &agent, &name)
        .await
        .map_err(|error| unavailable(&error, &uri, &request_id))?
        .ok_or_else(|| skill_not_enabled(&uri, &request_id))?;
    let row =
        sqlx::query("SELECT content FROM agent_skill_files WHERE skill_id = $1 AND path = $2")
            .bind(&id)
            .bind(&path)
            .fetch_optional(state.database().pool())
            .await
            .map_err(|error| unavailable(&error, &uri, &request_id))?
            .ok_or_else(|| not_found(&uri, &request_id))?;
    let content: Vec<u8> = row.get("content");
    let size = content.len();
    Ok(Json(match String::from_utf8(content) {
        Ok(text) => json!({ "path": path, "size": size, "text": text }),
        Err(bytes) => json!({
            "path": path,
            "size": size,
            "base64": base64::engine::general_purpose::STANDARD.encode(bytes.into_bytes()),
        }),
    }))
}
