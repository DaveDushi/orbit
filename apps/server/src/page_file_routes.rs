//! Docs page files: `POST .../pages/{page_id}/files` (multipart, one `file` field) and
//! `GET .../pages/{page_id}/files/{file_id}` (download), and for the preview panel and the agent:
//! the file's content in parts, saving, and its versions. Access follows the page's visibility.

use std::io;
use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::{DefaultBodyLimit, Extension, Path, Query, Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use orbit_platform::{BlobStoreError, Id, RequestId, UploadError, UploadService};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::attachment_routes::{StageFailure, download_response, stage_single_file};
use crate::auth_routes::CookieMode;
use crate::file_content::{ContentQuery, FileReader, ReadError, is_tsv, read_problem};
use crate::file_parse::{self, FileKind};
use crate::repositories::identity::IdentityRepository;
use crate::repositories::page_files::{
    PageFile, PageFileError, PageFileRepository, PageFileVersion,
};
use crate::task_routes::{ApiError, authenticate_session, request_id_value};
use crate::xlsx_patch::{CellEdit, PatchError, PatchRequest};

#[derive(Clone)]
pub struct PageFileState {
    identity: Arc<IdentityRepository>,
    files: Arc<PageFileRepository>,
    cookie_mode: CookieMode,
    reader: FileReader,
}

impl PageFileState {
    #[must_use]
    pub fn new(
        identity: Arc<IdentityRepository>,
        uploads: UploadService,
        cookie_mode: CookieMode,
    ) -> Self {
        Self {
            files: Arc::new(PageFileRepository::new(
                identity.database().clone(),
                uploads,
            )),
            identity,
            cookie_mode,
            reader: FileReader::new(),
        }
    }

    /// Tests run in a test binary, which cannot be the parser.
    #[must_use]
    pub fn with_parser(mut self, program: impl Into<PathBuf>) -> Self {
        self.reader = FileReader::with_parser(program);
        self
    }

    /// The repository behind the routes (also used for server-side imports).
    #[must_use]
    pub fn repository(&self) -> Arc<PageFileRepository> {
        Arc::clone(&self.files)
    }
}

pub fn page_file_router(state: PageFileState) -> Router {
    let request_limit =
        usize::try_from(state.files.uploads().limits().max_request_bytes()).unwrap_or(usize::MAX);
    Router::new()
        .route(
            "/api/v1/workspaces/{workspace_id}/pages/{page_id}/files",
            post(upload_page_file).get(list_page_files),
        )
        .route(
            "/api/v1/workspaces/{workspace_id}/pages/{page_id}/files/{file_id}",
            get(download_page_file),
        )
        .route(
            "/api/v1/workspaces/{workspace_id}/pages/{page_id}/files/{file_id}/content",
            get(get_page_file_content).put(save_page_file),
        )
        .route(
            "/api/v1/workspaces/{workspace_id}/pages/{page_id}/files/{file_id}/versions",
            get(list_page_file_versions),
        )
        .route(
            "/api/v1/workspaces/{workspace_id}/pages/{page_id}/files/{file_id}/versions/{version}",
            get(download_page_file_version),
        )
        .route(
            "/api/v1/workspaces/{workspace_id}/pages/{page_id}/files/{file_id}/versions/{version}/restore",
            post(restore_page_file_version),
        )
        // Multipart stays streaming; this bounds boundaries, headers, and ignored fields too.
        .layer(DefaultBodyLimit::max(request_limit))
        .with_state(state)
}

#[derive(ToSchema)]
#[allow(dead_code)]
struct PageFileUploadBody {
    #[schema(value_type = String, format = Binary)]
    file: Vec<u8>,
}

#[derive(ToSchema)]
#[schema(value_type = String, format = Binary)]
#[allow(dead_code)]
struct PageFileDownload(Vec<u8>);

#[utoipa::path(post, path = "/api/v1/workspaces/{workspace_id}/pages/{page_id}/files", params(("workspace_id" = String, Path), ("page_id" = String, Path)), request_body(content = PageFileUploadBody, content_type = "multipart/form-data"), responses((status = 201, body = PageFile)))]
async fn upload_page_file(
    State(state): State<PageFileState>,
    Path((workspace, page)): Path<(String, String)>,
    request: Request,
) -> Result<Response, ApiError> {
    let instance = format!("/api/v1/workspaces/{workspace}/pages/{page}/files");
    let request_id = request
        .extensions()
        .get::<RequestId>()
        .cloned()
        .map(Extension);
    let request_id = request_id.as_ref();
    // Authorize before the body is read.
    let session = authenticate_session(
        &state.identity,
        state.cookie_mode,
        request.headers(),
        &instance,
        request_id,
    )
    .await?;
    let actor_id = session.user.id;
    let workspace_id = parse_id(&workspace, &instance, request_id)?;
    let page_id = parse_id(&page, &instance, request_id)?;
    state
        .files
        .authorize_upload(workspace_id, page_id, actor_id)
        .await
        .map_err(|error| problem(error, &instance, request_id))?;
    let mut staged = stage_single_file(state.files.uploads(), request, workspace_id, actor_id)
        .await
        .map_err(|failure| match failure {
            StageFailure::TooLarge => too_large(&instance, request_id),
            StageFailure::InvalidMultipart => invalid_multipart(&instance, request_id),
            StageFailure::Upload(error) => upload_problem(error, &instance, request_id),
        })?;
    let upload = staged.upload().clone();
    match state
        .files
        .finalize(
            workspace_id,
            page_id,
            actor_id,
            &upload,
            request_id_value(request_id),
        )
        .await
    {
        Ok(file) => {
            staged.complete();
            Ok((StatusCode::CREATED, Json(file)).into_response())
        }
        Err(error) => {
            staged.discard().await;
            Err(problem(error, &instance, request_id))
        }
    }
}

#[utoipa::path(get, path = "/api/v1/workspaces/{workspace_id}/pages/{page_id}/files/{file_id}", params(("workspace_id" = String, Path), ("page_id" = String, Path), ("file_id" = String, Path)), responses((status = 200, body = PageFileDownload, content_type = "application/octet-stream")))]
async fn download_page_file(
    State(state): State<PageFileState>,
    Path((workspace, page, file)): Path<(String, String, String)>,
    headers: HeaderMap,
    request_id: Option<Extension<RequestId>>,
) -> Result<Response, ApiError> {
    let instance = format!("/api/v1/workspaces/{workspace}/pages/{page}/files/{file}");
    let request_id = request_id.as_ref();
    let session = authenticate_session(
        &state.identity,
        state.cookie_mode,
        &headers,
        &instance,
        request_id,
    )
    .await?;
    let workspace_id = parse_id(&workspace, &instance, request_id)?;
    let page_id = parse_id(&page, &instance, request_id)?;
    let file_id = parse_id(&file, &instance, request_id)?;
    let download = state
        .files
        .download(workspace_id, page_id, file_id, session.user.id)
        .await
        .map_err(|error| problem(error, &instance, request_id))?;
    download_response(download).ok_or_else(|| internal(&instance, request_id))
}

#[derive(Serialize, ToSchema)]
struct PageFileList {
    items: Vec<PageFile>,
}

#[derive(Serialize, ToSchema)]
struct PageFileVersionList {
    items: Vec<PageFileVersion>,
}

/// A file and one part of its content.
#[derive(Serialize, ToSchema)]
struct PageFileContent {
    file: PageFile,
    /// Sheet names of a workbook.
    sheets: Vec<String>,
    sheet: Option<String>,
    rows: Option<Vec<Vec<String>>>,
    row_numbers: Option<Vec<u64>>,
    /// Cells among `rows` that hold a formula, as `B2`. They are read-only.
    formula_cells: Vec<String>,
    text: Option<String>,
    /// Where the next part starts; `null` at the end.
    next_offset: Option<u64>,
    /// A PDF or Word file without text, such as a scan.
    no_text: bool,
}

/// `text` replaces the file; for `csv` and `tsv`, `rows` writes the rows in the file's delimiter,
/// quoting and line ends; for `xlsx`, `edits` changes cells of `sheet` (default: the first) and
/// leaves the rest of the workbook as it is.
#[derive(Deserialize, ToSchema)]
struct SavePageFileBody {
    expected_version: u64,
    text: Option<String>,
    rows: Option<Vec<Vec<String>>>,
    sheet: Option<String>,
    edits: Option<Vec<CellEdit>>,
}

#[derive(Deserialize, ToSchema)]
struct RestorePageFileBody {
    expected_version: u64,
}

#[utoipa::path(get, path = "/api/v1/workspaces/{workspace_id}/pages/{page_id}/files", params(("workspace_id" = String, Path), ("page_id" = String, Path)), responses((status = 200, body = PageFileList)))]
async fn list_page_files(
    State(state): State<PageFileState>,
    Path((workspace, page)): Path<(String, String)>,
    headers: HeaderMap,
    request_id: Option<Extension<RequestId>>,
) -> Result<Json<PageFileList>, ApiError> {
    let instance = format!("/api/v1/workspaces/{workspace}/pages/{page}/files");
    let request_id = request_id.as_ref();
    let session = authenticate_session(
        &state.identity,
        state.cookie_mode,
        &headers,
        &instance,
        request_id,
    )
    .await?;
    let workspace_id = parse_id(&workspace, &instance, request_id)?;
    let page_id = parse_id(&page, &instance, request_id)?;
    let items = state
        .files
        .list(workspace_id, page_id, session.user.id)
        .await
        .map_err(|error| problem(error, &instance, request_id))?;
    Ok(Json(PageFileList { items }))
}

#[utoipa::path(get, path = "/api/v1/workspaces/{workspace_id}/pages/{page_id}/files/{file_id}/content", params(("workspace_id" = String, Path), ("page_id" = String, Path), ("file_id" = String, Path), ContentQuery), responses((status = 200, body = PageFileContent)))]
async fn get_page_file_content(
    State(state): State<PageFileState>,
    Path((workspace, page, file)): Path<(String, String, String)>,
    Query(query): Query<ContentQuery>,
    headers: HeaderMap,
    request_id: Option<Extension<RequestId>>,
) -> Result<Json<PageFileContent>, ApiError> {
    let instance = format!("/api/v1/workspaces/{workspace}/pages/{page}/files/{file}/content");
    let request_id = request_id.as_ref();
    let session = authenticate_session(
        &state.identity,
        state.cookie_mode,
        &headers,
        &instance,
        request_id,
    )
    .await?;
    let workspace_id = parse_id(&workspace, &instance, request_id)?;
    let page_id = parse_id(&page, &instance, request_id)?;
    let file_id = parse_id(&file, &instance, request_id)?;
    let record = state
        .files
        .record(workspace_id, page_id, file_id, session.user.id)
        .await
        .map_err(|error| problem(error, &instance, request_id))?;
    if record.deleted {
        return Err(problem(PageFileError::Deleted, &instance, request_id));
    }
    let kind = record.file.kind;
    let uploads = state.files.uploads();
    let parsed = state
        .reader
        .read(
            kind,
            &record.file.file_name,
            record.file.size_bytes,
            query,
            uploads.read_blob(&record.storage_key),
        )
        .await
        .map_err(|error| read_error(error, &instance, request_id))?;
    let parses = parsed.is_some();
    let parsed = parsed.unwrap_or_default();
    let mut file = record.file;
    // Bytes that are not UTF-8 would not survive an edit.
    file.can_edit =
        file.can_edit && file.size_bytes <= kind.max_edit_bytes() && (parsed.valid_utf8 || !parses);
    Ok(Json(PageFileContent {
        file,
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

#[utoipa::path(put, path = "/api/v1/workspaces/{workspace_id}/pages/{page_id}/files/{file_id}/content", params(("workspace_id" = String, Path), ("page_id" = String, Path), ("file_id" = String, Path)), request_body = SavePageFileBody, responses((status = 200, body = PageFile)))]
async fn save_page_file(
    State(state): State<PageFileState>,
    Path((workspace, page, file)): Path<(String, String, String)>,
    headers: HeaderMap,
    request_id: Option<Extension<RequestId>>,
    Json(body): Json<SavePageFileBody>,
) -> Result<Json<PageFile>, ApiError> {
    let instance = format!("/api/v1/workspaces/{workspace}/pages/{page}/files/{file}/content");
    let request_id = request_id.as_ref();
    let session = authenticate_session(
        &state.identity,
        state.cookie_mode,
        &headers,
        &instance,
        request_id,
    )
    .await?;
    let workspace_id = parse_id(&workspace, &instance, request_id)?;
    let page_id = parse_id(&page, &instance, request_id)?;
    let file_id = parse_id(&file, &instance, request_id)?;
    let actor = session.user.id;
    let record = state
        .files
        .record(workspace_id, page_id, file_id, actor)
        .await
        .map_err(|error| problem(error, &instance, request_id))?;
    if !record.file.kind.editable() {
        return Err(problem(PageFileError::NotEditable, &instance, request_id));
    }
    let kind = record.file.kind;
    let bytes = match (body.text, body.rows, body.edits) {
        (Some(text), None, None) if kind != FileKind::Sheet && !text.contains('\0') => {
            text.into_bytes()
        }
        (None, None, Some(edits)) if kind == FileKind::Sheet => {
            // Checked before the work; the save checks them again in its transaction.
            if record.locked {
                return Err(problem(PageFileError::Locked, &instance, request_id));
            }
            if record.file.version != body.expected_version {
                return Err(problem(
                    PageFileError::VersionConflict {
                        current: Box::new(record.file),
                    },
                    &instance,
                    request_id,
                ));
            }
            if record.deleted {
                return Err(problem(PageFileError::Deleted, &instance, request_id));
            }
            if record.file.size_bytes > kind.max_edit_bytes() {
                return Err(problem(
                    PageFileError::TooLargeToEdit,
                    &instance,
                    request_id,
                ));
            }
            let _permit = state
                .reader
                .permit()
                .await
                .map_err(|error| read_error(error, &instance, request_id))?;
            let current = state
                .files
                .uploads()
                .read_blob(&record.storage_key)
                .await
                .map_err(|error| upload_problem(error, &instance, request_id))?;
            let program = state
                .reader
                .parser()
                .map_err(|error| read_error(error, &instance, request_id))?;
            let request = PatchRequest {
                sheet: body.sheet,
                edits,
            };
            file_parse::patch_in_child(program, current, &request)
                .await
                .map_err(|error| patch_problem(error, &instance, request_id))?
        }
        (None, Some(rows), None) if kind == FileKind::Delimited => {
            // The file's own style; a file too large to read here cannot be saved anyway.
            if record.file.size_bytes > file_parse::MAX_EDIT_BYTES {
                return Err(problem(
                    PageFileError::TooLargeToEdit,
                    &instance,
                    request_id,
                ));
            }
            let current = state
                .files
                .uploads()
                .read_blob(&record.storage_key)
                .await
                .map_err(|error| upload_problem(error, &instance, request_id))?;
            let style = file_parse::delimited_style(&current, is_tsv(&record.file.file_name));
            file_parse::write_delimited(&rows, style)
                .map_err(|_| invalid_content(&instance, request_id))?
        }
        _ => return Err(invalid_content(&instance, request_id)),
    };
    let saved = state
        .files
        .save(
            workspace_id,
            page_id,
            file_id,
            actor,
            body.expected_version,
            &bytes,
            request_id_value(request_id),
        )
        .await
        .map_err(|error| problem(error, &instance, request_id))?;
    Ok(Json(saved))
}

#[utoipa::path(get, path = "/api/v1/workspaces/{workspace_id}/pages/{page_id}/files/{file_id}/versions", params(("workspace_id" = String, Path), ("page_id" = String, Path), ("file_id" = String, Path)), responses((status = 200, body = PageFileVersionList)))]
async fn list_page_file_versions(
    State(state): State<PageFileState>,
    Path((workspace, page, file)): Path<(String, String, String)>,
    headers: HeaderMap,
    request_id: Option<Extension<RequestId>>,
) -> Result<Json<PageFileVersionList>, ApiError> {
    let instance = format!("/api/v1/workspaces/{workspace}/pages/{page}/files/{file}/versions");
    let request_id = request_id.as_ref();
    let session = authenticate_session(
        &state.identity,
        state.cookie_mode,
        &headers,
        &instance,
        request_id,
    )
    .await?;
    let workspace_id = parse_id(&workspace, &instance, request_id)?;
    let page_id = parse_id(&page, &instance, request_id)?;
    let file_id = parse_id(&file, &instance, request_id)?;
    let items = state
        .files
        .versions(workspace_id, page_id, file_id, session.user.id)
        .await
        .map_err(|error| problem(error, &instance, request_id))?;
    Ok(Json(PageFileVersionList { items }))
}

#[utoipa::path(get, path = "/api/v1/workspaces/{workspace_id}/pages/{page_id}/files/{file_id}/versions/{version}", params(("workspace_id" = String, Path), ("page_id" = String, Path), ("file_id" = String, Path), ("version" = u64, Path)), responses((status = 200, body = PageFileDownload, content_type = "application/octet-stream")))]
async fn download_page_file_version(
    State(state): State<PageFileState>,
    Path((workspace, page, file, version)): Path<(String, String, String, u64)>,
    headers: HeaderMap,
    request_id: Option<Extension<RequestId>>,
) -> Result<Response, ApiError> {
    let instance =
        format!("/api/v1/workspaces/{workspace}/pages/{page}/files/{file}/versions/{version}");
    let request_id = request_id.as_ref();
    let session = authenticate_session(
        &state.identity,
        state.cookie_mode,
        &headers,
        &instance,
        request_id,
    )
    .await?;
    let workspace_id = parse_id(&workspace, &instance, request_id)?;
    let page_id = parse_id(&page, &instance, request_id)?;
    let file_id = parse_id(&file, &instance, request_id)?;
    let download = state
        .files
        .download_version(workspace_id, page_id, file_id, version, session.user.id)
        .await
        .map_err(|error| problem(error, &instance, request_id))?;
    download_response(download).ok_or_else(|| internal(&instance, request_id))
}

#[utoipa::path(post, path = "/api/v1/workspaces/{workspace_id}/pages/{page_id}/files/{file_id}/versions/{version}/restore", params(("workspace_id" = String, Path), ("page_id" = String, Path), ("file_id" = String, Path), ("version" = u64, Path)), request_body = RestorePageFileBody, responses((status = 200, body = PageFile)))]
async fn restore_page_file_version(
    State(state): State<PageFileState>,
    Path((workspace, page, file, version)): Path<(String, String, String, u64)>,
    headers: HeaderMap,
    request_id: Option<Extension<RequestId>>,
    Json(body): Json<RestorePageFileBody>,
) -> Result<Json<PageFile>, ApiError> {
    let instance = format!(
        "/api/v1/workspaces/{workspace}/pages/{page}/files/{file}/versions/{version}/restore"
    );
    let request_id = request_id.as_ref();
    let session = authenticate_session(
        &state.identity,
        state.cookie_mode,
        &headers,
        &instance,
        request_id,
    )
    .await?;
    let workspace_id = parse_id(&workspace, &instance, request_id)?;
    let page_id = parse_id(&page, &instance, request_id)?;
    let file_id = parse_id(&file, &instance, request_id)?;
    let restored = state
        .files
        .restore(
            workspace_id,
            page_id,
            file_id,
            version,
            session.user.id,
            body.expected_version,
            request_id_value(request_id),
        )
        .await
        .map_err(|error| problem(error, &instance, request_id))?;
    Ok(Json(restored))
}

fn parse_id(
    value: &str,
    instance: &str,
    request_id: Option<&Extension<RequestId>>,
) -> Result<Id, ApiError> {
    value.parse().map_err(|_| not_found(instance, request_id))
}

fn not_found(instance: &str, request_id: Option<&Extension<RequestId>>) -> ApiError {
    ApiError::new(
        StatusCode::NOT_FOUND,
        "page_file_not_found",
        "File not found",
        "The requested page or file was not found.",
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

fn invalid_content(instance: &str, request_id: Option<&Extension<RequestId>>) -> ApiError {
    ApiError::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        "validation_failed",
        "Validation failed",
        "Send the file's content as `text`, as `rows` for a csv or tsv file, or as `edits` for an xlsx file.",
        instance,
        request_id,
    )
}

fn patch_problem(
    error: PatchError,
    instance: &str,
    request_id: Option<&Extension<RequestId>>,
) -> ApiError {
    let detail = error.to_string();
    let code = match error {
        PatchError::Formula(_) | PatchError::TableHeader(_) | PatchError::Protected(_) => {
            "cell_read_only"
        }
        PatchError::InvalidCell(_)
        | PatchError::InvalidValue(_)
        | PatchError::TooManyEdits
        | PatchError::UnknownSheet => "validation_failed",
        PatchError::Limit => "file_over_limit",
        PatchError::Unreadable => "file_unreadable",
    };
    ApiError::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        code,
        "File not changed",
        "The file was not changed.",
        instance,
        request_id,
    )
    .with_detail(detail)
}

fn read_error(
    error: ReadError,
    instance: &str,
    request_id: Option<&Extension<RequestId>>,
) -> ApiError {
    match error {
        ReadError::Upload(error) => upload_problem(error, instance, request_id),
        error => read_problem(error, instance, request_id),
    }
}

fn internal(instance: &str, request_id: Option<&Extension<RequestId>>) -> ApiError {
    ApiError::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        "internal_error",
        "Internal server error",
        "An unexpected error occurred. Use the request ID when contacting support.",
        instance,
        request_id,
    )
}

fn upload_problem(
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
        UploadError::InvalidDisplayName => ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "validation_failed",
            "Validation failed",
            "The file name is invalid.",
            instance,
            request_id,
        ),
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
        UploadError::Unauthorized => not_found(instance, request_id),
        _ => internal(instance, request_id),
    }
}

fn problem(
    error: PageFileError,
    instance: &str,
    request_id: Option<&Extension<RequestId>>,
) -> ApiError {
    match error {
        PageFileError::NotFound => not_found(instance, request_id),
        PageFileError::Upload(error) => upload_problem(error, instance, request_id),
        PageFileError::Database(_) => internal(instance, request_id),
        PageFileError::VersionConflict { current } => ApiError::version_conflict(
            serde_json::to_value(&*current).unwrap_or_default(),
            instance.to_owned(),
            request_id,
        ),
        PageFileError::NotEditable => ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "file_not_editable",
            "File cannot be edited",
            "Only csv, tsv, text and xlsx files can change. PDF, xls and other files are read-only.",
            instance,
            request_id,
        ),
        PageFileError::TooLargeToEdit => ApiError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "file_too_large_to_edit",
            "File too large to edit",
            "This file is too large to edit here (2 MB for text and csv, 25 MB for xlsx).",
            instance,
            request_id,
        ),
        PageFileError::Locked => ApiError::new(
            StatusCode::LOCKED,
            "page_locked",
            "Page locked",
            "The page is locked. Unlock it to change its files.",
            instance,
            request_id,
        ),
        PageFileError::Deleted => ApiError::new(
            StatusCode::GONE,
            "file_deleted",
            "File deleted",
            "An admin deleted this stored file.",
            instance,
            request_id,
        ),
    }
}
