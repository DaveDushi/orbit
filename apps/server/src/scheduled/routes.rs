//! `/api/v1/workspaces/{workspace_id}/routines` and `/scheduled-messages`, for the web app (cookie)
//! and for the agent (the driver's agent session, which `agent::scope` lets only list there).
//! The agent creates through `/agent-tools/scheduled`, in a run of its driver: a reminder (a DM
//! from the agent to the driver) at once, a routine or a message posted as the driver only after
//! the driver's "yes" in the thread to exactly that action (`crate::agent::integrations::approve`),
//! so text in a task, page, file or message cannot make the agent post as them. A routine's later
//! runs need no new approval. Like the agent's other routes, they are not in the OpenAPI contract.

use axum::extract::{Extension, Path, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use orbit_platform::{Id, RequestId, TimestampMillis};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::store::{
    NewRoutine, NewScheduled, RoutineChange, RoutineRecord, ScheduledChange, ScheduledError,
    ScheduledRecord, Store,
};
use crate::agent::AgentState;
use crate::agent::integrations::{
    Approval, Approves, approve, canonical, driver, hash, require_running, unavailable,
};
use crate::task_routes::{ApiError, ApiJson, authenticate_session};

type RequestIdExtension = Option<Extension<RequestId>>;

pub fn router(state: AgentState) -> Router {
    let base = "/api/v1/workspaces/{workspace_id}";
    Router::new()
        .route(
            &format!("{base}/routines"),
            get(list_routines).post(create_routine),
        )
        .route(
            &format!("{base}/routines/{{routine_id}}"),
            patch(update_routine).delete(delete_routine),
        )
        .route(
            &format!("{base}/scheduled-messages"),
            get(list_scheduled).post(create_scheduled),
        )
        .route(
            &format!("{base}/scheduled-messages/{{scheduled_id}}"),
            patch(update_scheduled).delete(delete_scheduled),
        )
        .route(&format!("{base}/agent-tools/scheduled"), post(agent_call))
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

fn failure(error: ScheduledError, uri: &Uri, request_id: &RequestIdExtension) -> ApiError {
    match error {
        ScheduledError::NotFound => problem(
            StatusCode::NOT_FOUND,
            "not_found",
            "Not found.",
            uri,
            request_id,
        ),
        ScheduledError::Forbidden(detail) => {
            problem(StatusCode::FORBIDDEN, "forbidden", detail, uri, request_id)
        }
        ScheduledError::Invalid(detail) => problem(
            StatusCode::UNPROCESSABLE_ENTITY,
            "validation_failed",
            detail,
            uri,
            request_id,
        ),
        ScheduledError::Limit(detail) => problem(
            StatusCode::CONFLICT,
            "limit_reached",
            detail,
            uri,
            request_id,
        ),
        ScheduledError::AgentOff => problem(
            StatusCode::CONFLICT,
            "agent_disabled",
            "The agent is not set up on this server.",
            uri,
            request_id,
        ),
        ScheduledError::Database(error) => {
            tracing::warn!(%error, "routines and scheduled messages");
            problem(
                StatusCode::SERVICE_UNAVAILABLE,
                "unavailable",
                "Try again in a moment.",
                uri,
                request_id,
            )
        }
    }
}

/// The caller and the workspace.
async fn caller(
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
    let workspace_id = workspace_id
        .parse()
        .map_err(|_| failure(ScheduledError::NotFound, uri, request_id))?;
    Ok((session.user.id, workspace_id))
}

fn id(text: &str, uri: &Uri, request_id: &RequestIdExtension) -> Result<Id, ApiError> {
    text.parse()
        .map_err(|_| failure(ScheduledError::NotFound, uri, request_id))
}

fn store(state: &AgentState) -> Store {
    Store::new(state.database().clone())
}

async fn list_routines(
    State(state): State<AgentState>,
    Path(workspace_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
) -> Result<Json<Value>, ApiError> {
    let (user_id, workspace_id) =
        caller(&state, &headers, &workspace_id, &uri, &request_id).await?;
    let routines = store(&state)
        .list_routines(workspace_id, user_id)
        .await
        .map_err(|error| failure(error, &uri, &request_id))?;
    Ok(Json(json!({ "routines": routines })))
}

async fn create_routine(
    State(state): State<AgentState>,
    Path(workspace_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
    ApiJson(body): ApiJson<NewRoutine>,
) -> Result<Json<RoutineRecord>, ApiError> {
    let (user_id, workspace_id) =
        caller(&state, &headers, &workspace_id, &uri, &request_id).await?;
    if state.config.is_none() {
        return Err(failure(ScheduledError::AgentOff, &uri, &request_id));
    }
    // An agent makes routines for itself: its session decides the agent, not the body.
    let session = authenticate_session(
        &state.identity,
        state.cookie_mode,
        &headers,
        uri.path(),
        request_id.as_ref(),
    )
    .await?;
    let session_agent = crate::agent::sessions::agent_of(state.database(), session.id)
        .await
        .map_err(|error| failure(ScheduledError::Database(error), &uri, &request_id))?;
    store(&state)
        .create_routine(
            &state.chat,
            workspace_id,
            user_id,
            body,
            session_agent,
            TimestampMillis::now(),
        )
        .await
        .map(Json)
        .map_err(|error| failure(error, &uri, &request_id))
}

async fn update_routine(
    State(state): State<AgentState>,
    Path((workspace_id, routine_id)): Path<(String, String)>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
    ApiJson(body): ApiJson<RoutineChange>,
) -> Result<Json<RoutineRecord>, ApiError> {
    let (user_id, workspace_id) =
        caller(&state, &headers, &workspace_id, &uri, &request_id).await?;
    let routine_id = id(&routine_id, &uri, &request_id)?;
    store(&state)
        .update_routine(
            workspace_id,
            user_id,
            routine_id,
            body,
            TimestampMillis::now(),
        )
        .await
        .map(Json)
        .map_err(|error| failure(error, &uri, &request_id))
}

async fn delete_routine(
    State(state): State<AgentState>,
    Path((workspace_id, routine_id)): Path<(String, String)>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
) -> Result<StatusCode, ApiError> {
    let (user_id, workspace_id) =
        caller(&state, &headers, &workspace_id, &uri, &request_id).await?;
    let routine_id = id(&routine_id, &uri, &request_id)?;
    store(&state)
        .delete_routine(workspace_id, user_id, routine_id)
        .await
        .map(|()| StatusCode::NO_CONTENT)
        .map_err(|error| failure(error, &uri, &request_id))
}

async fn list_scheduled(
    State(state): State<AgentState>,
    Path(workspace_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
) -> Result<Json<Value>, ApiError> {
    let (user_id, workspace_id) =
        caller(&state, &headers, &workspace_id, &uri, &request_id).await?;
    let scheduled = store(&state)
        .list_scheduled(workspace_id, user_id)
        .await
        .map_err(|error| failure(error, &uri, &request_id))?;
    Ok(Json(json!({ "scheduled": scheduled })))
}

async fn create_scheduled(
    State(state): State<AgentState>,
    Path(workspace_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
    ApiJson(body): ApiJson<NewScheduled>,
) -> Result<Json<ScheduledRecord>, ApiError> {
    let (user_id, workspace_id) =
        caller(&state, &headers, &workspace_id, &uri, &request_id).await?;
    // A reminder comes from the agent.
    if body.kind == "reminder" && state.config.is_none() {
        return Err(failure(ScheduledError::AgentOff, &uri, &request_id));
    }
    store(&state)
        .create_scheduled(workspace_id, user_id, body, TimestampMillis::now())
        .await
        .map(Json)
        .map_err(|error| failure(error, &uri, &request_id))
}

async fn update_scheduled(
    State(state): State<AgentState>,
    Path((workspace_id, scheduled_id)): Path<(String, String)>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
    ApiJson(body): ApiJson<ScheduledChange>,
) -> Result<Json<ScheduledRecord>, ApiError> {
    let (user_id, workspace_id) =
        caller(&state, &headers, &workspace_id, &uri, &request_id).await?;
    let scheduled_id = id(&scheduled_id, &uri, &request_id)?;
    store(&state)
        .update_scheduled(
            workspace_id,
            user_id,
            scheduled_id,
            body,
            TimestampMillis::now(),
        )
        .await
        .map(Json)
        .map_err(|error| failure(error, &uri, &request_id))
}

async fn delete_scheduled(
    State(state): State<AgentState>,
    Path((workspace_id, scheduled_id)): Path<(String, String)>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
) -> Result<StatusCode, ApiError> {
    let (user_id, workspace_id) =
        caller(&state, &headers, &workspace_id, &uri, &request_id).await?;
    let scheduled_id = id(&scheduled_id, &uri, &request_id)?;
    store(&state)
        .delete_scheduled(workspace_id, user_id, scheduled_id)
        .await
        .map(|()| StatusCode::NO_CONTENT)
        .map_err(|error| failure(error, &uri, &request_id))
}

// ---------------------------------------------------------------------------------------------
// The agent

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentCall {
    /// `schedule_routine`, `send_later` or `remind_me`.
    tool: String,
    #[serde(default)]
    arguments: Map<String, Value>,
    /// The thread of the run: approvals belong to it.
    conversation_id: String,
    root_message_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MessageArgs {
    conversation_id: Id,
    body: String,
    send_at: String,
    #[serde(default)]
    time_zone: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReminderArgs {
    body: String,
    send_at: String,
    #[serde(default)]
    time_zone: Option<String>,
}

fn refused(detail: &str) -> Json<Value> {
    Json(json!({ "status": "refused", "detail": detail }))
}

/// A store error as the agent reads it; only a database error is an HTTP error.
fn refusal(
    error: ScheduledError,
    uri: &Uri,
    request_id: &RequestIdExtension,
) -> Result<Json<Value>, ApiError> {
    match error {
        ScheduledError::Database(_) => Err(failure(error, uri, request_id)),
        ScheduledError::NotFound => Ok(refused("Not found.")),
        ScheduledError::AgentOff => Ok(refused("The agent is not set up on this server.")),
        other => Ok(refused(&other.to_string())),
    }
}

fn created(label: &str, record: impl Serialize) -> Json<Value> {
    Json(json!({ "status": "ok", "label": label, "result": record }))
}

/// `None` when the driver approved exactly `input` for `tool` in this thread (the approval is now
/// used); else the question to ask them.
async fn approval_question(
    state: &AgentState,
    (workspace_id, user_id): (Id, Id),
    thread: (&str, Option<&str>),
    tool: &str,
    action: String,
    input: &impl Serialize,
    (uri, request_id): (&Uri, &RequestIdExtension),
) -> Result<Option<Json<Value>>, ApiError> {
    let input = serde_json::to_value(input).unwrap_or(Value::Null);
    let workspace = workspace_id.to_string();
    let approval = approve(
        state.database(),
        Approves::Scheduled(&workspace),
        user_id,
        thread,
        tool,
        &hash(&canonical(&input)),
    )
    .await
    .map_err(unavailable(uri, request_id))?;
    Ok(match approval {
        Approval::Granted { .. } => None,
        Approval::Needed { id, expires_at } => Some(Json(json!({
            "status": "approval_required",
            "approval_id": id,
            "action": action,
            "expires_at": expires_at,
        }))),
    })
}

async fn agent_call(
    State(state): State<AgentState>,
    Path(workspace_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
    custom: Option<Extension<crate::agent::profiles::AgentTools>>,
    Json(body): Json<AgentCall>,
) -> Result<Json<Value>, ApiError> {
    let (user_id, workspace_id, _role) =
        driver(&state, &headers, &workspace_id, &uri, &request_id).await?;
    // A custom agent runs only the scheduling tools it has.
    if let Some(Extension(crate::agent::profiles::AgentTools(tools))) = &custom
        && !tools.iter().any(|own| *own == body.tool)
    {
        return Ok(refused("This agent's tools do not include this."));
    }
    let thread = (
        body.conversation_id.as_str(),
        body.root_message_id.as_deref(),
    );
    require_running(
        state.database(),
        (workspace_id, user_id),
        thread,
        &uri,
        &request_id,
    )
    .await?;
    let store = store(&state);
    let now = TimestampMillis::now();
    let caller = (workspace_id, user_id);
    let arguments = Value::Object(body.arguments);
    match body.tool.as_str() {
        // Only for the driver, from the agent: nobody else reads it.
        "remind_me" => {
            let Ok(args) = serde_json::from_value::<ReminderArgs>(arguments) else {
                return Ok(refused("Give body, send_at and time_zone only."));
            };
            let input = NewScheduled {
                kind: "reminder".to_owned(),
                conversation_id: None,
                body: args.body,
                send_at: args.send_at,
                time_zone: args.time_zone,
            };
            match store
                .create_scheduled(workspace_id, user_id, input, now)
                .await
            {
                Ok(record) => Ok(created("reminder scheduled", record)),
                Err(error) => refusal(error, &uri, &request_id),
            }
        }
        "send_later" => {
            let Ok(args) = serde_json::from_value::<MessageArgs>(arguments) else {
                return Ok(refused(
                    "Give conversation_id, body, send_at and time_zone only.",
                ));
            };
            let input = NewScheduled {
                kind: "message".to_owned(),
                conversation_id: Some(args.conversation_id),
                body: args.body,
                send_at: args.send_at,
                time_zone: args.time_zone,
            };
            let preview = match store
                .preview_message(workspace_id, user_id, input, now)
                .await
            {
                Ok(preview) => preview,
                Err(error) => return refusal(error, &uri, &request_id),
            };
            if let Some(question) = approval_question(
                &state,
                caller,
                thread,
                "send_later",
                preview.action,
                &preview.input,
                (&uri, &request_id),
            )
            .await?
            {
                return Ok(question);
            }
            match store
                .create_scheduled(workspace_id, user_id, preview.input, now)
                .await
            {
                Ok(record) => Ok(created("message scheduled", record)),
                Err(error) => refusal(error, &uri, &request_id),
            }
        }
        "schedule_routine" => {
            let Ok(input) = serde_json::from_value::<NewRoutine>(arguments) else {
                return Ok(refused(
                    "Give name, prompt, schedule, time_zone and conversation_id only.",
                ));
            };
            // The routine is the session's agent's: the default agent or this custom agent.
            let session = authenticate_session(
                &state.identity,
                state.cookie_mode,
                &headers,
                uri.path(),
                request_id.as_ref(),
            )
            .await?;
            let agent = crate::agent::sessions::agent_of(state.database(), session.id)
                .await
                .map_err(unavailable(&uri, &request_id))?
                .flatten();
            let preview = match store
                .preview_routine(workspace_id, user_id, input, agent, now)
                .await
            {
                Ok(preview) => preview,
                Err(error) => return refusal(error, &uri, &request_id),
            };
            if let Some(question) = approval_question(
                &state,
                caller,
                thread,
                "schedule_routine",
                preview.action,
                &preview.input,
                (&uri, &request_id),
            )
            .await?
            {
                return Ok(question);
            }
            match store
                .create_routine(
                    &state.chat,
                    workspace_id,
                    user_id,
                    preview.input,
                    Some(agent),
                    now,
                )
                .await
            {
                Ok(record) => Ok(created("routine created", record)),
                Err(error) => refusal(error, &uri, &request_id),
            }
        }
        _ => Ok(refused("Unknown tool.")),
    }
}
