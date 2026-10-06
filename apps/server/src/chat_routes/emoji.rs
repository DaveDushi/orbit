//! Custom emoji: the list, upload (one multipart `file` field), delete, and the image.

use axum::Json;
use axum::extract::{Multipart, Path, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::Response;
use serde::Deserialize;
use utoipa::{IntoParams, ToSchema};

use super::images::{self, ImageRules};
use super::{Call, ChatState, RequestIdExtension};
use crate::repositories::chat::{CustomEmojiRecord, EMOJI_MAX_BYTES};
use crate::task_routes::{ApiError, ApiQuery};

pub(super) const RULES: ImageRules = ImageRules {
    max_bytes: EMOJI_MAX_BYTES,
    too_large_code: "emoji_too_large",
    too_large_detail: "The image must be at most 256 KiB.",
    invalid_code: "invalid_emoji",
};

#[derive(Deserialize, IntoParams)]
#[serde(deny_unknown_fields)]
#[into_params(parameter_in = Query)]
pub(crate) struct ChatEmojiQuery {
    /// 2 to 32 characters of `a-z 0-9 _`. Upper case letters are stored in lower case.
    name: String,
}

#[derive(ToSchema)]
#[allow(dead_code)]
pub(crate) struct ChatEmojiUploadBody {
    /// A PNG, JPEG, WebP or GIF image of at most 256 KiB.
    #[schema(value_type = String, format = Binary)]
    file: Vec<u8>,
}

#[derive(ToSchema)]
#[schema(value_type = String, format = Binary)]
#[allow(dead_code)]
pub(crate) struct ChatEmojiImage(Vec<u8>);

/// The custom emoji of the workspace, by name.
#[utoipa::path(get, path = "/api/v1/workspaces/{workspace_id}/chat/emoji", params(("workspace_id" = String, Path)), responses((status = 200, body = Vec<CustomEmojiRecord>)))]
pub(crate) async fn list_chat_emoji(
    State(state): State<ChatState>,
    Path(workspace): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
) -> Result<Json<Vec<CustomEmojiRecord>>, ApiError> {
    let call = Call::enter(&state, &headers, &uri, &workspace, &request_id).await?;
    call.read(
        state
            .chat
            .list_emoji(call.workspace_id, call.actor_id)
            .await,
    )
}

/// Adds a custom emoji. Only workspace owners and admins. A workspace has at most 200.
#[utoipa::path(post, path = "/api/v1/workspaces/{workspace_id}/chat/emoji", params(ChatEmojiQuery, ("workspace_id" = String, Path)), request_body(content = ChatEmojiUploadBody, content_type = "multipart/form-data"), responses((status = 201, body = CustomEmojiRecord)))]
pub(crate) async fn create_chat_emoji(
    State(state): State<ChatState>,
    Path(workspace): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
    ApiQuery(query): ApiQuery<ChatEmojiQuery>,
    mut multipart: Multipart,
) -> Result<(StatusCode, Json<CustomEmojiRecord>), ApiError> {
    let call = Call::enter(&state, &headers, &uri, &workspace, &request_id).await?;
    // Authorize before the body is read.
    state
        .chat
        .authorize_emoji(call.workspace_id, call.actor_id)
        .await
        .map_err(|error| call.problem(error))?;
    let (image, mime_type) = images::read_upload(&call, &mut multipart, &RULES).await?;
    let (written, _) = state
        .publish(
            call.workspace_id,
            call.actor_id,
            state.chat.create_emoji(
                call.workspace_id,
                call.actor_id,
                &query.name,
                mime_type,
                &image,
            ),
        )
        .await
        .map_err(|error| call.problem(error))?;
    Ok((StatusCode::CREATED, Json(written.value)))
}

/// Deletes a custom emoji. Only workspace owners and admins. Messages and reactions that name
/// it keep their `:name:` text.
#[utoipa::path(delete, path = "/api/v1/workspaces/{workspace_id}/chat/emoji/{emoji_id}", params(("workspace_id" = String, Path), ("emoji_id" = String, Path)), responses((status = 204)))]
pub(crate) async fn delete_chat_emoji(
    State(state): State<ChatState>,
    Path((workspace, emoji)): Path<(String, String)>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
) -> Result<StatusCode, ApiError> {
    let call = Call::enter(&state, &headers, &uri, &workspace, &request_id).await?;
    let emoji_id = call.id(&emoji)?;
    state
        .publish(
            call.workspace_id,
            call.actor_id,
            state
                .chat
                .delete_emoji(call.workspace_id, call.actor_id, emoji_id),
        )
        .await
        .map_err(|error| call.problem(error))?;
    Ok(StatusCode::NO_CONTENT)
}

/// The image of a custom emoji. It never changes, so browsers keep it.
#[utoipa::path(get, path = "/api/v1/workspaces/{workspace_id}/chat/emoji/{emoji_id}/image", params(("workspace_id" = String, Path), ("emoji_id" = String, Path)), responses((status = 200, body = ChatEmojiImage, content_type = "image/*")))]
pub(crate) async fn get_chat_emoji_image(
    State(state): State<ChatState>,
    Path((workspace, emoji)): Path<(String, String)>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
) -> Result<Response, ApiError> {
    let call = Call::enter(&state, &headers, &uri, &workspace, &request_id).await?;
    let emoji_id = call.id(&emoji)?;
    let image = state
        .chat
        .emoji_image(call.workspace_id, call.actor_id, emoji_id)
        .await
        .map_err(|error| call.problem(error))?;
    images::response(&call, &image.mime_type, image.bytes)
}
