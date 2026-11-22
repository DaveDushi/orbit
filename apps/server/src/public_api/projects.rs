//! Project trash, task bulk writes and reorders, cycle rollover and GitHub links
//! for API tokens. A trashed project is in no token's approved projects, so the project trash
//! and a restore need a token for all projects.
use axum::Router;
use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::extract::{Json, Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use orbit_platform::{Id, TimestampMillis};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::{
    Ctx, CycleOut, ExpectedVersionQuery, List, MilestoneOut, PageQuery, ProjectOut, PublicApiState,
    StatusOut, list, present, problem_response,
};
use crate::repositories::api_tokens::ApiTokenScope;
use crate::repositories::task_recurrences::TaskRecurrence;
use crate::repositories::tasks::{GithubLink, ProjectRecord};
use crate::token_api::{
    Authorized, BulkTaskUpdate, BulkTasks, CycleMove, ReorderItem, TaskPatch, TaskView,
    TokenApiError,
};

pub(super) fn routes() -> Router<PublicApiState> {
    Router::new()
        .route("/tasks/bulk", post(bulk_update_tasks))
        .route("/tasks/reorder", post(reorder_tasks))
        .route("/tasks/{task}/github-links", get(github_links))
        .route("/projects/trash", get(project_trash))
        .route("/projects/trash/{project_id}", get(trashed_project))
        .route("/projects/{project_id}", delete(delete_project))
        .route("/projects/{project_id}/restore", post(restore_project))
        .route(
            "/projects/{project_id}/statuses/reorder",
            post(reorder_statuses),
        )
        .route(
            "/projects/{project_id}/milestones/reorder",
            post(reorder_milestones),
        )
        .route(
            "/projects/{project_id}/cycles/{cycle_id}/move-unfinished",
            post(move_unfinished),
        )
}

/// One task of a bulk update. A missing field keeps its value.
#[derive(Deserialize, ToSchema)]
#[schema(as = PublicBulkTaskUpdate)]
#[serde(deny_unknown_fields)]
pub(super) struct BulkTaskItem {
    /// A task UUID or identifier such as `ENG-12`, in an approved project.
    task: String,
    /// The task's `version` that you read.
    expected_version: u64,
    /// 1 to 500 characters.
    title: Option<String>,
    /// Markdown, up to 50,000 bytes.
    description: Option<String>,
    #[schema(value_type = Option<String>)]
    status_id: Option<Id>,
    /// `none`, `low`, `medium`, `high` or `urgent`.
    priority: Option<String>,
    /// Replaces the assignees.
    #[schema(value_type = Option<Vec<String>>)]
    assignee_ids: Option<Vec<Id>>,
    /// Replaces the labels.
    #[schema(value_type = Option<Vec<String>>)]
    label_ids: Option<Vec<Id>>,
    /// `null` clears the date.
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>, format = DateTime)]
    due_start_at: Option<Option<TimestampMillis>>,
    /// `null` clears the date.
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>, format = DateTime)]
    due_at: Option<Option<TimestampMillis>>,
    /// A task in an approved project (UUID or identifier) makes this task its sub-issue; `null`
    /// detaches it.
    #[serde(default, deserialize_with = "present")]
    parent_task_id: Option<Option<String>>,
    /// Makes the task repeat; `null` stops repeating.
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<TaskRecurrence>)]
    recurrence: Option<Option<TaskRecurrence>>,
    /// A milestone of the task's project; `null` clears it.
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>)]
    milestone_id: Option<Option<Id>>,
    /// A cycle of the task's project; `null` clears it.
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>)]
    cycle_id: Option<Option<Id>>,
    /// Points, 0 to 1000; `null` clears the estimate.
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<i64>, minimum = 0, maximum = 1000)]
    estimate: Option<Option<i64>>,
}

#[derive(Deserialize, ToSchema)]
#[schema(as = PublicBulkTasksBody)]
#[serde(deny_unknown_fields)]
pub(super) struct BulkTasksBody {
    /// 1 to 100 updates, each of a different task. All are written, or none.
    updates: Vec<BulkTaskItem>,
}

#[derive(Deserialize, ToSchema)]
#[schema(as = PublicReorderItem)]
#[serde(deny_unknown_fields)]
pub(super) struct ReorderItemBody {
    /// A task (UUID or identifier such as `ENG-12`), status or milestone id.
    id: String,
    /// Its `version` that you read.
    expected_version: u64,
    /// The new position; lower comes first.
    position: i64,
}

#[derive(Deserialize, ToSchema)]
#[schema(as = PublicReorderBody)]
#[serde(deny_unknown_fields)]
pub(super) struct ReorderBody {
    /// 1 to 100 items, each a different one.
    items: Vec<ReorderItemBody>,
}

impl ReorderBody {
    fn items(self) -> Vec<ReorderItem> {
        self.items
            .into_iter()
            .map(|item| ReorderItem {
                id: item.id,
                expected_version: item.expected_version,
                position: item.position,
            })
            .collect()
    }
}

#[derive(Deserialize, ToSchema)]
#[schema(as = PublicRestoreProjectBody)]
#[serde(deny_unknown_fields)]
pub(super) struct RestoreProjectBody {
    /// The `version` that you read in the project trash.
    expected_version: u64,
}

/// A project in the trash.
#[derive(Serialize, ToSchema)]
#[schema(as = PublicTrashedProject)]
struct TrashedProjectOut {
    #[serde(flatten)]
    project: ProjectOut,
    #[schema(value_type = Option<String>, format = DateTime, required = true)]
    deleted_at: Option<TimestampMillis>,
}

impl From<ProjectRecord> for TrashedProjectOut {
    fn from(p: ProjectRecord) -> Self {
        Self {
            deleted_at: p.deleted_at,
            project: ProjectOut::from(p),
        }
    }
}

/// A project in the trash, with its statuses and who trashed it.
#[derive(Serialize, ToSchema)]
#[schema(as = PublicTrashedProjectDetail)]
struct TrashedProjectDetail {
    #[serde(flatten)]
    project: TrashedProjectOut,
    statuses: Vec<StatusOut>,
    /// The user who moved the project to the trash; `null` when unknown.
    #[schema(value_type = Option<String>, required = true)]
    deleted_by: Option<Id>,
}

/// What "move unfinished tasks" did.
#[derive(Serialize, ToSchema)]
#[schema(as = PublicCycleMoveOutcome)]
struct CycleMoveOut {
    /// How many tasks moved.
    moved: i64,
    /// The cycle they moved to, with its new counts.
    to_cycle: CycleOut,
}

impl From<CycleMove> for CycleMoveOut {
    fn from(m: CycleMove) -> Self {
        Self {
            moved: m.moved,
            to_cycle: CycleOut::from(m.to_cycle),
        }
    }
}

fn no_content(ctx: &Ctx, result: Result<(), TokenApiError>) -> Response {
    match result {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => ctx.error(error),
    }
}

async fn patch(api: &Authorized<'_>, item: BulkTaskItem) -> Result<BulkTaskUpdate, TokenApiError> {
    Ok(BulkTaskUpdate {
        task: item.task,
        expected_version: item.expected_version,
        patch: TaskPatch {
            title: item.title,
            description: item.description,
            status_id: item.status_id,
            priority: item.priority,
            assignee_ids: item.assignee_ids,
            label_ids: item.label_ids,
            due_start_at: item.due_start_at,
            due_at: item.due_at,
            parent_task_id: match item.parent_task_id {
                Some(parent) => Some(api.optional_task_id(parent.as_deref()).await?),
                None => None,
            },
            recurrence: item.recurrence,
            milestone_id: item.milestone_id,
            cycle_id: item.cycle_id,
            estimate: item.estimate,
        },
    })
}

#[utoipa::path(
    post,
    path = "/api/public/v1/workspaces/{workspace_id}/tasks/bulk",
    operation_id = "public_bulk_update_tasks",
    tag = "public-api",
    summary = "Update up to 100 tasks at once",
    description = "Each update takes the fields of `PATCH /tasks/{task}`. All updates are written in one transaction, or none: one stale `expected_version` answers `409` for the whole request.",
    params(("workspace_id" = String, Path, description = "The token's workspace, from `GET /me`.")),
    request_body = BulkTasksBody,
    responses((status = 200, description = "The updated tasks, and the parents that the sub-issue automation closed", body = BulkTasks)),
    security(("bearerAuth" = []))
)]
pub(super) async fn bulk_update_tasks(
    State(s): State<PublicApiState>,
    ctx: Ctx,
    body: Result<Json<BulkTasksBody>, JsonRejection>,
) -> Response {
    let body = input!(ctx, ApiTokenScope::TasksWrite, body);
    let result = async {
        let api = s
            .api
            .authorize(&ctx.principal, ApiTokenScope::TasksWrite, &ctx.request_id)
            .await?;
        if body.updates.len() > 100 {
            return Err(TokenApiError::Invalid("Send 1 to 100 updates"));
        }
        let mut updates = Vec::with_capacity(body.updates.len());
        for item in body.updates {
            updates.push(patch(&api, item).await?);
        }
        api.bulk_update_tasks(updates).await
    }
    .await;
    ctx.reply(StatusCode::OK, result)
}

#[utoipa::path(
    post,
    path = "/api/public/v1/workspaces/{workspace_id}/tasks/reorder",
    operation_id = "public_reorder_tasks",
    tag = "public-api",
    summary = "Set the board position of up to 100 tasks",
    description = "`id` is a task UUID or identifier. All moves are written, or none.",
    params(("workspace_id" = String, Path, description = "The token's workspace, from `GET /me`.")),
    request_body = ReorderBody,
    responses((status = 200, description = "The moved tasks; `next_cursor` is `null`", body = List<TaskView>)),
    security(("bearerAuth" = []))
)]
pub(super) async fn reorder_tasks(
    State(s): State<PublicApiState>,
    ctx: Ctx,
    body: Result<Json<ReorderBody>, JsonRejection>,
) -> Response {
    let body = input!(ctx, ApiTokenScope::TasksWrite, body);
    let result = async {
        let api = s
            .api
            .authorize(&ctx.principal, ApiTokenScope::TasksWrite, &ctx.request_id)
            .await?;
        let tasks = api.reorder_tasks(body.items()).await?;
        Ok(list::<_, TaskView>(tasks, None))
    }
    .await;
    ctx.reply(StatusCode::OK, result)
}

#[utoipa::path(
    get,
    path = "/api/public/v1/workspaces/{workspace_id}/tasks/{task}/github-links",
    operation_id = "public_list_task_github_links",
    tag = "public-api",
    summary = "The GitHub issues and pull requests linked to a task",
    params(("workspace_id" = String, Path, description = "The token's workspace, from `GET /me`."), ("task" = String, Path, description = "A task UUID or identifier such as `ENG-12` (any letter case).")),
    responses((status = 200, description = "Every link; `next_cursor` is `null`", body = List<GithubLink>)),
    security(("bearerAuth" = []))
)]
pub(super) async fn github_links(
    State(s): State<PublicApiState>,
    ctx: Ctx,
    Path((_workspace, task)): Path<(String, String)>,
) -> Response {
    let result = async {
        let api = s
            .api
            .authorize(&ctx.principal, ApiTokenScope::TasksRead, &ctx.request_id)
            .await?;
        Ok(list::<_, GithubLink>(api.github_links(&task).await?, None))
    }
    .await;
    ctx.reply(StatusCode::OK, result)
}

#[utoipa::path(
    get,
    path = "/api/public/v1/workspaces/{workspace_id}/projects/trash",
    operation_id = "public_list_project_trash",
    tag = "public-api",
    summary = "The projects in the trash, last trashed first",
    description = "Projects stay in the trash for 30 days. Needs a token for all projects (`403` otherwise): a trashed project is in no token's approved projects.",
    params(("workspace_id" = String, Path, description = "The token's workspace, from `GET /me`."), PageQuery),
    responses((status = 200, description = "A page of trashed projects", body = List<TrashedProjectOut>)),
    security(("bearerAuth" = []))
)]
pub(super) async fn project_trash(
    State(s): State<PublicApiState>,
    ctx: Ctx,
    query: Result<Query<PageQuery>, QueryRejection>,
) -> Response {
    let query = input!(ctx, ApiTokenScope::TasksRead, query);
    let result = async {
        let api = s
            .api
            .authorize(&ctx.principal, ApiTokenScope::TasksRead, &ctx.request_id)
            .await?;
        let page = api
            .project_trash(query.cursor.as_deref(), query.limit)
            .await?;
        Ok(list::<_, TrashedProjectOut>(page.items, page.next_cursor))
    }
    .await;
    ctx.reply(StatusCode::OK, result)
}

#[utoipa::path(
    get,
    path = "/api/public/v1/workspaces/{workspace_id}/projects/trash/{project_id}",
    operation_id = "public_get_trashed_project",
    tag = "public-api",
    summary = "A project in the trash",
    description = "With its statuses and the user who trashed it. Needs a token for all projects.",
    params(("workspace_id" = String, Path, description = "The token's workspace, from `GET /me`."), ("project_id" = String, Path)),
    responses((status = 200, description = "The trashed project", body = TrashedProjectDetail)),
    security(("bearerAuth" = []))
)]
pub(super) async fn trashed_project(
    State(s): State<PublicApiState>,
    ctx: Ctx,
    Path((_workspace, project)): Path<(String, String)>,
) -> Response {
    let project = path_id!(ctx, project);
    let result = async {
        let api = s
            .api
            .authorize(&ctx.principal, ApiTokenScope::TasksRead, &ctx.request_id)
            .await?;
        let trashed = api.trashed_project(project).await?;
        Ok(TrashedProjectDetail {
            project: TrashedProjectOut::from(trashed.project),
            statuses: trashed.statuses.into_iter().map(StatusOut::from).collect(),
            deleted_by: trashed.deleted_by,
        })
    }
    .await;
    ctx.reply(StatusCode::OK, result)
}

#[utoipa::path(
    delete,
    path = "/api/public/v1/workspaces/{workspace_id}/projects/{project_id}",
    operation_id = "public_delete_project",
    tag = "public-api",
    summary = "Move a project with its tasks to the trash",
    description = "The project stays in the trash for 30 days; `POST /projects/{project_id}/restore` brings it back. The token's creator must be allowed to manage the project.",
    params(("workspace_id" = String, Path, description = "The token's workspace, from `GET /me`."), ("project_id" = String, Path), ExpectedVersionQuery),
    responses((status = 204, description = "The project is in the trash")),
    security(("bearerAuth" = []))
)]
pub(super) async fn delete_project(
    State(s): State<PublicApiState>,
    ctx: Ctx,
    Path((_workspace, project)): Path<(String, String)>,
    query: Result<Query<ExpectedVersionQuery>, QueryRejection>,
) -> Response {
    let project = path_id!(ctx, project);
    let query = input!(ctx, ApiTokenScope::TasksManage, query);
    let result = async {
        let api = s
            .api
            .authorize(&ctx.principal, ApiTokenScope::TasksManage, &ctx.request_id)
            .await?;
        api.delete_project(project, query.expected_version).await
    }
    .await;
    no_content(&ctx, result)
}

#[utoipa::path(
    post,
    path = "/api/public/v1/workspaces/{workspace_id}/projects/{project_id}/restore",
    operation_id = "public_restore_project",
    tag = "public-api",
    summary = "Restore a project from the trash",
    description = "Needs a token for all projects; the restored project is approved for it from the next request on. `409` when a live project took its key.",
    params(("workspace_id" = String, Path, description = "The token's workspace, from `GET /me`."), ("project_id" = String, Path)),
    request_body = RestoreProjectBody,
    responses((status = 200, description = "The restored project", body = ProjectOut)),
    security(("bearerAuth" = []))
)]
pub(super) async fn restore_project(
    State(s): State<PublicApiState>,
    ctx: Ctx,
    Path((_workspace, project)): Path<(String, String)>,
    body: Result<Json<RestoreProjectBody>, JsonRejection>,
) -> Response {
    let project = path_id!(ctx, project);
    let body = input!(ctx, ApiTokenScope::TasksManage, body);
    let result = async {
        let api = s
            .api
            .authorize(&ctx.principal, ApiTokenScope::TasksManage, &ctx.request_id)
            .await?;
        api.restore_project(project, body.expected_version)
            .await
            .map(ProjectOut::from)
    }
    .await;
    ctx.reply(StatusCode::OK, result)
}

#[utoipa::path(
    post,
    path = "/api/public/v1/workspaces/{workspace_id}/projects/{project_id}/statuses/reorder",
    operation_id = "public_reorder_statuses",
    tag = "public-api",
    summary = "Set the board order of a project's statuses",
    description = "All moves are written, or none. The token's creator must be allowed to manage the project.",
    params(("workspace_id" = String, Path, description = "The token's workspace, from `GET /me`."), ("project_id" = String, Path)),
    request_body = ReorderBody,
    responses((status = 200, description = "The moved statuses; `next_cursor` is `null`", body = List<StatusOut>)),
    security(("bearerAuth" = []))
)]
pub(super) async fn reorder_statuses(
    State(s): State<PublicApiState>,
    ctx: Ctx,
    Path((_workspace, project)): Path<(String, String)>,
    body: Result<Json<ReorderBody>, JsonRejection>,
) -> Response {
    let project = path_id!(ctx, project);
    let body = input!(ctx, ApiTokenScope::TasksManage, body);
    let result = async {
        let api = s
            .api
            .authorize(&ctx.principal, ApiTokenScope::TasksManage, &ctx.request_id)
            .await?;
        let statuses = api.reorder_statuses(project, body.items()).await?;
        Ok(list::<_, StatusOut>(statuses, None))
    }
    .await;
    ctx.reply(StatusCode::OK, result)
}

#[utoipa::path(
    post,
    path = "/api/public/v1/workspaces/{workspace_id}/projects/{project_id}/milestones/reorder",
    operation_id = "public_reorder_milestones",
    tag = "public-api",
    summary = "Set the order of a project's milestones",
    description = "All moves are written, or none.",
    params(("workspace_id" = String, Path, description = "The token's workspace, from `GET /me`."), ("project_id" = String, Path)),
    request_body = ReorderBody,
    responses((status = 200, description = "The project's milestones; `next_cursor` is `null`", body = List<MilestoneOut>)),
    security(("bearerAuth" = []))
)]
pub(super) async fn reorder_milestones(
    State(s): State<PublicApiState>,
    ctx: Ctx,
    Path((_workspace, project)): Path<(String, String)>,
    body: Result<Json<ReorderBody>, JsonRejection>,
) -> Response {
    let project = path_id!(ctx, project);
    let body = input!(ctx, ApiTokenScope::TasksWrite, body);
    let result = async {
        let api = s
            .api
            .authorize(&ctx.principal, ApiTokenScope::TasksWrite, &ctx.request_id)
            .await?;
        let milestones = api.reorder_milestones(project, body.items()).await?;
        Ok(list::<_, MilestoneOut>(milestones, None))
    }
    .await;
    ctx.reply(StatusCode::OK, result)
}

#[utoipa::path(
    post,
    path = "/api/public/v1/workspaces/{workspace_id}/projects/{project_id}/cycles/{cycle_id}/move-unfinished",
    operation_id = "public_move_unfinished_cycle_tasks",
    tag = "public-api",
    summary = "Move a cycle's unfinished tasks to the next cycle",
    description = "`409` when the project has no cycle after this one.",
    params(("workspace_id" = String, Path, description = "The token's workspace, from `GET /me`."), ("project_id" = String, Path), ("cycle_id" = String, Path)),
    responses((status = 200, description = "How many tasks moved, and the cycle they moved to", body = CycleMoveOut)),
    security(("bearerAuth" = []))
)]
pub(super) async fn move_unfinished(
    State(s): State<PublicApiState>,
    ctx: Ctx,
    Path((_workspace, project, cycle)): Path<(String, String, String)>,
) -> Response {
    let project = path_id!(ctx, project);
    let cycle = path_id!(ctx, cycle);
    let result = async {
        let api = s
            .api
            .authorize(&ctx.principal, ApiTokenScope::TasksWrite, &ctx.request_id)
            .await?;
        api.move_unfinished_cycle_tasks(project, cycle)
            .await
            .map(CycleMoveOut::from)
    }
    .await;
    ctx.reply(StatusCode::OK, result)
}
