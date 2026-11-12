//! Chat files: upload (one multipart `file` field), download, the content of a sent file in
//! parts (for the agent), and the files of a conversation.

use std::io;

use axum::extract::{DefaultBodyLimit, Extension, Path, Query, Request, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::Response;
use axum::routing::{get, post};
use axum::{Json, Router};
use orbit_platform::{BlobStoreError, RequestId, UploadError};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use super::{Call, ChatState, RequestIdExtension};
use crate::attachment_routes::{StageFailure, ranged_download_response, stage_single_file};
use crate::file_content::{ContentQuery, ReadError, read_problem};
use crate::file_parse::{self, FileKind};
use crate::previews::{FilePreview, PreviewOutput};
use crate::repositories::chat::{ChatError, ChatFileRecord, ChatFileSource, MessageRecord};
use crate::repositories::drive::DriveNode;
use crate::task_routes::{ApiError, validation};

pub(super) fn file_router(state: ChatState) -> Router {
    let request_limit =
        usize::try_from(state.chat.uploads().limits().max_request_bytes()).unwrap_or(usize::MAX);
    Router::new()
        .route(
            "/api/v1/workspaces/{workspace_id}/chat/files",
            post(upload_chat_file),
        )
        // Multipart stays streaming; this bounds boundaries, headers, and ignored fields too.
        .layer(DefaultBodyLimit::max(request_limit))
        .route(
            "/api/v1/workspaces/{workspace_id}/chat/files/{file_id}",
            get(download_chat_file),
        )
        .route(
            "/api/v1/workspaces/{workspace_id}/chat/files/{file_id}/preview",
            get(get_chat_file_preview).post(request_chat_file_preview),
        )
        .route(
            "/api/v1/workspaces/{workspace_id}/chat/files/{file_id}/preview/{role}",
            get(get_chat_file_preview_file),
        )
        .route(
            "/api/v1/workspaces/{workspace_id}/chat/files/drive",
            post(attach_chat_drive_file),
        )
        .route(
            "/api/v1/workspaces/{workspace_id}/chat/upload-limit",
            get(get_chat_upload_limit),
        )
        .route(
            "/api/v1/workspaces/{workspace_id}/chat/conversations/{conversation_id}/drive-folder",
            post(get_chat_drive_folder),
        )
        .route(
            "/api/v1/workspaces/{workspace_id}/chat/files/{file_id}/content",
            get(get_chat_file_content),
        )
        .route(
            "/api/v1/workspaces/{workspace_id}/chat/conversations/{conversation_id}/files",
            get(list_chat_files),
        )
        .with_state(state)
}

#[derive(ToSchema)]
#[allow(dead_code)]
pub(crate) struct ChatFileUploadBody {
    #[schema(value_type = String, format = Binary)]
    file: Vec<u8>,
}

#[derive(ToSchema)]
#[schema(value_type = String, format = Binary)]
#[allow(dead_code)]
pub(crate) struct ChatFileDownload(Vec<u8>);

/// The size of an image in pixels, as the uploader measured it. Only a hint for the message
/// list to reserve space; values out of range are dropped.
#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct ChatFileSize {
    width: Option<u32>,
    height: Option<u32>,
}

/// Uploads a file for a message that is about to be sent. Pass its `id` in `file_ids` of the
/// send. A file that is not sent within a day is removed.
#[utoipa::path(post, path = "/api/v1/workspaces/{workspace_id}/chat/files", params(ChatFileSize, ("workspace_id" = String, Path)), request_body(content = ChatFileUploadBody, content_type = "multipart/form-data"), responses((status = 201, body = ChatFileRecord)))]
pub(crate) async fn upload_chat_file(
    State(state): State<ChatState>,
    Path(workspace): Path<String>,
    request: Request,
) -> Result<(StatusCode, Json<ChatFileRecord>), ApiError> {
    let uri = request.uri().clone();
    let request_id: RequestIdExtension = request
        .extensions()
        .get::<RequestId>()
        .cloned()
        .map(Extension);
    // Authorize before the body is read.
    let call = Call::enter(&state, request.headers(), &uri, &workspace, &request_id).await?;
    let Query(size) = Query::<ChatFileSize>::try_from_uri(&uri)
        .map_err(|_| call.problem(ChatError::Invalid { field: "width" }))?;
    state
        .chat
        .authorize_upload(call.workspace_id, call.actor_id)
        .await
        .map_err(|error| call.problem(error))?;
    let mut staged = stage_single_file(
        state.chat.uploads(),
        request,
        call.workspace_id,
        call.actor_id,
    )
    .await
    .map_err(|failure| match failure {
        StageFailure::TooLarge => too_large(&call.instance, call.request_id),
        StageFailure::InvalidMultipart => invalid_multipart(&call.instance, call.request_id),
        StageFailure::Upload(error) => upload_problem(error, &call.instance, call.request_id),
    })?;
    let upload = staged.upload().clone();
    match state
        .chat
        .finalize_file(
            call.workspace_id,
            call.actor_id,
            &upload,
            size.width,
            size.height,
        )
        .await
    {
        Ok(file) => {
            staged.complete();
            Ok((StatusCode::CREATED, Json(file)))
        }
        Err(error) => {
            staged.discard().await;
            Err(call.problem(error))
        }
    }
}

/// `?inline=true`: a file kept in Drive opens in the browser instead of downloading.
#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct ChatFileDownloadQuery {
    #[serde(default)]
    inline: bool,
}

/// The bytes of a chat file (one `Range` is answered with 206). A file kept in Drive sends to its bytes in the
/// bucket (a 5-minute URL).
#[utoipa::path(get, path = "/api/v1/workspaces/{workspace_id}/chat/files/{file_id}", params(("workspace_id" = String, Path), ("file_id" = String, Path), ChatFileDownloadQuery), responses((status = 200, body = ChatFileDownload, content_type = "application/octet-stream"), (status = 307, description = "To a file kept in Drive")))]
pub(crate) async fn download_chat_file(
    State(state): State<ChatState>,
    Path((workspace, file)): Path<(String, String)>,
    Query(query): Query<ChatFileDownloadQuery>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
) -> Result<Response, ApiError> {
    let call = Call::enter(&state, &headers, &uri, &workspace, &request_id).await?;
    let file_id = call.id(&file)?;
    let source = state
        .chat
        .file_source(call.workspace_id, call.actor_id, file_id)
        .await
        .map_err(|error| call.problem(error))?;
    match source {
        ChatFileSource::Drive {
            key: Some(key),
            file_name,
            ..
        } => {
            let name = (!query.inline).then_some(file_name.as_str());
            drive_redirect(&state, &call, &key, name)
        }
        ChatFileSource::Drive { key: None, .. } => Err(file_deleted(&call)),
        ChatFileSource::Stored { size_bytes, .. } => {
            let download = state
                .chat
                .download_file(call.workspace_id, call.actor_id, file_id)
                .await
                .map_err(|error| call.problem(error))?;
            let size = u64::try_from(size_bytes).unwrap_or(0);
            ranged_download_response(download, size, headers.get(axum::http::header::RANGE))
                .await
                .ok_or_else(|| unavailable(&call))
        }
    }
}

/// The preview of a chat file. 404 for a file that gets no preview.
#[utoipa::path(get, path = "/api/v1/workspaces/{workspace_id}/chat/files/{file_id}/preview", params(("workspace_id" = String, Path), ("file_id" = String, Path)), responses((status = 200, body = FilePreview)))]
pub(crate) async fn get_chat_file_preview(
    State(state): State<ChatState>,
    Path((workspace, file)): Path<(String, String)>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
) -> Result<Json<FilePreview>, ApiError> {
    chat_file_preview(
        &state,
        &workspace,
        &file,
        &headers,
        &uri,
        &request_id,
        false,
    )
    .await
}

/// Queues the preview of a video or office file that has none yet (a file from before previews).
#[utoipa::path(post, path = "/api/v1/workspaces/{workspace_id}/chat/files/{file_id}/preview", params(("workspace_id" = String, Path), ("file_id" = String, Path)), responses((status = 200, body = FilePreview)))]
pub(crate) async fn request_chat_file_preview(
    State(state): State<ChatState>,
    Path((workspace, file)): Path<(String, String)>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
) -> Result<Json<FilePreview>, ApiError> {
    chat_file_preview(&state, &workspace, &file, &headers, &uri, &request_id, true).await
}

async fn chat_file_preview(
    state: &ChatState,
    workspace: &str,
    file: &str,
    headers: &HeaderMap,
    uri: &Uri,
    request_id: &RequestIdExtension,
    request: bool,
) -> Result<Json<FilePreview>, ApiError> {
    let call = Call::enter(state, headers, uri, workspace, request_id).await?;
    let file_id = call.id(file)?;
    state
        .chat
        .file_preview(call.workspace_id, call.actor_id, file_id, request)
        .await
        .map_err(|error| call.problem(error))?
        .map(Json)
        .ok_or_else(|| call.problem(ChatError::NotFound))
}

/// A file of a chat file's preview: `play` (an MP4 every browser plays; one `Range` is answered with 206),
/// `poster` (a JPEG) or `pdf`.
#[utoipa::path(get, path = "/api/v1/workspaces/{workspace_id}/chat/files/{file_id}/preview/{role}", params(("workspace_id" = String, Path), ("file_id" = String, Path), ("role" = String, Path)), responses((status = 200, body = ChatFileDownload, content_type = "application/octet-stream"), (status = 307, description = "To the file in Drive's bucket")))]
pub(crate) async fn get_chat_file_preview_file(
    State(state): State<ChatState>,
    Path((workspace, file, role)): Path<(String, String, String)>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
) -> Result<Response, ApiError> {
    let call = Call::enter(&state, &headers, &uri, &workspace, &request_id).await?;
    let file_id = call.id(&file)?;
    if !matches!(role.as_str(), "play" | "poster" | "pdf") {
        return Err(call.problem(ChatError::NotFound));
    }
    let (output, name) = state
        .chat
        .preview_output(call.workspace_id, call.actor_id, file_id, &role)
        .await
        .map_err(|error| call.problem(error))?;
    match output {
        PreviewOutput::Object { key, .. } => drive_redirect(&state, &call, &key, None),
        PreviewOutput::Stored {
            storage_key,
            mime_type,
            size_bytes,
        } => {
            let stem = name
                .rsplit_once('.')
                .map_or(name.as_str(), |(stem, _)| stem);
            let extension = match role.as_str() {
                "play" => "mp4",
                "poster" => "jpg",
                _ => "pdf",
            };
            let download = state
                .chat
                .uploads()
                .download(|| async move {
                    Ok(orbit_platform::AuthorizedAttachment {
                        storage_key,
                        media_type: mime_type,
                        display_name: format!("{stem}.{extension}"),
                        deleted: false,
                    })
                })
                .await
                .map_err(|error| upload_problem(error, &call.instance, call.request_id))?;
            let size = u64::try_from(size_bytes).unwrap_or(0);
            ranged_download_response(download, size, headers.get(axum::http::header::RANGE))
                .await
                .ok_or_else(|| unavailable(&call))
        }
    }
}

/// How large a chat file may be, and whether larger files can go to Drive.
#[derive(Serialize, ToSchema)]
pub(crate) struct ChatUploadLimit {
    /// Larger files cannot be chat files.
    max_file_bytes: u64,
    /// Larger files go to the conversation's folder in Drive (the workspace has Drive storage).
    drive: bool,
}

#[utoipa::path(get, path = "/api/v1/workspaces/{workspace_id}/chat/upload-limit", params(("workspace_id" = String, Path)), responses((status = 200, body = ChatUploadLimit)))]
pub(crate) async fn get_chat_upload_limit(
    State(state): State<ChatState>,
    Path(workspace): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
) -> Result<Json<ChatUploadLimit>, ApiError> {
    let call = Call::enter(&state, &headers, &uri, &workspace, &request_id).await?;
    state
        .chat
        .authorize_upload(call.workspace_id, call.actor_id)
        .await
        .map_err(|error| call.problem(error))?;
    // Guests have no Drive: their files stay within the chat limit.
    let drive = match &state.drive {
        Some(drive) if drive.bucket(call.workspace_id).is_ok() => drive
            .repository()
            .require_member(call.workspace_id, call.actor_id)
            .await
            .is_ok(),
        _ => false,
    };
    Ok(Json(ChatUploadLimit {
        max_file_bytes: state.chat.uploads().limits().max_file_bytes(),
        drive,
    }))
}

/// The conversation's folder in Drive (`Uploads/#channel`, or a private folder of a direct message's members),
/// made when first needed. Upload a file that is too large for chat there with the Drive upload routes, then
/// attach it with `POST …/chat/files/drive`.
#[utoipa::path(post, path = "/api/v1/workspaces/{workspace_id}/chat/conversations/{conversation_id}/drive-folder", params(("workspace_id" = String, Path), ("conversation_id" = String, Path)), responses((status = 200, body = DriveNode)))]
pub(crate) async fn get_chat_drive_folder(
    State(state): State<ChatState>,
    Path((workspace, conversation)): Path<(String, String)>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
) -> Result<Json<DriveNode>, ApiError> {
    let call = Call::enter(&state, &headers, &uri, &workspace, &request_id).await?;
    let conversation_id = call.id(&conversation)?;
    let drive = drive_of(&state, &call)?;
    drive
        .repository()
        .chat_folder(
            call.workspace_id,
            call.actor_id,
            conversation_id,
            orbit_platform::TimestampMillis::now(),
        )
        .await
        .map(Json)
        .map_err(|error| crate::drive_routes::problem(error, &call.instance, call.request_id))
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct AttachChatDriveFile {
    /// A file the caller just uploaded to the conversation's Drive folder.
    node_id: String,
}

/// Makes a file the caller uploaded to a conversation's Drive folder a chat file for the next message: pass its
/// `id` in `file_ids` of the send, in that conversation.
#[utoipa::path(post, path = "/api/v1/workspaces/{workspace_id}/chat/files/drive", params(("workspace_id" = String, Path)), request_body = AttachChatDriveFile, responses((status = 201, body = ChatFileRecord)))]
pub(crate) async fn attach_chat_drive_file(
    State(state): State<ChatState>,
    Path(workspace): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
    crate::task_routes::ApiJson(body): crate::task_routes::ApiJson<AttachChatDriveFile>,
) -> Result<(StatusCode, Json<ChatFileRecord>), ApiError> {
    let call = Call::enter(&state, &headers, &uri, &workspace, &request_id).await?;
    let node_id = call.id(&body.node_id)?;
    state
        .chat
        .attach_drive_upload(call.workspace_id, call.actor_id, node_id)
        .await
        .map(|file| (StatusCode::CREATED, Json(file)))
        .map_err(|error| call.problem(error))
}

fn drive_of<'a>(
    state: &'a ChatState,
    call: &Call<'_>,
) -> Result<&'a crate::drive::DriveService, ApiError> {
    state.drive.as_ref().ok_or_else(|| {
        crate::drive_routes::problem(
            crate::repositories::drive::DriveError::NoStorage,
            &call.instance,
            call.request_id,
        )
    })
}

/// Sends to an object of the workspace's Drive bucket; `download_name` makes the browser save it.
fn drive_redirect(
    state: &ChatState,
    call: &Call<'_>,
    key: &str,
    download_name: Option<&str>,
) -> Result<Response, ApiError> {
    let url = drive_of(state, call)?
        .object_url(call.workspace_id, key, download_name)
        .map_err(|error| crate::drive_routes::problem(error, &call.instance, call.request_id))?;
    Ok(axum::response::IntoResponse::into_response(
        axum::response::Redirect::temporary(&url),
    ))
}

fn file_deleted(call: &Call<'_>) -> ApiError {
    ApiError::new(
        StatusCode::GONE,
        "file_deleted",
        "File deleted",
        "The file was moved to the trash or deleted in Drive.",
        &call.instance,
        call.request_id,
    )
}

fn unavailable(call: &Call<'_>) -> ApiError {
    call.problem(ChatError::Unavailable(sqlx::Error::Protocol(
        "the download response could not be built".to_owned(),
    )))
}

/// A chat file as its content shows it. Chat files have no versions and are read-only.
#[derive(Serialize, ToSchema)]
pub(crate) struct ChatContentFile {
    #[schema(value_type = String)]
    id: orbit_platform::Id,
    file_name: String,
    mime_type: String,
    kind: FileKind,
    size_bytes: i64,
    #[schema(value_type = String)]
    message_id: orbit_platform::Id,
    #[schema(value_type = String)]
    conversation_id: orbit_platform::Id,
}

/// A chat file and one part of its content: the fields of a Docs page file's content.
#[derive(Serialize, ToSchema)]
pub(crate) struct ChatFileContent {
    file: ChatContentFile,
    /// Sheet names of a workbook.
    sheets: Vec<String>,
    sheet: Option<String>,
    rows: Option<Vec<Vec<String>>>,
    row_numbers: Option<Vec<u64>>,
    /// Cells among `rows` that hold a formula, as `B2`.
    formula_cells: Vec<String>,
    text: Option<String>,
    /// Where the next part starts; `null` at the end.
    next_offset: Option<u64>,
    /// A PDF or Word file without text, such as a scan.
    no_text: bool,
}

/// One part of a file of a sent message, as rows or text, with the query of a Docs page file's
/// content. Only for a person who can read the message; an upload that waits for its message
/// and a file of a deleted message are not found.
#[utoipa::path(get, path = "/api/v1/workspaces/{workspace_id}/chat/files/{file_id}/content", params(("workspace_id" = String, Path), ("file_id" = String, Path), ContentQuery), responses((status = 200, body = ChatFileContent)))]
pub(crate) async fn get_chat_file_content(
    State(state): State<ChatState>,
    Path((workspace, file)): Path<(String, String)>,
    Query(query): Query<ContentQuery>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
) -> Result<Json<ChatFileContent>, ApiError> {
    let call = Call::enter(&state, &headers, &uri, &workspace, &request_id).await?;
    let file_id = call.id(&file)?;
    let sent = state
        .chat
        .sent_file(call.workspace_id, call.actor_id, file_id)
        .await
        .map_err(|error| call.problem(error))?;
    if sent.file.deleted {
        return Err(ApiError::new(
            StatusCode::GONE,
            "file_deleted",
            "File deleted",
            "An admin deleted this stored file.",
            &call.instance,
            call.request_id,
        ));
    }
    let kind = file_parse::classify(&sent.file.file_name, &sent.file.mime_type);
    let parsed = state
        .reader
        .read(
            kind,
            &sent.file.file_name,
            u64::try_from(sent.file.size_bytes).unwrap_or(u64::MAX),
            query,
            state.chat.uploads().read_blob(&sent.storage_key),
        )
        .await
        .map_err(|error| match error {
            ReadError::Upload(error) => upload_problem(error, &call.instance, call.request_id),
            error => read_problem(error, &call.instance, call.request_id),
        })?
        .unwrap_or_default();
    Ok(Json(ChatFileContent {
        file: ChatContentFile {
            id: sent.file.id,
            file_name: sent.file.file_name,
            mime_type: sent.file.mime_type,
            kind,
            size_bytes: sent.file.size_bytes,
            message_id: sent.message_id,
            conversation_id: sent.conversation_id,
        },
        sheets: parsed.sheets,
        sheet: parsed.sheet,
        rows: parsed.rows,
        row_numbers: parsed.row_numbers,
        formula_cells: parsed.formula_cells,
        text: parsed.text,
        next_offset: parsed.next_offset,
        no_text: parsed.no_text,
    }))
}

/// The messages of a conversation that have files, newest first (at most 100).
#[utoipa::path(get, path = "/api/v1/workspaces/{workspace_id}/chat/conversations/{conversation_id}/files", params(("workspace_id" = String, Path), ("conversation_id" = String, Path)), responses((status = 200, body = Vec<MessageRecord>)))]
pub(crate) async fn list_chat_files(
    State(state): State<ChatState>,
    Path((workspace, conversation)): Path<(String, String)>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
) -> Result<Json<Vec<MessageRecord>>, ApiError> {
    let call = Call::enter(&state, &headers, &uri, &workspace, &request_id).await?;
    let conversation_id = call.id(&conversation)?;
    call.read(
        state
            .chat
            .list_files(call.workspace_id, call.actor_id, conversation_id)
            .await,
    )
}

fn too_large(instance: &str, request_id: Option<&Extension<RequestId>>) -> ApiError {
    ApiError::new(
        StatusCode::PAYLOAD_TOO_LARGE,
        "upload_too_large",
        "Upload too large",
        "The upload exceeds the configured size limit.",
        instance,
        request_id,
    )
}

fn invalid_multipart(instance: &str, request_id: Option<&Extension<RequestId>>) -> ApiError {
    ApiError::new(
        StatusCode::BAD_REQUEST,
        "invalid_multipart",
        "Invalid multipart upload",
        "The request must contain exactly one file field named \"file\".",
        instance,
        request_id,
    )
}

pub(super) fn upload_problem(
    error: UploadError,
    instance: &str,
    request_id: Option<&Extension<RequestId>>,
) -> ApiError {
    match error {
        UploadError::FileTooLarge { .. } | UploadError::RequestTooLarge { .. } => {
            too_large(instance, request_id)
        }
        UploadError::StorageQuotaExceeded { .. } => {
            crate::attachment_routes::storage_quota_exceeded(instance, request_id)
        }
        UploadError::InvalidDisplayName => validation("file", instance, request_id),
        UploadError::BlobStore(BlobStoreError::Io { source, .. })
            if source.kind() == io::ErrorKind::FileTooLarge =>
        {
            too_large(instance, request_id)
        }
        UploadError::BlobStore(BlobStoreError::Io { source, .. })
            if source.kind() == io::ErrorKind::InvalidData =>
        {
            invalid_multipart(instance, request_id)
        }
        error => ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            "Internal server error",
            "An unexpected error occurred. Use the request ID when contacting support.",
            instance,
            request_id,
        )
        .cause(error),
    }
}
