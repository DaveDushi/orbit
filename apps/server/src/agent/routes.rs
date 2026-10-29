//! HTTP routes of the agent.
//!
//! - `/api/v1/agent…`: the signed-in person's ChatGPT connection (session cookie).
//! - `/api/internal/agent/…`: the sidecar only, with the shared secret in `x-orbit-agent-secret`.
//!   Not in the public OpenAPI contract.

use std::time::Duration;

use axum::extract::{Extension, Path, Query, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post, put};
use axum::{Json, Router};
use orbit_platform::{Id, RequestId};
use serde::Deserialize;
use serde_json::{Value, json};

use super::inputs::{self, ThreadUpdate};
use super::{AgentState, bot, credentials, sessions};
use crate::push::PushService;
use crate::repositories::chat::SendInput;
use crate::task_routes::{ApiError, authenticate_session};

const SECRET_HEADER: &str = "x-orbit-agent-secret";

type RequestIdExtension = Option<Extension<RequestId>>;

pub fn router(state: AgentState) -> Router {
    Router::new()
        .route("/api/v1/agent", get(agent_status))
        .route(
            "/api/v1/agent/chatgpt",
            axum::routing::delete(disconnect_chatgpt),
        )
        .route("/api/v1/agent/chatgpt/login", post(start_chatgpt_login))
        .route("/api/internal/agent/inputs", get(list_inputs))
        .route(
            "/api/internal/agent/inputs/{message_id}/ack",
            post(acknowledge_input),
        )
        .route(
            "/api/internal/agent/threads/{thread_key}",
            put(update_thread),
        )
        .route("/api/internal/agent/messages", post(post_message))
        .route(
            "/api/internal/agent/messages/{message_id}",
            patch(edit_message),
        )
        .route("/api/internal/agent/sessions", post(issue_session))
        .route(
            "/api/internal/agent/credentials/{user_id}",
            get(read_credential)
                .put(write_credential)
                .delete(delete_credential),
        )
        .route(
            "/api/internal/agent/credentials/{user_id}/invalid",
            post(invalidate_credential),
        )
        .with_state(state)
}

fn error(
    status: StatusCode,
    code: &'static str,
    detail: &'static str,
    uri: &Uri,
    request_id: &RequestIdExtension,
) -> ApiError {
    let title = status.canonical_reason().unwrap_or("Error");
    ApiError::new(status, code, title, detail, uri.path(), request_id.as_ref())
}

fn unavailable(uri: &Uri, request_id: &RequestIdExtension) -> ApiError {
    error(
        StatusCode::SERVICE_UNAVAILABLE,
        "unavailable",
        "Try again in a moment.",
        uri,
        request_id,
    )
}

fn off(uri: &Uri, request_id: &RequestIdExtension) -> ApiError {
    error(
        StatusCode::NOT_FOUND,
        "agent_disabled",
        "The agent is not set up on this server.",
        uri,
        request_id,
    )
}

// ---------------------------------------------------------------------------------------------
// The person's routes.

async fn agent_status(
    State(state): State<AgentState>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
) -> Result<Json<Value>, ApiError> {
    let session = authenticate_session(
        &state.identity,
        state.cookie_mode,
        &headers,
        uri.path(),
        request_id.as_ref(),
    )
    .await?;
    let Some(config) = &state.config else {
        return Ok(Json(json!({ "enabled": false })));
    };
    let mut conn = state
        .database()
        .pool()
        .acquire()
        .await
        .map_err(|_| unavailable(&uri, &request_id))?;
    let bot_user_id = inputs::bot_user_id(&mut conn)
        .await
        .map_err(|_| unavailable(&uri, &request_id))?;
    drop(conn);
    let chatgpt = credentials::status(state.database(), session.user.id)
        .await
        .map_err(|_| unavailable(&uri, &request_id))?;
    let login = sidecar(&state, config, reqwest::Method::GET, session.user.id).await;
    Ok(Json(json!({
        "enabled": true,
        "online": login.is_some(),
        "can_connect": state.app_key.is_some(),
        "bot_user_id": bot_user_id.map(|id| id.to_string()),
        "chatgpt": chatgpt,
        "login": login.and_then(|value| value.get("login").cloned()).unwrap_or(Value::Null),
    })))
}

async fn start_chatgpt_login(
    State(state): State<AgentState>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
) -> Result<Json<Value>, ApiError> {
    let session = authenticate_session(
        &state.identity,
        state.cookie_mode,
        &headers,
        uri.path(),
        request_id.as_ref(),
    )
    .await?;
    let Some(config) = &state.config else {
        return Err(off(&uri, &request_id));
    };
    if state.app_key.is_none() {
        return Err(error(
            StatusCode::CONFLICT,
            "agent_no_app_key",
            "The server has no app key, so it cannot store a ChatGPT sign-in.",
            &uri,
            &request_id,
        ));
    }
    let started = sidecar(&state, config, reqwest::Method::POST, session.user.id)
        .await
        .ok_or_else(|| {
            error(
                StatusCode::BAD_GATEWAY,
                "agent_offline",
                "The agent service is offline.",
                &uri,
                &request_id,
            )
        })?;
    Ok(Json(json!({
        "login": started.get("login").cloned().unwrap_or(Value::Null),
    })))
}

async fn disconnect_chatgpt(
    State(state): State<AgentState>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
) -> Result<StatusCode, ApiError> {
    let session = authenticate_session(
        &state.identity,
        state.cookie_mode,
        &headers,
        uri.path(),
        request_id.as_ref(),
    )
    .await?;
    if let Some(config) = &state.config {
        sidecar(&state, config, reqwest::Method::DELETE, session.user.id).await;
    }
    credentials::delete(state.database(), session.user.id)
        .await
        .map_err(|_| unavailable(&uri, &request_id))?;
    Ok(StatusCode::NO_CONTENT)
}

/// Calls the sidecar's login route for one person; `None` when it does not answer.
async fn sidecar(
    state: &AgentState,
    config: &super::AgentConfig,
    method: reqwest::Method,
    user_id: Id,
) -> Option<Value> {
    let response = state
        .http
        .request(method, format!("{}/login/{user_id}", config.url))
        .header(SECRET_HEADER, &config.secret)
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        tracing::warn!(status = %response.status(), "agent sidecar login route");
        return None;
    }
    response.json().await.ok()
}

// ---------------------------------------------------------------------------------------------
// The sidecar's routes.

/// Constant-time check of the shared secret.
fn sidecar_only(
    state: &AgentState,
    headers: &HeaderMap,
    uri: &Uri,
    request_id: &RequestIdExtension,
) -> Result<(), ApiError> {
    let Some(config) = &state.config else {
        return Err(off(uri, request_id));
    };
    let given = headers
        .get(SECRET_HEADER)
        .map(|value| value.as_bytes())
        .unwrap_or_default();
    let expected = config.secret.as_bytes();
    let same = given.len() == expected.len()
        && given
            .iter()
            .zip(expected)
            .fold(0_u8, |difference, (a, b)| difference | (a ^ b))
            == 0;
    if same {
        Ok(())
    } else {
        Err(error(
            StatusCode::UNAUTHORIZED,
            "authentication_required",
            "The agent secret is missing or wrong.",
            uri,
            request_id,
        ))
    }
}

fn id_of(value: &str, uri: &Uri, request_id: &RequestIdExtension) -> Result<Id, ApiError> {
    value.parse().map_err(|_| {
        error(
            StatusCode::NOT_FOUND,
            "not_found",
            "Not found.",
            uri,
            request_id,
        )
    })
}

#[derive(Deserialize)]
struct InputsQuery {
    /// Seconds to wait for an input when none is queued (0–25).
    #[serde(default)]
    wait: u64,
}

/// The queued inputs, oldest first. Waits up to `wait` seconds for one. Also puts the bot into
/// workspaces made since the last call.
async fn list_inputs(
    State(state): State<AgentState>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
    Query(query): Query<InputsQuery>,
) -> Result<Json<Value>, ApiError> {
    sidecar_only(&state, &headers, &uri, &request_id)?;
    let database = state.database();
    let bot = bot::ensure(database)
        .await
        .map_err(|_| unavailable(&uri, &request_id))?;
    if let Err(error) = bot::join_workspaces(database, bot).await {
        tracing::warn!(%error, "agent bot workspaces");
    }
    let deadline = tokio::time::Instant::now() + Duration::from_secs(query.wait.min(25));
    loop {
        let pending = inputs::pending(database, 50)
            .await
            .map_err(|_| unavailable(&uri, &request_id))?;
        if !pending.is_empty() || tokio::time::Instant::now() >= deadline {
            return Ok(Json(json!({
                "bot_user_id": bot.to_string(),
                "inputs": pending,
            })));
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

async fn acknowledge_input(
    State(state): State<AgentState>,
    Path(message_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
) -> Result<StatusCode, ApiError> {
    sidecar_only(&state, &headers, &uri, &request_id)?;
    let message_id = id_of(&message_id, &uri, &request_id)?;
    inputs::acknowledge(state.database(), message_id)
        .await
        .map_err(|_| unavailable(&uri, &request_id))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn update_thread(
    State(state): State<AgentState>,
    Path(thread_key): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
    Json(update): Json<ThreadUpdate>,
) -> Result<StatusCode, ApiError> {
    sidecar_only(&state, &headers, &uri, &request_id)?;
    let thread_key = id_of(&thread_key, &uri, &request_id)?;
    inputs::update_thread(state.database(), thread_key, &update)
        .await
        .map_err(|error| {
            tracing::warn!(%error, "agent thread update");
            unavailable(&uri, &request_id)
        })?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PostMessageBody {
    workspace_id: Id,
    conversation_id: Id,
    /// The thread; `null` posts in the main list (a DM with the agent).
    root_message_id: Option<Id>,
    body: String,
    /// The same nonce returns the first message.
    nonce: String,
}

/// Sends a message as the bot. Runs in a task of its own so a dropped request cannot stop it
/// between its commit and its live events (as the chat routes do).
async fn post_message(
    State(state): State<AgentState>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
    Json(body): Json<PostMessageBody>,
) -> Result<Json<Value>, ApiError> {
    sidecar_only(&state, &headers, &uri, &request_id)?;
    let chat = state.chat.clone();
    let database = state.database().clone();
    let written = tokio::spawn(async move {
        let bot = bot::ensure(&database).await?;
        let input = SendInput {
            conversation_id: body.conversation_id,
            thread_root_id: body.root_message_id,
            reply_to_id: None,
            body: body.body,
            sticker_id: None,
            file_ids: Vec::new(),
            online: chat.hub.online(body.workspace_id),
            also_in_channel: false,
            nonce: body.nonce,
        };
        let (written, _) = chat
            .publish(
                body.workspace_id,
                bot,
                chat.chat.send_message(body.workspace_id, bot, input),
            )
            .await
            .map_err(|error| sqlx::Error::Protocol(error.to_string()))?;
        PushService::of(&database).chat_message(&written.value);
        Ok::<_, sqlx::Error>(written.value)
    })
    .await;
    match written {
        Ok(Ok(message)) => Ok(Json(json!({ "id": message.id.to_string() }))),
        Ok(Err(failure)) => {
            tracing::warn!(error = %failure, "agent message");
            Err(error(
                StatusCode::UNPROCESSABLE_ENTITY,
                "agent_message_failed",
                "The agent's message could not be sent.",
                &uri,
                &request_id,
            ))
        }
        Err(_) => Err(unavailable(&uri, &request_id)),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EditMessageBody {
    workspace_id: Id,
    body: String,
}

async fn edit_message(
    State(state): State<AgentState>,
    Path(message_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
    Json(body): Json<EditMessageBody>,
) -> Result<StatusCode, ApiError> {
    sidecar_only(&state, &headers, &uri, &request_id)?;
    let message_id = id_of(&message_id, &uri, &request_id)?;
    let chat = state.chat.clone();
    let database = state.database().clone();
    let edited = tokio::spawn(async move {
        let bot = bot::ensure(&database).await?;
        chat.publish(
            body.workspace_id,
            bot,
            chat.chat
                .edit_message(body.workspace_id, bot, message_id, &body.body),
        )
        .await
        .map_err(|error| sqlx::Error::Protocol(error.to_string()))?;
        Ok::<_, sqlx::Error>(())
    })
    .await;
    match edited {
        Ok(Ok(())) => Ok(StatusCode::NO_CONTENT),
        Ok(Err(failure)) => {
            tracing::warn!(error = %failure, "agent message edit");
            Err(error(
                StatusCode::UNPROCESSABLE_ENTITY,
                "agent_message_failed",
                "The agent's message could not be changed.",
                &uri,
                &request_id,
            ))
        }
        Err(_) => Err(unavailable(&uri, &request_id)),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SessionBody {
    user_id: Id,
    workspace_id: Id,
}

async fn issue_session(
    State(state): State<AgentState>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
    Json(body): Json<SessionBody>,
) -> Result<Response, ApiError> {
    sidecar_only(&state, &headers, &uri, &request_id)?;
    match sessions::issue(state.database(), body.user_id, body.workspace_id).await {
        Ok(issued) => Ok(Json(issued).into_response()),
        Err(sessions::SessionRefused::Database(failure)) => {
            tracing::warn!(error = %failure, "agent session");
            Err(unavailable(&uri, &request_id))
        }
        Err(_) => Err(error(
            StatusCode::FORBIDDEN,
            "agent_session_refused",
            "The person cannot act in this workspace now.",
            &uri,
            &request_id,
        )),
    }
}

fn app_key<'a>(
    state: &'a AgentState,
    uri: &Uri,
    request_id: &RequestIdExtension,
) -> Result<&'a [u8; 32], ApiError> {
    state.app_key.as_ref().ok_or_else(|| {
        error(
            StatusCode::CONFLICT,
            "agent_no_app_key",
            "The server has no app key.",
            uri,
            request_id,
        )
    })
}

async fn read_credential(
    State(state): State<AgentState>,
    Path(user_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
) -> Result<Json<Value>, ApiError> {
    sidecar_only(&state, &headers, &uri, &request_id)?;
    let user_id = id_of(&user_id, &uri, &request_id)?;
    let key = app_key(&state, &uri, &request_id)?;
    match credentials::read(state.database(), key, user_id).await {
        Ok(credential) => Ok(Json(json!({ "credential": credential }))),
        Err(credentials::CredentialError::Unreadable) => {
            // Sealed under another app key: the person connects again.
            let _ = credentials::mark_invalid(state.database(), user_id).await;
            Ok(Json(json!({ "credential": null })))
        }
        Err(_) => Err(unavailable(&uri, &request_id)),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CredentialBody {
    credential: Value,
    account_id: Option<String>,
}

async fn write_credential(
    State(state): State<AgentState>,
    Path(user_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
    Json(body): Json<CredentialBody>,
) -> Result<StatusCode, ApiError> {
    sidecar_only(&state, &headers, &uri, &request_id)?;
    let user_id = id_of(&user_id, &uri, &request_id)?;
    let key = app_key(&state, &uri, &request_id)?;
    credentials::write(
        state.database(),
        key,
        user_id,
        &body.credential,
        body.account_id.as_deref(),
    )
    .await
    .map_err(|_| unavailable(&uri, &request_id))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_credential(
    State(state): State<AgentState>,
    Path(user_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
) -> Result<StatusCode, ApiError> {
    sidecar_only(&state, &headers, &uri, &request_id)?;
    let user_id = id_of(&user_id, &uri, &request_id)?;
    credentials::delete(state.database(), user_id)
        .await
        .map_err(|_| unavailable(&uri, &request_id))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn invalidate_credential(
    State(state): State<AgentState>,
    Path(user_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
) -> Result<StatusCode, ApiError> {
    sidecar_only(&state, &headers, &uri, &request_id)?;
    let user_id = id_of(&user_id, &uri, &request_id)?;
    credentials::mark_invalid(state.database(), user_id)
        .await
        .map_err(|_| unavailable(&uri, &request_id))?;
    Ok(StatusCode::NO_CONTENT)
}
