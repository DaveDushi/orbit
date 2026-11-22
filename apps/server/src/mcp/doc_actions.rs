//! Doc search, recents, favorites, copies, locks, version restores, and doc comment
//! thread edits for API tokens and MCP.
use rmcp::ErrorData;
use rmcp::model::{CallToolRequestParams, Tool};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{EmptyArgs, arguments, definition, parse_id, write_definition};
use crate::repositories::api_tokens::ApiTokenScope;
use crate::token_api::{Authorized, TokenApiError};

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SearchArgs {
    /// Words to find in titles and text, 1 to 200 characters. Every word
    /// must match, the last one as a prefix; case and accents are ignored.
    query: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RecentArgs {
    /// Docs to return, from 1 to 50. Default: 10.
    limit: Option<usize>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct DocArgs {
    /// A doc id, from list_docs or search_docs.
    page_id: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct MoveFavoriteArgs {
    /// A favorite doc, from list_favorite_docs.
    page_id: String,
    /// The new index among the favorites, from 0; past the last one moves it last.
    position: i64,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct DuplicateArgs {
    /// The doc to copy, from list_docs.
    page_id: String,
    /// Also copy its live sub-docs. Default: false.
    #[serde(default)]
    include_children: bool,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct LockArgs {
    page_id: String,
    /// true locks the doc (its title, icon and content cannot change), false unlocks it.
    locked: bool,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RestoreVersionArgs {
    page_id: String,
    /// A version id, from list_doc_versions.
    version_id: String,
    /// The doc's version, from get_doc (not the version id).
    expected_version: u64,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct UpdateCommentArgs {
    page_id: String,
    /// The thread, from list_doc_comments.
    thread_id: String,
    /// One of your own comments in the thread, from list_doc_comments.
    comment_id: String,
    /// The new comment as Markdown, 1 to 100,000 bytes; @Name mentions a member.
    body: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CommentArgs {
    page_id: String,
    /// The thread, from list_doc_comments.
    thread_id: String,
    /// One of your own comments in the thread, from list_doc_comments.
    comment_id: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ThreadArgs {
    page_id: String,
    /// The thread, from list_doc_comments.
    thread_id: String,
}

pub(super) fn scope(name: &str) -> Option<ApiTokenScope> {
    match name {
        "search_docs" | "list_recent_docs" | "list_favorite_docs" => Some(ApiTokenScope::DocsRead),
        "add_favorite_doc"
        | "remove_favorite_doc"
        | "move_favorite_doc"
        | "duplicate_doc"
        | "restore_doc_version"
        | "update_doc_comment"
        | "resolve_doc_thread"
        | "reopen_doc_thread" => Some(ApiTokenScope::DocsWrite),
        "set_doc_lock" | "delete_doc_comment" | "delete_doc_thread" => {
            Some(ApiTokenScope::DocsManage)
        }
        _ => None,
    }
}

pub(super) fn tools() -> Vec<Tool> {
    vec![
        definition::<SearchArgs>(
            "search_docs",
            "Search the docs that you can see by title and text, best match first. Each result has a snippet; read a doc with get_doc.",
        ),
        definition::<RecentArgs>(
            "list_recent_docs",
            "List the docs that you opened last in Orbit, most recent first.",
        ),
        definition::<EmptyArgs>(
            "list_favorite_docs",
            "List your favorite docs in their order.",
        ),
        write_definition::<DocArgs>(
            "add_favorite_doc",
            "Add a doc to your favorites, as the last one. A favorite stays where it is.",
            false,
        ),
        write_definition::<DocArgs>(
            "remove_favorite_doc",
            "Remove a doc from your favorites. The doc itself stays.",
            false,
        ),
        write_definition::<MoveFavoriteArgs>(
            "move_favorite_doc",
            "Move one of your favorite docs to another place. Returns the favorites in the new order.",
            false,
        ),
        write_definition::<DuplicateArgs>(
            "duplicate_doc",
            "Copy a doc (and with include_children its sub-docs) right after it, titled \"<title> (copy)\". Returns the copy as get_doc does.",
            false,
        ),
        write_definition::<LockArgs>(
            "set_doc_lock",
            "Lock or unlock a doc. A locked doc's title, icon and content cannot change until someone unlocks it. Returns the doc as get_doc does.",
            true,
        ),
        write_definition::<RestoreVersionArgs>(
            "restore_doc_version",
            "Replace a doc's content with a saved version from list_doc_versions. The current content is saved as a version first. Returns the doc as get_doc does.",
            true,
        ),
        write_definition::<UpdateCommentArgs>(
            "update_doc_comment",
            "Edit one of your own comments on a doc. Returns the comment as list_doc_comments does.",
            true,
        ),
        write_definition::<CommentArgs>(
            "delete_doc_comment",
            "Delete one of your own comments on a doc. It keeps its place in the thread, empty. This cannot be undone.",
            true,
        ),
        write_definition::<ThreadArgs>(
            "resolve_doc_thread",
            "Resolve a comment thread of a doc. Returns the thread as list_doc_comments does.",
            false,
        ),
        write_definition::<ThreadArgs>(
            "reopen_doc_thread",
            "Reopen a resolved comment thread of a doc. Returns the thread as list_doc_comments does.",
            false,
        ),
        write_definition::<ThreadArgs>(
            "delete_doc_thread",
            "Delete a comment thread of a doc with all its comments. Its creator may delete it while nobody else commented; a member who may moderate comments may delete any thread. This cannot be undone.",
            true,
        ),
    ]
}

/// Runs a tool of this module; `None` for another tool's name.
pub(super) async fn call(
    api: &Authorized<'_>,
    request: &CallToolRequestParams,
) -> Option<Result<Result<Value, TokenApiError>, ErrorData>> {
    let result = match request.name.as_ref() {
        "search_docs" => {
            let run = async {
                let a: SearchArgs = arguments(request)?;
                Ok(api.search_docs(&a.query).await.map(|found| json!(found)))
            };
            run.await
        }
        "list_recent_docs" => {
            let run = async {
                let a: RecentArgs = arguments(request)?;
                Ok(api.recent_docs(a.limit).await.map(|list| json!(list)))
            };
            run.await
        }
        "list_favorite_docs" => {
            let run = async {
                let _: EmptyArgs = arguments(request)?;
                Ok(api.favorite_docs().await.map(|list| json!(list)))
            };
            run.await
        }
        "add_favorite_doc" => {
            let run = async {
                let a: DocArgs = arguments(request)?;
                Ok(api
                    .add_favorite_doc(parse_id(&a.page_id)?)
                    .await
                    .map(|favorite| json!(favorite)))
            };
            run.await
        }
        "remove_favorite_doc" => {
            let run = async {
                let a: DocArgs = arguments(request)?;
                Ok(api
                    .remove_favorite_doc(parse_id(&a.page_id)?)
                    .await
                    .map(|()| json!({ "removed": true })))
            };
            run.await
        }
        "move_favorite_doc" => {
            let run = async {
                let a: MoveFavoriteArgs = arguments(request)?;
                Ok(api
                    .move_favorite_doc(parse_id(&a.page_id)?, a.position)
                    .await
                    .map(|list| json!(list)))
            };
            run.await
        }
        "duplicate_doc" => {
            let run = async {
                let a: DuplicateArgs = arguments(request)?;
                Ok(api
                    .duplicate_doc(parse_id(&a.page_id)?, a.include_children)
                    .await
                    .map(|doc| json!(doc)))
            };
            run.await
        }
        "set_doc_lock" => {
            let run = async {
                let a: LockArgs = arguments(request)?;
                Ok(api
                    .set_doc_lock(parse_id(&a.page_id)?, a.locked)
                    .await
                    .map(|doc| json!(doc)))
            };
            run.await
        }
        "restore_doc_version" => {
            let run = async {
                let a: RestoreVersionArgs = arguments(request)?;
                Ok(api
                    .restore_doc_version(
                        parse_id(&a.page_id)?,
                        parse_id(&a.version_id)?,
                        a.expected_version,
                    )
                    .await
                    .map(|doc| json!(doc)))
            };
            run.await
        }
        "update_doc_comment" => {
            let run = async {
                let a: UpdateCommentArgs = arguments(request)?;
                Ok(api
                    .update_doc_comment(
                        parse_id(&a.page_id)?,
                        parse_id(&a.thread_id)?,
                        parse_id(&a.comment_id)?,
                        &a.body,
                    )
                    .await
                    .map(|comment| json!(comment)))
            };
            run.await
        }
        "delete_doc_comment" => {
            let run = async {
                let a: CommentArgs = arguments(request)?;
                Ok(api
                    .delete_doc_comment(
                        parse_id(&a.page_id)?,
                        parse_id(&a.thread_id)?,
                        parse_id(&a.comment_id)?,
                    )
                    .await
                    .map(|()| json!({ "deleted": true })))
            };
            run.await
        }
        "resolve_doc_thread" | "reopen_doc_thread" => {
            let run = async {
                let a: ThreadArgs = arguments(request)?;
                let resolved = request.name == "resolve_doc_thread";
                Ok(api
                    .set_doc_thread_resolved(
                        parse_id(&a.page_id)?,
                        parse_id(&a.thread_id)?,
                        resolved,
                    )
                    .await
                    .map(|thread| json!(thread)))
            };
            run.await
        }
        "delete_doc_thread" => {
            let run = async {
                let a: ThreadArgs = arguments(request)?;
                Ok(api
                    .delete_doc_thread(parse_id(&a.page_id)?, parse_id(&a.thread_id)?)
                    .await
                    .map(|()| json!({ "deleted": true })))
            };
            run.await
        }
        _ => return None,
    };
    Some(result)
}
