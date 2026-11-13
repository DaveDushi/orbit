//! One part of a Drive file as rows or text, for the viewer's sheet grid. Read-only, with the checks of the
//! file's download URL: a member of the workspace (not a guest), and a live file.

use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;
use utoipa::ToSchema;

use crate::drive_routes::{DriveState, caller};
use crate::file_content::{ContentQuery, ReadError, read_problem};
use crate::file_parse::{FileKind, ParseError};
use crate::mcp_drive::drive_kind;
use crate::repositories::drive::{DriveError, DriveNode};
use crate::task_routes::ApiError;

pub fn drive_content_router(state: DriveState) -> Router {
    Router::new()
        .route(
            "/api/v1/workspaces/{workspace_id}/drive/nodes/{node_id}/content",
            get(get_drive_file_content),
        )
        .with_state(state)
}

/// A Drive file and one part of its content: the fields of a chat file's content.
#[derive(Serialize, ToSchema)]
pub(crate) struct DriveFileContent {
    file: DriveNode,
    kind: FileKind,
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

/// One part of a Drive file, as rows or text, with the query of a chat file's content. Images and other files
/// have no content here. Files over 16 MiB are not read (`file_over_limit`).
#[utoipa::path(get, path = "/api/v1/workspaces/{workspace_id}/drive/nodes/{node_id}/content", params(("workspace_id" = String, Path), ("node_id" = String, Path), ContentQuery), responses((status = 200, body = DriveFileContent)))]
pub(crate) async fn get_drive_file_content(
    State(state): State<DriveState>,
    Path((workspace, node)): Path<(String, String)>,
    Query(query): Query<ContentQuery>,
    headers: HeaderMap,
    request_id: Option<axum::extract::Extension<orbit_platform::RequestId>>,
) -> Result<Json<DriveFileContent>, ApiError> {
    let instance = format!("/api/v1/workspaces/{workspace}/drive/nodes/{node}/content");
    let caller = caller(&state, &headers, &workspace, instance, request_id.as_ref()).await?;
    let node_id = caller.id(&node)?;
    let file = state
        .drive
        .repository()
        .node(caller.workspace_id, caller.user_id, node_id)
        .await
        .map_err(|error| caller.fail(error))?
        .node;
    let kind = drive_kind(&file.name, file.mime.as_deref().unwrap_or_default());
    let parsed = if matches!(kind, FileKind::Image | FileKind::Other) {
        None
    } else {
        // The same checks as the download URL (a live file of the caller's workspace), then the bytes.
        let bytes = match state
            .drive
            .file_bytes(caller.workspace_id, caller.user_id, node_id)
            .await
        {
            Ok((_, bytes)) => bytes,
            Err(DriveError::TooLarge) => {
                return Err(read_problem(
                    ReadError::Parse(ParseError::Limit),
                    &caller.instance,
                    caller.request_id,
                ));
            }
            Err(error) => return Err(caller.fail(error)),
        };
        let size = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        state
            .reader
            .read(kind, &file.name, size, query, async { Ok(bytes) })
            .await
            .map_err(|error| read_problem(error, &caller.instance, caller.request_id))?
    }
    .unwrap_or_default();
    Ok(Json(DriveFileContent {
        file,
        kind,
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
