//! Docs pages and tasks to the Trash for the team agent: `POST /api/v1/workspaces/{id}/agent-tools/trash`, for
//! agent sessions of the driver of a running thread only (as `/agent-tools/drive`). One tool, `move_to_trash`,
//! takes pages and tasks together, so a batch needs one approval with the whole list.
//!
//! Each call runs only after the driver answered "yes" in the thread (`super::integrations::approve`, one approval
//! for one run of exactly these arguments). It runs the web's own soft deletes as the driver
//! (`PageRepository::delete_page`, `TaskRepository::delete_task`): a page takes its live sub-pages along, a task
//! its live sub-issues, and members restore them from the Trash for 30 days. The agent cannot delete forever, and
//! agent sessions cannot reach the direct delete routes (`super::scope`).

use axum::extract::{Extension, Path, State};
use axum::http::{HeaderMap, Uri};
use axum::routing::post;
use axum::{Json, Router};
use orbit_platform::{Database, Id, RequestId, TimestampMillis};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::AgentState;
use super::integrations::{
    Approval, Approves, approve, canonical, driver, hash, require_running, unavailable,
};
use crate::audit::{self, AuditOutcome};
use crate::repositories::pages::{PageError, PageRepository};
use crate::repositories::sub_issues;
use crate::repositories::tasks::{TaskError, TaskRepository};
use crate::task_routes::ApiError;

type RequestIdExtension = Option<Extension<RequestId>>;

pub const TOOL: &str = "move_to_trash";
/// The most pages and tasks one call (one approval) moves.
const MAX_ITEMS: usize = 50;

pub fn router(state: AgentState) -> Router {
    Router::new()
        .route(
            "/api/v1/workspaces/{workspace_id}/agent-tools/trash",
            post(call),
        )
        .with_state(state)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CallBody {
    tool: String,
    #[serde(default)]
    arguments: Map<String, Value>,
    /// The thread of the run: approvals belong to it.
    conversation_id: String,
    root_message_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TrashArgs {
    #[serde(default)]
    page_ids: Vec<String>,
    #[serde(default)]
    task_ids: Vec<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Page,
    Task,
}

/// One item as the driver sees it now.
struct Item {
    kind: Kind,
    id: Id,
    title: String,
    /// Live sub-pages or sub-issues that go to the Trash with it.
    children: usize,
}

impl Item {
    fn line(&self) -> String {
        let (kind, child, children) = match self.kind {
            Kind::Page => ("Docs page", "sub-page", "sub-pages"),
            Kind::Task => ("Task", "sub-issue", "sub-issues"),
        };
        let along = match self.children {
            0 => String::new(),
            1 => format!(", also moves 1 {child} to the Trash"),
            n => format!(", also moves {n} {children} to the Trash"),
        };
        format!("- {kind} \"{}\"{along} (id {})", self.title, self.id)
    }

    fn view(&self) -> Value {
        let (kind, children) = match self.kind {
            Kind::Page => ("page", "sub_pages"),
            Kind::Task => ("task", "sub_issues"),
        };
        json!({ "kind": kind, "id": self.id, "title": self.title, children: self.children })
    }
}

fn refused(detail: &str) -> Json<Value> {
    Json(json!({ "status": "refused", "detail": detail }))
}

/// The ids of one kind: valid, and each one once.
fn ids(raw: &[String], seen: &mut Vec<Id>) -> Option<Vec<Id>> {
    let mut ids = Vec::with_capacity(raw.len());
    for text in raw {
        let id = text.trim().parse::<Id>().ok()?;
        if seen.contains(&id) {
            return None;
        }
        seen.push(id);
        ids.push(id);
    }
    Some(ids)
}

/// Live pages below `page_id` at every depth: what `delete_page` trashes along with it.
async fn sub_page_count(database: &Database, page_id: Id) -> Result<usize, sqlx::Error> {
    let count: i64 = sqlx::query_scalar(
        "WITH RECURSIVE subtree(id) AS ( \
             SELECT $1 \
             UNION ALL \
             SELECT pages.id FROM pages JOIN subtree ON pages.parent_id = subtree.id \
             WHERE pages.deleted_at IS NULL \
         ) SELECT COUNT(*) FROM subtree",
    )
    .bind(page_id.to_string())
    .fetch_one(database.pool())
    .await?;
    Ok(usize::try_from(count - 1).unwrap_or(0))
}

fn page_failure(error: &PageError) -> &'static str {
    match error {
        PageError::NotFound => "not found, or the driver cannot see it",
        PageError::VersionConflict { .. } => "it changed meanwhile; ask again",
        PageError::Forbidden => "the driver may not delete it",
        _ => "Orbit could not move it to the Trash",
    }
}

fn task_failure(error: &TaskError) -> &'static str {
    match error {
        TaskError::NotFound => "not found, or the driver cannot see it",
        TaskError::VersionConflict { .. } => "it changed meanwhile; ask again",
        TaskError::Forbidden => "the driver may not delete it",
        _ => "Orbit could not move it to the Trash",
    }
}

// An axum handler: each extractor is one argument.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
async fn call(
    State(state): State<AgentState>,
    Path(workspace_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
    custom: Option<Extension<super::profiles::AgentTools>>,
    dry: Option<Extension<super::scope::DryRun>>,
    Json(body): Json<CallBody>,
) -> Result<Json<Value>, ApiError> {
    // A member's agent session only: guests have no membership actor, so they never get here.
    let (user, workspace_id, _role) =
        driver(&state, &headers, &workspace_id, &uri, &request_id).await?;
    if body.tool != TOOL {
        return Ok(refused("Unknown tool."));
    }
    // The scope layer lets a custom agent reach this route only with the tool; checked again here.
    if let Some(Extension(super::profiles::AgentTools(tools))) = &custom
        && !tools.iter().any(|own| own == TOOL)
    {
        return Ok(refused("This agent's tools do not include move_to_trash."));
    }
    let database = state.database();
    let thread = (
        body.conversation_id.as_str(),
        body.root_message_id.as_deref(),
    );
    require_running(database, (workspace_id, user), thread, &uri, &request_id).await?;
    let raw = Value::Object(body.arguments.clone());
    let Ok(args) = serde_json::from_value::<TrashArgs>(raw.clone()) else {
        return Ok(refused("Give page_ids and/or task_ids (lists of ids)."));
    };
    let mut seen = Vec::new();
    let (Some(page_ids), Some(task_ids)) = (
        ids(&args.page_ids, &mut seen),
        ids(&args.task_ids, &mut seen),
    ) else {
        return Ok(refused("Give valid page and task ids, each one once."));
    };
    if seen.is_empty() || seen.len() > MAX_ITEMS {
        return Ok(refused("Give from 1 to 50 pages and tasks in one call."));
    }
    // A test run only says what it would move.
    if dry.is_some() {
        return Ok(Json(json!({
            "status": "dry_run",
            "action": format!("Move to the Trash: {raw}"),
        })));
    }
    let pages = PageRepository::new(database.clone());
    let tasks = TaskRepository::new(database.clone()).with_agent_session(true);
    // Only what the driver can see is described, so nobody is asked about an item they cannot reach.
    let mut items = Vec::with_capacity(seen.len());
    let mut missing = Vec::new();
    for &id in &page_ids {
        match pages.get_page(workspace_id, id, user).await {
            Ok(page) => items.push(Item {
                kind: Kind::Page,
                id,
                title: page.title,
                children: sub_page_count(database, id)
                    .await
                    .map_err(unavailable(&uri, &request_id))?,
            }),
            Err(PageError::Unavailable(error)) => {
                return Err(unavailable(&uri, &request_id)(error));
            }
            Err(_) => missing.push(format!("page {id}")),
        }
    }
    for &id in &task_ids {
        match tasks.get_task(workspace_id, id, user).await {
            Ok(task) => items.push(Item {
                kind: Kind::Task,
                id,
                title: task.title,
                children: sub_issues::live_descendants(database.pool(), id)
                    .await
                    .map_or(0, |ids| ids.len()),
            }),
            Err(TaskError::Unavailable(error)) => {
                return Err(unavailable(&uri, &request_id)(error));
            }
            Err(_) => missing.push(format!("task {id}")),
        }
    }
    if !missing.is_empty() {
        return Ok(Json(json!({
            "status": "failed",
            "detail": format!(
                "Not found, or the driver cannot see it: {}. Nothing was moved and nobody was asked.",
                missing.join(", ")
            ),
        })));
    }
    let workspace = workspace_id.to_string();
    let approval = approve(
        database,
        Approves::Trash(&workspace),
        user,
        thread,
        TOOL,
        &hash(&canonical(&raw)),
    )
    .await
    .map_err(unavailable(&uri, &request_id))?;
    let approval_id = match approval {
        Approval::Needed { id, expires_at } => {
            let lines = items.iter().map(Item::line).collect::<Vec<_>>().join("\n");
            return Ok(Json(json!({
                "status": "approval_required",
                "approval_id": id,
                "action": format!(
                    "Orbit: move {} to the Trash:\n{lines}\nMembers can restore them from the Trash for 30 days.",
                    if items.len() == 1 { "this item".to_owned() } else { format!("these {} items", items.len()) },
                ),
                "expires_at": expires_at,
            })));
        }
        Approval::Granted { id } => id,
    };
    let request = request_id
        .as_ref()
        .map_or("agent-trash", |Extension(id)| id.as_str());
    let mut trashed = Vec::new();
    let mut failed = Vec::new();
    for item in &mut items {
        let now = TimestampMillis::now();
        // The same soft delete as the web's Delete, at the version the driver approves now.
        let outcome = match item.kind {
            Kind::Page => {
                let result = match pages.get_page(workspace_id, item.id, user).await {
                    Ok(page) => {
                        item.children = sub_page_count(database, item.id)
                            .await
                            .unwrap_or(item.children);
                        pages
                            .delete_page(workspace_id, item.id, user, page.version, request, now)
                            .await
                    }
                    Err(error) => Err(error),
                };
                result.map_err(|error| page_failure(&error))
            }
            Kind::Task => {
                let result = match tasks.get_task(workspace_id, item.id, user).await {
                    Ok(task) => {
                        if let Ok(ids) =
                            sub_issues::live_descendants(database.pool(), item.id).await
                        {
                            item.children = ids.len();
                        }
                        tasks
                            .delete_task(workspace_id, item.id, user, task.version, request, now)
                            .await
                    }
                    Err(error) => Err(error),
                };
                result.map_err(|error| task_failure(&error))
            }
        };
        match outcome {
            Ok(()) => trashed.push(item.view()),
            Err(why) => {
                let mut view = item.view();
                view["error"] = json!(why);
                failed.push(view);
            }
        }
    }
    record(
        database,
        (workspace_id, user),
        &trashed,
        &failed,
        request,
        json!({
            "tool": TOOL,
            "approval_id": approval_id,
            "approved_by": user.to_string(),
            "conversation_id": body.conversation_id,
            "root_message_id": body.root_message_id,
        }),
    )
    .await;
    Ok(Json(json!({
        "status": "ok",
        "label": "trash",
        "result": {
            "trashed": trashed,
            "failed": failed,
            "restore": "Members restore them from the Trash (Tasks → Trash, Docs → Trash) for 30 days.",
        },
    })))
}

/// One audit line per item: what moved, and the approval of whom.
async fn record(
    database: &Database,
    (workspace_id, user): (Id, Id),
    trashed: &[Value],
    failed: &[Value],
    request: &str,
    metadata: Value,
) {
    let result = async {
        let mut tx = database.immediate_transaction().await?;
        for (item, outcome) in trashed
            .iter()
            .map(|item| (item, AuditOutcome::Success))
            .chain(failed.iter().map(|item| (item, AuditOutcome::Failure)))
        {
            let mut metadata = metadata.clone();
            if let Some(error) = item.get("error") {
                metadata["error"] = error.clone();
            }
            audit::record(
                &mut tx,
                workspace_id,
                Some(user),
                "agent_trash.approved_action",
                outcome,
                item["kind"].as_str().unwrap_or("page"),
                item["id"].as_str().and_then(|id| id.parse().ok()),
                request,
                metadata,
                TimestampMillis::now(),
            )
            .await?;
        }
        tx.commit().await
    }
    .await;
    if let Err(error) = result {
        tracing::warn!(%error, "agent trash audit");
    }
}
