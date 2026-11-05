//! Orbit Drive routes under `/api/v1/workspaces/{workspace_id}/drive`: the folder tree, trash, search, download
//! URLs and uploads. Every route needs a session of a workspace member; URLs are signed only after that check
//! ([`crate::drive`]).

use std::sync::Arc;

use axum::extract::{Extension, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use orbit_platform::{Id, RequestId, TimestampMillis};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::auth_routes::CookieMode;
use crate::drive::{DOWNLOAD_URL_LIFETIME, DriveService, StartUpload, UPLOAD_URL_LIFETIME};
use crate::repositories::drive::{
    DriveEntryPage, DriveError, DriveNode, DriveNodeDetail, DriveNodeKind, DriveNodePage,
    DriveSearch, DriveSort, DriveUpload, MediaFacts, OnConflict, Paging, SortOrder,
};
use crate::repositories::identity::IdentityRepository;
use crate::task_routes::{ApiError, ApiJson, ApiQuery, authenticate_session, request_id_value};

type RequestIdExtension = Option<Extension<RequestId>>;

#[derive(Clone)]
pub struct DriveState {
    identity: Arc<IdentityRepository>,
    drive: DriveService,
    cookie_mode: CookieMode,
}

impl DriveState {
    #[must_use]
    pub fn new(
        identity: Arc<IdentityRepository>,
        drive: DriveService,
        cookie_mode: CookieMode,
    ) -> Self {
        Self {
            identity,
            drive,
            cookie_mode,
        }
    }
}

pub fn drive_router(state: DriveState) -> Router {
    const BASE: &str = "/api/v1/workspaces/{workspace_id}/drive";
    let route = |path: &str| format!("{BASE}{path}");
    Router::new()
        .route(&route("/nodes"), get(list_drive_nodes))
        .route(&route("/folders"), post(create_drive_folder))
        .route(
            &route("/nodes/{node_id}"),
            get(get_drive_node).delete(purge_drive_node),
        )
        .route(&route("/nodes/{node_id}/rename"), post(rename_drive_node))
        .route(&route("/nodes/{node_id}/move"), post(move_drive_node))
        .route(&route("/nodes/{node_id}/trash"), post(trash_drive_node))
        .route(&route("/nodes/{node_id}/restore"), post(restore_drive_node))
        .route(
            &route("/nodes/{node_id}/files"),
            get(list_drive_folder_files),
        )
        .route(&route("/nodes/{node_id}/download"), get(get_drive_download))
        .route(&route("/nodes/{node_id}/media"), post(begin_drive_media))
        .route(
            &route("/nodes/{node_id}/media/complete"),
            post(complete_drive_media),
        )
        .route(&route("/trash"), get(list_drive_trash))
        .route(&route("/trash/empty"), post(empty_drive_trash))
        .route(&route("/search"), get(search_drive))
        .route(&route("/downloads"), post(create_drive_downloads))
        .route(
            &route("/uploads"),
            get(list_drive_uploads).post(start_drive_upload),
        )
        .route(
            &route("/uploads/{upload_id}"),
            get(get_drive_upload).delete(cancel_drive_upload),
        )
        .route(
            &route("/uploads/{upload_id}/parts"),
            post(sign_drive_upload_parts),
        )
        .route(
            &route("/uploads/{upload_id}/complete"),
            post(complete_drive_upload),
        )
        .with_state(state)
}

#[derive(Deserialize, IntoParams, ToSchema)]
#[into_params(parameter_in = Query)]
#[serde(deny_unknown_fields)]
pub struct DriveListQuery {
    /// The folder; absent for the top level.
    parent_id: Option<String>,
    #[serde(default)]
    sort: DriveSort,
    #[serde(default)]
    order: SortOrder,
    offset: Option<i64>,
    /// 1 to 500, default 100.
    limit: Option<i64>,
}

#[derive(Deserialize, IntoParams, ToSchema)]
#[into_params(parameter_in = Query)]
#[serde(deny_unknown_fields)]
pub struct DrivePageQuery {
    offset: Option<i64>,
    limit: Option<i64>,
}

#[derive(Deserialize, IntoParams, ToSchema)]
#[into_params(parameter_in = Query)]
#[serde(deny_unknown_fields)]
pub struct DriveSearchQuery {
    /// Part of the name, any case.
    q: Option<String>,
    kind: Option<DriveNodeKind>,
    /// Start of the MIME type, such as `image/` or `application/pdf`.
    mime: Option<String>,
    #[param(value_type = Option<String>, format = DateTime)]
    #[schema(value_type = Option<String>, format = DateTime)]
    updated_after: Option<TimestampMillis>,
    #[param(value_type = Option<String>, format = DateTime)]
    #[schema(value_type = Option<String>, format = DateTime)]
    updated_before: Option<TimestampMillis>,
    offset: Option<i64>,
    limit: Option<i64>,
}

#[derive(Deserialize, IntoParams, ToSchema)]
#[into_params(parameter_in = Query)]
#[serde(deny_unknown_fields)]
pub struct DriveDownloadQuery {
    /// Let the browser show the file (preview) instead of saving it.
    #[serde(default)]
    inline: bool,
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateDriveFolder {
    /// `null` or absent for the top level.
    parent_id: Option<String>,
    name: String,
    /// `fail` (default) or `rename`.
    #[serde(default)]
    on_conflict: OnConflict,
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RenameDriveNode {
    name: String,
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct MoveDriveNode {
    /// The target folder; `null` for the top level.
    parent_id: Option<String>,
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateDriveDownloads {
    /// At most 100 file ids; ids that are not live files are left out of the answer.
    node_ids: Vec<String>,
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct StartDriveUpload {
    /// The target folder; `null` or absent for the top level.
    parent_id: Option<String>,
    name: String,
    /// Bytes.
    size: i64,
    /// Default `application/octet-stream`.
    mime: Option<String>,
    #[serde(default)]
    on_conflict: OnConflict,
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SignDriveUploadParts {
    /// At most 100 numbers from 1 to `part_count`.
    part_numbers: Vec<u32>,
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct BeginDriveMedia {
    /// Pixels, as the browser measured them.
    width: Option<i64>,
    height: Option<i64>,
    /// Videos.
    duration_ms: Option<i64>,
    /// The browser could not read the file: it gets no thumbnail.
    #[serde(default)]
    failed: bool,
}

/// Where the browser PUTs the thumbnail (and a video's poster); then it calls `media/complete`.
#[derive(Serialize, ToSchema)]
pub struct DriveMediaUpload {
    thumbnail_put_url: Option<String>,
    poster_put_url: Option<String>,
    #[schema(value_type = String, format = DateTime)]
    expires_at: TimestampMillis,
}

#[derive(Serialize, ToSchema)]
pub struct DriveDownload {
    url: String,
    #[schema(value_type = String, format = DateTime)]
    expires_at: TimestampMillis,
}

#[derive(Serialize, ToSchema)]
pub struct DriveDownloadUrl {
    #[schema(value_type = String)]
    node_id: Id,
    url: String,
}

#[derive(Serialize, ToSchema)]
pub struct DriveDownloads {
    items: Vec<DriveDownloadUrl>,
    #[schema(value_type = String, format = DateTime)]
    expires_at: TimestampMillis,
}

/// An upload in progress. A single-PUT upload (`part_size` null) sends the whole file to `url` with one PUT;
/// a multipart upload asks for part URLs. Either then calls `complete`.
#[derive(Serialize, ToSchema)]
pub struct DriveUploadStarted {
    upload: DriveUploadView,
    /// The PUT URL of a single-PUT upload.
    url: Option<String>,
    #[schema(value_type = String, format = DateTime)]
    expires_at: TimestampMillis,
}

#[derive(Serialize, ToSchema)]
pub struct DriveUploadView {
    #[serde(flatten)]
    upload: DriveUpload,
    part_count: i64,
}

#[derive(Serialize, ToSchema)]
pub struct DriveUploadList {
    items: Vec<DriveUploadView>,
}

#[derive(Serialize, ToSchema)]
pub struct DriveUploadedPart {
    part_number: u32,
    size: u64,
}

/// What the bucket already has: after a reload the browser sends only the missing parts.
#[derive(Serialize, ToSchema)]
pub struct DriveUploadStatus {
    upload: DriveUploadView,
    uploaded_parts: Vec<DriveUploadedPart>,
}

#[derive(Serialize, ToSchema)]
pub struct DrivePartUrl {
    part_number: u32,
    url: String,
}

#[derive(Serialize, ToSchema)]
pub struct DrivePartUrls {
    items: Vec<DrivePartUrl>,
    #[schema(value_type = String, format = DateTime)]
    expires_at: TimestampMillis,
}

#[derive(Serialize, ToSchema)]
pub struct DriveTrashEmptied {
    /// Files and folders deleted forever.
    deleted: u64,
}

/// The session's user and the workspace of the path.
struct Caller<'a> {
    user_id: Id,
    workspace_id: Id,
    instance: String,
    request_id: Option<&'a Extension<RequestId>>,
}

impl Caller<'_> {
    fn fail(&self, error: DriveError) -> ApiError {
        problem(error, &self.instance, self.request_id)
    }

    fn id(&self, value: &str) -> Result<Id, ApiError> {
        value.parse().map_err(|_| self.fail(DriveError::NotFound))
    }

    fn optional_id(&self, value: Option<&str>) -> Result<Option<Id>, ApiError> {
        value.map(|value| self.id(value)).transpose()
    }
}

async fn caller<'a>(
    state: &DriveState,
    headers: &HeaderMap,
    workspace: &str,
    instance: String,
    request_id: Option<&'a Extension<RequestId>>,
) -> Result<Caller<'a>, ApiError> {
    let session = authenticate_session(
        &state.identity,
        state.cookie_mode,
        headers,
        &instance,
        request_id,
    )
    .await?;
    let workspace_id = workspace
        .parse()
        .map_err(|_| problem(DriveError::NotFound, &instance, request_id))?;
    Ok(Caller {
        user_id: session.user.id,
        workspace_id,
        instance,
        request_id,
    })
}

fn expires_in(lifetime: std::time::Duration) -> TimestampMillis {
    TimestampMillis::from_millis(
        TimestampMillis::now().as_millis()
            + i64::try_from(lifetime.as_millis()).unwrap_or(i64::MAX),
    )
}

fn upload_view(upload: DriveUpload) -> DriveUploadView {
    DriveUploadView {
        part_count: upload.part_count(),
        upload,
    }
}

/// The live items of a folder (or the top level): folders first.
#[utoipa::path(get, path = "/api/v1/workspaces/{workspace_id}/drive/nodes", params(("workspace_id" = String, Path), DriveListQuery), responses((status = 200, body = DriveNodePage)))]
async fn list_drive_nodes(
    State(state): State<DriveState>,
    Path(workspace): Path<String>,
    ApiQuery(query): ApiQuery<DriveListQuery>,
    headers: HeaderMap,
    request_id: RequestIdExtension,
) -> Result<Json<DriveNodePage>, ApiError> {
    let instance = format!("/api/v1/workspaces/{workspace}/drive/nodes");
    let caller = caller(&state, &headers, &workspace, instance, request_id.as_ref()).await?;
    let parent_id = caller.optional_id(query.parent_id.as_deref())?;
    state
        .drive
        .repository()
        .children(
            caller.workspace_id,
            caller.user_id,
            parent_id,
            query.sort,
            query.order,
            Paging::new(query.offset, query.limit),
        )
        .await
        .map(|mut page| {
            state.drive.sign_media(caller.workspace_id, &mut page.items);
            Json(page)
        })
        .map_err(|error| caller.fail(error))
}

#[utoipa::path(post, path = "/api/v1/workspaces/{workspace_id}/drive/folders", params(("workspace_id" = String, Path)), request_body = CreateDriveFolder, responses((status = 201, body = DriveNode)))]
async fn create_drive_folder(
    State(state): State<DriveState>,
    Path(workspace): Path<String>,
    headers: HeaderMap,
    request_id: RequestIdExtension,
    ApiJson(body): ApiJson<CreateDriveFolder>,
) -> Result<(StatusCode, Json<DriveNode>), ApiError> {
    let instance = format!("/api/v1/workspaces/{workspace}/drive/folders");
    let caller = caller(&state, &headers, &workspace, instance, request_id.as_ref()).await?;
    let parent_id = caller.optional_id(body.parent_id.as_deref())?;
    state
        .drive
        .repository()
        .create_folder(
            caller.workspace_id,
            caller.user_id,
            parent_id,
            &body.name,
            body.on_conflict,
            TimestampMillis::now(),
        )
        .await
        .map(|node| (StatusCode::CREATED, Json(node)))
        .map_err(|error| caller.fail(error))
}

/// A live item and the folders above it (breadcrumbs).
#[utoipa::path(get, path = "/api/v1/workspaces/{workspace_id}/drive/nodes/{node_id}", params(("workspace_id" = String, Path), ("node_id" = String, Path)), responses((status = 200, body = DriveNodeDetail)))]
async fn get_drive_node(
    State(state): State<DriveState>,
    Path((workspace, node)): Path<(String, String)>,
    headers: HeaderMap,
    request_id: RequestIdExtension,
) -> Result<Json<DriveNodeDetail>, ApiError> {
    let instance = format!("/api/v1/workspaces/{workspace}/drive/nodes/{node}");
    let caller = caller(&state, &headers, &workspace, instance, request_id.as_ref()).await?;
    let node_id = caller.id(&node)?;
    state
        .drive
        .repository()
        .node(caller.workspace_id, caller.user_id, node_id)
        .await
        .map(|mut detail| {
            state
                .drive
                .sign_media(caller.workspace_id, std::slice::from_mut(&mut detail.node));
            Json(detail)
        })
        .map_err(|error| caller.fail(error))
}

/// Deletes a trashed item and everything in it forever (`drive.purge`).
#[utoipa::path(delete, path = "/api/v1/workspaces/{workspace_id}/drive/nodes/{node_id}", params(("workspace_id" = String, Path), ("node_id" = String, Path)), responses((status = 204)))]
async fn purge_drive_node(
    State(state): State<DriveState>,
    Path((workspace, node)): Path<(String, String)>,
    headers: HeaderMap,
    request_id: RequestIdExtension,
) -> Result<StatusCode, ApiError> {
    let instance = format!("/api/v1/workspaces/{workspace}/drive/nodes/{node}");
    let caller = caller(&state, &headers, &workspace, instance, request_id.as_ref()).await?;
    let node_id = caller.id(&node)?;
    state
        .drive
        .purge(
            caller.workspace_id,
            caller.user_id,
            node_id,
            request_id_value(caller.request_id),
            TimestampMillis::now(),
        )
        .await
        .map(|()| StatusCode::NO_CONTENT)
        .map_err(|error| caller.fail(error))
}

#[utoipa::path(post, path = "/api/v1/workspaces/{workspace_id}/drive/nodes/{node_id}/rename", params(("workspace_id" = String, Path), ("node_id" = String, Path)), request_body = RenameDriveNode, responses((status = 200, body = DriveNode)))]
async fn rename_drive_node(
    State(state): State<DriveState>,
    Path((workspace, node)): Path<(String, String)>,
    headers: HeaderMap,
    request_id: RequestIdExtension,
    ApiJson(body): ApiJson<RenameDriveNode>,
) -> Result<Json<DriveNode>, ApiError> {
    let instance = format!("/api/v1/workspaces/{workspace}/drive/nodes/{node}/rename");
    let caller = caller(&state, &headers, &workspace, instance, request_id.as_ref()).await?;
    let node_id = caller.id(&node)?;
    state
        .drive
        .repository()
        .rename(
            caller.workspace_id,
            caller.user_id,
            node_id,
            &body.name,
            TimestampMillis::now(),
        )
        .await
        .map(Json)
        .map_err(|error| caller.fail(error))
}

#[utoipa::path(post, path = "/api/v1/workspaces/{workspace_id}/drive/nodes/{node_id}/move", params(("workspace_id" = String, Path), ("node_id" = String, Path)), request_body = MoveDriveNode, responses((status = 200, body = DriveNode)))]
async fn move_drive_node(
    State(state): State<DriveState>,
    Path((workspace, node)): Path<(String, String)>,
    headers: HeaderMap,
    request_id: RequestIdExtension,
    ApiJson(body): ApiJson<MoveDriveNode>,
) -> Result<Json<DriveNode>, ApiError> {
    let instance = format!("/api/v1/workspaces/{workspace}/drive/nodes/{node}/move");
    let caller = caller(&state, &headers, &workspace, instance, request_id.as_ref()).await?;
    let node_id = caller.id(&node)?;
    let parent_id = caller.optional_id(body.parent_id.as_deref())?;
    state
        .drive
        .repository()
        .move_node(
            caller.workspace_id,
            caller.user_id,
            node_id,
            parent_id,
            TimestampMillis::now(),
        )
        .await
        .map(Json)
        .map_err(|error| caller.fail(error))
}

/// Moves an item, with everything in it, to the trash (deleted forever after 30 days).
#[utoipa::path(post, path = "/api/v1/workspaces/{workspace_id}/drive/nodes/{node_id}/trash", params(("workspace_id" = String, Path), ("node_id" = String, Path)), responses((status = 204)))]
async fn trash_drive_node(
    State(state): State<DriveState>,
    Path((workspace, node)): Path<(String, String)>,
    headers: HeaderMap,
    request_id: RequestIdExtension,
) -> Result<StatusCode, ApiError> {
    let instance = format!("/api/v1/workspaces/{workspace}/drive/nodes/{node}/trash");
    let caller = caller(&state, &headers, &workspace, instance, request_id.as_ref()).await?;
    let node_id = caller.id(&node)?;
    state
        .drive
        .repository()
        .trash(
            caller.workspace_id,
            caller.user_id,
            node_id,
            TimestampMillis::now(),
        )
        .await
        .map(|()| StatusCode::NO_CONTENT)
        .map_err(|error| caller.fail(error))
}

/// Brings a trashed item back into its folder (the top level when that folder is gone); a used name gets
/// `name (2)`.
#[utoipa::path(post, path = "/api/v1/workspaces/{workspace_id}/drive/nodes/{node_id}/restore", params(("workspace_id" = String, Path), ("node_id" = String, Path)), responses((status = 200, body = DriveNode)))]
async fn restore_drive_node(
    State(state): State<DriveState>,
    Path((workspace, node)): Path<(String, String)>,
    headers: HeaderMap,
    request_id: RequestIdExtension,
) -> Result<Json<DriveNode>, ApiError> {
    let instance = format!("/api/v1/workspaces/{workspace}/drive/nodes/{node}/restore");
    let caller = caller(&state, &headers, &workspace, instance, request_id.as_ref()).await?;
    let node_id = caller.id(&node)?;
    state
        .drive
        .repository()
        .restore(
            caller.workspace_id,
            caller.user_id,
            node_id,
            TimestampMillis::now(),
        )
        .await
        .map(Json)
        .map_err(|error| caller.fail(error))
}

/// The live files below a folder at any depth, with paths relative to it (for a ZIP download).
#[utoipa::path(get, path = "/api/v1/workspaces/{workspace_id}/drive/nodes/{node_id}/files", params(("workspace_id" = String, Path), ("node_id" = String, Path), DrivePageQuery), responses((status = 200, body = DriveEntryPage)))]
async fn list_drive_folder_files(
    State(state): State<DriveState>,
    Path((workspace, node)): Path<(String, String)>,
    ApiQuery(query): ApiQuery<DrivePageQuery>,
    headers: HeaderMap,
    request_id: RequestIdExtension,
) -> Result<Json<DriveEntryPage>, ApiError> {
    let instance = format!("/api/v1/workspaces/{workspace}/drive/nodes/{node}/files");
    let caller = caller(&state, &headers, &workspace, instance, request_id.as_ref()).await?;
    let node_id = caller.id(&node)?;
    state
        .drive
        .repository()
        .files_below(
            caller.workspace_id,
            caller.user_id,
            node_id,
            Paging::new(query.offset, query.limit),
        )
        .await
        .map(Json)
        .map_err(|error| caller.fail(error))
}

/// A URL to the file's bytes in the bucket, valid for 5 minutes.
#[utoipa::path(get, path = "/api/v1/workspaces/{workspace_id}/drive/nodes/{node_id}/download", params(("workspace_id" = String, Path), ("node_id" = String, Path), DriveDownloadQuery), responses((status = 200, body = DriveDownload)))]
async fn get_drive_download(
    State(state): State<DriveState>,
    Path((workspace, node)): Path<(String, String)>,
    ApiQuery(query): ApiQuery<DriveDownloadQuery>,
    headers: HeaderMap,
    request_id: RequestIdExtension,
) -> Result<Json<DriveDownload>, ApiError> {
    let instance = format!("/api/v1/workspaces/{workspace}/drive/nodes/{node}/download");
    let caller = caller(&state, &headers, &workspace, instance, request_id.as_ref()).await?;
    let node_id = caller.id(&node)?;
    let urls = state
        .drive
        .download_urls(
            caller.workspace_id,
            caller.user_id,
            &[node_id],
            query.inline,
        )
        .await
        .map_err(|error| caller.fail(error))?;
    let (_, url) = urls
        .into_iter()
        .next()
        .ok_or_else(|| caller.fail(DriveError::NotFound))?;
    Ok(Json(DriveDownload {
        url,
        expires_at: expires_in(DOWNLOAD_URL_LIFETIME),
    }))
}

/// Records what the browser measured of an image or video and gives PUT URLs for its thumbnail (at most 1 MiB)
/// and, for a video, its poster (at most 4 MiB). Thumbnails are made in the browser, never on the server.
#[utoipa::path(post, path = "/api/v1/workspaces/{workspace_id}/drive/nodes/{node_id}/media", params(("workspace_id" = String, Path), ("node_id" = String, Path)), request_body = BeginDriveMedia, responses((status = 200, body = DriveMediaUpload)))]
async fn begin_drive_media(
    State(state): State<DriveState>,
    Path((workspace, node)): Path<(String, String)>,
    headers: HeaderMap,
    request_id: RequestIdExtension,
    ApiJson(body): ApiJson<BeginDriveMedia>,
) -> Result<Json<DriveMediaUpload>, ApiError> {
    let instance = format!("/api/v1/workspaces/{workspace}/drive/nodes/{node}/media");
    let caller = caller(&state, &headers, &workspace, instance, request_id.as_ref()).await?;
    let node_id = caller.id(&node)?;
    let (thumbnail_put_url, poster_put_url) = state
        .drive
        .begin_media(
            caller.workspace_id,
            caller.user_id,
            node_id,
            MediaFacts {
                width: body.width,
                height: body.height,
                duration_ms: body.duration_ms,
            },
            body.failed,
            TimestampMillis::now(),
        )
        .await
        .map_err(|error| caller.fail(error))?;
    Ok(Json(DriveMediaUpload {
        thumbnail_put_url,
        poster_put_url,
        expires_at: expires_in(UPLOAD_URL_LIFETIME),
    }))
}

/// Keeps the thumbnail and poster the browser sent (when they are in the bucket and small enough) and returns
/// the file with their URLs.
#[utoipa::path(post, path = "/api/v1/workspaces/{workspace_id}/drive/nodes/{node_id}/media/complete", params(("workspace_id" = String, Path), ("node_id" = String, Path)), responses((status = 200, body = DriveNode)))]
async fn complete_drive_media(
    State(state): State<DriveState>,
    Path((workspace, node)): Path<(String, String)>,
    headers: HeaderMap,
    request_id: RequestIdExtension,
) -> Result<Json<DriveNode>, ApiError> {
    let instance = format!("/api/v1/workspaces/{workspace}/drive/nodes/{node}/media/complete");
    let caller = caller(&state, &headers, &workspace, instance, request_id.as_ref()).await?;
    let node_id = caller.id(&node)?;
    state
        .drive
        .finish_media(
            caller.workspace_id,
            caller.user_id,
            node_id,
            TimestampMillis::now(),
        )
        .await
        .map(Json)
        .map_err(|error| caller.fail(error))
}

/// Download URLs for many files at once (a ZIP built in the browser).
#[utoipa::path(post, path = "/api/v1/workspaces/{workspace_id}/drive/downloads", params(("workspace_id" = String, Path)), request_body = CreateDriveDownloads, responses((status = 200, body = DriveDownloads)))]
async fn create_drive_downloads(
    State(state): State<DriveState>,
    Path(workspace): Path<String>,
    headers: HeaderMap,
    request_id: RequestIdExtension,
    ApiJson(body): ApiJson<CreateDriveDownloads>,
) -> Result<Json<DriveDownloads>, ApiError> {
    let instance = format!("/api/v1/workspaces/{workspace}/drive/downloads");
    let caller = caller(&state, &headers, &workspace, instance, request_id.as_ref()).await?;
    if body.node_ids.len() > crate::drive::MAX_URLS {
        return Err(caller.fail(DriveError::TooManyItems));
    }
    // An id that is not one cannot be a live file: it is left out like the others.
    let node_ids: Vec<Id> = body
        .node_ids
        .iter()
        .filter_map(|id| id.parse().ok())
        .collect();
    let items = state
        .drive
        .download_urls(caller.workspace_id, caller.user_id, &node_ids, false)
        .await
        .map_err(|error| caller.fail(error))?
        .into_iter()
        .map(|(node_id, url)| DriveDownloadUrl { node_id, url })
        .collect();
    Ok(Json(DriveDownloads {
        items,
        expires_at: expires_in(DOWNLOAD_URL_LIFETIME),
    }))
}

/// The trashed items, newest first.
#[utoipa::path(get, path = "/api/v1/workspaces/{workspace_id}/drive/trash", params(("workspace_id" = String, Path), DrivePageQuery), responses((status = 200, body = DriveNodePage)))]
async fn list_drive_trash(
    State(state): State<DriveState>,
    Path(workspace): Path<String>,
    ApiQuery(query): ApiQuery<DrivePageQuery>,
    headers: HeaderMap,
    request_id: RequestIdExtension,
) -> Result<Json<DriveNodePage>, ApiError> {
    let instance = format!("/api/v1/workspaces/{workspace}/drive/trash");
    let caller = caller(&state, &headers, &workspace, instance, request_id.as_ref()).await?;
    state
        .drive
        .repository()
        .trashed(
            caller.workspace_id,
            caller.user_id,
            Paging::new(query.offset, query.limit),
        )
        .await
        .map(Json)
        .map_err(|error| caller.fail(error))
}

/// Deletes everything in the trash forever (`drive.purge`).
#[utoipa::path(post, path = "/api/v1/workspaces/{workspace_id}/drive/trash/empty", params(("workspace_id" = String, Path)), responses((status = 200, body = DriveTrashEmptied)))]
async fn empty_drive_trash(
    State(state): State<DriveState>,
    Path(workspace): Path<String>,
    headers: HeaderMap,
    request_id: RequestIdExtension,
) -> Result<Json<DriveTrashEmptied>, ApiError> {
    let instance = format!("/api/v1/workspaces/{workspace}/drive/trash/empty");
    let caller = caller(&state, &headers, &workspace, instance, request_id.as_ref()).await?;
    state
        .drive
        .empty_trash(
            caller.workspace_id,
            caller.user_id,
            request_id_value(caller.request_id),
            TimestampMillis::now(),
        )
        .await
        .map(|deleted| Json(DriveTrashEmptied { deleted }))
        .map_err(|error| caller.fail(error))
}

/// Live files and folders by name, type and date, with their folder paths.
#[utoipa::path(get, path = "/api/v1/workspaces/{workspace_id}/drive/search", params(("workspace_id" = String, Path), DriveSearchQuery), responses((status = 200, body = DriveEntryPage)))]
async fn search_drive(
    State(state): State<DriveState>,
    Path(workspace): Path<String>,
    ApiQuery(query): ApiQuery<DriveSearchQuery>,
    headers: HeaderMap,
    request_id: RequestIdExtension,
) -> Result<Json<DriveEntryPage>, ApiError> {
    let instance = format!("/api/v1/workspaces/{workspace}/drive/search");
    let caller = caller(&state, &headers, &workspace, instance, request_id.as_ref()).await?;
    let search = DriveSearch {
        query: query.q,
        kind: query.kind,
        mime_prefix: query.mime,
        updated_after: query.updated_after,
        updated_before: query.updated_before,
    };
    state
        .drive
        .repository()
        .search(
            caller.workspace_id,
            caller.user_id,
            &search,
            Paging::new(query.offset, query.limit),
        )
        .await
        .map(|mut page| {
            for entry in &mut page.items {
                state
                    .drive
                    .sign_media(caller.workspace_id, std::slice::from_mut(&mut entry.node));
            }
            Json(page)
        })
        .map_err(|error| caller.fail(error))
}

/// The caller's uploads that are not complete (to resume them after a reload).
#[utoipa::path(get, path = "/api/v1/workspaces/{workspace_id}/drive/uploads", params(("workspace_id" = String, Path)), responses((status = 200, body = DriveUploadList)))]
async fn list_drive_uploads(
    State(state): State<DriveState>,
    Path(workspace): Path<String>,
    headers: HeaderMap,
    request_id: RequestIdExtension,
) -> Result<Json<DriveUploadList>, ApiError> {
    let instance = format!("/api/v1/workspaces/{workspace}/drive/uploads");
    let caller = caller(&state, &headers, &workspace, instance, request_id.as_ref()).await?;
    let uploads = state
        .drive
        .repository()
        .uploads(caller.workspace_id, caller.user_id)
        .await
        .map_err(|error| caller.fail(error))?;
    Ok(Json(DriveUploadList {
        items: uploads.into_iter().map(upload_view).collect(),
    }))
}

/// Starts an upload. Files up to 32 MiB get one PUT URL; larger ones are sent in parts of `part_size`.
#[utoipa::path(post, path = "/api/v1/workspaces/{workspace_id}/drive/uploads", params(("workspace_id" = String, Path)), request_body = StartDriveUpload, responses((status = 201, body = DriveUploadStarted)))]
async fn start_drive_upload(
    State(state): State<DriveState>,
    Path(workspace): Path<String>,
    headers: HeaderMap,
    request_id: RequestIdExtension,
    ApiJson(body): ApiJson<StartDriveUpload>,
) -> Result<(StatusCode, Json<DriveUploadStarted>), ApiError> {
    let instance = format!("/api/v1/workspaces/{workspace}/drive/uploads");
    let caller = caller(&state, &headers, &workspace, instance, request_id.as_ref()).await?;
    let parent_id = caller.optional_id(body.parent_id.as_deref())?;
    let mime = body
        .mime
        .map(|mime| mime.trim().to_ascii_lowercase())
        .filter(|mime| valid_mime(mime))
        .unwrap_or_else(|| "application/octet-stream".to_owned());
    let started = state
        .drive
        .start_upload(
            caller.workspace_id,
            caller.user_id,
            StartUpload {
                parent_id,
                name: body.name,
                size: body.size,
                mime,
                on_conflict: body.on_conflict,
            },
            TimestampMillis::now(),
        )
        .await
        .map_err(|error| caller.fail(error))?;
    Ok((
        StatusCode::CREATED,
        Json(DriveUploadStarted {
            upload: upload_view(started.upload),
            url: started.url,
            expires_at: expires_in(UPLOAD_URL_LIFETIME),
        }),
    ))
}

/// The upload and the parts the bucket already has.
#[utoipa::path(get, path = "/api/v1/workspaces/{workspace_id}/drive/uploads/{upload_id}", params(("workspace_id" = String, Path), ("upload_id" = String, Path)), responses((status = 200, body = DriveUploadStatus)))]
async fn get_drive_upload(
    State(state): State<DriveState>,
    Path((workspace, upload)): Path<(String, String)>,
    headers: HeaderMap,
    request_id: RequestIdExtension,
) -> Result<Json<DriveUploadStatus>, ApiError> {
    let instance = format!("/api/v1/workspaces/{workspace}/drive/uploads/{upload}");
    let caller = caller(&state, &headers, &workspace, instance, request_id.as_ref()).await?;
    let blob_id = caller.id(&upload)?;
    let (upload, parts) = state
        .drive
        .upload_status(caller.workspace_id, caller.user_id, blob_id)
        .await
        .map_err(|error| caller.fail(error))?;
    Ok(Json(DriveUploadStatus {
        upload: upload_view(upload),
        uploaded_parts: parts
            .into_iter()
            .map(|part| DriveUploadedPart {
                part_number: part.part_number,
                size: part.size,
            })
            .collect(),
    }))
}

/// Cancels an upload; the parts sent so far are deleted.
#[utoipa::path(delete, path = "/api/v1/workspaces/{workspace_id}/drive/uploads/{upload_id}", params(("workspace_id" = String, Path), ("upload_id" = String, Path)), responses((status = 204)))]
async fn cancel_drive_upload(
    State(state): State<DriveState>,
    Path((workspace, upload)): Path<(String, String)>,
    headers: HeaderMap,
    request_id: RequestIdExtension,
) -> Result<StatusCode, ApiError> {
    let instance = format!("/api/v1/workspaces/{workspace}/drive/uploads/{upload}");
    let caller = caller(&state, &headers, &workspace, instance, request_id.as_ref()).await?;
    let blob_id = caller.id(&upload)?;
    state
        .drive
        .cancel_upload(
            caller.workspace_id,
            caller.user_id,
            blob_id,
            TimestampMillis::now(),
        )
        .await
        .map(|()| StatusCode::NO_CONTENT)
        .map_err(|error| caller.fail(error))
}

/// PUT URLs for parts of a multipart upload, valid for one hour. The bucket's CORS rules must expose `ETag`.
#[utoipa::path(post, path = "/api/v1/workspaces/{workspace_id}/drive/uploads/{upload_id}/parts", params(("workspace_id" = String, Path), ("upload_id" = String, Path)), request_body = SignDriveUploadParts, responses((status = 200, body = DrivePartUrls)))]
async fn sign_drive_upload_parts(
    State(state): State<DriveState>,
    Path((workspace, upload)): Path<(String, String)>,
    headers: HeaderMap,
    request_id: RequestIdExtension,
    ApiJson(body): ApiJson<SignDriveUploadParts>,
) -> Result<Json<DrivePartUrls>, ApiError> {
    let instance = format!("/api/v1/workspaces/{workspace}/drive/uploads/{upload}/parts");
    let caller = caller(&state, &headers, &workspace, instance, request_id.as_ref()).await?;
    let blob_id = caller.id(&upload)?;
    let items = state
        .drive
        .part_urls(
            caller.workspace_id,
            caller.user_id,
            blob_id,
            &body.part_numbers,
        )
        .await
        .map_err(|error| caller.fail(error))?
        .into_iter()
        .map(|(part_number, url)| DrivePartUrl { part_number, url })
        .collect();
    Ok(Json(DrivePartUrls {
        items,
        expires_at: expires_in(UPLOAD_URL_LIFETIME),
    }))
}

/// Checks that the bucket has the whole file and adds it to the drive. A name used meanwhile gets
/// `name (2)` (`replace` trashes the existing file); a folder deleted meanwhile puts the file at the top level.
#[utoipa::path(post, path = "/api/v1/workspaces/{workspace_id}/drive/uploads/{upload_id}/complete", params(("workspace_id" = String, Path), ("upload_id" = String, Path)), responses((status = 201, body = DriveNode)))]
async fn complete_drive_upload(
    State(state): State<DriveState>,
    Path((workspace, upload)): Path<(String, String)>,
    headers: HeaderMap,
    request_id: RequestIdExtension,
) -> Result<(StatusCode, Json<DriveNode>), ApiError> {
    let instance = format!("/api/v1/workspaces/{workspace}/drive/uploads/{upload}/complete");
    let caller = caller(&state, &headers, &workspace, instance, request_id.as_ref()).await?;
    let blob_id = caller.id(&upload)?;
    state
        .drive
        .complete_upload(
            caller.workspace_id,
            caller.user_id,
            blob_id,
            TimestampMillis::now(),
        )
        .await
        .map(|node| (StatusCode::CREATED, Json(node)))
        .map_err(|error| caller.fail(error))
}

/// `type/subtype` in plain characters, at most 255 long.
fn valid_mime(mime: &str) -> bool {
    mime.len() <= 255
        && mime.split_once('/').is_some_and(|(kind, subtype)| {
            !kind.is_empty()
                && !subtype.is_empty()
                && mime
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '+' | '-' | '_'))
        })
}

fn problem(
    error: DriveError,
    instance: &str,
    request_id: Option<&Extension<RequestId>>,
) -> ApiError {
    let (status, code, title, detail) = match &error {
        DriveError::NotFound => (
            StatusCode::NOT_FOUND,
            "drive_not_found",
            "Not found",
            "The file or folder was not found.",
        ),
        DriveError::Forbidden => (
            StatusCode::FORBIDDEN,
            "drive_forbidden",
            "Not allowed",
            "Only workspace owners and admins may delete files forever.",
        ),
        DriveError::InvalidName => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "drive_invalid_name",
            "Invalid name",
            "A name has 1 to 255 characters and no '/', '\\' or control characters.",
        ),
        DriveError::NameTaken => (
            StatusCode::CONFLICT,
            "drive_name_taken",
            "Name already used",
            "This folder already has an item with this name.",
        ),
        DriveError::NotAFolder => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "drive_not_a_folder",
            "Not a folder",
            "The target is not a folder.",
        ),
        DriveError::MoveCycle => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "drive_move_cycle",
            "Invalid move",
            "A folder cannot move into itself or one of its subfolders.",
        ),
        DriveError::NotTrashed => (
            StatusCode::CONFLICT,
            "drive_not_trashed",
            "Not in the trash",
            "Move the item to the trash before you delete it forever.",
        ),
        DriveError::UploadIncomplete => (
            StatusCode::CONFLICT,
            "drive_upload_incomplete",
            "Upload not complete",
            "The storage does not have every part of the file yet.",
        ),
        DriveError::TooLarge => (
            StatusCode::PAYLOAD_TOO_LARGE,
            "drive_file_too_large",
            "File too large",
            "Drive files can have up to 4 TiB.",
        ),
        DriveError::InvalidPart => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "drive_invalid_part",
            "Invalid part",
            "Ask for 1 to 100 part numbers of a multipart upload, from 1 to its part count.",
        ),
        DriveError::NotMedia => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "drive_not_media",
            "Not an image or video",
            "Only images and videos have a thumbnail.",
        ),
        DriveError::TooManyItems => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "drive_too_many_items",
            "Too many items",
            "Ask for at most 100 items at once.",
        ),
        DriveError::NoStorage => (
            StatusCode::CONFLICT,
            "drive_storage_not_configured",
            "No storage",
            "Drive needs an S3 bucket: connect one in Settings → File storage or Admin → Storage.",
        ),
        DriveError::Storage(error) => {
            tracing::error!(error = %error, "Drive storage request failed");
            (
                StatusCode::BAD_GATEWAY,
                "drive_storage_failed",
                "Storage failed",
                "The file storage did not answer as expected. Try again.",
            )
        }
        DriveError::Database(error) => {
            tracing::error!(error = %error, "Drive database request failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "Internal server error",
                "An unexpected error occurred. Use the request ID when contacting support.",
            )
        }
    };
    ApiError::new(status, code, title, detail, instance, request_id)
}
