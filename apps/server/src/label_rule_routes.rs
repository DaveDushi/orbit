//! A project's label rules (`/projects/{id}/label-rules`) and its owner switch
//! (`/projects/{id}/label-rule-settings`). Session-authenticated like the task routes (the
//! agent's tools use them with the driver's session); only owners and admins pass
//! (`repositories::label_rules`). Errors use `task_problem`.

use axum::extract::{Extension, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, put};
use axum::{Json, Router};
use orbit_platform::{RequestId, TimestampMillis};
use serde::Deserialize;
use utoipa::ToSchema;

use crate::repositories::label_rules::{
    LabelRulePatch, LabelRuleRecord, LabelRuleRepository, LabelRuleSettings,
};
use crate::task_routes::{
    ApiError, ApiJson, TaskState, parse_id, request_id_value, scope, task_problem,
};

pub fn label_rule_router(state: TaskState) -> Router {
    Router::new()
        .route(
            "/api/v1/workspaces/{workspace_id}/projects/{project_id}/label-rules",
            get(list_label_rules).post(create_label_rule),
        )
        .route(
            "/api/v1/workspaces/{workspace_id}/projects/{project_id}/label-rules/{rule_id}",
            patch(update_label_rule).delete(delete_label_rule),
        )
        .route(
            "/api/v1/workspaces/{workspace_id}/projects/{project_id}/label-rule-settings",
            put(put_label_rule_settings),
        )
        .with_state(state)
}

fn repository(state: &TaskState) -> LabelRuleRepository {
    LabelRuleRepository::new(state.identity.database().clone())
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct LabelRuleBody {
    /// A label of this project.
    label_id: String,
    /// The member to assign (not a guest, not suspended).
    user_id: String,
    /// Default `true`.
    enabled: Option<bool>,
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct LabelRulePatchBody {
    label_id: Option<String>,
    user_id: Option<String>,
    enabled: Option<bool>,
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct LabelRuleSettingsBody {
    /// `owner:<name>` labels assign the member `<name>`.
    owner_labels: bool,
}

/// The project's rules, its owner switch, and the `owner:` labels that name no member.
#[utoipa::path(get, path = "/api/v1/workspaces/{workspace_id}/projects/{project_id}/label-rules", params(("workspace_id" = String, Path), ("project_id" = String, Path)), responses((status = 200, body = LabelRuleSettings)))]
pub(crate) async fn list_label_rules(
    State(state): State<TaskState>,
    Path((workspace, project)): Path<(String, String)>,
    headers: HeaderMap,
    request_id: Option<Extension<RequestId>>,
) -> Result<Json<LabelRuleSettings>, ApiError> {
    let instance = format!("/api/v1/workspaces/{workspace}/projects/{project}/label-rules");
    let (workspace_id, actor_id) =
        scope(&state, &headers, &workspace, &instance, request_id.as_ref()).await?;
    let project_id = parse_id(&project, &instance, request_id.as_ref())?;
    repository(&state)
        .settings(workspace_id, project_id, actor_id)
        .await
        .map(Json)
        .map_err(|error| task_problem(error, instance, request_id.as_ref()))
}

#[utoipa::path(post, path = "/api/v1/workspaces/{workspace_id}/projects/{project_id}/label-rules", params(("workspace_id" = String, Path), ("project_id" = String, Path)), request_body = LabelRuleBody, responses((status = 201, body = LabelRuleRecord)))]
pub(crate) async fn create_label_rule(
    State(state): State<TaskState>,
    Path((workspace, project)): Path<(String, String)>,
    headers: HeaderMap,
    request_id: Option<Extension<RequestId>>,
    ApiJson(body): ApiJson<LabelRuleBody>,
) -> Result<Response, ApiError> {
    let instance = format!("/api/v1/workspaces/{workspace}/projects/{project}/label-rules");
    let (workspace_id, actor_id) =
        scope(&state, &headers, &workspace, &instance, request_id.as_ref()).await?;
    let project_id = parse_id(&project, &instance, request_id.as_ref())?;
    let invalid = |field| {
        task_problem(
            crate::repositories::tasks::TaskError::Invalid { field },
            &instance,
            request_id.as_ref(),
        )
    };
    let label_id = body.label_id.parse().map_err(|_| invalid("label_id"))?;
    let user_id = body.user_id.parse().map_err(|_| invalid("user_id"))?;
    repository(&state)
        .create_rule(
            workspace_id,
            project_id,
            actor_id,
            label_id,
            user_id,
            body.enabled.unwrap_or(true),
            request_id_value(request_id.as_ref()),
            TimestampMillis::now(),
        )
        .await
        .map(|rule| (StatusCode::CREATED, Json(rule)).into_response())
        .map_err(|error| task_problem(error, instance, request_id.as_ref()))
}

#[utoipa::path(patch, path = "/api/v1/workspaces/{workspace_id}/projects/{project_id}/label-rules/{rule_id}", params(("workspace_id" = String, Path), ("project_id" = String, Path), ("rule_id" = String, Path)), request_body = LabelRulePatchBody, responses((status = 200, body = LabelRuleRecord)))]
pub(crate) async fn update_label_rule(
    State(state): State<TaskState>,
    Path((workspace, project, rule)): Path<(String, String, String)>,
    headers: HeaderMap,
    request_id: Option<Extension<RequestId>>,
    ApiJson(body): ApiJson<LabelRulePatchBody>,
) -> Result<Json<LabelRuleRecord>, ApiError> {
    let instance = format!("/api/v1/workspaces/{workspace}/projects/{project}/label-rules/{rule}");
    let (workspace_id, actor_id) =
        scope(&state, &headers, &workspace, &instance, request_id.as_ref()).await?;
    let project_id = parse_id(&project, &instance, request_id.as_ref())?;
    let rule_id = parse_id(&rule, &instance, request_id.as_ref())?;
    let invalid = |field| {
        task_problem(
            crate::repositories::tasks::TaskError::Invalid { field },
            &instance,
            request_id.as_ref(),
        )
    };
    let patch = LabelRulePatch {
        label_id: body
            .label_id
            .map(|id| id.parse().map_err(|_| invalid("label_id")))
            .transpose()?,
        user_id: body
            .user_id
            .map(|id| id.parse().map_err(|_| invalid("user_id")))
            .transpose()?,
        enabled: body.enabled,
    };
    repository(&state)
        .update_rule(
            workspace_id,
            project_id,
            rule_id,
            actor_id,
            patch,
            request_id_value(request_id.as_ref()),
            TimestampMillis::now(),
        )
        .await
        .map(Json)
        .map_err(|error| task_problem(error, instance, request_id.as_ref()))
}

#[utoipa::path(delete, path = "/api/v1/workspaces/{workspace_id}/projects/{project_id}/label-rules/{rule_id}", params(("workspace_id" = String, Path), ("project_id" = String, Path), ("rule_id" = String, Path)), responses((status = 204)))]
pub(crate) async fn delete_label_rule(
    State(state): State<TaskState>,
    Path((workspace, project, rule)): Path<(String, String, String)>,
    headers: HeaderMap,
    request_id: Option<Extension<RequestId>>,
) -> Result<StatusCode, ApiError> {
    let instance = format!("/api/v1/workspaces/{workspace}/projects/{project}/label-rules/{rule}");
    let (workspace_id, actor_id) =
        scope(&state, &headers, &workspace, &instance, request_id.as_ref()).await?;
    let project_id = parse_id(&project, &instance, request_id.as_ref())?;
    let rule_id = parse_id(&rule, &instance, request_id.as_ref())?;
    repository(&state)
        .delete_rule(
            workspace_id,
            project_id,
            rule_id,
            actor_id,
            request_id_value(request_id.as_ref()),
            TimestampMillis::now(),
        )
        .await
        .map(|()| StatusCode::NO_CONTENT)
        .map_err(|error| task_problem(error, instance, request_id.as_ref()))
}

/// Turns the owner switch on or off; answers with the rules and the switch.
#[utoipa::path(put, path = "/api/v1/workspaces/{workspace_id}/projects/{project_id}/label-rule-settings", params(("workspace_id" = String, Path), ("project_id" = String, Path)), request_body = LabelRuleSettingsBody, responses((status = 200, body = LabelRuleSettings)))]
pub(crate) async fn put_label_rule_settings(
    State(state): State<TaskState>,
    Path((workspace, project)): Path<(String, String)>,
    headers: HeaderMap,
    request_id: Option<Extension<RequestId>>,
    ApiJson(body): ApiJson<LabelRuleSettingsBody>,
) -> Result<Json<LabelRuleSettings>, ApiError> {
    let instance = format!("/api/v1/workspaces/{workspace}/projects/{project}/label-rule-settings");
    let (workspace_id, actor_id) =
        scope(&state, &headers, &workspace, &instance, request_id.as_ref()).await?;
    let project_id = parse_id(&project, &instance, request_id.as_ref())?;
    repository(&state)
        .set_owner_labels(
            workspace_id,
            project_id,
            actor_id,
            body.owner_labels,
            request_id_value(request_id.as_ref()),
            TimestampMillis::now(),
        )
        .await
        .map(Json)
        .map_err(|error| task_problem(error, instance, request_id.as_ref()))
}
