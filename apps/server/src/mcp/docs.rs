//! Doc trash, moves and stored versions.
use rmcp::ErrorData;
use rmcp::model::{CallToolRequestParams, Tool};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{arguments, definition, parse_id, write_definition};
use crate::repositories::api_tokens::ApiTokenScope;
use crate::token_api::{Authorized, DocMove, TokenApiError};

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct DeleteDocArgs {
    page_id: String,
    /// The `version` that you read.
    expected_version: u64,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct MoveDocArgs {
    page_id: String,
    /// The `version` that you read.
    expected_version: u64,
    /// The new parent document. Omit it to move the document to a space root.
    parent_id: Option<String>,
    /// Root moves only: the target teamspace. Omit it to keep the document's space.
    teamspace_id: Option<String>,
    /// Root moves only: `true` moves the document to the connected user's private space.
    private: Option<bool>,
    /// The place among the new siblings (0 is first). Default: last.
    position: Option<u64>,
    /// `true` checks the move and returns the document as it would be, without moving it.
    #[serde(default)]
    dry_run: bool,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct DocVersionsArgs {
    page_id: String,
    /// Continue from the next_cursor of the previous response.
    cursor: Option<String>,
    /// Maximum number of versions, from 1 to 100. Default: 50.
    limit: Option<usize>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct DocVersionArgs {
    page_id: String,
    /// A version id from list_doc_versions.
    version_id: String,
}

pub(super) fn scope(name: &str) -> Option<ApiTokenScope> {
    match name {
        "list_doc_versions" | "get_doc_version" => Some(ApiTokenScope::DocsRead),
        "delete_doc" | "move_doc" => Some(ApiTokenScope::DocsManage),
        _ => None,
    }
}

pub(super) fn tools() -> Vec<Tool> {
    vec![
        write_definition::<DeleteDocArgs>(
            "delete_doc",
            "Move a visible document and its sub-documents to the trash, using its current version. People can restore them from the trash; deleting forever is not available here.",
            true,
        ),
        write_definition::<MoveDocArgs>(
            "move_doc",
            "Move a visible document, with its sub-documents, under another document or to a space root, using its current version. Who sees the document follows its new place. Send dry_run: true first to check the move without doing it; version stays the current one then.",
            true,
        ),
        definition::<DocVersionsArgs>(
            "list_doc_versions",
            "List the stored versions of a visible document, newest first, without content.",
        ),
        definition::<DocVersionArgs>(
            "get_doc_version",
            "Read one stored version of a visible document, with its content as Markdown. To restore it, send that Markdown to update_doc.",
        ),
    ]
}

/// Runs a tool of this module; `None` for another tool's name.
pub(super) async fn call(
    api: &Authorized<'_>,
    request: &CallToolRequestParams,
) -> Option<Result<Result<Value, TokenApiError>, ErrorData>> {
    scope(&request.name)?;
    Some(run(api, request).await)
}

async fn run(
    api: &Authorized<'_>,
    request: &CallToolRequestParams,
) -> Result<Result<Value, TokenApiError>, ErrorData> {
    Ok(match request.name.as_ref() {
        "delete_doc" => {
            let a: DeleteDocArgs = arguments(request)?;
            api.delete_doc(parse_id(&a.page_id)?, a.expected_version)
                .await
                .map(|()| json!({"deleted": true}))
        }
        "move_doc" => {
            let a: MoveDocArgs = arguments(request)?;
            let target = DocMove {
                parent_id: a.parent_id.as_deref().map(parse_id).transpose()?,
                teamspace_id: a.teamspace_id.as_deref().map(parse_id).transpose()?,
                private: a.private,
                position: a.position,
                dry_run: a.dry_run,
            };
            api.move_doc(parse_id(&a.page_id)?, a.expected_version, target)
                .await
                .map(|doc| json!(doc))
        }
        "list_doc_versions" => {
            let a: DocVersionsArgs = arguments(request)?;
            api.doc_versions(parse_id(&a.page_id)?, a.cursor.as_deref(), a.limit)
                .await
                .map(|page| json!({"items": page.items, "next_cursor": page.next_cursor}))
        }
        "get_doc_version" => {
            let a: DocVersionArgs = arguments(request)?;
            api.doc_version(parse_id(&a.page_id)?, parse_id(&a.version_id)?)
                .await
                .map(|version| json!(version))
        }
        _ => return Err(ErrorData::invalid_params("Unknown tool", None)),
    })
}
