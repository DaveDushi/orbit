//! Previews of the URLs in messages.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, Uri};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use super::{Call, ChatState, RequestIdExtension};
use crate::link_preview::LinkPreview;
use crate::task_routes::{ApiError, ApiQuery};

#[derive(Deserialize, IntoParams)]
#[serde(deny_unknown_fields)]
#[into_params(parameter_in = Query)]
pub(crate) struct ChatLinkPreviewQuery {
    /// An absolute `http(s)` URL.
    url: String,
}

#[derive(Serialize, ToSchema)]
pub(crate) struct ChatLinkPreview {
    /// `null`: the URL has nothing to show, or the server may not or cannot read it.
    preview: Option<LinkPreview>,
}

/// What the URL of a message shows as a card. The server reads the page; the answer is kept for a while.
#[utoipa::path(get, path = "/api/v1/workspaces/{workspace_id}/chat/link-preview", params(ChatLinkPreviewQuery, ("workspace_id" = String, Path)), responses((status = 200, body = ChatLinkPreview)))]
pub(crate) async fn get_chat_link_preview(
    State(state): State<ChatState>,
    Path(workspace): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
    ApiQuery(query): ApiQuery<ChatLinkPreviewQuery>,
) -> Result<Json<ChatLinkPreview>, ApiError> {
    let call = Call::enter(&state, &headers, &uri, &workspace, &request_id).await?;
    state
        .chat
        .require_member(call.workspace_id, call.actor_id)
        .await
        .map_err(|error| call.problem(error))?;
    Ok(Json(ChatLinkPreview {
        preview: state.link_previews.preview(&query.url).await,
    }))
}
