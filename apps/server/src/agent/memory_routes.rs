//! `/api/v1/workspaces/{workspace_id}/agent-memory/{team|private}/…`: the agent's memory for the
//! web app (cookie) and for the sidecar (the driver's agent session, through `agent::scope`).
//! Like the other agent routes, they are not in the OpenAPI contract.

use axum::extract::{Extension, Path, Query, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::routing::{get, post};
use axum::{Json, Router};
use orbit_platform::{Id, RequestId};
use serde::Deserialize;

use super::AgentState;
use super::memory::{
    self, MemoryError, MemoryPage, MemoryStore, Noted, Scope, Source, Summarized, Wake,
};
use crate::task_routes::{ApiError, authenticate_session};

type RequestIdExtension = Option<Extension<RequestId>>;

pub fn router(state: AgentState) -> Router {
    let base = "/api/v1/workspaces/{workspace_id}/agent-memory/{scope}";
    Router::new()
        .route(base, get(list).post(note))
        .route(&format!("{base}/wake"), get(wake))
        .route(&format!("{base}/summaries"), post(summarize))
        .route(
            &format!("{base}/summaries/{{block}}"),
            get(zoom).delete(forget),
        )
        .route(
            &format!("{base}/memories/{{position}}"),
            axum::routing::delete(delete),
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

fn not_found(uri: &Uri, request_id: &RequestIdExtension) -> ApiError {
    problem(
        StatusCode::NOT_FOUND,
        "not_found",
        "Not found.",
        uri,
        request_id,
    )
}

fn failure(error: MemoryError, uri: &Uri, request_id: &RequestIdExtension) -> ApiError {
    match error {
        MemoryError::NotFound => not_found(uri, request_id),
        MemoryError::Forbidden => problem(
            StatusCode::FORBIDDEN,
            "forbidden",
            "You cannot delete from this memory.",
            uri,
            request_id,
        ),
        MemoryError::Invalid(detail) => problem(
            StatusCode::UNPROCESSABLE_ENTITY,
            "validation_failed",
            detail,
            uri,
            request_id,
        ),
        MemoryError::Full => problem(
            StatusCode::CONFLICT,
            "agent_memory_full",
            "This memory is full. Delete old memories first.",
            uri,
            request_id,
        ),
        MemoryError::Database(error) => {
            tracing::warn!(%error, "agent memory");
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

/// The caller, the workspace, the scope and the store.
async fn context(
    state: &AgentState,
    headers: &HeaderMap,
    (workspace_id, scope): (&str, &str),
    uri: &Uri,
    request_id: &RequestIdExtension,
) -> Result<(Id, Id, Scope, MemoryStore), ApiError> {
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
        .map_err(|_| not_found(uri, request_id))?;
    let scope = Scope::parse(scope).ok_or_else(|| not_found(uri, request_id))?;
    Ok((
        session.user.id,
        workspace_id,
        scope,
        MemoryStore::new(state.database().clone()),
    ))
}

#[derive(Deserialize)]
struct ListQuery {
    query: Option<String>,
    before: Option<u64>,
    limit: Option<u64>,
}

async fn list(
    State(state): State<AgentState>,
    Path((workspace_id, scope)): Path<(String, String)>,
    Query(query): Query<ListQuery>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
) -> Result<Json<MemoryPage>, ApiError> {
    let (user_id, workspace_id, scope, store) =
        context(&state, &headers, (&workspace_id, &scope), &uri, &request_id).await?;
    store
        .list(
            workspace_id,
            user_id,
            scope,
            query.query.as_deref(),
            query.before,
            query.limit.unwrap_or(50),
        )
        .await
        .map(Json)
        .map_err(|error| failure(error, &uri, &request_id))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NoteBody {
    body: String,
    #[serde(default)]
    source: Source,
}

async fn note(
    State(state): State<AgentState>,
    Path((workspace_id, scope)): Path<(String, String)>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
    Json(body): Json<NoteBody>,
) -> Result<Json<Noted>, ApiError> {
    let (user_id, workspace_id, scope, store) =
        context(&state, &headers, (&workspace_id, &scope), &uri, &request_id).await?;
    store
        .note(workspace_id, user_id, scope, &body.body, &body.source)
        .await
        .map(Json)
        .map_err(|error| failure(error, &uri, &request_id))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SummaryBody {
    /// `lo-hi`, as the due block names it.
    block: String,
    claim: String,
    body: String,
}

async fn summarize(
    State(state): State<AgentState>,
    Path((workspace_id, scope)): Path<(String, String)>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
    Json(body): Json<SummaryBody>,
) -> Result<Json<Summarized>, ApiError> {
    let (user_id, workspace_id, scope, store) =
        context(&state, &headers, (&workspace_id, &scope), &uri, &request_id).await?;
    let block = memory::parse_block(&body.block).ok_or_else(|| {
        problem(
            StatusCode::UNPROCESSABLE_ENTITY,
            "validation_failed",
            "Name the block as it was given, for example 4-7.",
            &uri,
            &request_id,
        )
    })?;
    store
        .summarize(workspace_id, user_id, scope, block, &body.claim, &body.body)
        .await
        .map(Json)
        .map_err(|error| failure(error, &uri, &request_id))
}

#[derive(Deserialize)]
struct WakeQuery {
    /// The view's size in bytes of text.
    bytes: Option<usize>,
}

async fn wake(
    State(state): State<AgentState>,
    Path((workspace_id, scope)): Path<(String, String)>,
    Query(query): Query<WakeQuery>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
) -> Result<Json<Wake>, ApiError> {
    let (user_id, workspace_id, scope, store) =
        context(&state, &headers, (&workspace_id, &scope), &uri, &request_id).await?;
    store
        .wake(workspace_id, user_id, scope, query.bytes.unwrap_or(8_000))
        .await
        .map(Json)
        .map_err(|error| failure(error, &uri, &request_id))
}

async fn zoom(
    State(state): State<AgentState>,
    Path((workspace_id, scope, block)): Path<(String, String, String)>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
) -> Result<Json<serde_json::Value>, ApiError> {
    let (user_id, workspace_id, scope, store) =
        context(&state, &headers, (&workspace_id, &scope), &uri, &request_id).await?;
    let block = memory::parse_block(&block).ok_or_else(|| not_found(&uri, &request_id))?;
    let halves = store
        .zoom(workspace_id, user_id, scope, block)
        .await
        .map_err(|error| failure(error, &uri, &request_id))?;
    Ok(Json(serde_json::json!({ "lines": halves })))
}

async fn forget(
    State(state): State<AgentState>,
    Path((workspace_id, scope, block)): Path<(String, String, String)>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
) -> Result<StatusCode, ApiError> {
    let (user_id, workspace_id, scope, store) =
        context(&state, &headers, (&workspace_id, &scope), &uri, &request_id).await?;
    let block = memory::parse_block(&block).ok_or_else(|| not_found(&uri, &request_id))?;
    store
        .forget(workspace_id, user_id, scope, block)
        .await
        .map_err(|error| failure(error, &uri, &request_id))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete(
    State(state): State<AgentState>,
    Path((workspace_id, scope, position)): Path<(String, String, u64)>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
) -> Result<StatusCode, ApiError> {
    let (user_id, workspace_id, scope, store) =
        context(&state, &headers, (&workspace_id, &scope), &uri, &request_id).await?;
    store
        .delete(workspace_id, user_id, scope, position)
        .await
        .map_err(|error| failure(error, &uri, &request_id))?;
    Ok(StatusCode::NO_CONTENT)
}
