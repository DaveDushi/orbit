//! Project trash, task bulk writes and reorders, cycle rollover and GitHub links for API tokens and MCP.
use rmcp::ErrorData;
use rmcp::model::{CallToolRequestParams, Tool};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{UpdateArgs, arguments, definition, parse_id, task_patch, write_definition};
use crate::repositories::api_tokens::ApiTokenScope;
use crate::token_api::{Authorized, BulkTaskUpdate, ReorderItem, TokenApiError};

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct BulkUpdateArgs {
    /// 1 to 100 updates, each of a different task, with the fields of update_task. All are
    /// written, or none.
    updates: Vec<UpdateArgs>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ReorderItemArgs {
    /// The task (UUID or identifier such as ENG-12), status or milestone.
    id: String,
    /// Its version, from the list tool.
    expected_version: u64,
    /// The new position; lower comes first.
    position: i64,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ReorderTasksArgs {
    /// 1 to 100 tasks, each once.
    items: Vec<ReorderItemArgs>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ProjectReorderArgs {
    project_id: String,
    /// 1 to 100 items of the project, each once.
    items: Vec<ReorderItemArgs>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct TaskArgs {
    /// A task UUID or identifier such as ENG-12.
    task_id: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ProjectArgs {
    project_id: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ProjectVersionArgs {
    project_id: String,
    /// The project's version, from list_projects (or list_project_trash for a restore).
    expected_version: u64,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ProjectTrashArgs {
    /// The next_cursor of the previous page.
    cursor: Option<String>,
    /// Projects per page, from 1 to 100. Default: 50.
    limit: Option<usize>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CycleArgs {
    project_id: String,
    /// The cycle whose unfinished tasks move, from list_cycles.
    cycle_id: String,
}

pub(super) fn scope(name: &str) -> Option<ApiTokenScope> {
    match name {
        "list_task_github_links" | "list_project_trash" | "get_trashed_project" => {
            Some(ApiTokenScope::TasksRead)
        }
        "bulk_update_tasks"
        | "reorder_tasks"
        | "reorder_milestones"
        | "move_unfinished_cycle_tasks" => Some(ApiTokenScope::TasksWrite),
        "delete_project" | "restore_project" | "reorder_task_statuses" => {
            Some(ApiTokenScope::TasksManage)
        }
        _ => None,
    }
}

pub(super) fn tools() -> Vec<Tool> {
    vec![
        write_definition::<BulkUpdateArgs>(
            "bulk_update_tasks",
            "Update up to 100 tasks of approved projects at once; each update takes the arguments of update_task. All are written or none: one stale expected_version fails the whole call. Returns the tasks and the parents that the sub-issue automation closed.",
            true,
        ),
        write_definition::<ReorderTasksArgs>(
            "reorder_tasks",
            "Set the board position of up to 100 tasks of approved projects (lower comes first). expected_version is each task's version from list_tasks or get_task. Returns the moved tasks.",
            false,
        ),
        definition::<TaskArgs>(
            "list_task_github_links",
            "The GitHub issues and pull requests linked to a task of an approved project.",
        ),
        write_definition::<ProjectVersionArgs>(
            "delete_project",
            "Move an approved project with its tasks to the trash for 30 days (restore_project brings it back).",
            true,
        ),
        definition::<ProjectTrashArgs>(
            "list_project_trash",
            "The projects in the trash, last trashed first, with their version for restore_project. Needs a connection for all projects.",
        ),
        definition::<ProjectArgs>(
            "get_trashed_project",
            "A project in the trash, with its statuses and who trashed it. Needs a connection for all projects.",
        ),
        write_definition::<ProjectVersionArgs>(
            "restore_project",
            "Restore a project from the trash with its tasks; expected_version comes from list_project_trash. Needs a connection for all projects.",
            false,
        ),
        write_definition::<ProjectReorderArgs>(
            "reorder_task_statuses",
            "Set the board order of statuses of an approved project (ids and versions from list_task_statuses; lower comes first). Returns the moved statuses.",
            false,
        ),
        write_definition::<ProjectReorderArgs>(
            "reorder_milestones",
            "Set the order of milestones of an approved project (ids and versions from list_milestones; lower comes first). Returns the project's milestones.",
            false,
        ),
        write_definition::<CycleArgs>(
            "move_unfinished_cycle_tasks",
            "Move the unfinished tasks of a cycle of an approved project to the project's next cycle (create it first with create_cycle). Returns how many moved and the next cycle.",
            false,
        ),
    ]
}

/// Runs a tool of this module; `None` for another tool's name.
pub(super) async fn call(
    api: &Authorized<'_>,
    request: &CallToolRequestParams,
) -> Option<Result<Result<Value, TokenApiError>, ErrorData>> {
    let result = match request.name.as_ref() {
        "bulk_update_tasks" => {
            let run = async {
                let a: BulkUpdateArgs = arguments(request)?;
                if a.updates.len() > 100 {
                    return Ok(Err(TokenApiError::Invalid("Send 1 to 100 updates")));
                }
                let mut updates = Vec::with_capacity(a.updates.len());
                for item in a.updates {
                    let (task, expected_version) = (item.task_id.clone(), item.expected_version);
                    match task_patch(api, item).await? {
                        Ok(patch) => updates.push(BulkTaskUpdate {
                            task,
                            expected_version,
                            patch,
                        }),
                        Err(error) => return Ok(Err(error)),
                    }
                }
                Ok(api.bulk_update_tasks(updates).await.map(|out| json!(out)))
            };
            run.await
        }
        "reorder_tasks" => {
            let run = async {
                let a: ReorderTasksArgs = arguments(request)?;
                Ok(api
                    .reorder_tasks(items(a.items))
                    .await
                    .map(|tasks| json!({ "items": tasks })))
            };
            run.await
        }
        "list_task_github_links" => {
            let run = async {
                let a: TaskArgs = arguments(request)?;
                Ok(api
                    .github_links(&a.task_id)
                    .await
                    .map(|links| json!({ "items": links })))
            };
            run.await
        }
        "delete_project" => {
            let run = async {
                let a: ProjectVersionArgs = arguments(request)?;
                Ok(api
                    .delete_project(parse_id(&a.project_id)?, a.expected_version)
                    .await
                    .map(|()| json!({ "deleted": true })))
            };
            run.await
        }
        "list_project_trash" => {
            let run = async {
                let a: ProjectTrashArgs = arguments(request)?;
                Ok(api
                    .project_trash(a.cursor.as_deref(), a.limit)
                    .await
                    .map(|page| json!(page)))
            };
            run.await
        }
        "get_trashed_project" => {
            let run = async {
                let a: ProjectArgs = arguments(request)?;
                Ok(api
                    .trashed_project(parse_id(&a.project_id)?)
                    .await
                    .map(|project| json!(project)))
            };
            run.await
        }
        "restore_project" => {
            let run = async {
                let a: ProjectVersionArgs = arguments(request)?;
                Ok(api
                    .restore_project(parse_id(&a.project_id)?, a.expected_version)
                    .await
                    .map(|project| json!(project)))
            };
            run.await
        }
        "reorder_task_statuses" => {
            let run = async {
                let a: ProjectReorderArgs = arguments(request)?;
                Ok(api
                    .reorder_statuses(parse_id(&a.project_id)?, items(a.items))
                    .await
                    .map(|statuses| json!({ "items": statuses })))
            };
            run.await
        }
        "reorder_milestones" => {
            let run = async {
                let a: ProjectReorderArgs = arguments(request)?;
                Ok(api
                    .reorder_milestones(parse_id(&a.project_id)?, items(a.items))
                    .await
                    .map(|milestones| json!({ "items": milestones })))
            };
            run.await
        }
        "move_unfinished_cycle_tasks" => {
            let run = async {
                let a: CycleArgs = arguments(request)?;
                Ok(api
                    .move_unfinished_cycle_tasks(parse_id(&a.project_id)?, parse_id(&a.cycle_id)?)
                    .await
                    .map(|outcome| json!(outcome)))
            };
            run.await
        }
        _ => return None,
    };
    Some(result)
}

fn items(items: Vec<ReorderItemArgs>) -> Vec<ReorderItem> {
    items
        .into_iter()
        .map(|item| ReorderItem {
            id: item.id,
            expected_version: item.expected_version,
            position: item.position,
        })
        .collect()
}
