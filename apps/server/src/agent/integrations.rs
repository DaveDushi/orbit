//! Tools outside Orbit for the team agent: remote MCP servers and HTTP APIs that workspace owners
//! and admins set up. Orbit keeps the one shared credential of each (sealed with the app key) and
//! makes every external call itself (`super::outbound`); the sidecar only names a tool and its
//! arguments with the driver's agent session.
//!
//! - `/api/v1/workspaces/{id}/agent-integrations…`: set up, test and remove (owners and admins,
//!   normal sessions only; `agent::scope` keeps agent sessions out).
//! - `/api/v1/workspaces/{id}/agent-tools…`: the tools the driver's role may use, and a call
//!   (agent sessions of the driver of a running thread only).
//!
//! A write (an MCP tool that is not read-only, an HTTP method other than GET or HEAD) runs only
//! after the driver answered "yes" in the thread: see [`approve`]. Design and grilling:
//! `.ai/agent-design.md`, "Integrations". Like the other agent routes, these are not in the
//! OpenAPI contract.

use axum::extract::{Extension, Path, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use orbit_domain::{Permission, WorkspaceRole};
use orbit_platform::{Database, Id, RequestId, Row, TimestampMillis};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use url::Url;

use super::AgentState;
use super::outbound::{Credential, OutboundError, check_url, join_api_url};
use crate::audit::{self, AuditOutcome};
use crate::repositories::membership;
use crate::secret_box::{decrypt_secret, encrypt_secret};
use crate::task_routes::{ApiError, authenticate_session};

type RequestIdExtension = Option<Extension<RequestId>>;

const ROLES: [&str; 3] = ["owner", "admin", "member"];
const METHODS: [&str; 6] = ["GET", "HEAD", "POST", "PUT", "PATCH", "DELETE"];
/// Separates the integration's slug from the tool in the agent's tool names.
pub(crate) const SEPARATOR: &str = "__";
/// The one tool of an HTTP API integration.
const API_TOOL: &str = "api_request";
/// How long the driver has to answer "yes", and how long that answer counts.
pub(crate) const APPROVAL_LIFETIME_MS: i64 = 30 * 60 * 1_000;
const MAX_INTEGRATIONS: i64 = 50;
const MAX_ARGUMENT_BYTES: usize = 64 * 1024;

pub fn router(state: AgentState) -> Router {
    let admin = "/api/v1/workspaces/{workspace_id}/agent-integrations";
    let tools = "/api/v1/workspaces/{workspace_id}/agent-tools";
    Router::new()
        .route(admin, get(list).post(create))
        .route(
            &format!("{admin}/{{integration_id}}"),
            patch(update).delete(remove),
        )
        .route(&format!("{admin}/{{integration_id}}/test"), post(test))
        .route(tools, get(agent_tools))
        .route(&format!("{tools}/call"), post(call))
        .with_state(state)
}

// ---------------------------------------------------------------------------------------------
// Errors.

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

fn not_found(uri: &Uri, request_id: &RequestIdExtension) -> ApiError {
    problem(
        StatusCode::NOT_FOUND,
        "not_found",
        "Not found.",
        uri,
        request_id,
    )
}

fn unavailable(uri: &Uri, request_id: &RequestIdExtension) -> impl Fn(sqlx::Error) -> ApiError {
    let uri = uri.clone();
    let request_id = request_id.clone();
    move |error| {
        tracing::warn!(%error, "agent integrations");
        problem(
            StatusCode::SERVICE_UNAVAILABLE,
            "unavailable",
            "Try again in a moment.",
            &uri,
            &request_id,
        )
    }
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
// Storage.

#[derive(Clone, Debug)]
struct Integration {
    id: String,
    slug: String,
    name: String,
    kind: String,
    url: String,
    auth_header: Option<String>,
    sealed_secret: Option<Vec<u8>>,
    allowed_roles: Vec<String>,
    enabled_tools: Vec<String>,
    allowed_methods: Vec<String>,
    /// MCP: the server's tools at the last test.
    tools: Option<Vec<Value>>,
    tools_checked_at: Option<i64>,
    created_at: i64,
    updated_at: i64,
}

const COLUMNS: &str = "id, slug, name, kind, url, auth_header, sealed_secret, allowed_roles, \
     enabled_tools, allowed_methods, tools, tools_checked_at, created_at, updated_at";

fn strings(json: &str) -> Vec<String> {
    serde_json::from_str(json).unwrap_or_default()
}

impl Integration {
    fn from_row(row: &sqlx::any::AnyRow) -> Self {
        Self {
            id: row.get("id"),
            slug: row.get("slug"),
            name: row.get("name"),
            kind: row.get("kind"),
            url: row.get("url"),
            auth_header: row.get("auth_header"),
            sealed_secret: row.get("sealed_secret"),
            allowed_roles: strings(&row.get::<String, _>("allowed_roles")),
            enabled_tools: strings(&row.get::<String, _>("enabled_tools")),
            allowed_methods: strings(&row.get::<String, _>("allowed_methods")),
            tools: row
                .get::<Option<String>, _>("tools")
                .and_then(|tools| serde_json::from_str(&tools).ok()),
            tools_checked_at: row.get("tools_checked_at"),
            created_at: row.get("created_at"),
            updated_at: row.get("updated_at"),
        }
    }

    fn allows(&self, role: WorkspaceRole) -> bool {
        self.allowed_roles
            .iter()
            .any(|allowed| allowed == role.as_str())
    }

    fn mcp_tool(&self, name: &str) -> Option<&Value> {
        self.tools
            .as_deref()?
            .iter()
            .find(|tool| tool.get("name").and_then(Value::as_str) == Some(name))
    }

    /// The open credential, only for the call that needs it.
    fn credential(&self, key: Option<&[u8; 32]>) -> Result<Option<Credential>, &'static str> {
        let (Some(name), Some(sealed)) = (&self.auth_header, &self.sealed_secret) else {
            return Ok(None);
        };
        let key = key.ok_or("The server has no ORBIT_APP_KEY to open the credential.")?;
        let value = decrypt_secret(key, sealed)
            .map_err(|()| "The saved credential cannot be opened. Enter it again.")?;
        Ok(Some(Credential {
            name: name.clone(),
            value,
        }))
    }
}

fn read_only(tool: &Value) -> bool {
    tool.pointer("/annotations/readOnlyHint")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

async fn load_all(database: &Database, workspace_id: Id) -> Result<Vec<Integration>, sqlx::Error> {
    let rows = sqlx::query(sqlx::AssertSqlSafe(format!(
        "SELECT {COLUMNS} FROM agent_integrations WHERE workspace_id = $1 ORDER BY lower(name), id"
    )))
    .bind(workspace_id.to_string())
    .fetch_all(database.pool())
    .await?;
    Ok(rows.iter().map(Integration::from_row).collect())
}

/// `column` is `id` or `slug`, never input.
async fn load_one(
    database: &Database,
    workspace_id: Id,
    column: &'static str,
    value: &str,
) -> Result<Option<Integration>, sqlx::Error> {
    let row = sqlx::query(sqlx::AssertSqlSafe(format!(
        "SELECT {COLUMNS} FROM agent_integrations WHERE workspace_id = $1 AND {column} = $2"
    )))
    .bind(workspace_id.to_string())
    .bind(value)
    .fetch_optional(database.pool())
    .await?;
    Ok(row.as_ref().map(Integration::from_row))
}

#[allow(clippy::too_many_arguments)]
async fn record(
    database: &Database,
    workspace_id: Id,
    actor: Id,
    action: &str,
    outcome: AuditOutcome,
    integration_id: &str,
    request_id: &RequestIdExtension,
    metadata: Value,
) {
    let result = async {
        let mut tx = database.immediate_transaction().await?;
        audit::record(
            &mut tx,
            workspace_id,
            Some(actor),
            action,
            outcome,
            "agent_integration",
            integration_id.parse().ok(),
            request_id
                .as_ref()
                .map_or("agent-integration", |Extension(id)| id.as_str()),
            metadata,
            TimestampMillis::now(),
        )
        .await?;
        tx.commit().await
    }
    .await;
    if let Err(error) = result {
        tracing::warn!(%error, action, "agent integration audit");
    }
}

// ---------------------------------------------------------------------------------------------
// What the admin sees. The secret never leaves Orbit: only whether one is set.

#[derive(Serialize)]
struct ToolView {
    name: String,
    title: Option<String>,
    description: String,
    read_only: bool,
    enabled: bool,
}

#[derive(Serialize)]
struct IntegrationView {
    id: String,
    slug: String,
    name: String,
    kind: String,
    url: String,
    auth_header: Option<String>,
    secret_set: bool,
    allowed_roles: Vec<String>,
    allowed_methods: Vec<String>,
    /// MCP: every tool of the server at the last test, with whether the agent may use it.
    tools: Option<Vec<ToolView>>,
    tools_checked_at: Option<i64>,
    created_at: i64,
    updated_at: i64,
}

impl From<&Integration> for IntegrationView {
    fn from(integration: &Integration) -> Self {
        Self {
            id: integration.id.clone(),
            slug: integration.slug.clone(),
            name: integration.name.clone(),
            kind: integration.kind.clone(),
            url: integration.url.clone(),
            auth_header: integration.auth_header.clone(),
            secret_set: integration.sealed_secret.is_some(),
            allowed_roles: integration.allowed_roles.clone(),
            allowed_methods: integration.allowed_methods.clone(),
            tools: integration.tools.as_ref().map(|tools| {
                tools
                    .iter()
                    .filter_map(|tool| {
                        let name = tool.get("name")?.as_str()?.to_owned();
                        Some(ToolView {
                            enabled: integration.enabled_tools.contains(&name),
                            name,
                            title: tool.get("title").and_then(Value::as_str).map(str::to_owned),
                            description: tool
                                .get("description")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_owned(),
                            read_only: read_only(tool),
                        })
                    })
                    .collect()
            }),
            tools_checked_at: integration.tools_checked_at,
            created_at: integration.created_at,
            updated_at: integration.updated_at,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Admin routes.

/// The caller as an owner or admin of the workspace, with a normal (not an agent) session.
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
    if is_agent_session(state.database(), session.id, None)
        .await
        .map_err(unavailable(uri, request_id))?
    {
        return Err(problem(
            StatusCode::FORBIDDEN,
            "agent_scope",
            "The agent cannot use this route.",
            uri,
            request_id,
        ));
    }
    let workspace_id: Id = workspace_id
        .parse()
        .map_err(|_| not_found(uri, request_id))?;
    let actor = membership::actor(state.database().pool(), workspace_id, session.user.id)
        .await
        .map_err(unavailable(uri, request_id))?
        .ok_or_else(|| not_found(uri, request_id))?;
    if !actor.can(Permission::IntegrationsManage) {
        return Err(problem(
            StatusCode::FORBIDDEN,
            "forbidden",
            "Only workspace owners and admins can set up agent tools.",
            uri,
            request_id,
        ));
    }
    Ok((session.user.id, workspace_id))
}

async fn is_agent_session(
    database: &Database,
    session_id: Id,
    workspace_id: Option<Id>,
) -> Result<bool, sqlx::Error> {
    let found = sqlx::query_scalar::<_, String>(
        "SELECT workspace_id FROM agent_sessions WHERE session_id = $1",
    )
    .bind(session_id.to_string())
    .fetch_optional(database.pool())
    .await?;
    Ok(match (found, workspace_id) {
        (Some(found), Some(wanted)) => found == wanted.to_string(),
        (found, None) => found.is_some(),
        (None, Some(_)) => false,
    })
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct Input {
    name: Option<String>,
    slug: Option<String>,
    kind: Option<String>,
    url: Option<String>,
    /// `""` removes the credential.
    auth_header: Option<String>,
    /// Absent keeps the saved secret, `""` removes it.
    secret: Option<String>,
    allowed_roles: Option<Vec<String>>,
    enabled_tools: Option<Vec<String>>,
    allowed_methods: Option<Vec<String>>,
}

fn slug_from(name: &str) -> String {
    let mut slug = String::new();
    for character in name.chars() {
        let character = character.to_ascii_lowercase();
        if character.is_ascii_alphanumeric() {
            slug.push(character);
        } else if !slug.is_empty() && !slug.ends_with('_') {
            slug.push('_');
        }
    }
    let slug = slug.trim_end_matches('_');
    let slug: String = slug.chars().take(32).collect();
    slug.trim_end_matches('_').to_owned()
}

/// `[a-z][a-z0-9_]{0,31}` without `__`, which separates the slug from the tool name.
pub(crate) fn valid_slug(slug: &str) -> bool {
    (1..=32).contains(&slug.len())
        && slug.starts_with(|character: char| character.is_ascii_lowercase())
        && slug.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_'
        })
        && !slug.contains(SEPARATOR)
        && !slug.ends_with('_')
}

/// Header names that would change how the request is sent, not who sends it.
fn valid_auth_header(name: &str) -> bool {
    reqwest::header::HeaderName::from_bytes(name.as_bytes()).is_ok()
        && ![
            "host",
            "content-length",
            "content-type",
            "transfer-encoding",
            "connection",
            "accept",
            "mcp-session-id",
            "mcp-protocol-version",
        ]
        .contains(&name.to_ascii_lowercase().as_str())
}

fn subset(values: &[String], allowed: &[&str]) -> Option<Vec<String>> {
    let mut result: Vec<String> = Vec::new();
    for value in values {
        let value = value.trim();
        let known = allowed
            .iter()
            .find(|candidate| candidate.eq_ignore_ascii_case(value))?;
        if !result.iter().any(|seen| seen == known) {
            result.push((*known).to_owned());
        }
    }
    Some(result)
}

/// The integration after `input`, checked. `current` is `None` on create.
fn apply(
    state: &AgentState,
    current: Option<&Integration>,
    input: Input,
    uri: &Uri,
    request_id: &RequestIdExtension,
) -> Result<Integration, ApiError> {
    let now = TimestampMillis::now().as_millis();
    let mut next = current.cloned().unwrap_or_else(|| Integration {
        id: Id::new_v7().to_string(),
        slug: String::new(),
        name: String::new(),
        kind: String::new(),
        url: String::new(),
        auth_header: None,
        sealed_secret: None,
        allowed_roles: ROLES.iter().map(|role| (*role).to_owned()).collect(),
        enabled_tools: Vec::new(),
        allowed_methods: vec!["GET".to_owned()],
        tools: None,
        tools_checked_at: None,
        created_at: now,
        updated_at: now,
    });
    next.updated_at = now;
    if current.is_none() {
        let kind = input.kind.as_deref().unwrap_or_default();
        if !matches!(kind, "mcp" | "http") {
            return Err(invalid("Choose the kind: mcp or http.", uri, request_id));
        }
        next.kind = kind.to_owned();
    } else if input.kind.as_deref().is_some_and(|kind| kind != next.kind) {
        return Err(invalid(
            "The kind of an integration cannot change.",
            uri,
            request_id,
        ));
    }
    if let Some(name) = input.name {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 80 {
            return Err(invalid(
                "The name needs 1 to 80 characters.",
                uri,
                request_id,
            ));
        }
        next.name = name.to_owned();
    }
    if next.name.is_empty() {
        return Err(invalid("Enter a name.", uri, request_id));
    }
    if let Some(slug) = input.slug {
        next.slug = slug.trim().to_owned();
    } else if current.is_none() {
        next.slug = slug_from(&next.name);
    }
    if !valid_slug(&next.slug) {
        return Err(invalid(
            "The tool prefix needs 1 to 32 characters: a lowercase letter first, then lowercase letters, digits or single underscores.",
            uri,
            request_id,
        ));
    }
    if let Some(raw) = input.url {
        let url = check_url(&raw, state.outbound.test_hosts()).map_err(|error| match error {
            OutboundError::Refused(detail) => invalid(detail, uri, request_id),
            _ => invalid("The URL is not valid.", uri, request_id),
        })?;
        if raw.len() > 2_048 {
            return Err(invalid("The URL is too long.", uri, request_id));
        }
        let url = url.to_string();
        if current.is_some_and(|current| current.url != url) && next.kind == "mcp" {
            // Another server: its tools are unknown until the next test.
            next.tools = None;
            next.tools_checked_at = None;
            next.enabled_tools.clear();
        }
        next.url = url;
    }
    if next.url.is_empty() {
        return Err(invalid("Enter the URL.", uri, request_id));
    }
    if let Some(header) = input.auth_header {
        let header = header.trim();
        if header.is_empty() {
            next.auth_header = None;
            next.sealed_secret = None;
        } else if valid_auth_header(header) {
            next.auth_header = Some(header.to_owned());
        } else {
            return Err(invalid(
                "The credential header name is not valid.",
                uri,
                request_id,
            ));
        }
    }
    if let Some(secret) = input.secret {
        if secret.is_empty() {
            next.sealed_secret = None;
        } else {
            if secret.len() > 8_192 || reqwest::header::HeaderValue::from_str(&secret).is_err() {
                return Err(invalid(
                    "The secret must be one line of at most 8 KiB.",
                    uri,
                    request_id,
                ));
            }
            let key = state.app_key.as_ref().ok_or_else(|| {
                problem(
                    StatusCode::CONFLICT,
                    "app_key_missing",
                    "The server has no ORBIT_APP_KEY, so it cannot keep a secret.",
                    uri,
                    request_id,
                )
            })?;
            next.sealed_secret = Some(
                encrypt_secret(key, &secret)
                    .map_err(|()| invalid("The secret cannot be sealed.", uri, request_id))?,
            );
            if next.auth_header.is_none() {
                next.auth_header = Some("Authorization".to_owned());
            }
        }
    }
    if next.sealed_secret.is_none() {
        next.auth_header = None;
    }
    if let Some(roles) = input.allowed_roles {
        next.allowed_roles = subset(&roles, &ROLES)
            .ok_or_else(|| invalid("Roles are owner, admin and member.", uri, request_id))?;
    }
    if let Some(methods) = input.allowed_methods {
        if next.kind != "http" {
            return Err(invalid("Only HTTP APIs have methods.", uri, request_id));
        }
        let methods = subset(&methods, &METHODS).ok_or_else(|| {
            invalid(
                "Methods are GET, HEAD, POST, PUT, PATCH and DELETE.",
                uri,
                request_id,
            )
        })?;
        if methods.is_empty() {
            return Err(invalid("Allow at least one method.", uri, request_id));
        }
        next.allowed_methods = methods;
    }
    if let Some(enabled) = input.enabled_tools {
        if next.kind != "mcp" {
            return Err(invalid("Only MCP servers have tools.", uri, request_id));
        }
        if enabled.iter().any(|name| next.mcp_tool(name).is_none()) {
            return Err(invalid(
                "Turn on only tools the server listed at the last test.",
                uri,
                request_id,
            ));
        }
        next.enabled_tools = enabled;
        next.enabled_tools.dedup();
    }
    Ok(next)
}

async fn save(
    database: &Database,
    workspace_id: Id,
    user: Id,
    integration: &Integration,
    create: bool,
) -> Result<bool, sqlx::Error> {
    let mut tx = database.immediate_transaction().await?;
    if create {
        let count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM agent_integrations WHERE workspace_id = $1",
        )
        .bind(workspace_id.to_string())
        .fetch_one(&mut *tx)
        .await?;
        if count >= MAX_INTEGRATIONS {
            return Ok(false);
        }
    }
    let taken = sqlx::query_scalar::<_, String>(
        "SELECT id FROM agent_integrations WHERE workspace_id = $1 AND slug = $2 AND id <> $3",
    )
    .bind(workspace_id.to_string())
    .bind(&integration.slug)
    .bind(&integration.id)
    .fetch_optional(&mut *tx)
    .await?;
    if taken.is_some() {
        return Ok(false);
    }
    let json =
        |value: &Vec<String>| serde_json::to_string(value).unwrap_or_else(|_| "[]".to_owned());
    let tools = integration
        .tools
        .as_ref()
        .map(|tools| Value::Array(tools.clone()).to_string());
    sqlx::query(
        "INSERT INTO agent_integrations (id, workspace_id, slug, name, kind, url, auth_header, \
         sealed_secret, allowed_roles, enabled_tools, allowed_methods, tools, tools_checked_at, \
         created_by, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16) \
         ON CONFLICT (id) DO UPDATE SET slug = excluded.slug, name = excluded.name, \
         url = excluded.url, auth_header = excluded.auth_header, \
         sealed_secret = excluded.sealed_secret, allowed_roles = excluded.allowed_roles, \
         enabled_tools = excluded.enabled_tools, allowed_methods = excluded.allowed_methods, \
         tools = excluded.tools, tools_checked_at = excluded.tools_checked_at, \
         updated_at = excluded.updated_at",
    )
    .bind(&integration.id)
    .bind(workspace_id.to_string())
    .bind(&integration.slug)
    .bind(&integration.name)
    .bind(&integration.kind)
    .bind(&integration.url)
    .bind(&integration.auth_header)
    .bind(&integration.sealed_secret)
    .bind(json(&integration.allowed_roles))
    .bind(json(&integration.enabled_tools))
    .bind(json(&integration.allowed_methods))
    .bind(tools)
    .bind(integration.tools_checked_at)
    .bind(user.to_string())
    .bind(integration.created_at)
    .bind(integration.updated_at)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(true)
}

fn conflict(uri: &Uri, request_id: &RequestIdExtension) -> ApiError {
    problem(
        StatusCode::CONFLICT,
        "agent_integration_conflict",
        "Another integration uses this tool prefix, or the workspace has 50 integrations.",
        uri,
        request_id,
    )
}

async fn list(
    State(state): State<AgentState>,
    Path(workspace_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
) -> Result<Json<Value>, ApiError> {
    let (_, workspace_id) = manager(&state, &headers, &workspace_id, &uri, &request_id).await?;
    let all = load_all(state.database(), workspace_id)
        .await
        .map_err(unavailable(&uri, &request_id))?;
    let items: Vec<IntegrationView> = all.iter().map(IntegrationView::from).collect();
    Ok(Json(json!({
        "items": items,
        "can_keep_secrets": state.app_key.is_some(),
    })))
}

async fn create(
    State(state): State<AgentState>,
    Path(workspace_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
    Json(input): Json<Input>,
) -> Result<(StatusCode, Json<IntegrationView>), ApiError> {
    let (user, workspace_id) = manager(&state, &headers, &workspace_id, &uri, &request_id).await?;
    let integration = apply(&state, None, input, &uri, &request_id)?;
    if !save(state.database(), workspace_id, user, &integration, true)
        .await
        .map_err(unavailable(&uri, &request_id))?
    {
        return Err(conflict(&uri, &request_id));
    }
    record(
        state.database(),
        workspace_id,
        user,
        "agent_integration.create",
        AuditOutcome::Success,
        &integration.id,
        &request_id,
        json!({ "slug": integration.slug, "kind": integration.kind }),
    )
    .await;
    Ok((
        StatusCode::CREATED,
        Json(IntegrationView::from(&integration)),
    ))
}

async fn update(
    State(state): State<AgentState>,
    Path((workspace_id, integration_id)): Path<(String, String)>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
    Json(input): Json<Input>,
) -> Result<Json<IntegrationView>, ApiError> {
    let (user, workspace_id) = manager(&state, &headers, &workspace_id, &uri, &request_id).await?;
    let current = load_one(state.database(), workspace_id, "id", &integration_id)
        .await
        .map_err(unavailable(&uri, &request_id))?
        .ok_or_else(|| not_found(&uri, &request_id))?;
    let secret_changed = input.secret.is_some();
    let integration = apply(&state, Some(&current), input, &uri, &request_id)?;
    if !save(state.database(), workspace_id, user, &integration, false)
        .await
        .map_err(unavailable(&uri, &request_id))?
    {
        return Err(conflict(&uri, &request_id));
    }
    record(
        state.database(),
        workspace_id,
        user,
        "agent_integration.update",
        AuditOutcome::Success,
        &integration.id,
        &request_id,
        json!({ "slug": integration.slug, "secret_changed": secret_changed }),
    )
    .await;
    Ok(Json(IntegrationView::from(&integration)))
}

async fn remove(
    State(state): State<AgentState>,
    Path((workspace_id, integration_id)): Path<(String, String)>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
) -> Result<StatusCode, ApiError> {
    let (user, workspace_id) = manager(&state, &headers, &workspace_id, &uri, &request_id).await?;
    let deleted = sqlx::query("DELETE FROM agent_integrations WHERE id = $1 AND workspace_id = $2")
        .bind(&integration_id)
        .bind(workspace_id.to_string())
        .execute(state.database().pool())
        .await
        .map_err(unavailable(&uri, &request_id))?;
    if deleted.rows_affected() == 0 {
        return Err(not_found(&uri, &request_id));
    }
    record(
        state.database(),
        workspace_id,
        user,
        "agent_integration.delete",
        AuditOutcome::Success,
        &integration_id,
        &request_id,
        json!({}),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

/// The parts of an MCP tool Orbit keeps.
fn keep_tool(tool: &Value) -> Option<Value> {
    let name = tool.get("name")?.as_str()?;
    if name.is_empty() || name.len() > 128 {
        return None;
    }
    let description: String = tool
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .chars()
        .take(2_000)
        .collect();
    let mut kept = json!({
        "name": name,
        "description": description,
        "inputSchema": tool.get("inputSchema").cloned().unwrap_or_else(|| json!({ "type": "object" })),
    });
    if let Some(title) = tool.get("title").and_then(Value::as_str) {
        kept["title"] = json!(title.chars().take(200).collect::<String>());
    }
    if let Some(annotations) = tool.get("annotations") {
        kept["annotations"] = annotations.clone();
    }
    Some(kept)
}

/// MCP: reads the tools and keeps them (the first time, read-only tools are turned on). HTTP: a
/// GET of the base URL.
async fn test(
    State(state): State<AgentState>,
    Path((workspace_id, integration_id)): Path<(String, String)>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
) -> Result<Json<Value>, ApiError> {
    let (user, workspace_id) = manager(&state, &headers, &workspace_id, &uri, &request_id).await?;
    let mut integration = load_one(state.database(), workspace_id, "id", &integration_id)
        .await
        .map_err(unavailable(&uri, &request_id))?
        .ok_or_else(|| not_found(&uri, &request_id))?;
    let credential = match integration.credential(state.app_key.as_ref()) {
        Ok(credential) => credential,
        Err(detail) => {
            return Ok(Json(
                json!({ "ok": false, "detail": detail, "integration": IntegrationView::from(&integration) }),
            ));
        }
    };
    let url = Url::parse(&integration.url)
        .map_err(|_| invalid("The URL is not valid.", &uri, &request_id))?;
    let outcome: Result<String, OutboundError> = if integration.kind == "mcp" {
        match state.outbound.mcp_tools(&url, credential.as_ref()).await {
            Ok(tools) => {
                let tools: Vec<Value> = tools.iter().filter_map(keep_tool).collect();
                if integration.tools.is_none() {
                    integration.enabled_tools = tools
                        .iter()
                        .filter(|tool| read_only(tool))
                        .filter_map(|tool| tool["name"].as_str().map(str::to_owned))
                        .collect();
                } else {
                    integration.enabled_tools.retain(|name| {
                        tools.iter().any(|tool| tool["name"].as_str() == Some(name))
                    });
                }
                let count = tools.len();
                integration.tools = Some(tools);
                integration.tools_checked_at = Some(TimestampMillis::now().as_millis());
                save(state.database(), workspace_id, user, &integration, false)
                    .await
                    .map_err(unavailable(&uri, &request_id))?;
                Ok(format!("Connected. The server has {count} tools."))
            }
            Err(error) => Err(error),
        }
    } else {
        state
            .outbound
            .api_request(&url, reqwest::Method::GET, None, credential.as_ref())
            .await
            .and_then(|answer| {
                if answer.status < 400 {
                    Ok(format!(
                        "Connected. GET of the base URL answered HTTP {}.",
                        answer.status
                    ))
                } else {
                    Err(OutboundError::Status(answer.status))
                }
            })
    };
    let ok = outcome.is_ok();
    record(
        state.database(),
        workspace_id,
        user,
        "agent_integration.test",
        if ok {
            AuditOutcome::Success
        } else {
            AuditOutcome::Failure
        },
        &integration.id,
        &request_id,
        json!({ "slug": integration.slug }),
    )
    .await;
    let detail = outcome.unwrap_or_else(|error| capitalized(&error.to_string()));
    Ok(Json(json!({
        "ok": ok,
        "detail": detail,
        "integration": IntegrationView::from(&integration),
    })))
}

fn capitalized(text: &str) -> String {
    let mut characters = text.chars();
    characters.next().map_or_else(String::new, |first| {
        format!(
            "{}{}.",
            first.to_uppercase(),
            characters.as_str().trim_end_matches('.')
        )
    })
}

// ---------------------------------------------------------------------------------------------
// The agent's routes.

/// The caller as the driver of a run, with an agent session of this workspace.
async fn driver(
    state: &AgentState,
    headers: &HeaderMap,
    workspace_id: &str,
    uri: &Uri,
    request_id: &RequestIdExtension,
) -> Result<(Id, Id, WorkspaceRole), ApiError> {
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
    if !is_agent_session(state.database(), session.id, Some(workspace_id))
        .await
        .map_err(unavailable(uri, request_id))?
    {
        return Err(problem(
            StatusCode::FORBIDDEN,
            "agent_session_required",
            "Only the team agent uses these tools, in a run of its driver.",
            uri,
            request_id,
        ));
    }
    let actor = membership::actor(state.database().pool(), workspace_id, session.user.id)
        .await
        .map_err(unavailable(uri, request_id))?
        .ok_or_else(|| not_found(uri, request_id))?;
    Ok((session.user.id, workspace_id, actor.role))
}

#[derive(Serialize)]
struct AgentTool {
    /// `<slug>__<tool>`.
    name: String,
    integration: String,
    description: String,
    input_schema: Value,
    /// `never`, `always`, or `writes` (HTTP: methods other than GET and HEAD).
    approval: &'static str,
}

fn agent_tools_of(integration: &Integration) -> Vec<AgentTool> {
    if integration.kind == "http" {
        let writes = integration
            .allowed_methods
            .iter()
            .any(|method| !matches!(method.as_str(), "GET" | "HEAD"));
        return vec![AgentTool {
            name: format!("{}{SEPARATOR}{API_TOOL}", integration.slug),
            integration: integration.name.clone(),
            description: format!(
                "Send one request to the {} HTTP API. The path is below its base URL {}.",
                integration.name, integration.url
            ),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "method": { "type": "string", "enum": integration.allowed_methods },
                    "path": { "type": "string", "description": "Starts with /, below the base URL" },
                    "query": { "type": "object", "additionalProperties": { "type": ["string", "number", "boolean"] } },
                    "body": { "description": "JSON body" },
                },
                "required": ["method", "path"],
            }),
            approval: if writes { "writes" } else { "never" },
        }];
    }
    integration
        .tools
        .iter()
        .flatten()
        .filter_map(|tool| {
            let name = tool.get("name")?.as_str()?;
            if !integration
                .enabled_tools
                .iter()
                .any(|enabled| enabled == name)
            {
                return None;
            }
            Some(AgentTool {
                name: format!("{}{SEPARATOR}{name}", integration.slug),
                integration: integration.name.clone(),
                description: tool
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                input_schema: tool.get("inputSchema").cloned().unwrap_or(Value::Null),
                approval: if read_only(tool) { "never" } else { "always" },
            })
        })
        .collect()
}

async fn agent_tools(
    State(state): State<AgentState>,
    Path(workspace_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
) -> Result<Json<Value>, ApiError> {
    let (_, workspace_id, role) =
        driver(&state, &headers, &workspace_id, &uri, &request_id).await?;
    let all = load_all(state.database(), workspace_id)
        .await
        .map_err(unavailable(&uri, &request_id))?;
    let tools: Vec<AgentTool> = all
        .iter()
        .filter(|integration| integration.allows(role))
        .flat_map(agent_tools_of)
        .collect();
    Ok(Json(json!({ "tools": tools })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CallBody {
    /// `<slug>__<tool>`.
    tool: String,
    #[serde(default)]
    arguments: Value,
    /// The thread of the run: approvals belong to it.
    conversation_id: String,
    root_message_id: Option<String>,
}

/// What one call will do, checked against the integration's settings.
enum Action {
    Mcp {
        tool: String,
        arguments: Value,
        write: bool,
    },
    Api {
        method: reqwest::Method,
        url: Url,
        path: String,
        body: Option<Value>,
    },
}

/// JSON with object keys in sorted order, so equal arguments always hash the same.
pub(crate) fn canonical(value: &Value) -> String {
    fn sorted(value: &Value) -> Value {
        match value {
            Value::Object(map) => {
                let mut keys: Vec<&String> = map.keys().collect();
                keys.sort();
                let mut result = Map::new();
                for key in keys {
                    result.insert(key.clone(), sorted(&map[key]));
                }
                Value::Object(result)
            }
            Value::Array(items) => Value::Array(items.iter().map(sorted).collect()),
            other => other.clone(),
        }
    }
    sorted(value).to_string()
}

fn hash(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn action_of(
    integration: &Integration,
    tool: &str,
    arguments: &Value,
) -> Result<Action, &'static str> {
    if integration.kind == "mcp" {
        let found = integration
            .mcp_tool(tool)
            .filter(|_| integration.enabled_tools.iter().any(|name| name == tool))
            .ok_or("This tool is not turned on for the agent.")?;
        let arguments = if arguments.is_null() {
            json!({})
        } else {
            arguments.clone()
        };
        if !arguments.is_object() {
            return Err("The arguments must be a JSON object.");
        }
        return Ok(Action::Mcp {
            tool: tool.to_owned(),
            arguments,
            write: !read_only(found),
        });
    }
    if tool != API_TOOL {
        return Err("This API has one tool: api_request.");
    }
    let method = arguments
        .get("method")
        .and_then(Value::as_str)
        .map(str::to_ascii_uppercase)
        .ok_or("Give the method.")?;
    if !integration.allowed_methods.contains(&method) {
        return Err("This method is not allowed for this API.");
    }
    let path = arguments
        .get("path")
        .and_then(Value::as_str)
        .ok_or("Give the path, starting with /.")?;
    let mut query = Vec::new();
    match arguments.get("query") {
        None | Some(Value::Null) => {}
        Some(Value::Object(pairs)) => {
            for (key, value) in pairs {
                let value = match value {
                    Value::String(text) => text.clone(),
                    Value::Number(_) | Value::Bool(_) => value.to_string(),
                    _ => return Err("Query values must be strings, numbers or booleans."),
                };
                query.push((key.clone(), value));
            }
        }
        Some(_) => return Err("The query must be an object."),
    }
    let base = Url::parse(&integration.url).map_err(|_| "The API's base URL is not valid.")?;
    let url = join_api_url(&base, path, &query)
        .map_err(|_| "The path must stay below the API's base URL.")?;
    let body = arguments
        .get("body")
        .filter(|body| !body.is_null())
        .cloned();
    if body.is_some() && matches!(method.as_str(), "GET" | "HEAD") {
        return Err("GET and HEAD requests have no body.");
    }
    Ok(Action::Api {
        method: reqwest::Method::from_bytes(method.as_bytes()).map_err(|_| "Unknown method.")?,
        url,
        path: path.to_owned(),
        body,
    })
}

impl Action {
    fn write(&self) -> bool {
        match self {
            Self::Mcp { write, .. } => *write,
            Self::Api { method, .. } => {
                !matches!(*method, reqwest::Method::GET | reqwest::Method::HEAD)
            }
        }
    }

    /// The exact action, as the driver approves it.
    fn describe(&self, integration: &Integration) -> String {
        let text = match self {
            Self::Mcp {
                tool, arguments, ..
            } => format!(
                "{} → {tool} with arguments {}",
                integration.name,
                canonical(arguments)
            ),
            Self::Api {
                method, url, body, ..
            } => {
                let body = body.as_ref().map_or_else(String::new, |body| {
                    format!(" with body {}", canonical(body))
                });
                format!("{}: {method} {url}{body}", integration.name)
            }
        };
        if text.chars().count() > 3_000 {
            format!("{}…", text.chars().take(3_000).collect::<String>())
        } else {
            text
        }
    }
}

/// The answer to "may this write run now?".
pub(crate) enum Approval {
    /// The driver said yes; the approval is now used.
    Granted,
    /// Ask the driver (again).
    Needed { id: String, expires_at: i64 },
}

/// Whether `body` is a "yes": the first word after any mentions is yes or approve(d).
pub(crate) fn is_yes(body: &str) -> bool {
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
    matches!(word.as_str(), "yes" | "approve" | "approved")
}

/// The approval of one write: bound to the driver, the thread, the integration, the tool and the
/// hash of the arguments. It is granted (and used up, once) only when the driver posted a "yes"
/// in that thread after it was made, and before it expired. Else a new one is made, or the open
/// one is returned, and the driver is asked.
pub(crate) async fn approve(
    database: &Database,
    integration_id: &str,
    user: Id,
    (conversation_id, root_message_id): (&str, Option<&str>),
    tool: &str,
    arguments_hash: &str,
) -> Result<Approval, sqlx::Error> {
    let now = TimestampMillis::now().as_millis();
    let mut tx = database.immediate_transaction().await?;
    let open = sqlx::query(
        "SELECT id, created_at, expires_at FROM agent_integration_approvals \
         WHERE integration_id = $1 AND user_id = $2 AND conversation_id = $3 \
         AND root_message_id IS NOT DISTINCT FROM $4 AND tool = $5 AND arguments_hash = $6 \
         AND used_at IS NULL AND expires_at > $7 ORDER BY created_at DESC LIMIT 1",
    )
    .bind(integration_id)
    .bind(user.to_string())
    .bind(conversation_id)
    .bind(root_message_id)
    .bind(tool)
    .bind(arguments_hash)
    .bind(now)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some(open) = open {
        let id: String = open.get("id");
        let created_at: i64 = open.get("created_at");
        let replies = sqlx::query_scalar::<_, String>(
            "SELECT body FROM chat_messages WHERE conversation_id = $1 \
             AND thread_root_id IS NOT DISTINCT FROM $2 AND author_id = $3 \
             AND kind = 'message' AND deleted_at IS NULL AND created_at >= $4 \
             ORDER BY created_at LIMIT 50",
        )
        .bind(conversation_id)
        .bind(root_message_id)
        .bind(user.to_string())
        .bind(created_at)
        .fetch_all(&mut *tx)
        .await?;
        if replies.iter().any(|body| is_yes(body)) {
            sqlx::query(
                "UPDATE agent_integration_approvals SET used_at = $1 WHERE id = $2 AND used_at IS NULL",
            )
            .bind(now)
            .bind(&id)
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
            return Ok(Approval::Granted);
        }
        tx.commit().await?;
        return Ok(Approval::Needed {
            id,
            expires_at: open.get("expires_at"),
        });
    }
    let id = Id::new_v7().to_string();
    let expires_at = now + APPROVAL_LIFETIME_MS;
    sqlx::query(
        "INSERT INTO agent_integration_approvals (id, integration_id, user_id, conversation_id, \
         root_message_id, tool, arguments_hash, created_at, expires_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
    )
    .bind(&id)
    .bind(integration_id)
    .bind(user.to_string())
    .bind(conversation_id)
    .bind(root_message_id)
    .bind(tool)
    .bind(arguments_hash)
    .bind(now)
    .bind(expires_at)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Approval::Needed { id, expires_at })
}

async fn call(
    State(state): State<AgentState>,
    Path(workspace_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
    Json(body): Json<CallBody>,
) -> Result<Json<Value>, ApiError> {
    let (user, workspace_id, role) =
        driver(&state, &headers, &workspace_id, &uri, &request_id).await?;
    let database = state.database();
    // The run's thread: the caller drives it now.
    let running = sqlx::query_scalar::<_, String>(
        "SELECT thread_key FROM agent_threads WHERE workspace_id = $1 AND conversation_id = $2 \
         AND root_message_id IS NOT DISTINCT FROM $3 AND driver_user_id = $4 AND status = 'running'",
    )
    .bind(workspace_id.to_string())
    .bind(&body.conversation_id)
    .bind(body.root_message_id.as_deref())
    .bind(user.to_string())
    .fetch_optional(database.pool())
    .await
    .map_err(unavailable(&uri, &request_id))?;
    if running.is_none() {
        return Err(problem(
            StatusCode::FORBIDDEN,
            "agent_run_required",
            "Tools run only in a thread whose current run the caller drives.",
            &uri,
            &request_id,
        ));
    }
    let (slug, tool) = body
        .tool
        .split_once(SEPARATOR)
        .ok_or_else(|| not_found(&uri, &request_id))?;
    let integration = load_one(database, workspace_id, "slug", slug)
        .await
        .map_err(unavailable(&uri, &request_id))?
        .ok_or_else(|| not_found(&uri, &request_id))?;
    if !integration.allows(role) {
        return Err(problem(
            StatusCode::FORBIDDEN,
            "agent_integration_role",
            "The driver's role may not use this integration.",
            &uri,
            &request_id,
        ));
    }
    if body.arguments.to_string().len() > MAX_ARGUMENT_BYTES {
        return Err(invalid(
            "The arguments are larger than 64 KiB.",
            &uri,
            &request_id,
        ));
    }
    let action = match action_of(&integration, tool, &body.arguments) {
        Ok(action) => action,
        Err(detail) => {
            return Ok(Json(json!({ "status": "refused", "detail": detail })));
        }
    };
    let describe = action.describe(&integration);
    if action.write() {
        let arguments_hash = hash(&canonical(&body.arguments));
        let approval = approve(
            database,
            &integration.id,
            user,
            (&body.conversation_id, body.root_message_id.as_deref()),
            tool,
            &arguments_hash,
        )
        .await
        .map_err(unavailable(&uri, &request_id))?;
        if let Approval::Needed { id, expires_at } = approval {
            return Ok(Json(json!({
                "status": "approval_required",
                "approval_id": id,
                "action": describe,
                "expires_at": expires_at,
            })));
        }
    }
    let credential = integration
        .credential(state.app_key.as_ref())
        .map_err(|detail| {
            problem(
                StatusCode::CONFLICT,
                "agent_integration_credential",
                detail,
                &uri,
                &request_id,
            )
        })?;
    let (outcome, mut metadata) = match &action {
        Action::Mcp {
            tool, arguments, ..
        } => {
            let url = Url::parse(&integration.url).map_err(|_| not_found(&uri, &request_id))?;
            let result = state
                .outbound
                .mcp_call(&url, credential.as_ref(), tool, arguments)
                .await
                .map(|result| json!({ "status": "ok", "result": result }));
            (result, json!({ "tool": tool }))
        }
        Action::Api {
            method,
            url,
            path,
            body: payload,
        } => {
            let result = state
                .outbound
                .api_request(url, method.clone(), payload.as_ref(), credential.as_ref())
                .await;
            let status = result.as_ref().ok().map(|answer| answer.status);
            (
                result.map(|answer| json!({ "status": "ok", "response": answer })),
                json!({ "method": method.as_str(), "path": path, "status": status }),
            )
        }
    };
    metadata["integration"] = json!(integration.slug);
    metadata["approved"] = json!(action.write());
    if let Err(error) = &outcome {
        metadata["error"] = json!(error.to_string().chars().take(200).collect::<String>());
    }
    record(
        database,
        workspace_id,
        user,
        "agent_integration.call",
        if outcome.is_ok() {
            AuditOutcome::Success
        } else {
            AuditOutcome::Failure
        },
        &integration.id,
        &request_id,
        metadata,
    )
    .await;
    Ok(Json(outcome.unwrap_or_else(
        |error| json!({ "status": "failed", "detail": capitalized(&error.to_string()) }),
    )))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{canonical, is_yes, slug_from, valid_slug};

    #[test]
    fn only_a_clear_yes_approves() {
        for yes in [
            "yes",
            "Yes!",
            "  YES please",
            "<@0199c3a0-0000-7000-8000-000000000001> yes",
            "approve",
            "Approved.",
        ] {
            assert!(is_yes(yes), "{yes}");
        }
        for no in [
            "no",
            "yesterday",
            "y",
            "ok",
            "not yes",
            "",
            "<@x>",
            "sure, yes",
        ] {
            assert!(!is_yes(no), "{no}");
        }
    }

    #[test]
    fn equal_arguments_hash_the_same_in_any_key_order() {
        assert_eq!(
            canonical(&json!({ "b": 1, "a": { "y": [1, { "d": 2, "c": 3 }], "x": null } })),
            canonical(&json!({ "a": { "x": null, "y": [1, { "c": 3, "d": 2 }] }, "b": 1 }))
        );
        assert_ne!(canonical(&json!({ "a": 1 })), canonical(&json!({ "a": 2 })));
    }

    #[test]
    fn slugs_name_tools_without_the_separator() {
        assert_eq!(slug_from("Cloudflare DNS"), "cloudflare_dns");
        assert_eq!(slug_from("  Shopify -- Admin  "), "shopify_admin");
        assert!(valid_slug("cloudflare"));
        assert!(valid_slug("a1_b2"));
        for bad in [
            "",
            "1abc",
            "a__b",
            "Abc",
            "a-b",
            "abc_",
            "a".repeat(33).as_str(),
        ] {
            assert!(!valid_slug(bad), "{bad}");
        }
    }
}
