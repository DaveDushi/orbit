//! Tools outside Orbit for the team agent: remote MCP servers and HTTP APIs that workspace owners
//! and admins set up. Orbit keeps the one shared credential of each (sealed with the app key) and
//! makes every external call itself (`super::outbound`); the sidecar only names a tool and its
//! arguments with the driver's agent session.
//!
//! - `/api/v1/workspaces/{id}/agent-integrations…`: set up, test and remove (owners and admins,
//!   normal sessions only; `agent::scope` keeps agent sessions out).
//! - `/api/v1/workspaces/{id}/agent-tools…`: the tools the driver's role and the run's agent (the
//!   ticks on its card, `profiles::IntegrationAccess`) may use, and a call (agent sessions of the
//!   driver of a running thread only).
//!
//! A write (an MCP tool that is not read-only, an HTTP method other than GET or HEAD) runs only
//! after the driver answered "yes" in the thread: see [`approve`]. Design and grilling:
//! `.ai/agent-design.md`, "Integrations". Like the other agent routes, these are not in the
//! OpenAPI contract.

use axum::extract::{Extension, Path, Query, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use orbit_domain::{Permission, WorkspaceRole};
use orbit_platform::{Database, Flag, Id, RequestId, Row, TimestampMillis};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use url::Url;

use super::AgentState;
use super::graphql;
use super::mcp_oauth;
use super::oauth::{AccessToken, OAuthClient};
use super::outbound::{ApiAnswer, Credential, Outbound, OutboundError, check_url, join_api_url};
use super::profiles::IntegrationAccess;
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
const DEFAULT_AUTH_HEADER: &str = "Authorization";
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
        .route(
            &format!("{admin}/{{integration_id}}/oauth"),
            post(connect_start).delete(disconnect),
        )
        .route(mcp_oauth::CALLBACK_PATH, get(connect_callback))
        .route(tools, get(agent_tools))
        .route(&format!("{tools}/call"), post(call))
        .with_state(state)
}

// ---------------------------------------------------------------------------------------------
// Errors.

pub(crate) fn problem(
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

pub(crate) fn unavailable(
    uri: &Uri,
    request_id: &RequestIdExtension,
) -> impl Fn(sqlx::Error) -> ApiError {
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

pub(crate) fn invalid(
    detail: &'static str,
    uri: &Uri,
    request_id: &RequestIdExtension,
) -> ApiError {
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
    /// OAuth client credentials: where to get the token, and the sealed [`OAuthClient`]. Apart
    /// from `sealed_secret`, so an older binary sends no credential after a rollback.
    oauth_token_url: Option<String>,
    oauth_sealed_client: Option<Vec<u8>>,
    /// HTTP: what Test does a GET of, below the base URL, without a leading `/`.
    test_path: Option<String>,
    /// HTTP: a POST to exactly this path (with a leading `/`) that [`graphql`] finds to be one
    /// query runs without approval, while `graphql_reads_enabled` is on.
    graphql_read_path: Option<String>,
    graphql_reads_enabled: bool,
    allowed_roles: Vec<String>,
    enabled_tools: Vec<String>,
    allowed_methods: Vec<String>,
    /// MCP: the server's tools at the last test.
    tools: Option<Vec<Value>>,
    tools_checked_at: Option<i64>,
    created_at: i64,
    updated_at: i64,
    /// MCP: the shared OAuth sign-in (`mcp_oauth`). The sealed tokens are never loaded here.
    connection: Option<ConnectionView>,
    /// [`save`] removes the connection: the server or the credential changed.
    drop_connection: bool,
}

/// What admins see of an MCP server's sign-in.
#[derive(Clone, Debug, Serialize)]
struct ConnectionView {
    /// Who signed in, when the server told.
    account: Option<String>,
    connected_at: Option<i64>,
    /// When the grant ends and the admin must connect again; unknown when the server does not say.
    grant_expires_at: Option<i64>,
    /// A refresh was refused: only Reconnect helps.
    needs_reconnect: bool,
}

const COLUMNS: &str = "id, slug, name, kind, url, auth_header, sealed_secret, oauth_token_url, \
     oauth_sealed_client, test_path, graphql_read_path, graphql_reads_enabled, allowed_roles, enabled_tools, allowed_methods, tools, \
     tools_checked_at, created_at, updated_at, oauth_sealed_connection IS NOT NULL AS oauth_connected, \
     oauth_account, oauth_connected_at, oauth_grant_expires_at, oauth_invalid_at";

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
            oauth_token_url: row.get("oauth_token_url"),
            oauth_sealed_client: row.get("oauth_sealed_client"),
            test_path: row.get("test_path"),
            graphql_read_path: row.get("graphql_read_path"),
            graphql_reads_enabled: row.get("graphql_reads_enabled"),
            allowed_roles: strings(&row.get::<String, _>("allowed_roles")),
            enabled_tools: strings(&row.get::<String, _>("enabled_tools")),
            allowed_methods: strings(&row.get::<String, _>("allowed_methods")),
            tools: row
                .get::<Option<String>, _>("tools")
                .and_then(|tools| serde_json::from_str(&tools).ok()),
            tools_checked_at: row.get("tools_checked_at"),
            created_at: row.get("created_at"),
            updated_at: row.get("updated_at"),
            connection: row
                .get::<Flag, _>("oauth_connected")
                .0
                .then(|| ConnectionView {
                    account: row.get("oauth_account"),
                    connected_at: row.get("oauth_connected_at"),
                    grant_expires_at: row.get("oauth_grant_expires_at"),
                    needs_reconnect: row.get::<Option<i64>, _>("oauth_invalid_at").is_some(),
                }),
            drop_connection: false,
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

    fn uses_oauth(&self) -> bool {
        self.oauth_token_url.is_some() && self.oauth_sealed_client.is_some()
    }

    /// The token URL and the open client, only for the call that needs them.
    fn oauth(&self, key: Option<&[u8; 32]>) -> Result<Option<(Url, OAuthClient)>, &'static str> {
        let (Some(token_url), Some(sealed)) = (&self.oauth_token_url, &self.oauth_sealed_client)
        else {
            return Ok(None);
        };
        let key = key.ok_or("The server has no ORBIT_APP_KEY to open the credential.")?;
        let cannot_open = "The saved client cannot be opened. Enter it again.";
        let client = decrypt_secret(key, sealed).map_err(|()| cannot_open)?;
        let client = serde_json::from_str(&client).map_err(|_| cannot_open)?;
        let token_url = Url::parse(token_url).map_err(|_| "The token URL is not valid.")?;
        Ok(Some((token_url, client)))
    }

    /// The header of a call with `token`. An older binary may have cleared `auth_header`.
    fn token_credential(&self, token: &AccessToken) -> Credential {
        token.credential(self.auth_header.as_deref().unwrap_or(DEFAULT_AUTH_HEADER))
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
    /// `none`, `header` (a fixed credential), `oauth` (client credentials) or `connect` (an MCP
    /// server's OAuth sign-in).
    auth_type: &'static str,
    /// MCP: the sign-in, when an admin connected the server.
    connection: Option<ConnectionView>,
    oauth_token_url: Option<String>,
    test_path: Option<String>,
    graphql_read_path: Option<String>,
    graphql_reads_enabled: bool,
    /// The fixed secret or the OAuth client is saved.
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
            auth_type: if integration.connection.is_some() {
                "connect"
            } else if integration.uses_oauth() {
                "oauth"
            } else if integration.sealed_secret.is_some() {
                "header"
            } else {
                "none"
            },
            connection: integration.connection.clone(),
            oauth_token_url: integration.oauth_token_url.clone(),
            test_path: integration.test_path.clone(),
            graphql_read_path: integration.graphql_read_path.clone(),
            graphql_reads_enabled: integration.graphql_reads_enabled,
            secret_set: integration.sealed_secret.is_some() || integration.uses_oauth(),
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
    manager_session(state, headers, workspace_id, uri, request_id)
        .await
        .map(|(_, user, workspace_id)| (user, workspace_id))
}

/// As [`manager`], with the session's ID first.
async fn manager_session(
    state: &AgentState,
    headers: &HeaderMap,
    workspace_id: &str,
    uri: &Uri,
    request_id: &RequestIdExtension,
) -> Result<(Id, Id, Id), ApiError> {
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
    Ok((session.id, session.user.id, workspace_id))
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
    /// `""` removes the OAuth client. With a new value, the client ID and secret must come too.
    oauth_token_url: Option<String>,
    /// Never sent back: a change of the base or token URL needs both again.
    oauth_client_id: Option<String>,
    oauth_client_secret: Option<String>,
    /// `""` removes it.
    test_path: Option<String>,
    /// `""` removes it, which needs `graphql_reads_enabled` off.
    graphql_read_path: Option<String>,
    graphql_reads_enabled: Option<bool>,
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

struct OAuthInput {
    token_url: Option<String>,
    client_id: Option<String>,
    client_secret: Option<String>,
}

impl OAuthInput {
    fn is_given(&self) -> bool {
        self.token_url.as_deref().is_some_and(|url| !url.is_empty())
            || self.client_id.is_some()
            || self.client_secret.is_some()
    }
}

/// The OAuth part of [`apply`], after the base URL. An admin cannot read the client, so a change
/// of where it (or a token) goes needs a new client.
fn apply_oauth(
    state: &AgentState,
    current: Option<&Integration>,
    next: &mut Integration,
    input: OAuthInput,
    header_given: bool,
    uri: &Uri,
    request_id: &RequestIdExtension,
) -> Result<(), ApiError> {
    if input.token_url.as_deref() == Some("") {
        if input.client_id.is_some() || input.client_secret.is_some() {
            return Err(invalid(
                "An OAuth client needs a token URL.",
                uri,
                request_id,
            ));
        }
        next.oauth_token_url = None;
        next.oauth_sealed_client = None;
        return Ok(());
    }
    if input.is_given() && next.kind != "http" {
        return Err(invalid(
            "Only HTTP APIs have OAuth client credentials.",
            uri,
            request_id,
        ));
    }
    if let Some(raw) = &input.token_url {
        let url = check_url(raw, state.outbound.test_hosts()).map_err(|error| match error {
            OutboundError::Refused(detail) => invalid(detail, uri, request_id),
            _ => invalid("The token URL is not valid.", uri, request_id),
        })?;
        if raw.len() > 2_048 {
            return Err(invalid("The token URL is too long.", uri, request_id));
        }
        next.oauth_token_url = Some(url.to_string());
    }
    let new_client = match (input.client_id, input.client_secret) {
        (None, None) => None,
        (Some(id), Some(secret)) if !id.trim().is_empty() && !secret.trim().is_empty() => {
            Some(OAuthClient {
                client_id: id.trim().to_owned(),
                client_secret: secret.trim().to_owned(),
            })
        }
        _ => {
            return Err(invalid(
                "Enter both the client ID and the client secret.",
                uri,
                request_id,
            ));
        }
    };
    let was_oauth = current.is_some_and(Integration::uses_oauth);
    if let Some(client) = new_client {
        if next.oauth_token_url.is_none() {
            return Err(invalid("Enter the token URL.", uri, request_id));
        }
        if client.client_id.len() + client.client_secret.len() > 8_192 {
            return Err(invalid(
                "The client ID and secret must be at most 8 KiB.",
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
        let json = serde_json::to_string(&client)
            .map_err(|_| invalid("The client cannot be sealed.", uri, request_id))?;
        next.oauth_sealed_client = Some(
            encrypt_secret(key, &json)
                .map_err(|()| invalid("The client cannot be sealed.", uri, request_id))?,
        );
        next.sealed_secret = None;
        if !was_oauth && !header_given {
            next.auth_header = None;
        }
    } else if next.oauth_token_url.is_some() {
        let moved = current.is_some_and(|current| {
            current.url != next.url || current.oauth_token_url != next.oauth_token_url
        });
        if moved || next.oauth_sealed_client.is_none() {
            return Err(invalid(
                "A new base URL or token URL needs the client ID and secret again.",
                uri,
                request_id,
            ));
        }
    }
    Ok(())
}

/// What Test does a GET of: the test path below the base URL, else the base URL.
fn test_url(integration: &Integration) -> Result<Url, OutboundError> {
    let base = Url::parse(&integration.url)
        .map_err(|_| OutboundError::Refused("The URL is not valid."))?;
    match &integration.test_path {
        Some(path) => join_api_url(&base, &format!("/{path}"), &[]),
        None => Ok(base),
    }
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
        oauth_token_url: None,
        oauth_sealed_client: None,
        test_path: None,
        graphql_read_path: None,
        graphql_reads_enabled: false,
        allowed_roles: ROLES.iter().map(|role| (*role).to_owned()).collect(),
        enabled_tools: Vec::new(),
        allowed_methods: vec!["GET".to_owned()],
        tools: None,
        tools_checked_at: None,
        created_at: now,
        updated_at: now,
        connection: None,
        drop_connection: false,
    });
    // Strictly newer, also in the same millisecond: it is part of the token cache's key.
    next.updated_at = current.map_or(now, |current| now.max(current.updated_at + 1));
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
            // Another server: its tools are unknown until the next test, and the sign-in was for
            // the old one.
            next.tools = None;
            next.tools_checked_at = None;
            next.enabled_tools.clear();
            next.connection = None;
        }
        next.url = url;
    }
    if next.url.is_empty() {
        return Err(invalid("Enter the URL.", uri, request_id));
    }
    let header_given = input.auth_header.is_some();
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
    let current_uses_oauth = current.is_some_and(Integration::uses_oauth);
    let oauth_input = OAuthInput {
        token_url: input.oauth_token_url,
        client_id: input.oauth_client_id,
        client_secret: input.oauth_client_secret,
    };
    if input
        .secret
        .as_ref()
        .is_some_and(|secret| !secret.is_empty())
        && oauth_input.is_given()
    {
        return Err(invalid(
            "Choose a fixed credential or OAuth client credentials, not both.",
            uri,
            request_id,
        ));
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
            // A fixed credential replaces the sign-in.
            next.connection = None;
            if next.auth_header.is_none() || (current_uses_oauth && !header_given) {
                next.auth_header = Some(DEFAULT_AUTH_HEADER.to_owned());
            }
            next.oauth_token_url = None;
            next.oauth_sealed_client = None;
        }
    }
    apply_oauth(
        state,
        current,
        &mut next,
        oauth_input,
        header_given,
        uri,
        request_id,
    )?;
    if next.uses_oauth() {
        next.auth_header
            .get_or_insert_with(|| DEFAULT_AUTH_HEADER.to_owned());
    } else if next.sealed_secret.is_none() {
        next.auth_header = None;
    }
    if let Some(path) = input.test_path {
        if next.kind != "http" {
            return Err(invalid("Only HTTP APIs have a test path.", uri, request_id));
        }
        let path = path.trim();
        next.test_path =
            (!path.is_empty()).then(|| path.strip_prefix('/').unwrap_or(path).to_owned());
    }
    if next.test_path.is_some() && test_url(&next).is_err() {
        return Err(invalid(
            "The test path must stay below the base URL.",
            uri,
            request_id,
        ));
    }
    if (input.graphql_read_path.is_some() || input.graphql_reads_enabled.is_some())
        && next.kind != "http"
    {
        return Err(invalid(
            "Only HTTP APIs have GraphQL reads.",
            uri,
            request_id,
        ));
    }
    if let Some(path) = input.graphql_read_path {
        let path = path.trim();
        next.graphql_read_path =
            (!path.is_empty()).then(|| format!("/{}", path.strip_prefix('/').unwrap_or(path)));
    }
    if let Some(enabled) = input.graphql_reads_enabled {
        next.graphql_reads_enabled = enabled;
    }
    if next.graphql_reads_enabled && next.graphql_read_path.is_none() {
        return Err(invalid(
            "Enter the GraphQL path before you turn on reads without approval.",
            uri,
            request_id,
        ));
    }
    if let Some(path) = &next.graphql_read_path {
        let below_base = Url::parse(&next.url)
            .ok()
            .is_some_and(|base| join_api_url(&base, path, &[]).is_ok());
        if !below_base {
            return Err(invalid(
                "The GraphQL path must stay below the base URL.",
                uri,
                request_id,
            ));
        }
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
    next.drop_connection =
        current.is_some_and(|current| current.connection.is_some()) && next.connection.is_none();
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
         created_by, created_at, updated_at, oauth_token_url, oauth_sealed_client, test_path, \
         graphql_read_path, graphql_reads_enabled) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21) \
         ON CONFLICT (id) DO UPDATE SET slug = excluded.slug, name = excluded.name, \
         url = excluded.url, auth_header = excluded.auth_header, \
         sealed_secret = excluded.sealed_secret, allowed_roles = excluded.allowed_roles, \
         enabled_tools = excluded.enabled_tools, allowed_methods = excluded.allowed_methods, \
         tools = excluded.tools, tools_checked_at = excluded.tools_checked_at, \
         updated_at = excluded.updated_at, oauth_token_url = excluded.oauth_token_url, \
         oauth_sealed_client = excluded.oauth_sealed_client, test_path = excluded.test_path, \
         graphql_read_path = excluded.graphql_read_path, \
         graphql_reads_enabled = excluded.graphql_reads_enabled",
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
    .bind(&integration.oauth_token_url)
    .bind(&integration.oauth_sealed_client)
    .bind(&integration.test_path)
    .bind(&integration.graphql_read_path)
    .bind(i64::from(integration.graphql_reads_enabled))
    .execute(&mut *tx)
    .await?;
    // Only a removal: a new connection is saved by `mcp_oauth`, so this save cannot undo it.
    if integration.drop_connection {
        sqlx::query(DROP_CONNECTION)
            .bind(&integration.id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(true)
}

const DROP_CONNECTION: &str = "UPDATE agent_integrations SET oauth_sealed_connection = NULL, \
     oauth_account = NULL, oauth_connected_at = NULL, oauth_grant_expires_at = NULL, \
     oauth_invalid_at = NULL, oauth_expiry_notice_at = NULL WHERE id = $1";

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
    let secret_changed = input.secret.is_some() || input.oauth_client_secret.is_some();
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
    state.tokens.forget(&integration_id);
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
/// GET of the test path, or else of the base URL. OAuth: always a new token, and a GET only of a
/// test path (the base URL can answer 404 on a working API).
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
    let opened = integration
        .credential(state.app_key.as_ref())
        .and_then(|credential| Ok((credential, integration.oauth(state.app_key.as_ref())?)));
    let (credential, oauth) = match opened {
        Ok(opened) => opened,
        Err(detail) => {
            return Ok(Json(
                json!({ "ok": false, "detail": detail, "integration": IntegrationView::from(&integration) }),
            ));
        }
    };
    let url = Url::parse(&integration.url)
        .map_err(|_| invalid("The URL is not valid.", &uri, &request_id))?;
    let outcome: Result<String, OutboundError> = if integration.kind == "mcp" {
        read_tools(
            &state,
            (workspace_id, user),
            &mut integration,
            &url,
            credential.as_ref(),
        )
        .await
    } else if let Some((token_url, client)) = &oauth {
        test_oauth(&state, &integration, token_url, client).await
    } else {
        let target = integration
            .test_path
            .as_deref()
            .map_or_else(|| "the base URL".to_owned(), str::to_owned);
        match test_url(&integration) {
            Ok(test_url) => state
                .outbound
                .api_request(&test_url, reqwest::Method::GET, None, credential.as_ref())
                .await
                .and_then(|answer| {
                    if answer.status < 400 {
                        Ok(format!(
                            "Connected. GET of {target} answered HTTP {}.",
                            answer.status
                        ))
                    } else {
                        Err(OutboundError::Status(answer.status))
                    }
                }),
            Err(error) => Err(error),
        }
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
    let detail = outcome.unwrap_or_else(|error| match error {
        OutboundError::Status(401)
            if integration.kind == "mcp" && integration.connection.is_none() =>
        {
            "The server needs a sign-in: click Connect.".to_owned()
        }
        error => capitalized(&error.to_string()),
    });
    Ok(Json(json!({
        "ok": ok,
        "detail": detail,
        "integration": IntegrationView::from(&integration),
    })))
}

/// Test of an OAuth integration: a new token (not from the cache, which it does not change), then
/// the test path if there is one.
async fn test_oauth(
    state: &AgentState,
    integration: &Integration,
    token_url: &Url,
    client: &OAuthClient,
) -> Result<String, OutboundError> {
    let (token, _) = state.outbound.client_credentials(token_url, client).await?;
    let Some(path) = &integration.test_path else {
        return Ok(
            "The token request worked. The API was not called: set a test path to check it."
                .to_owned(),
        );
    };
    let answer = state
        .outbound
        .api_request(
            &test_url(integration)?,
            reqwest::Method::GET,
            None,
            Some(&integration.token_credential(&token)),
        )
        .await?;
    if answer.status < 400 {
        Ok(format!(
            "The token request worked. GET of {path} answered HTTP {}.",
            answer.status
        ))
    } else {
        Err(OutboundError::TokenWorkedApiRefused(answer.status))
    }
}

/// Reads an MCP server's tools and keeps them (the first time, read-only tools are turned on).
/// With a sign-in, it uses the connection's token.
async fn read_tools(
    state: &AgentState,
    (workspace_id, user): (Id, Id),
    integration: &mut Integration,
    url: &Url,
    credential: Option<&Credential>,
) -> Result<String, OutboundError> {
    let tools = mcp(state, integration, url, credential, McpOperation::ListTools).await?;
    let tools = tools.as_array().map(Vec::as_slice).unwrap_or_default();
    let tools: Vec<Value> = tools.iter().filter_map(keep_tool).collect();
    if integration.tools.is_none() {
        integration.enabled_tools = tools
            .iter()
            .filter(|tool| read_only(tool))
            .filter_map(|tool| tool["name"].as_str().map(str::to_owned))
            .collect();
    } else {
        integration
            .enabled_tools
            .retain(|name| tools.iter().any(|tool| tool["name"].as_str() == Some(name)));
    }
    let count = tools.len();
    integration.tools = Some(tools);
    integration.tools_checked_at = Some(TimestampMillis::now().as_millis());
    if let Err(error) = save(state.database(), workspace_id, user, integration, false).await {
        tracing::warn!(%error, "agent integrations");
        return Err(OutboundError::Refused("Try again in a moment."));
    }
    Ok(format!("Connected. The server has {count} tools."))
}

/// One operation on an MCP server: `tools/list` (as an array) or one `tools/call`.
#[derive(Clone, Copy)]
enum McpOperation<'a> {
    ListTools,
    Call(&'a str, &'a Value),
}

impl McpOperation<'_> {
    async fn run(
        self,
        outbound: &Outbound,
        url: &Url,
        credential: Option<&Credential>,
    ) -> Result<Value, OutboundError> {
        match self {
            Self::ListTools => outbound.mcp_tools(url, credential).await.map(Value::Array),
            Self::Call(tool, arguments) => {
                outbound.mcp_call(url, credential, tool, arguments).await
            }
        }
    }
}

/// An MCP operation with the integration's credential. With a sign-in, its token: renewed when
/// it is due, and once more after a 401.
async fn mcp(
    state: &AgentState,
    integration: &Integration,
    url: &Url,
    credential: Option<&Credential>,
    operation: McpOperation<'_>,
) -> Result<Value, OutboundError> {
    if integration.connection.is_none() {
        return operation.run(&state.outbound, url, credential).await;
    }
    let token = mcp_oauth::access(state, &integration.id, None).await?;
    let answer = operation
        .run(
            &state.outbound,
            url,
            Some(&token.credential(DEFAULT_AUTH_HEADER)),
        )
        .await;
    if !matches!(answer, Err(OutboundError::Status(401))) {
        return answer;
    }
    let token = mcp_oauth::access(state, &integration.id, Some(&token)).await?;
    operation
        .run(
            &state.outbound,
            url,
            Some(&token.credential(DEFAULT_AUTH_HEADER)),
        )
        .await
}

/// Starts the sign-in of an MCP server: the browser opens `authorize_url`, and comes back to
/// [`connect_callback`]. A failure is `ok: false` with a detail for the admin.
async fn connect_start(
    State(state): State<AgentState>,
    Path((workspace_id, integration_id)): Path<(String, String)>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
) -> Result<Json<Value>, ApiError> {
    let (session_id, user, workspace_id) =
        manager_session(&state, &headers, &workspace_id, &uri, &request_id).await?;
    let integration = load_one(state.database(), workspace_id, "id", &integration_id)
        .await
        .map_err(unavailable(&uri, &request_id))?
        .ok_or_else(|| not_found(&uri, &request_id))?;
    if integration.kind != "mcp" {
        return Err(invalid(
            "Only MCP servers have a sign-in.",
            &uri,
            &request_id,
        ));
    }
    match mcp_oauth::start(
        &state,
        (&integration.id, &integration.url),
        (session_id, user),
    )
    .await
    {
        Ok(url) => Ok(Json(json!({ "ok": true, "authorize_url": url.as_str() }))),
        Err(detail) => {
            record(
                state.database(),
                workspace_id,
                user,
                "agent_integration.connect",
                AuditOutcome::Failure,
                &integration.id,
                &request_id,
                json!({ "slug": integration.slug, "step": "start" }),
            )
            .await;
            Ok(Json(json!({ "ok": false, "detail": detail })))
        }
    }
}

#[derive(Deserialize)]
struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    iss: Option<String>,
    error: Option<String>,
}

/// Where the authorization server sends the admin's browser back. The `state` is used up first,
/// whatever happens next; it must belong to this browser's session, whose user must still manage
/// the workspace. Then Orbit exchanges the code, saves the sealed tokens and reads the tools. The
/// browser goes back to the admin page with one fixed outcome word, never a detail.
async fn connect_callback(
    State(state): State<AgentState>,
    Query(query): Query<CallbackQuery>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
) -> Response {
    let back = |outcome: &str| {
        Redirect::to(&format!(
            "/settings/agentic/workspace?agent_tool_connect={outcome}"
        ))
        .into_response()
    };
    let (Some(key), Some(state_value)) = (state.app_key.as_ref(), query.state.as_deref()) else {
        return back("invalid");
    };
    let database = state.database();
    let started = match mcp_oauth::take(database, key, state_value).await {
        Ok(Some(started)) => started,
        Ok(None) => return back("expired"),
        Err(error) => {
            tracing::warn!(%error, "agent integration connect");
            return back("failed");
        }
    };
    let Ok(session) = authenticate_session(
        &state.identity,
        state.cookie_mode,
        &headers,
        uri.path(),
        request_id.as_ref(),
    )
    .await
    else {
        return back("session");
    };
    if session.id.to_string() != started.session_id
        || session.user.id.to_string() != started.user_id
    {
        return back("session");
    }
    let workspace_id = sqlx::query_scalar::<_, String>(
        "SELECT workspace_id FROM agent_integrations WHERE id = $1",
    )
    .bind(&started.integration_id)
    .fetch_optional(database.pool())
    .await;
    let Ok(Some(workspace_id)) = workspace_id else {
        return back("changed");
    };
    let (user, workspace_id) =
        match manager(&state, &headers, &workspace_id, &uri, &request_id).await {
            Ok(caller) => caller,
            Err(_) => return back("session"),
        };
    let Ok(Some(mut integration)) =
        load_one(database, workspace_id, "id", &started.integration_id).await
    else {
        return back("changed");
    };
    let failure = |step: &'static str| {
        record(
            database,
            workspace_id,
            user,
            "agent_integration.connect",
            AuditOutcome::Failure,
            &started.integration_id,
            &request_id,
            json!({ "step": step }),
        )
    };
    if query.error.is_some() {
        failure("denied").await;
        return back("denied");
    }
    let Some(code) = query.code.as_deref().filter(|code| !code.is_empty()) else {
        failure("no_code").await;
        return back("invalid");
    };
    if integration.url != started.mcp_url() {
        failure("url_changed").await;
        return back("changed");
    }
    let (connection, tokens) =
        match mcp_oauth::finish(&state.outbound, &started, code, query.iss.as_deref()).await {
            Ok(done) => done,
            Err(detail) => {
                tracing::info!(
                    detail,
                    "agent integration connect: the code exchange failed"
                );
                failure("token").await;
                return back("failed");
            }
        };
    match mcp_oauth::save_new(database, key, &started, &connection, &tokens).await {
        Ok(true) => {}
        Ok(false) => {
            failure("url_changed").await;
            return back("changed");
        }
        Err(error) => {
            tracing::warn!(%error, "agent integration connect");
            return back("failed");
        }
    }
    record(
        database,
        workspace_id,
        user,
        "agent_integration.connect",
        AuditOutcome::Success,
        &integration.id,
        &request_id,
        json!({ "slug": integration.slug, "issuer": connection.issuer }),
    )
    .await;
    // The tools, with the new token. A failure here leaves the connection; Test tries again.
    if let Ok(Some(saved)) = load_one(database, workspace_id, "id", &integration.id).await {
        integration = saved;
        if let Ok(url) = Url::parse(&integration.url)
            && let Err(error) =
                read_tools(&state, (workspace_id, user), &mut integration, &url, None).await
        {
            tracing::info!(%error, "agent integration connect: the tools could not be read");
        }
    }
    back("connected")
}

/// Removes an MCP server's sign-in. The server keeps its grant until it expires or the admin
/// revokes it there.
async fn disconnect(
    State(state): State<AgentState>,
    Path((workspace_id, integration_id)): Path<(String, String)>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
) -> Result<Json<IntegrationView>, ApiError> {
    let (user, workspace_id) = manager(&state, &headers, &workspace_id, &uri, &request_id).await?;
    let database = state.database();
    let integration = load_one(database, workspace_id, "id", &integration_id)
        .await
        .map_err(unavailable(&uri, &request_id))?
        .ok_or_else(|| not_found(&uri, &request_id))?;
    sqlx::query(DROP_CONNECTION)
        .bind(&integration.id)
        .execute(database.pool())
        .await
        .map_err(unavailable(&uri, &request_id))?;
    record(
        database,
        workspace_id,
        user,
        "agent_integration.disconnect",
        AuditOutcome::Success,
        &integration.id,
        &request_id,
        json!({ "slug": integration.slug }),
    )
    .await;
    let integration = load_one(database, workspace_id, "id", &integration_id)
        .await
        .map_err(unavailable(&uri, &request_id))?
        .ok_or_else(|| not_found(&uri, &request_id))?;
    Ok(Json(IntegrationView::from(&integration)))
}

pub(crate) fn capitalized(text: &str) -> String {
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
pub(crate) async fn driver(
    state: &AgentState,
    headers: &HeaderMap,
    workspace_id: &str,
    uri: &Uri,
    request_id: &RequestIdExtension,
) -> Result<(Id, Id, WorkspaceRole), ApiError> {
    let (_, user, workspace_id, role) =
        driver_session(state, headers, workspace_id, uri, request_id).await?;
    Ok((user, workspace_id, role))
}

/// As [`driver`], with the integrations the session's agent may use (its card's ticks).
async fn driver_access(
    state: &AgentState,
    headers: &HeaderMap,
    workspace_id: &str,
    uri: &Uri,
    request_id: &RequestIdExtension,
) -> Result<(Id, Id, WorkspaceRole, IntegrationAccess), ApiError> {
    let (session, user, workspace_id, role) =
        driver_session(state, headers, workspace_id, uri, request_id).await?;
    let database = state.database();
    let agent = super::sessions::agent_of(database, session)
        .await
        .map_err(unavailable(uri, request_id))?
        .flatten();
    let mut conn = database
        .pool()
        .acquire()
        .await
        .map_err(unavailable(uri, request_id))?;
    let access = super::profiles::integration_access(&mut conn, workspace_id, agent)
        .await
        .map_err(unavailable(uri, request_id))?;
    Ok((user, workspace_id, role, access))
}

async fn driver_session(
    state: &AgentState,
    headers: &HeaderMap,
    workspace_id: &str,
    uri: &Uri,
    request_id: &RequestIdExtension,
) -> Result<(Id, Id, Id, WorkspaceRole), ApiError> {
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
    Ok((session.id, session.user.id, workspace_id, actor.role))
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
                "Send one request to the {} HTTP API. The path is below its base URL {}.{}",
                integration.name,
                integration.url,
                match (
                    &integration.graphql_read_path,
                    integration.graphql_reads_enabled
                ) {
                    (Some(path), true) => format!(
                        " A POST to {path} with one GraphQL query (no mutation) needs no approval."
                    ),
                    _ => String::new(),
                }
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
    let (_, workspace_id, role, access) =
        driver_access(&state, &headers, &workspace_id, &uri, &request_id).await?;
    let all = load_all(state.database(), workspace_id)
        .await
        .map_err(unavailable(&uri, &request_id))?;
    // The role rule of each integration and the agent's ticks, both.
    let tools: Vec<AgentTool> = all
        .iter()
        .filter(|integration| integration.allows(role) && access.allows(&integration.id))
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
    sorted(value).to_string()
}

/// As [`canonical`], one key per line, for the driver to read.
pub(crate) fn readable(value: &Value) -> String {
    serde_json::to_string_pretty(&sorted(value)).unwrap_or_else(|_| canonical(value))
}

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

pub(crate) fn hash(text: &str) -> String {
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

    /// A POST to the integration's exact GraphQL path, with the setting on, whose body is one
    /// query. Only what Orbit will send counts: this very body, and no URL query.
    fn graphql_read(&self, integration: &Integration) -> bool {
        let Self::Api {
            method,
            url,
            path,
            body,
        } = self
        else {
            return false;
        };
        integration.graphql_reads_enabled
            && integration.graphql_read_path.as_deref() == Some(path.as_str())
            && *method == reqwest::Method::POST
            && url.query().is_none()
            && body.as_ref().is_some_and(graphql::is_read_only_request)
    }

    /// The exact action, as the driver approves it.
    fn describe(&self, integration: &Integration) -> String {
        let text = match self {
            Self::Mcp {
                tool, arguments, ..
            } => format!(
                "{} → {tool} with arguments\n{}",
                integration.name,
                readable(arguments)
            ),
            Self::Api {
                method, url, body, ..
            } => {
                let body = body.as_ref().map_or_else(String::new, |body| {
                    format!(" with body\n{}", readable(body))
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
    Granted { id: String },
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

/// What an approval belongs to: an integration, the Drive of a workspace, or the routines and
/// scheduled messages of a workspace.
#[derive(Clone, Copy)]
pub(crate) enum Approves<'a> {
    Integration(&'a str),
    Drive(&'a str),
    Scheduled(&'a str),
}

impl Approves<'_> {
    /// The table and the column that names what the approval belongs to.
    fn table(self) -> (&'static str, &'static str, String) {
        match self {
            Self::Integration(id) => (
                "agent_integration_approvals",
                "integration_id",
                id.to_owned(),
            ),
            // Routines and send later share the Drive's table: both are per workspace, and the
            // tool tells them apart.
            Self::Drive(workspace_id) | Self::Scheduled(workspace_id) => (
                "agent_drive_approvals",
                "workspace_id",
                workspace_id.to_owned(),
            ),
        }
    }
}

/// The approval of one write: bound to the driver, the thread, the integration (or the Drive), the
/// tool and the hash of the arguments. It is granted (and used up, once) only when the driver
/// posted a "yes" in that thread after it was made, and before it expired. Else a new one is made,
/// or the open one is returned, and the driver is asked.
pub(crate) async fn approve(
    database: &Database,
    subject: Approves<'_>,
    user: Id,
    (conversation_id, root_message_id): (&str, Option<&str>),
    tool: &str,
    arguments_hash: &str,
) -> Result<Approval, sqlx::Error> {
    // `table` and `column` are fixed names from `Approves::table`, never input.
    let (table, column, subject_id) = subject.table();
    let now = TimestampMillis::now().as_millis();
    // A run of a routine's saved script that its creator approved (the exact code, by its hash) needs
    // no new yes: the yes to the script covers its writes (`crate::scheduled::runs`). Never for
    // routines and messages posted as the driver: a script cannot schedule more work unasked.
    if !matches!(subject, Approves::Scheduled(_))
        && let Some(run) = crate::scheduled::runs::approved_script_run(
            database,
            conversation_id,
            root_message_id,
            user,
        )
        .await?
    {
        return Ok(Approval::Granted {
            id: format!("script-run:{run}"),
        });
    }
    let mut tx = database.immediate_transaction().await?;
    let open = sqlx::query(sqlx::AssertSqlSafe(format!(
        "SELECT id, created_at, expires_at FROM {table} \
         WHERE {column} = $1 AND user_id = $2 AND conversation_id = $3 \
         AND root_message_id IS NOT DISTINCT FROM $4 AND tool = $5 AND arguments_hash = $6 \
         AND used_at IS NULL AND expires_at > $7 ORDER BY created_at DESC LIMIT 1",
    )))
    .bind(&subject_id)
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
            sqlx::query(sqlx::AssertSqlSafe(format!(
                "UPDATE {table} SET used_at = $1 WHERE id = $2 AND used_at IS NULL"
            )))
            .bind(now)
            .bind(&id)
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
            return Ok(Approval::Granted { id });
        }
        tx.commit().await?;
        return Ok(Approval::Needed {
            id,
            expires_at: open.get("expires_at"),
        });
    }
    let id = Id::new_v7().to_string();
    let expires_at = now + APPROVAL_LIFETIME_MS;
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "INSERT INTO {table} (id, {column}, user_id, conversation_id, \
         root_message_id, tool, arguments_hash, created_at, expires_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
    )))
    .bind(&id)
    .bind(&subject_id)
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

/// One API call with the OAuth token. On a 401: one new token and one retry, within the same
/// approval. A 403 or a failed token request is not retried.
async fn api_request_with_token(
    state: &AgentState,
    integration: &Integration,
    (token_url, client): &(Url, OAuthClient),
    (url, method): (&Url, &reqwest::Method),
    body: Option<&Value>,
) -> Result<ApiAnswer, OutboundError> {
    let key = (integration.id.as_str(), integration.updated_at);
    let oauth = (token_url, client);
    let token = state.tokens.get(&state.outbound, key, oauth, None).await?;
    let credential = integration.token_credential(&token);
    let answer = state
        .outbound
        .api_request(url, method.clone(), body, Some(&credential))
        .await?;
    if answer.status != 401 {
        return Ok(answer);
    }
    let token = state
        .tokens
        .get(&state.outbound, key, oauth, Some(&token))
        .await?;
    let credential = integration.token_credential(&token);
    state
        .outbound
        .api_request(url, method.clone(), body, Some(&credential))
        .await
}

/// Refuses unless `user` drives the current run of this thread: tools act only there.
pub(crate) async fn require_running(
    database: &Database,
    (workspace_id, user): (Id, Id),
    (conversation_id, root_message_id): (&str, Option<&str>),
    uri: &Uri,
    request_id: &RequestIdExtension,
) -> Result<(), ApiError> {
    let running = sqlx::query_scalar::<_, String>(
        "SELECT thread_key FROM agent_threads WHERE workspace_id = $1 AND conversation_id = $2 \
         AND root_message_id IS NOT DISTINCT FROM $3 AND driver_user_id = $4 AND status = 'running'",
    )
    .bind(workspace_id.to_string())
    .bind(conversation_id)
    .bind(root_message_id)
    .bind(user.to_string())
    .fetch_optional(database.pool())
    .await
    .map_err(unavailable(uri, request_id))?;
    if running.is_none() {
        return Err(problem(
            StatusCode::FORBIDDEN,
            "agent_run_required",
            "Tools run only in a thread whose current run the caller drives.",
            uri,
            request_id,
        ));
    }
    Ok(())
}

async fn call(
    State(state): State<AgentState>,
    Path(workspace_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
    dry: Option<Extension<super::scope::DryRun>>,
    Json(body): Json<CallBody>,
) -> Result<Json<Value>, ApiError> {
    let (user, workspace_id, role, access) =
        driver_access(&state, &headers, &workspace_id, &uri, &request_id).await?;
    let database = state.database();
    require_running(
        database,
        (workspace_id, user),
        (&body.conversation_id, body.root_message_id.as_deref()),
        &uri,
        &request_id,
    )
    .await?;
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
    if !access.allows(&integration.id) {
        return Err(problem(
            StatusCode::FORBIDDEN,
            "agent_integration_agent",
            "This agent may not use this integration: it is not ticked on the agent's card.",
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
    let graphql_read = action.graphql_read(&integration);
    let needs_approval = action.write() && !graphql_read;
    // A test run reads, and only says which writes it would make.
    if needs_approval && dry.is_some() {
        return Ok(Json(json!({ "status": "dry_run", "action": describe })));
    }
    if needs_approval {
        let arguments_hash = hash(&canonical(&body.arguments));
        let approval = approve(
            database,
            Approves::Integration(&integration.id),
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
    let credential_problem = |detail| {
        problem(
            StatusCode::CONFLICT,
            "agent_integration_credential",
            detail,
            &uri,
            &request_id,
        )
    };
    let credential = integration
        .credential(state.app_key.as_ref())
        .map_err(credential_problem)?;
    let oauth = integration
        .oauth(state.app_key.as_ref())
        .map_err(credential_problem)?;
    let (outcome, mut metadata) = match &action {
        Action::Mcp {
            tool, arguments, ..
        } => {
            let url = Url::parse(&integration.url).map_err(|_| not_found(&uri, &request_id))?;
            let result = mcp(
                &state,
                &integration,
                &url,
                credential.as_ref(),
                McpOperation::Call(tool, arguments),
            )
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
            let result = match &oauth {
                Some(oauth) => {
                    api_request_with_token(
                        &state,
                        &integration,
                        oauth,
                        (url, method),
                        payload.as_ref(),
                    )
                    .await
                }
                None => {
                    state
                        .outbound
                        .api_request(url, method.clone(), payload.as_ref(), credential.as_ref())
                        .await
                }
            };
            let status = result.as_ref().ok().map(|answer| answer.status);
            (
                result.map(|answer| json!({ "status": "ok", "response": answer })),
                json!({ "method": method.as_str(), "path": path, "status": status }),
            )
        }
    };
    metadata["integration"] = json!(integration.slug);
    metadata["approved"] = json!(needs_approval);
    if graphql_read {
        metadata["graphql_read_without_approval"] = json!(true);
    }
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
