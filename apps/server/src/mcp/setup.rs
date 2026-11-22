//! Project setup (name, color, statuses, labels) and task relations.
use rmcp::ErrorData;
use rmcp::model::{CallToolRequestParams, Tool};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{TaskArgs, arguments, definition, parse_id, write_definition};
use crate::repositories::api_tokens::ApiTokenScope;
use crate::repositories::task_relations::NewTaskRelationType;
use crate::repositories::tasks::TaskError;
use crate::token_api::{Authorized, StatusPatch, TokenApiError};

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CreateRelationArgs {
    /// Task id, or its identifier such as ENG-12.
    task_id: String,
    /// `blocks` (task_id blocks other_task_id), `blocked_by` or `related`.
    #[serde(rename = "type")]
    kind: NewTaskRelationType,
    /// The other task, in an approved project: its id, or its identifier such as ENG-12.
    other_task_id: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct DeleteRelationArgs {
    /// Task id, or its identifier such as ENG-12.
    task_id: String,
    /// A relation id from list_task_relations of this task.
    relation_id: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct UpdateProjectArgs {
    project_id: String,
    /// The `version` that you read.
    expected_version: u64,
    /// 1 to 200 characters.
    name: Option<String>,
    /// `#rrggbb`.
    color: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CreateStatusArgs {
    project_id: String,
    /// 1 to 200 characters.
    name: String,
    /// `unstarted`, `started`, `completed` or `cancelled`. Tasks in a `completed` or
    /// `cancelled` status are done.
    category: String,
    /// `#rrggbb`. Default: a color that matches the category.
    color: Option<String>,
    /// Up to 20,000 characters.
    description: Option<String>,
    /// The place in the board order (0 is first). Default: after the last status.
    position: Option<i64>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct UpdateStatusArgs {
    project_id: String,
    status_id: String,
    /// The `version` that you read.
    expected_version: u64,
    /// 1 to 200 characters.
    name: Option<String>,
    /// `unstarted`, `started`, `completed` or `cancelled`.
    category: Option<String>,
    /// `#rrggbb`.
    color: Option<String>,
    /// Up to 20,000 characters.
    description: Option<String>,
    /// The place in the board order (0 is first).
    position: Option<i64>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CreateLabelArgs {
    project_id: String,
    /// 1 to 100 characters, unique in the project.
    name: String,
    /// `#rrggbb`. Default: gray.
    color: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct UpdateLabelArgs {
    project_id: String,
    label_id: String,
    /// The `version` that you read.
    expected_version: u64,
    /// 1 to 100 characters, unique in the project.
    name: Option<String>,
    /// `#rrggbb`.
    color: Option<String>,
}

pub(super) fn scope(name: &str) -> Option<ApiTokenScope> {
    match name {
        "list_task_relations" => Some(ApiTokenScope::TasksRead),
        "create_label" | "update_label" | "create_task_relation" | "delete_task_relation" => {
            Some(ApiTokenScope::TasksWrite)
        }
        "update_project" | "create_task_status" | "update_task_status" => {
            Some(ApiTokenScope::TasksManage)
        }
        _ => None,
    }
}

pub(super) fn tools() -> Vec<Tool> {
    vec![
        write_definition::<UpdateProjectArgs>(
            "update_project",
            "Rename or recolor an approved project, using its current version. A missing field keeps its value.",
            true,
        ),
        write_definition::<CreateStatusArgs>(
            "create_task_status",
            "Add a workflow status to an approved project.",
            false,
        ),
        write_definition::<UpdateStatusArgs>(
            "update_task_status",
            "Change a workflow status of an approved project, using its current version. A missing field keeps its value. The Duplicate status cannot change its category, and no other status can take it.",
            true,
        ),
        write_definition::<CreateLabelArgs>(
            "create_label",
            "Add a label to an approved project. The name must be new in the project.",
            false,
        ),
        write_definition::<UpdateLabelArgs>(
            "update_label",
            "Rename or recolor a label of an approved project, using its current version. A missing field keeps its value.",
            true,
        ),
        definition::<TaskArgs>(
            "list_task_relations",
            "List the relations of a live task in an approved project: blocks, related and duplicate. direction is outgoing when this task is the blocker or the duplicate, incoming otherwise. task is the other task (number, title, status_id). Relations to tasks outside the approved projects are left out.",
        ),
        write_definition::<CreateRelationArgs>(
            "create_task_relation",
            "Link two live tasks of approved projects: type blocks (task_id blocks other_task_id), blocked_by or related. A relation that already links the two tasks is replaced; the same relation again is returned unchanged. A duplicate relation, or a blocks relation in the other direction, is refused: remove it first. Duplicates are marked and unmarked in Orbit.",
            true,
        ),
        write_definition::<DeleteRelationArgs>(
            "delete_task_relation",
            "Remove a blocks or related relation of a task, by its id from list_task_relations. A duplicate is unmarked in Orbit.",
            true,
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
        "update_project" => {
            let a: UpdateProjectArgs = arguments(request)?;
            api.update_project(
                parse_id(&a.project_id)?,
                a.expected_version,
                a.name,
                a.color,
            )
            .await
            .map(|project| json!(project))
        }
        "create_task_status" => {
            let a: CreateStatusArgs = arguments(request)?;
            let status = StatusPatch {
                name: Some(a.name),
                description: a.description,
                color: a.color,
                category: Some(a.category),
                position: a.position,
            };
            api.create_status(parse_id(&a.project_id)?, status)
                .await
                .map(|status| json!(status))
        }
        "update_task_status" => {
            let a: UpdateStatusArgs = arguments(request)?;
            let patch = StatusPatch {
                name: a.name,
                description: a.description,
                color: a.color,
                category: a.category,
                position: a.position,
            };
            api.update_status(
                parse_id(&a.project_id)?,
                parse_id(&a.status_id)?,
                a.expected_version,
                patch,
            )
            .await
            .map(|status| json!(status))
        }
        "create_label" => {
            let a: CreateLabelArgs = arguments(request)?;
            api.create_label(parse_id(&a.project_id)?, &a.name, a.color)
                .await
                .map(|label| json!(label))
        }
        "update_label" => {
            let a: UpdateLabelArgs = arguments(request)?;
            api.update_label(
                parse_id(&a.project_id)?,
                parse_id(&a.label_id)?,
                a.expected_version,
                a.name,
                a.color,
            )
            .await
            .map(|label| json!(label))
        }
        "list_task_relations" => {
            let a: TaskArgs = arguments(request)?;
            api.relations(&a.task_id)
                .await
                .map(|items| json!({"items": items}))
        }
        "create_task_relation" => {
            let a: CreateRelationArgs = arguments(request)?;
            match api.add_relation(&a.task_id, a.kind, &a.other_task_id).await {
                Err(TokenApiError::Task(TaskError::Conflict)) => Err(TokenApiError::Conflict(
                    "The tasks have a duplicate relation or a blocks relation in the other direction; remove it first",
                )),
                result => result.map(|relation| json!(relation)),
            }
        }
        "delete_task_relation" => {
            let a: DeleteRelationArgs = arguments(request)?;
            api.remove_relation(&a.task_id, parse_id(&a.relation_id)?)
                .await
                .map(|()| json!({"deleted": true}))
        }
        _ => return Err(ErrorData::invalid_params("Unknown tool", None)),
    })
}
