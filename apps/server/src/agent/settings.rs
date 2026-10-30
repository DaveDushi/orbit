//! The agent's settings that people change: its name (the bot user's display name) and its model
//! and thinking level (one for the installation, set by an installation admin), and each
//! workspace's team instructions (set by the workspace's owners and admins, read by every
//! member). The sidecar reads them for each run (`routes::list_inputs`). Agent sessions reach neither: `scope` refuses `/agent` of a workspace
//! and every `/api/v1/admin` route. Like the other agent routes, these are not in the OpenAPI
//! contract.

use axum::extract::{Extension, Path, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::routing::get;
use axum::{Json, Router};
use orbit_domain::Permission;
use orbit_platform::{Database, Id, RequestId, Row, TimestampMillis};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::AnyConnection;

use super::{AgentState, bot};
use crate::audit::{self, AuditOutcome};
use crate::repositories::membership;
use crate::task_routes::{ApiError, ApiJson, authenticate_session, request_id_value};

type RequestIdExtension = Option<Extension<RequestId>>;

pub const MIN_NAME_CHARS: usize = 2;
pub const MAX_NAME_CHARS: usize = 40;
pub const MAX_INSTRUCTIONS_CHARS: usize = 4_000;
/// Pi's thinking levels without `off`, which is its default; the CHECK of `agent_model_settings`.
pub const THINKING_LEVELS: [&str; 6] = ["minimal", "low", "medium", "high", "xhigh", "max"];

/// The trimmed name, or why it cannot be the agent's name. The name is the agent's @handle: the
/// sidecar turns `@Name` in its answers into a mention, so it has only word characters and single
/// spaces, as people's names do.
pub fn checked_name(raw: &str) -> Result<String, &'static str> {
    let name = raw.trim();
    let count = name.chars().count();
    if !(MIN_NAME_CHARS..=MAX_NAME_CHARS).contains(&count) {
        return Err("The name must have 2 to 40 characters.");
    }
    if !name.chars().next().is_some_and(char::is_alphanumeric) {
        return Err("The name must start with a letter or a digit.");
    }
    let allowed = |c: char| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.' | '\'');
    if !name.chars().all(allowed) || name.contains("  ") {
        return Err("Use letters, digits, single spaces and - _ . ' only.");
    }
    Ok(name.to_owned())
}

/// The instructions as stored: line breaks as `\n`, no space around them, at most 4,000
/// characters. Empty means none.
pub fn checked_instructions(raw: &str) -> Result<String, &'static str> {
    let text = raw.replace("\r\n", "\n");
    let text = text.trim();
    if text.chars().count() > MAX_INSTRUCTIONS_CHARS {
        return Err("Team instructions may have at most 4,000 characters.");
    }
    if text
        .chars()
        .any(|c| c.is_control() && c != '\n' && c != '\t')
    {
        return Err("Team instructions may not contain control characters.");
    }
    Ok(text.to_owned())
}

/// Two names that the sidecar cannot tell apart in `@Name`.
#[must_use]
pub fn same_name(a: &str, b: &str) -> bool {
    a.trim().to_lowercase() == b.trim().to_lowercase()
}

/// Whether another account has this name. The bot joins every workspace, also the ones made since
/// the sidecar last asked for inputs, so every account shares or will share a workspace with it.
/// Compared in Rust, because SQLite's `lower` folds ASCII only.
async fn name_taken(conn: &mut AnyConnection, bot: Id, name: &str) -> Result<bool, sqlx::Error> {
    let names =
        sqlx::query_scalar::<_, String>("SELECT DISTINCT display_name FROM users WHERE id <> $1")
            .bind(bot.to_string())
            .fetch_all(&mut *conn)
            .await?;
    Ok(names.iter().any(|other| same_name(other, name)))
}

enum RenameError {
    Taken,
    Database(sqlx::Error),
}

impl From<sqlx::Error> for RenameError {
    fn from(error: sqlx::Error) -> Self {
        Self::Database(error)
    }
}

/// Renames the bot. The check and the write are in one serialized transaction, so a member
/// cannot take the name between them through this route.
async fn rename(
    database: &Database,
    actor_id: Id,
    name: &str,
    request_id: &str,
) -> Result<(), RenameError> {
    let bot = bot::ensure(database).await?;
    let mut tx = database.immediate_transaction().await?;
    if name_taken(&mut tx, bot, name).await? {
        return Err(RenameError::Taken);
    }
    let now = TimestampMillis::now();
    sqlx::query(
        "UPDATE users SET display_name = $1, updated_at = $2, version = version + 1 WHERE id = $3",
    )
    .bind(name)
    .bind(now.as_millis())
    .bind(bot.to_string())
    .execute(&mut *tx)
    .await?;
    audit::record_global(
        &mut tx,
        Some(actor_id),
        "agent.renamed",
        AuditOutcome::Success,
        "user",
        Some(bot),
        request_id,
        json!({ "name": name }),
        now,
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

pub fn router(state: AgentState) -> Router {
    Router::new()
        .route("/api/v1/admin/agent", get(read_agent).put(save_agent))
        .route("/api/v1/admin/agent/model", get(read_model).put(save_model))
        .route(
            "/api/v1/workspaces/{workspace_id}/agent",
            get(read_team).put(save_team),
        )
        .with_state(state)
}

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
    tracing::warn!(%error, "agent settings");
    problem(
        StatusCode::SERVICE_UNAVAILABLE,
        "unavailable",
        "Try again in a moment.",
        uri,
        request_id,
    )
}

fn off(uri: &Uri, request_id: &RequestIdExtension) -> ApiError {
    problem(
        StatusCode::NOT_FOUND,
        "agent_disabled",
        "The agent is not set up on this server.",
        uri,
        request_id,
    )
}

fn invalid(detail: &'static str, uri: &Uri, request_id: &RequestIdExtension) -> ApiError {
    problem(
        StatusCode::UNPROCESSABLE_ENTITY,
        "validation_failed",
        detail,
        uri,
        request_id,
    )
}

// ---------------------------------------------------------------------------------------------
// The name: Admin → AI agent.

#[derive(Serialize)]
struct AgentView {
    name: String,
    bot_user_id: Option<String>,
    min_name_chars: usize,
    max_name_chars: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentBody {
    name: String,
}

/// The signed-in installation admin, or the error to answer with.
async fn installation_admin(
    state: &AgentState,
    headers: &HeaderMap,
    uri: &Uri,
    request_id: &RequestIdExtension,
) -> Result<Id, ApiError> {
    let session = authenticate_session(
        &state.identity,
        state.cookie_mode,
        headers,
        uri.path(),
        request_id.as_ref(),
    )
    .await?;
    let admin = state
        .identity
        .is_installation_admin(session.user.id)
        .await
        .map_err(|_| {
            problem(
                StatusCode::SERVICE_UNAVAILABLE,
                "unavailable",
                "Try again in a moment.",
                uri,
                request_id,
            )
        })?;
    if !admin {
        return Err(problem(
            StatusCode::FORBIDDEN,
            "installation_admin_required",
            "Only an installation administrator may do this.",
            uri,
            request_id,
        ));
    }
    if state.config.is_none() {
        return Err(off(uri, request_id));
    }
    Ok(session.user.id)
}

async fn agent_view(database: &Database) -> Result<AgentView, sqlx::Error> {
    let mut conn = database.pool().acquire().await?;
    let bot_user_id = super::inputs::bot_user_id(&mut conn).await?;
    let name = bot::name(&mut conn).await?;
    Ok(AgentView {
        name,
        bot_user_id: bot_user_id.map(|id| id.to_string()),
        min_name_chars: MIN_NAME_CHARS,
        max_name_chars: MAX_NAME_CHARS,
    })
}

async fn read_agent(
    State(state): State<AgentState>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
) -> Result<Json<AgentView>, ApiError> {
    installation_admin(&state, &headers, &uri, &request_id).await?;
    agent_view(state.database())
        .await
        .map(Json)
        .map_err(|error| unavailable(&error, &uri, &request_id))
}

/// Renames the agent for the whole installation. Its chat messages and every mention of it show
/// the new name, and the sidecar uses it from its next run.
async fn save_agent(
    State(state): State<AgentState>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
    ApiJson(body): ApiJson<AgentBody>,
) -> Result<Json<AgentView>, ApiError> {
    let admin = installation_admin(&state, &headers, &uri, &request_id).await?;
    let name = checked_name(&body.name).map_err(|detail| invalid(detail, &uri, &request_id))?;
    match rename(
        state.database(),
        admin,
        &name,
        request_id_value(request_id.as_ref()),
    )
    .await
    {
        Ok(()) => {}
        Err(RenameError::Taken) => {
            return Err(problem(
                StatusCode::CONFLICT,
                "agent_name_taken",
                "Someone in Orbit already has this name. Mentions of the agent would reach them, so pick another name.",
                &uri,
                &request_id,
            ));
        }
        Err(RenameError::Database(error)) => return Err(unavailable(&error, &uri, &request_id)),
    }
    agent_view(state.database())
        .await
        .map(Json)
        .map_err(|error| unavailable(&error, &uri, &request_id))
}

// ---------------------------------------------------------------------------------------------
// The model and thinking level: Admin → AI agent.

/// The saved choice; `None` is the sidecar's default.
pub(crate) async fn model_choice(
    conn: &mut AnyConnection,
) -> Result<(Option<String>, Option<String>), sqlx::Error> {
    let row = sqlx::query("SELECT model, thinking FROM agent_model_settings WHERE id = 1")
        .fetch_optional(&mut *conn)
        .await?;
    Ok(row.map_or((None, None), |row| (row.get("model"), row.get("thinking"))))
}

/// What the sidecar offers: its default model and every model of the ChatGPT (Codex) provider.
#[derive(Deserialize, Serialize)]
struct SidecarModels {
    default_model: String,
    models: Vec<ModelOption>,
}

#[derive(Deserialize, Serialize)]
struct ModelOption {
    id: String,
    name: String,
}

/// The sidecar's models; `None` when it does not answer.
async fn sidecar_models(state: &AgentState) -> Option<SidecarModels> {
    let config = state.config.as_ref()?;
    let response = state
        .http
        .get(format!("{}/models", config.url))
        .header(super::routes::SECRET_HEADER, &config.secret)
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        tracing::warn!(status = %response.status(), "agent sidecar models route");
        return None;
    }
    response.json().await.ok()
}

#[derive(Serialize)]
struct ModelView {
    model: Option<String>,
    thinking: Option<String>,
    /// `None` when the sidecar is offline: then only "default" can be saved.
    available: Option<SidecarModels>,
    thinking_levels: [&'static str; 6],
    updated_at: Option<i64>,
    updated_by_name: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ModelBody {
    model: Option<String>,
    thinking: Option<String>,
}

async fn model_view(state: &AgentState) -> Result<ModelView, sqlx::Error> {
    let row = sqlx::query(
        "SELECT s.model, s.thinking, s.updated_at, u.display_name FROM agent_model_settings s \
         LEFT JOIN users u ON u.id = s.updated_by WHERE s.id = 1",
    )
    .fetch_optional(state.database().pool())
    .await?;
    Ok(ModelView {
        model: row.as_ref().and_then(|row| row.get("model")),
        thinking: row.as_ref().and_then(|row| row.get("thinking")),
        available: sidecar_models(state).await,
        thinking_levels: THINKING_LEVELS,
        updated_at: row.as_ref().map(|row| row.get("updated_at")),
        updated_by_name: row.and_then(|row| row.get("display_name")),
    })
}

async fn read_model(
    State(state): State<AgentState>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
) -> Result<Json<ModelView>, ApiError> {
    installation_admin(&state, &headers, &uri, &request_id).await?;
    model_view(&state)
        .await
        .map(Json)
        .map_err(|error| unavailable(&error, &uri, &request_id))
}

/// Sets the model and thinking level of every run from now on; `null` is the default. A model must
/// be one the sidecar offers now, so it must answer unless the model goes back to the default.
async fn save_model(
    State(state): State<AgentState>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
    ApiJson(body): ApiJson<ModelBody>,
) -> Result<Json<ModelView>, ApiError> {
    let admin = installation_admin(&state, &headers, &uri, &request_id).await?;
    let thinking = body.thinking.filter(|level| !level.is_empty());
    if thinking
        .as_deref()
        .is_some_and(|level| !THINKING_LEVELS.contains(&level))
    {
        return Err(invalid(
            "The thinking level must be minimal, low, medium, high, xhigh or max.",
            &uri,
            &request_id,
        ));
    }
    let model = body
        .model
        .map(|model| model.trim().to_owned())
        .filter(|model| !model.is_empty());
    if let Some(model) = &model {
        let available = sidecar_models(&state).await.ok_or_else(|| {
            problem(
                StatusCode::BAD_GATEWAY,
                "agent_offline",
                "The agent service is offline, so the model cannot be checked. Try again when it runs.",
                &uri,
                &request_id,
            )
        })?;
        if !available.models.iter().any(|option| &option.id == model) {
            return Err(invalid(
                "The agent does not know this model.",
                &uri,
                &request_id,
            ));
        }
    }
    let database = state.database();
    async {
        let mut tx = database.immediate_transaction().await?;
        let now = TimestampMillis::now();
        sqlx::query(
            "INSERT INTO agent_model_settings (id, model, thinking, updated_by, updated_at) \
             VALUES (1, $1, $2, $3, $4) ON CONFLICT (id) DO UPDATE SET model = excluded.model, \
             thinking = excluded.thinking, updated_by = excluded.updated_by, \
             updated_at = excluded.updated_at",
        )
        .bind(model.as_deref())
        .bind(thinking.as_deref())
        .bind(admin.to_string())
        .bind(now.as_millis())
        .execute(&mut *tx)
        .await?;
        audit::record_global(
            &mut tx,
            Some(admin),
            "agent.model_changed",
            AuditOutcome::Success,
            "agent",
            None,
            request_id_value(request_id.as_ref()),
            json!({ "model": model, "thinking": thinking }),
            now,
        )
        .await?;
        tx.commit().await
    }
    .await
    .map_err(|error| unavailable(&error, &uri, &request_id))?;
    model_view(&state)
        .await
        .map(Json)
        .map_err(|error| unavailable(&error, &uri, &request_id))
}

// ---------------------------------------------------------------------------------------------
// Team instructions: Workspace settings → AI agent.

#[derive(Serialize)]
struct TeamView {
    instructions: String,
    /// Owners and admins (`workspace.update`); members read only.
    can_edit: bool,
    max_chars: usize,
    updated_at: Option<i64>,
    updated_by_name: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TeamBody {
    instructions: String,
}

/// The caller's role in the workspace, or 404 for outsiders and when the agent is off.
async fn member(
    state: &AgentState,
    headers: &HeaderMap,
    workspace_id: &str,
    uri: &Uri,
    request_id: &RequestIdExtension,
) -> Result<(Id, orbit_domain::Actor), ApiError> {
    let session = authenticate_session(
        &state.identity,
        state.cookie_mode,
        headers,
        uri.path(),
        request_id.as_ref(),
    )
    .await?;
    if state.config.is_none() {
        return Err(off(uri, request_id));
    }
    let not_found = || {
        problem(
            StatusCode::NOT_FOUND,
            "not_found",
            "Not found.",
            uri,
            request_id,
        )
    };
    let workspace_id: Id = workspace_id.parse().map_err(|_| not_found())?;
    let actor = membership::actor(state.database().pool(), workspace_id, session.user.id)
        .await
        .map_err(|error| unavailable(&error, uri, request_id))?
        .ok_or_else(not_found)?;
    Ok((workspace_id, actor))
}

async fn team_view(
    database: &Database,
    workspace_id: Id,
    actor: orbit_domain::Actor,
) -> Result<TeamView, sqlx::Error> {
    let row = sqlx::query(
        "SELECT t.body, t.updated_at, u.display_name FROM agent_team_instructions t \
         LEFT JOIN users u ON u.id = t.updated_by WHERE t.workspace_id = $1",
    )
    .bind(workspace_id.to_string())
    .fetch_optional(database.pool())
    .await?;
    Ok(TeamView {
        instructions: row.as_ref().map(|row| row.get("body")).unwrap_or_default(),
        can_edit: actor.can(Permission::WorkspaceUpdate),
        max_chars: MAX_INSTRUCTIONS_CHARS,
        updated_at: row.as_ref().map(|row| row.get("updated_at")),
        updated_by_name: row.and_then(|row| row.get("display_name")),
    })
}

async fn read_team(
    State(state): State<AgentState>,
    Path(workspace_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
) -> Result<Json<TeamView>, ApiError> {
    let (workspace_id, actor) = member(&state, &headers, &workspace_id, &uri, &request_id).await?;
    team_view(state.database(), workspace_id, actor)
        .await
        .map(Json)
        .map_err(|error| unavailable(&error, &uri, &request_id))
}

/// Replaces the team instructions. The role is checked again in the write transaction.
async fn save_team(
    State(state): State<AgentState>,
    Path(workspace_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
    ApiJson(body): ApiJson<TeamBody>,
) -> Result<Json<TeamView>, ApiError> {
    let (workspace_id, actor) = member(&state, &headers, &workspace_id, &uri, &request_id).await?;
    let forbidden = || {
        problem(
            StatusCode::FORBIDDEN,
            "forbidden",
            "Only owners and admins can change the team instructions.",
            &uri,
            &request_id,
        )
    };
    if !actor.can(Permission::WorkspaceUpdate) {
        return Err(forbidden());
    }
    let text = checked_instructions(&body.instructions)
        .map_err(|detail| invalid(detail, &uri, &request_id))?;
    let database = state.database();
    let saved = async {
        let mut tx = database.immediate_transaction().await?;
        let allowed = membership::actor(&mut *tx, workspace_id, actor.user_id)
            .await?
            .is_some_and(|actor| actor.can(Permission::WorkspaceUpdate));
        if !allowed {
            return Ok(false);
        }
        let now = TimestampMillis::now();
        sqlx::query(
            "INSERT INTO agent_team_instructions (workspace_id, body, updated_by, updated_at) \
             VALUES ($1, $2, $3, $4) ON CONFLICT (workspace_id) DO UPDATE SET \
             body = excluded.body, updated_by = excluded.updated_by, updated_at = excluded.updated_at",
        )
        .bind(workspace_id.to_string())
        .bind(&text)
        .bind(actor.user_id.to_string())
        .bind(now.as_millis())
        .execute(&mut *tx)
        .await?;
        let chars = i64::try_from(text.chars().count()).unwrap_or(i64::MAX);
        audit::record(
            &mut tx,
            workspace_id,
            Some(actor.user_id),
            "agent.instructions_updated",
            AuditOutcome::Success,
            "workspace",
            Some(workspace_id),
            request_id_value(request_id.as_ref()),
            json!({ "chars": chars }),
            now,
        )
        .await?;
        tx.commit().await?;
        Ok::<_, sqlx::Error>(true)
    }
    .await
    .map_err(|error| unavailable(&error, &uri, &request_id))?;
    if !saved {
        return Err(forbidden());
    }
    team_view(database, workspace_id, actor)
        .await
        .map(Json)
        .map_err(|error| unavailable(&error, &uri, &request_id))
}

#[cfg(test)]
mod tests {
    use super::{checked_instructions, checked_name, same_name};

    #[test]
    fn names_are_trimmed_and_limited_to_handle_characters() {
        assert_eq!(checked_name("  Ada Bot ").as_deref(), Ok("Ada Bot"));
        assert_eq!(checked_name("Zoë-2.0").as_deref(), Ok("Zoë-2.0"));
        assert_eq!(checked_name("O'Brien_AI").as_deref(), Ok("O'Brien_AI"));
        assert_eq!(checked_name(&"a".repeat(40)).map(|name| name.len()), Ok(40));
        for refused in [
            "",
            " ",
            "A",
            &"a".repeat(41),
            "@Bot",
            "-Bot",
            "Bot <b>",
            "Bot\nTwo",
            "Two  spaces",
            "Bot@home",
            "#bot",
        ] {
            assert!(checked_name(refused).is_err(), "{refused:?}");
        }
    }

    #[test]
    fn names_compare_trimmed_and_in_any_case() {
        assert!(same_name("Ada Bot", " ada BOT "));
        assert!(same_name("ÉLAN", "élan"));
        assert!(!same_name("Ada", "Ada Bot"));
    }

    #[test]
    fn instructions_are_trimmed_and_at_most_4000_characters() {
        assert_eq!(
            checked_instructions("  Use British English.\r\nBe brief.  ").as_deref(),
            Ok("Use British English.\nBe brief.")
        );
        assert_eq!(checked_instructions("").as_deref(), Ok(""));
        // Characters, not bytes: 4,000 two-byte letters are allowed.
        assert!(checked_instructions(&"é".repeat(4_000)).is_ok());
        assert!(checked_instructions(&"a".repeat(4_001)).is_err());
        assert!(checked_instructions(&format!("{}   ", "a".repeat(4_000))).is_ok());
        assert!(checked_instructions("bell\u{7}").is_err());
        assert!(checked_instructions("tab\tand\nline").is_ok());
    }
}
