//! Orbit Drive for the team agent: `POST /api/v1/workspaces/{id}/agent-tools/drive`, for agent
//! sessions of the driver of a running thread only (as `/agent-tools/call`). It runs the Drive tools
//! of MCP clients (`crate::mcp_drive`) as the driver, so every Drive permission of that person
//! applies. Agent sessions cannot reach the normal Drive routes (`super::scope`): this route is
//! the agent's only way into Drive.
//!
//! Two actions run only after the driver answered "yes" in the thread (`super::integrations::approve`,
//! one approval for one run of exactly these arguments): `drive_trash` (restorable for 30 days)
//! and `drive_share` (a public link without a password). The audit log keeps who approved them.
//! There is no tool to delete forever, empty the trash, restore, or turn off a link.

use axum::extract::{Extension, Path, State};
use axum::http::{HeaderMap, Uri};
use axum::routing::post;
use axum::{Json, Router};
use orbit_platform::{Id, RequestId, TimestampMillis};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::AgentState;
use super::integrations::{
    Approval, Approves, approve, canonical, driver, hash, invalid, require_running, unavailable,
};
use crate::audit::{self, AuditOutcome};
use crate::mcp_drive::{self, Outcome};
use crate::repositories::drive::{DriveError, DriveNodeDetail, DriveNodeKind};
use crate::repositories::drive_shares::DriveShareRepository;
use crate::task_routes::ApiError;

type RequestIdExtension = Option<Extension<RequestId>>;

/// Tools that run without an approval: they read, or change what the driver asked for.
const TOOLS: [&str; 8] = [
    "drive_list",
    "drive_search",
    "drive_info",
    "drive_read",
    "drive_create_folder",
    "drive_upload_text",
    "drive_move",
    "drive_rename",
];

/// The Drive service, the file parser and the public origin of share links.
#[derive(Clone)]
pub(crate) struct AgentDrive {
    pub(crate) service: crate::drive::DriveService,
    pub(crate) reader: crate::file_content::FileReader,
    pub(crate) public_origin: String,
}

pub fn router(state: AgentState) -> Router {
    Router::new()
        .route(
            "/api/v1/workspaces/{workspace_id}/agent-tools/drive",
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
    node_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ShareArgs {
    node_id: String,
    /// 1 to 365; absent: the link works until someone turns it off.
    expires_in_days: Option<i64>,
}

fn answer(outcome: Outcome) -> Json<Value> {
    Json(match outcome {
        Outcome::Data(label, value) => json!({ "status": "ok", "label": label, "result": value }),
        Outcome::Failed(detail) => json!({ "status": "failed", "detail": detail }),
    })
}

fn refused(detail: &str) -> Json<Value> {
    Json(json!({ "status": "refused", "detail": detail }))
}

/// The item as the driver reads it in the approval question: kind and path.
fn about(detail: &DriveNodeDetail) -> (&'static str, String) {
    let path = detail
        .path
        .iter()
        .map(|crumb| crumb.name.as_str())
        .chain([detail.node.name.as_str()])
        .collect::<Vec<_>>()
        .join("/");
    match detail.node.kind {
        DriveNodeKind::Folder => ("folder", path),
        DriveNodeKind::File => ("file", path),
    }
}

/// The exact action, as the driver approves it.
fn describe(tool: &str, detail: &DriveNodeDetail, expires_in_days: Option<i64>) -> String {
    let (kind, path) = about(detail);
    let everything = if kind == "folder" {
        ", with everything in it,"
    } else {
        ""
    };
    if tool == "drive_trash" {
        return format!(
            "Orbit Drive: move the {kind} \"{path}\"{everything} to the trash (id {}). Members can restore it for 30 days.",
            detail.node.id
        );
    }
    let until = expires_in_days.map_or_else(
        || "until someone turns the link off".to_owned(),
        |days| format!("for {days} days"),
    );
    format!(
        "Orbit Drive: make a public link to the {kind} \"{path}\" (id {}). Anyone with the link can open it{everything} without signing in, {until}.",
        detail.node.id
    )
}

async fn call(
    State(state): State<AgentState>,
    Path(workspace_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
    Json(body): Json<CallBody>,
) -> Result<Json<Value>, ApiError> {
    let (user, workspace_id, _role) =
        driver(&state, &headers, &workspace_id, &uri, &request_id).await?;
    let database = state.database();
    let thread = (
        body.conversation_id.as_str(),
        body.root_message_id.as_deref(),
    );
    require_running(database, (workspace_id, user), thread, &uri, &request_id).await?;
    let Some(drive) = &state.drive else {
        return Ok(refused("Drive is not available on this server."));
    };
    let tool = body.tool.as_str();
    // `replace` would move the old file to the trash, which needs an approval.
    if tool == "drive_upload_text" && body.arguments.get("on_conflict") == Some(&json!("replace")) {
        return Ok(refused(
            "A text file cannot replace a file; use another name or on_conflict rename.",
        ));
    }
    if TOOLS.contains(&tool) {
        return Ok(
            match mcp_drive::run(
                &drive.service,
                &drive.reader,
                (workspace_id, user),
                tool,
                &body.arguments,
            )
            .await
            {
                Ok(outcome) => answer(outcome),
                Err(error) => refused(&error.message),
            },
        );
    }
    if !matches!(tool, "drive_trash" | "drive_share") {
        return Ok(refused("Unknown Drive tool."));
    }
    let raw = Value::Object(body.arguments.clone());
    let (node, expires_in_days) = if tool == "drive_trash" {
        match serde_json::from_value::<TrashArgs>(raw.clone()) {
            Ok(args) => (args.node_id, None),
            Err(_) => return Ok(refused("Give only node_id.")),
        }
    } else {
        match serde_json::from_value::<ShareArgs>(raw.clone()) {
            Ok(args)
                if args
                    .expires_in_days
                    .is_none_or(|days| (1..=365).contains(&days)) =>
            {
                (args.node_id, args.expires_in_days)
            }
            _ => {
                return Ok(refused(
                    "Give node_id, and expires_in_days from 1 to 365 or none.",
                ));
            }
        }
    };
    let Ok(node_id) = node.parse::<Id>() else {
        return Ok(answer(DriveError::NotFound.into()));
    };
    // Only what the driver can see is described, so nobody is asked about an item they cannot reach.
    let detail = match drive
        .service
        .repository()
        .node(workspace_id, user, node_id)
        .await
    {
        Ok(detail) => detail,
        Err(error) => return Ok(answer(error.into())),
    };
    if tool == "drive_share" && state.app_key.is_none() {
        return Ok(refused(
            "Public links are off on this server (it has no app key).",
        ));
    }
    let workspace = workspace_id.to_string();
    let approval = approve(
        database,
        Approves::Drive(&workspace),
        user,
        thread,
        tool,
        &hash(&canonical(&raw)),
    )
    .await
    .map_err(unavailable(&uri, &request_id))?;
    let approval_id = match approval {
        Approval::Needed { id, expires_at } => {
            return Ok(Json(json!({
                "status": "approval_required",
                "approval_id": id,
                "action": describe(tool, &detail, expires_in_days),
                "expires_at": expires_at,
            })));
        }
        Approval::Granted { id } => id,
    };
    let request = request_id
        .as_ref()
        .map_or("agent-drive", |Extension(id)| id.as_str());
    let now = TimestampMillis::now();
    let outcome = if tool == "drive_trash" {
        mcp_drive::run(
            &drive.service,
            &drive.reader,
            (workspace_id, user),
            tool,
            &body.arguments,
        )
        .await
        .map_err(|_| invalid("Invalid tool arguments.", &uri, &request_id))?
    } else {
        let (token, token_hash) = crate::drive_share_routes::new_token();
        let expires_at = expires_in_days.map(|days| {
            TimestampMillis::from_millis(now.as_millis() + days * 24 * 60 * 60 * 1_000)
        });
        match DriveShareRepository::new(database.clone())
            .create(
                workspace_id,
                user,
                node_id,
                &token_hash,
                expires_at,
                None,
                request,
                now,
            )
            .await
        {
            Ok(share) => Outcome::Data(
                "drive:share".to_owned(),
                json!({
                    "url": format!("{}/s/{token}", drive.public_origin.trim_end_matches('/')),
                    "share_id": share.id,
                    "expires_at": share.expires_at,
                }),
            ),
            Err(error) => error.into(),
        }
    };
    record(
        database,
        (workspace_id, user, node_id),
        &outcome,
        request,
        json!({
            "tool": tool,
            "approval_id": approval_id,
            "approved_by": user.to_string(),
            "conversation_id": body.conversation_id,
            "root_message_id": body.root_message_id,
        }),
    )
    .await;
    Ok(answer(outcome))
}

/// The audit line of an approved action: what ran, and the approval of whom.
async fn record(
    database: &orbit_platform::Database,
    (workspace_id, user, node_id): (Id, Id, Id),
    outcome: &Outcome,
    request: &str,
    mut metadata: Value,
) {
    if let Outcome::Failed(detail) = outcome {
        metadata["error"] = json!(detail);
    }
    let result = async {
        let mut tx = database.immediate_transaction().await?;
        audit::record(
            &mut tx,
            workspace_id,
            Some(user),
            "agent_drive.approved_action",
            if matches!(outcome, Outcome::Data(..)) {
                AuditOutcome::Success
            } else {
                AuditOutcome::Failure
            },
            "drive_node",
            Some(node_id),
            request,
            metadata,
            TimestampMillis::now(),
        )
        .await?;
        tx.commit().await
    }
    .await;
    if let Err(error) = result {
        tracing::warn!(%error, "agent drive audit");
    }
}
