//! Drive files in Docs pages (`driveFile` blocks, [`crate::repositories::drive_refs`]): the files of a page as the
//! caller may see them, the pages that show a file, and adding, changing and removing a reference. Every read checks
//! the caller's Drive access to each file again; URLs are signed only after that check.

use axum::extract::{Extension, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use orbit_platform::{Id, RequestId};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::drive_routes::DriveState;
use crate::page_routes::page_problem;
use crate::repositories::drive::DriveNode;
use crate::repositories::drive_refs::{DriveRef, DriveRefError, DriveRefRepository, DriveRefState};
use crate::repositories::pages::PageError;
use crate::task_routes::{ApiError, ApiJson, authenticate_session, request_id_value};

type RequestIdExtension = Option<Extension<RequestId>>;

pub fn page_drive_router(state: DriveState) -> Router {
    Router::new()
        .route(
            "/api/v1/workspaces/{workspace_id}/pages/{page_id}/drive-files",
            get(list_page_drive_files).post(insert_page_drive_file),
        )
        .route(
            "/api/v1/workspaces/{workspace_id}/pages/{page_id}/drive-files/resolve",
            post(resolve_page_drive_files),
        )
        .route(
            "/api/v1/workspaces/{workspace_id}/pages/{page_id}/drive-files/{block_id}",
            put(replace_page_drive_file).delete(remove_page_drive_file),
        )
        .route(
            "/api/v1/workspaces/{workspace_id}/pages/drive-file-uses/{node_id}",
            get(list_drive_file_uses),
        )
        .with_state(state)
}

/// What the caller may see of one Drive file reference.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PageDriveFileStatus {
    /// A live file the caller has Drive access to.
    Available,
    /// The caller has no Drive access to the file: nothing about it is shown.
    Locked,
    /// In the trash, deleted, or not a file.
    Unavailable,
}

/// What the caller may see of one Drive file reference, now.
#[derive(Serialize, ToSchema)]
pub struct DriveFileReference {
    /// The Drive's workspace, as the block stores it.
    pub workspace_id: String,
    pub node_id: String,
    pub status: PageDriveFileStatus,
    /// The file as it is now; only when `available`.
    pub file: Option<DriveNode>,
    /// A 5-minute URL that shows an available image or video inline.
    pub url: Option<String>,
}

/// A `driveFile` block of a page.
#[derive(Serialize, ToSchema)]
pub struct PageDriveFile {
    pub block_id: String,
    /// The block's index among the page's top-level blocks; `null` for a nested block.
    pub position: Option<i64>,
    #[serde(flatten)]
    pub reference: DriveFileReference,
}

#[derive(Serialize, ToSchema)]
pub struct PageDriveFileList {
    pub items: Vec<PageDriveFile>,
}

/// One reference to resolve: what a `driveFile` block stores.
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct DriveFileReferenceInput {
    pub workspace_id: String,
    pub node_id: String,
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ResolveDriveFiles {
    /// At most 100.
    pub references: Vec<DriveFileReferenceInput>,
}

#[derive(Serialize, ToSchema)]
pub struct DriveFileReferenceList {
    /// In the order of the request.
    pub items: Vec<DriveFileReference>,
}

/// References one resolve call takes.
const MAX_RESOLVE: usize = 100;

/// A page that shows a Drive file.
#[derive(Serialize, ToSchema)]
pub struct DriveFileUse {
    #[schema(value_type = String)]
    pub page_id: Id,
    pub title: String,
    /// How many blocks of the page show the file.
    pub blocks: i64,
}

#[derive(Serialize, ToSchema)]
pub struct DriveFileUseList {
    pub items: Vec<DriveFileUse>,
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct InsertPageDriveFile {
    /// A file of the workspace's Drive.
    pub node_id: String,
    /// Index among the page's top-level blocks (0: first); absent: at the end.
    pub position: Option<i64>,
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ReplacePageDriveFile {
    /// The file the block shows from now on.
    pub node_id: String,
}

struct Call<'a> {
    user_id: Id,
    workspace_id: Id,
    instance: String,
    request_id: Option<&'a Extension<RequestId>>,
    repository: DriveRefRepository,
}

impl Call<'_> {
    fn fail(&self, error: DriveRefError) -> ApiError {
        match error {
            DriveRefError::FileNotFound => crate::drive_routes::problem(
                crate::repositories::drive::DriveError::NotFound,
                &self.instance,
                self.request_id,
            ),
            DriveRefError::BlockNotFound => ApiError::new(
                StatusCode::NOT_FOUND,
                "drive_file_block_not_found",
                "Not found",
                "The page has no Drive file block with this id.",
                self.instance.clone(),
                self.request_id,
            ),
            DriveRefError::Page(error) => {
                page_problem(error, self.instance.clone(), self.request_id)
            }
        }
    }

    fn id(&self, value: &str) -> Result<Id, ApiError> {
        value
            .parse()
            .map_err(|_| self.fail(DriveRefError::Page(PageError::NotFound)))
    }
}

async fn call<'a>(
    state: &DriveState,
    headers: &HeaderMap,
    workspace: &str,
    instance: String,
    request_id: Option<&'a Extension<RequestId>>,
) -> Result<Call<'a>, ApiError> {
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
        .map_err(|_| page_problem(PageError::NotFound, instance.clone(), request_id))?;
    Ok(Call {
        user_id: session.user.id,
        workspace_id,
        instance,
        request_id,
        repository: DriveRefRepository::new(state.identity.database().clone()),
    })
}

/// The view of one block; signs URLs only for an available file.
async fn view(
    state: &DriveState,
    user_id: Id,
    reference: DriveRef,
    refs: DriveRefState,
) -> PageDriveFile {
    PageDriveFile {
        block_id: reference.block_id,
        position: reference
            .position
            .and_then(|index| i64::try_from(index).ok()),
        reference: reference_view(
            state,
            user_id,
            reference.workspace_id,
            reference.node_id,
            refs,
        )
        .await,
    }
}

/// The view of one reference; signs URLs only for an available file.
async fn reference_view(
    state: &DriveState,
    user_id: Id,
    workspace: String,
    node: String,
    refs: DriveRefState,
) -> DriveFileReference {
    let (status, file) = match refs {
        DriveRefState::Available(node) => (PageDriveFileStatus::Available, Some(*node)),
        DriveRefState::Locked => (PageDriveFileStatus::Locked, None),
        DriveRefState::Unavailable => (PageDriveFileStatus::Unavailable, None),
    };
    let mut url = None;
    let file = match (file, workspace.parse::<Id>()) {
        (Some(mut node), Ok(workspace_id)) => {
            state
                .drive
                .sign_media(workspace_id, std::slice::from_mut(&mut node));
            let media = node
                .mime
                .as_deref()
                .is_some_and(|mime| mime.starts_with("image/") || mime.starts_with("video/"));
            if media {
                // The service checks the caller's access again; no bucket: no URL.
                url = state
                    .drive
                    .download_urls(workspace_id, user_id, &[node.id], true)
                    .await
                    .ok()
                    .and_then(|urls| urls.into_iter().next())
                    .map(|(_, url)| url);
            }
            Some(node)
        }
        _ => None,
    };
    DriveFileReference {
        workspace_id: workspace,
        node_id: node,
        status,
        file,
        url,
    }
}

/// The Drive files of a page, as the caller may see each of them now (checked on every call).
#[utoipa::path(get, path = "/api/v1/workspaces/{workspace_id}/pages/{page_id}/drive-files", params(("workspace_id" = String, Path), ("page_id" = String, Path)), responses((status = 200, body = PageDriveFileList)))]
async fn list_page_drive_files(
    State(state): State<DriveState>,
    Path((workspace, page)): Path<(String, String)>,
    headers: HeaderMap,
    request_id: RequestIdExtension,
) -> Result<Json<PageDriveFileList>, ApiError> {
    let instance = format!("/api/v1/workspaces/{workspace}/pages/{page}/drive-files");
    let call = call(&state, &headers, &workspace, instance, request_id.as_ref()).await?;
    let page_id = call.id(&page)?;
    let files = call
        .repository
        .page_files(call.workspace_id, page_id, call.user_id)
        .await
        .map_err(|error| call.fail(error))?;
    let mut items = Vec::with_capacity(files.len());
    for (reference, refs) in files {
        items.push(view(&state, call.user_id, reference, refs).await);
    }
    Ok(Json(PageDriveFileList { items }))
}

/// What the caller may see of Drive file references of a page's document, such as blocks that the editor has not
/// saved yet (checked on every call).
#[utoipa::path(post, path = "/api/v1/workspaces/{workspace_id}/pages/{page_id}/drive-files/resolve", params(("workspace_id" = String, Path), ("page_id" = String, Path)), request_body = ResolveDriveFiles, responses((status = 200, body = DriveFileReferenceList)))]
async fn resolve_page_drive_files(
    State(state): State<DriveState>,
    Path((workspace, page)): Path<(String, String)>,
    headers: HeaderMap,
    request_id: RequestIdExtension,
    ApiJson(body): ApiJson<ResolveDriveFiles>,
) -> Result<Json<DriveFileReferenceList>, ApiError> {
    let instance = format!("/api/v1/workspaces/{workspace}/pages/{page}/drive-files/resolve");
    let call = call(&state, &headers, &workspace, instance, request_id.as_ref()).await?;
    let page_id = call.id(&page)?;
    if body.references.len() > MAX_RESOLVE {
        return Err(crate::task_routes::validation(
            "references",
            &call.instance,
            call.request_id,
        ));
    }
    let references: Vec<(String, String)> = body
        .references
        .into_iter()
        .map(|reference| (reference.workspace_id, reference.node_id))
        .collect();
    let states = call
        .repository
        .resolve_for_page(call.workspace_id, page_id, call.user_id, &references)
        .await
        .map_err(|error| call.fail(error))?;
    let mut items = Vec::with_capacity(references.len());
    for ((workspace, node), refs) in references.into_iter().zip(states) {
        items.push(reference_view(&state, call.user_id, workspace, node, refs).await);
    }
    Ok(Json(DriveFileReferenceList { items }))
}

/// Adds a Drive file of the workspace to the page as a reference (never a copy). Needs Drive access to the file
/// and the right to edit the page.
#[utoipa::path(post, path = "/api/v1/workspaces/{workspace_id}/pages/{page_id}/drive-files", params(("workspace_id" = String, Path), ("page_id" = String, Path)), request_body = InsertPageDriveFile, responses((status = 201, body = PageDriveFile)))]
async fn insert_page_drive_file(
    State(state): State<DriveState>,
    Path((workspace, page)): Path<(String, String)>,
    headers: HeaderMap,
    request_id: RequestIdExtension,
    ApiJson(body): ApiJson<InsertPageDriveFile>,
) -> Result<(StatusCode, Json<PageDriveFile>), ApiError> {
    let instance = format!("/api/v1/workspaces/{workspace}/pages/{page}/drive-files");
    let call = call(&state, &headers, &workspace, instance, request_id.as_ref()).await?;
    let page_id = call.id(&page)?;
    let node_id: Id = body
        .node_id
        .parse()
        .map_err(|_| call.fail(DriveRefError::FileNotFound))?;
    let (reference, node) = call
        .repository
        .insert(
            call.workspace_id,
            page_id,
            call.user_id,
            node_id,
            body.position,
            request_id_value(call.request_id),
        )
        .await
        .map_err(|error| call.fail(error))?;
    let view = view(
        &state,
        call.user_id,
        reference,
        DriveRefState::Available(Box::new(node)),
    )
    .await;
    Ok((StatusCode::CREATED, Json(view)))
}

/// Points a Drive file block of the page at another file of the workspace's Drive.
#[utoipa::path(put, path = "/api/v1/workspaces/{workspace_id}/pages/{page_id}/drive-files/{block_id}", params(("workspace_id" = String, Path), ("page_id" = String, Path), ("block_id" = String, Path)), request_body = ReplacePageDriveFile, responses((status = 200, body = PageDriveFile)))]
async fn replace_page_drive_file(
    State(state): State<DriveState>,
    Path((workspace, page, block)): Path<(String, String, String)>,
    headers: HeaderMap,
    request_id: RequestIdExtension,
    ApiJson(body): ApiJson<ReplacePageDriveFile>,
) -> Result<Json<PageDriveFile>, ApiError> {
    let instance = format!("/api/v1/workspaces/{workspace}/pages/{page}/drive-files/{block}");
    let call = call(&state, &headers, &workspace, instance, request_id.as_ref()).await?;
    let page_id = call.id(&page)?;
    let node_id: Id = body
        .node_id
        .parse()
        .map_err(|_| call.fail(DriveRefError::FileNotFound))?;
    let (reference, node) = call
        .repository
        .replace(
            call.workspace_id,
            page_id,
            call.user_id,
            &block,
            node_id,
            request_id_value(call.request_id),
        )
        .await
        .map_err(|error| call.fail(error))?;
    Ok(Json(
        view(
            &state,
            call.user_id,
            reference,
            DriveRefState::Available(Box::new(node)),
        )
        .await,
    ))
}

/// Removes a Drive file block from the page. The file stays in Drive.
#[utoipa::path(delete, path = "/api/v1/workspaces/{workspace_id}/pages/{page_id}/drive-files/{block_id}", params(("workspace_id" = String, Path), ("page_id" = String, Path), ("block_id" = String, Path)), responses((status = 204)))]
async fn remove_page_drive_file(
    State(state): State<DriveState>,
    Path((workspace, page, block)): Path<(String, String, String)>,
    headers: HeaderMap,
    request_id: RequestIdExtension,
) -> Result<StatusCode, ApiError> {
    let instance = format!("/api/v1/workspaces/{workspace}/pages/{page}/drive-files/{block}");
    let call = call(&state, &headers, &workspace, instance, request_id.as_ref()).await?;
    let page_id = call.id(&page)?;
    call.repository
        .remove(
            call.workspace_id,
            page_id,
            call.user_id,
            &block,
            request_id_value(call.request_id),
        )
        .await
        .map_err(|error| call.fail(error))?;
    Ok(StatusCode::NO_CONTENT)
}

/// The pages the caller can see that show a file of the workspace's Drive.
#[utoipa::path(get, path = "/api/v1/workspaces/{workspace_id}/pages/drive-file-uses/{node_id}", params(("workspace_id" = String, Path), ("node_id" = String, Path)), responses((status = 200, body = DriveFileUseList)))]
async fn list_drive_file_uses(
    State(state): State<DriveState>,
    Path((workspace, node)): Path<(String, String)>,
    headers: HeaderMap,
    request_id: RequestIdExtension,
) -> Result<Json<DriveFileUseList>, ApiError> {
    let instance = format!("/api/v1/workspaces/{workspace}/pages/drive-file-uses/{node}");
    let call = call(&state, &headers, &workspace, instance, request_id.as_ref()).await?;
    let node_id: Id = node
        .parse()
        .map_err(|_| call.fail(DriveRefError::FileNotFound))?;
    let uses = call
        .repository
        .pages_using(call.workspace_id, call.user_id, node_id)
        .await
        .map_err(|error| call.fail(error))?;
    Ok(Json(DriveFileUseList {
        items: uses
            .into_iter()
            .map(|page| DriveFileUse {
                page_id: page.page_id,
                title: page.title,
                blocks: i64::try_from(page.blocks).unwrap_or(i64::MAX),
            })
            .collect(),
    }))
}
