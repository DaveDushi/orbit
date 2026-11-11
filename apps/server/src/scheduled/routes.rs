//! `/api/v1/workspaces/{workspace_id}/routines` and `/scheduled-messages`, for the web app (cookie)
//! and for the agent (the driver's agent session, which `agent::scope` lets list and create only).
//! Like the agent's other routes, they are not in the OpenAPI contract.

use axum::extract::{Extension, Path, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::routing::{get, patch};
use axum::{Json, Router};
use orbit_platform::{Id, RequestId, TimestampMillis};
use serde_json::{Value, json};

use super::store::{
    NewRoutine, NewScheduled, RoutineChange, RoutineRecord, ScheduledChange, ScheduledError,
    ScheduledRecord, Store,
};
use crate::agent::AgentState;
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
