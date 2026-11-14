//! Agent profiles: more agents than the default one (step 6). Owners and admins of a workspace
//! (`workspace.update`) make them in Settings → Agentic → Agents. Each agent is a bot user of its
//! own, so people mention it and DM it like a person, with a profile here: its instructions, the
//! tools it may use (a subset of the default agent's), its model and the channels where it works.
//!
//! The tool list is enforced by Orbit, not only by the prompt: an agent session of a custom agent
//! reaches only the routes that its tools need ([`route_need`], checked by `super::scope`), and
//! the Drive route checks the exact tool ([`drive_tool_allowed`]). The agent still acts with the
//! driver's permissions only, like the default agent.
//!
//! The default agent is the first card there (step 11): its name and avatar (its bot user's, the same in
//! every workspace, so only installation admins change them), and per workspace its instructions, model
//! and thinking level ([`agent_default_profiles`]). It cannot be deleted and works in every channel.
//!
//! Every agent, the default one too, uses only the integrations ticked on its card
//! ([`IntegrationAccess`], checked by `super::integrations` on top of each integration's role rule).
//! A new integration is off for custom agents and on for the default agent.
//!
//! Each person's answer level (Short, Normal, Thorough) is here too: it applies to every agent.
//!
//! Like the other agent routes, these are not in the OpenAPI contract.

use axum::extract::{Extension, Multipart, Path, State};
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::routing::{get, patch, put};
use axum::{Json, Router};
use orbit_domain::Permission;
use orbit_platform::{
    Database, Flag, Id, PasswordService, RequestId, Row, SqlBuilder, TimestampMillis,
    generate_opaque_token,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::AnyConnection;

use super::settings::{THINKING_LEVELS, checked_name, same_name};
use super::{AgentState, bot};
use crate::audit::{self, AuditOutcome};
use crate::repositories::membership;
use crate::task_routes::{ApiError, ApiJson, authenticate_session, request_id_value};

type RequestIdExtension = Option<Extension<RequestId>>;

/// Agents one workspace may have (deleted ones do not count).
pub const MAX_AGENTS: i64 = 20;
pub const MAX_INSTRUCTIONS_CHARS: usize = 8_000;
/// Channels one agent may list.
pub const MAX_CHANNELS: usize = 100;

/// The default agent's tools, by group, as the sidecar names them (`apps/agent/src/tools.ts`,
/// `media.ts`; `apps/agent/test/profiles.test.ts` keeps both lists the same). `true`: the tool
/// writes or posts.
pub const TOOL_CATALOG: [(&str, &str, bool); 59] = [
    ("list_projects", "Tasks", false),
    ("search_tasks", "Tasks", false),
    ("get_task", "Tasks", false),
    ("create_task", "Tasks", true),
    ("update_task", "Tasks", true),
    ("comment_on_task", "Tasks", true),
    ("follow_task", "Tasks", true),
    ("link_doc_to_task", "Tasks", true),
    ("unlink_doc_from_task", "Tasks", true),
    ("list_label_rules", "Tasks", false),
    ("create_label_rule", "Tasks", true),
    ("list_milestones_cycles", "Tasks", false),
    ("list_members", "People", false),
    ("ask_teammate", "People", true),
    ("list_pages", "Docs", false),
    ("search_pages", "Docs", false),
    ("read_page", "Docs", false),
    ("read_file", "Docs", false),
    ("create_page", "Docs", true),
    ("update_page", "Docs", true),
    ("write_file", "Docs", true),
    ("list_page_drive_files", "Docs", false),
    ("find_drive_file_pages", "Docs", false),
    ("insert_drive_file", "Docs", true),
    ("replace_drive_file", "Docs", true),
    ("remove_drive_file", "Docs", true),
    ("read_messages", "Chat", false),
    ("search_messages", "Chat", false),
    ("read_chat_file", "Chat", false),
    ("drive_list", "Drive", false),
    ("drive_search", "Drive", false),
    ("drive_info", "Drive", false),
    ("drive_read", "Drive", false),
    ("drive_create_folder", "Drive", true),
    ("drive_save_text", "Drive", true),
    ("drive_move", "Drive", true),
    ("drive_rename", "Drive", true),
    ("drive_trash", "Drive", true),
    ("drive_share", "Drive", true),
    ("memory_recall", "Memory", false),
    ("memory_zoom", "Memory", false),
    ("memory_note", "Memory", true),
    ("memory_summarize", "Memory", true),
    ("list_routines", "Routines and reminders", false),
    ("schedule_routine", "Routines and reminders", true),
    ("remind_me", "Routines and reminders", true),
    ("send_later", "Routines and reminders", true),
    ("list_scripts", "Routines and reminders", false),
    ("save_script", "Routines and reminders", true),
    ("test_routine", "Routines and reminders", true),
    ("approve_routine_script", "Routines and reminders", true),
    ("set_routine_script", "Routines and reminders", true),
    ("repair_routine_script", "Routines and reminders", true),
    ("use_integration", "Connected services", true),
    ("drive_image_to_shopify", "Connected services", true),
    ("media_shell", "Media", true),
    ("work_from_drive", "Media", false),
    ("view_image", "Media", false),
    ("work_to_drive", "Media", true),
];

/// The catalog, in the order the settings show it.
pub fn tool_catalog() -> impl Iterator<Item = (&'static str, &'static str, bool)> {
    TOOL_CATALOG.into_iter()
}

const TASK_TOOLS: &[&str] = &[
    "list_projects",
    "search_tasks",
    "get_task",
    "create_task",
    "update_task",
    "comment_on_task",
    "follow_task",
    "link_doc_to_task",
    "unlink_doc_from_task",
    "list_label_rules",
    "create_label_rule",
    "list_milestones_cycles",
];
const PAGE_TOOLS: &[&str] = &[
    "list_pages",
    "search_pages",
    "read_page",
    "read_file",
    "create_page",
    "update_page",
    "write_file",
    "list_page_drive_files",
    "find_drive_file_pages",
    // `after_text` reads the page first.
    "insert_drive_file",
];
const MEMORY_TOOLS: &[&str] = &[
    "memory_recall",
    "memory_zoom",
    "memory_note",
    "memory_summarize",
];
const DRIVE_ROUTE_TOOLS: &[&str] = &[
    "drive_list",
    "drive_search",
    "drive_info",
    "drive_read",
    "drive_create_folder",
    "drive_save_text",
    "drive_move",
    "drive_rename",
    "drive_trash",
    "drive_share",
    "work_from_drive",
    "view_image",
    "work_to_drive",
];
/// The tools that find a conversation by name (`conversationFor` in the sidecar).
const CONVERSATION_LIST_TOOLS: &[&str] = &[
    "read_messages",
    "schedule_routine",
    "list_routines",
    "send_later",
];
/// The tools that create through `/agent-tools/scheduled` (each one is checked there).
const SCHEDULED_ROUTE_TOOLS: &[&str] = &[
    "schedule_routine",
    "remind_me",
    "send_later",
    "save_script",
    "test_routine",
    "approve_routine_script",
    "set_routine_script",
    "repair_routine_script",
];

/// What a workspace route needs from a custom agent's tools.
#[derive(Debug, Eq, PartialEq)]
pub enum RouteNeed {
    /// Every agent reads it (the workspace and its member list: names for mentions).
    Always,
    /// One of these tools.
    Any(&'static [&'static str]),
    /// No tool uses it: refused.
    Never,
}

/// What `method rest` (the path after `/api/v1/workspaces/{id}/`, no trailing slash) needs. A
/// route that no tool uses is refused, so a new route stays closed to custom agents until a tool
/// names it here.
#[must_use]
pub fn route_need(method: &Method, rest: &str) -> RouteNeed {
    let parts: Vec<&str> = rest.split('/').collect();
    let get = method == Method::GET;
    match parts.as_slice() {
        [""] if get => RouteNeed::Always,
        ["members"] if get => RouteNeed::Always,
        ["projects", ..] if get => RouteNeed::Any(TASK_TOOLS),
        ["tasks", ..] if get => RouteNeed::Any(TASK_TOOLS),
        ["tasks"] if method == Method::POST => RouteNeed::Any(&["create_task"]),
        ["projects", _, "label-rules"] if method == Method::POST => {
            RouteNeed::Any(&["create_label_rule"])
        }
        ["tasks", _] if method == Method::PATCH => RouteNeed::Any(&["update_task"]),
        ["tasks", _, "comments"] if method == Method::POST => RouteNeed::Any(&["comment_on_task"]),
        ["tasks", _, "followers", "me"] if method == Method::PUT || method == Method::DELETE => {
            RouteNeed::Any(&["follow_task"])
        }
        ["tasks", _, "linked-docs"] if method == Method::POST => {
            RouteNeed::Any(&["link_doc_to_task"])
        }
        ["tasks", _, "linked-docs", _] if method == Method::DELETE => {
            RouteNeed::Any(&["unlink_doc_from_task"])
        }
        ["pages", ..] if get => RouteNeed::Any(PAGE_TOOLS),
        ["pages"] if method == Method::POST => RouteNeed::Any(&["create_page"]),
        ["pages", _] if method == Method::PATCH => RouteNeed::Any(&["create_page", "update_page"]),
        // Not `drive-files/resolve`: only the web editor reads with it.
        ["pages", _, "drive-files"] if method == Method::POST => {
            RouteNeed::Any(&["insert_drive_file"])
        }
        ["pages", _, "drive-files", block] if method == Method::PUT && *block != "resolve" => {
            RouteNeed::Any(&["replace_drive_file"])
        }
        ["pages", _, "drive-files", block] if method == Method::DELETE && *block != "resolve" => {
            RouteNeed::Any(&["remove_drive_file"])
        }
        ["pages", _, "files", _, "content"] if method == Method::PUT => {
            RouteNeed::Any(&["write_file"])
        }
        ["chat", "conversations"] if get => RouteNeed::Any(CONVERSATION_LIST_TOOLS),
        ["chat", "conversations", _, "messages"] if get => RouteNeed::Any(&["read_messages"]),
        ["chat", "threads", _] if get => RouteNeed::Any(&["read_messages"]),
        ["chat", "search"] if get => RouteNeed::Any(&["search_messages"]),
        ["chat", "files", _, "content"] if get => RouteNeed::Any(&["read_chat_file"]),
        ["agent-memory", ..] => RouteNeed::Any(MEMORY_TOOLS),
        ["agent-tools"] if get => RouteNeed::Any(&["use_integration"]),
        ["agent-tools", "call"] if method == Method::POST => RouteNeed::Any(&["use_integration"]),
        ["agent-tools", "drive"] if method == Method::POST => RouteNeed::Any(DRIVE_ROUTE_TOOLS),
        ["agent-tools", "shopify-image"] if method == Method::POST => {
            RouteNeed::Any(&["drive_image_to_shopify"])
        }
        // The skills turned on for this agent (`super::skills` checks which agent asks).
        ["agent-tools", "skills", ..] if get => RouteNeed::Always,
        // The tool itself is checked there (`crate::scheduled`).
        ["agent-tools", "scheduled"] if method == Method::POST => {
            RouteNeed::Any(SCHEDULED_ROUTE_TOOLS)
        }
        ["routines"] if get => RouteNeed::Any(&[
            "schedule_routine",
            "list_routines",
            "test_routine",
            "approve_routine_script",
            "set_routine_script",
        ]),
        ["scripts"] if get => RouteNeed::Any(&[
            "list_scripts",
            "save_script",
            "schedule_routine",
            "set_routine_script",
        ]),
        ["scheduled-messages"] if get => RouteNeed::Any(&["remind_me", "send_later"]),
        _ => RouteNeed::Never,
    }
}

/// Whether an agent with these tools may make this request (after `scope::allows`).
#[must_use]
pub fn tools_allow(tools: &[String], method: &Method, rest: &str) -> bool {
    match route_need(method, rest) {
        RouteNeed::Always => true,
        RouteNeed::Any(needed) => needed
            .iter()
            .any(|tool| tools.iter().any(|own| own == tool)),
        RouteNeed::Never => false,
    }
}

/// Whether an agent with these tools may run this tool of `/agent-tools/drive` (`super::drive`).
#[must_use]
pub fn drive_tool_allowed(tools: &[String], server_tool: &str) -> bool {
    let needed: &[&str] = match server_tool {
        "drive_upload_text" => &["drive_save_text"],
        "drive_fetch" => &["work_from_drive", "view_image"],
        "drive_upload_begin" | "drive_upload_complete" | "drive_upload_cancel" => {
            &["work_to_drive"]
        }
        other => return tools.iter().any(|own| own == other),
    };
    needed
        .iter()
        .any(|tool| tools.iter().any(|own| own == tool))
}

/// The tools of a request of a custom agent's session (`super::scope` adds it): `Drive` checks
/// the exact tool with it. Absent for the default agent and for people.
#[derive(Clone, Debug)]
pub struct AgentTools(pub std::sync::Arc<Vec<String>>);

/// Known tool names, each once, in the catalog's order. Writing memory needs its summaries.
pub fn checked_tools(raw: &[String]) -> Result<Vec<String>, &'static str> {
    if raw
        .iter()
        .any(|name| !tool_catalog().any(|(known, _, _)| known == name.as_str()))
    {
        return Err("One of the tools is not a tool of the agent.");
    }
    let wanted = |name: &str| {
        raw.iter().any(|own| own == name)
            || (name == "memory_summarize" && raw.iter().any(|own| own == "memory_note"))
    };
    Ok(tool_catalog()
        .filter(|(name, _, _)| wanted(name))
        .map(|(name, _, _)| name.to_owned())
        .collect())
}

/// The instructions as stored: `\n` line breaks, trimmed, at most 8,000 characters.
pub fn checked_instructions(raw: &str) -> Result<String, &'static str> {
    let text = raw.replace("\r\n", "\n");
    let text = text.trim();
    if text.chars().count() > MAX_INSTRUCTIONS_CHARS {
        return Err("Instructions may have at most 8,000 characters.");
    }
    if text
        .chars()
        .any(|c| c.is_control() && c != '\n' && c != '\t')
    {
        return Err("Instructions may not contain control characters.");
    }
    Ok(text.to_owned())
}

/// A custom agent that is not deleted.
#[derive(Clone, Debug)]
pub struct LiveAgent {
    pub profile_id: Id,
    pub user_id: Id,
    pub all_channels: bool,
}

/// The live custom agent with this bot user in this workspace.
pub(crate) async fn live_agent(
    conn: &mut AnyConnection,
    workspace_id: Id,
    user_id: Id,
) -> Result<Option<LiveAgent>, sqlx::Error> {
    let row = sqlx::query(
        "SELECT id, all_channels FROM agent_profiles WHERE workspace_id = $1 AND user_id = $2 \
         AND deleted_at IS NULL",
    )
    .bind(workspace_id.to_string())
    .bind(user_id.to_string())
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row.and_then(|row| {
        Some(LiveAgent {
            profile_id: row.get::<String, _>("id").parse().ok()?,
            user_id,
            all_channels: row.get::<bool, _>("all_channels"),
        })
    }))
}

/// Every bot user of a custom agent of the workspace, with whether it is deleted.
pub(crate) async fn workspace_agents(
    conn: &mut AnyConnection,
    workspace_id: Id,
) -> Result<Vec<(Id, bool)>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT user_id, deleted_at FROM agent_profiles WHERE workspace_id = $1 ORDER BY created_at",
    )
    .bind(workspace_id.to_string())
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows
        .iter()
        .filter_map(|row| {
            Some((
                row.get::<String, _>("user_id").parse().ok()?,
                row.get::<Option<i64>, _>("deleted_at").is_some(),
            ))
        })
        .collect())
}

/// Whether the agent may be used in this conversation: a DM or group DM it is in, every channel,
/// or a channel of its list.
pub(crate) async fn allowed_in(
    conn: &mut AnyConnection,
    agent: &LiveAgent,
    conversation_id: Id,
) -> Result<bool, sqlx::Error> {
    let kind: Option<String> =
        sqlx::query_scalar("SELECT kind FROM chat_conversations WHERE id = $1")
            .bind(conversation_id.to_string())
            .fetch_optional(&mut *conn)
            .await?;
    match kind.as_deref() {
        None => Ok(false),
        Some("dm") => Ok(true),
        Some(_) if agent.all_channels => Ok(true),
        Some(_) => Ok(sqlx::query_scalar::<_, Flag>(
            "SELECT EXISTS (SELECT 1 FROM agent_profile_channels WHERE agent_id = $1 AND conversation_id = $2)",
        )
        .bind(agent.profile_id.to_string())
        .bind(conversation_id.to_string())
        .fetch_one(&mut *conn)
        .await?
        .0),
    }
}

/// The names of the channels where the agent works, for its refusal message.
pub(crate) async fn channel_names(
    conn: &mut AnyConnection,
    agent: &LiveAgent,
) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT c.name FROM agent_profile_channels a JOIN chat_conversations c ON c.id = a.conversation_id \
         WHERE a.agent_id = $1 AND c.archived_at IS NULL ORDER BY c.name",
    )
    .bind(agent.profile_id.to_string())
    .fetch_all(&mut *conn)
    .await
}

// ---------------------------------------------------------------------------------------------
// Integrations per agent

/// The integrations of a workspace that one agent may use, before each integration's role rule.
#[derive(Debug, Eq, PartialEq)]
pub enum IntegrationAccess {
    /// The default agent: every integration but these (new ones are on).
    AllBut(Vec<String>),
    /// A custom agent: only these (new ones are off).
    Only(Vec<String>),
}

impl IntegrationAccess {
    #[must_use]
    pub fn allows(&self, integration_id: &str) -> bool {
        match self {
            Self::AllBut(off) => !off.iter().any(|id| id == integration_id),
            Self::Only(on) => on.iter().any(|id| id == integration_id),
        }
    }
}

/// What `agent` (`None`: the default agent) may use in the workspace. A deleted agent: nothing.
pub(crate) async fn integration_access(
    conn: &mut AnyConnection,
    workspace_id: Id,
    agent: Option<Id>,
) -> Result<IntegrationAccess, sqlx::Error> {
    Ok(match agent {
        None => IntegrationAccess::AllBut(
            sqlx::query_scalar(
                "SELECT o.integration_id FROM agent_default_integrations_off o \
                 JOIN agent_integrations i ON i.id = o.integration_id WHERE i.workspace_id = $1",
            )
            .bind(workspace_id.to_string())
            .fetch_all(&mut *conn)
            .await?,
        ),
        Some(agent) => IntegrationAccess::Only(
            sqlx::query_scalar(
                "SELECT a.integration_id FROM agent_profile_integrations a \
                 JOIN agent_profiles p ON p.id = a.agent_id \
                 WHERE p.workspace_id = $1 AND p.user_id = $2 AND p.deleted_at IS NULL",
            )
            .bind(workspace_id.to_string())
            .bind(agent.to_string())
            .fetch_all(&mut *conn)
            .await?,
        ),
    })
}

/// The workspace's integrations, by name: the ticks of every agent card.
async fn workspace_integrations(
    conn: &mut AnyConnection,
    workspace_id: Id,
) -> Result<Vec<IntegrationOption>, sqlx::Error> {
    Ok(sqlx::query(
        "SELECT id, name, kind FROM agent_integrations WHERE workspace_id = $1 ORDER BY lower(name), id",
    )
    .bind(workspace_id.to_string())
    .fetch_all(&mut *conn)
    .await?
    .iter()
    .map(|row| IntegrationOption {
        id: row.get("id"),
        name: row.get("name"),
        kind: row.get("kind"),
    })
    .collect())
}

/// The listed integrations, each once, if every one is the workspace's.
async fn checked_integrations(
    conn: &mut AnyConnection,
    workspace_id: Id,
    ids: &[String],
) -> Result<Vec<String>, SaveError> {
    let known: Vec<String> = workspace_integrations(conn, workspace_id)
        .await?
        .into_iter()
        .map(|integration| integration.id)
        .collect();
    if ids.iter().any(|id| !known.contains(id)) {
        return Err(SaveError::Invalid(
            "One of the connected services is not this workspace's.",
        ));
    }
    Ok(known.into_iter().filter(|id| ids.contains(id)).collect())
}

/// A custom agent's ticks: exactly `ids`.
async fn set_integrations(
    conn: &mut AnyConnection,
    profile_id: Id,
    ids: &[String],
) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM agent_profile_integrations WHERE agent_id = $1")
        .bind(profile_id.to_string())
        .execute(&mut *conn)
        .await?;
    for id in ids {
        sqlx::query(
            "INSERT INTO agent_profile_integrations (agent_id, integration_id) VALUES ($1, $2)",
        )
        .bind(profile_id.to_string())
        .bind(id)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Answer level

/// How much an agent thinks and writes, set on its card.
pub const ANSWER_LEVELS: [&str; 3] = ["short", "normal", "thorough"];

fn checked_answer_level(raw: Option<&str>) -> Result<Option<String>, &'static str> {
    match raw {
        None => Ok(None),
        Some(level) if ANSWER_LEVELS.contains(&level) => Ok(Some(level.to_owned())),
        Some(_) => Err("The answer level must be short, normal or thorough."),
    }
}

// ---------------------------------------------------------------------------------------------
// Routes

pub fn router(state: AgentState) -> Router {
    let base = "/api/v1/workspaces/{workspace_id}/agents";
    Router::new()
        .route(base, get(list_agents).post(create_agent))
        // The default agent: no DELETE, it cannot be deleted.
        .route(&format!("{base}/default"), patch(update_default))
        .route(
            &format!("{base}/default/avatar"),
            put(upload_default_avatar).delete(remove_default_avatar),
        )
        .route(
            &format!("{base}/{{agent_id}}"),
            patch(update_agent).delete(delete_agent),
        )
        .route(
            &format!("{base}/{{agent_id}}/avatar"),
            put(upload_avatar).delete(remove_avatar),
        )
        .with_state(state)
}

fn problem(
    status: StatusCode,
    code: &'static str,
    detail: &'static str,
    uri: &Uri,
    request_id: &RequestIdExtension,
) -> ApiError {
    let title = status.canonical_reason().unwrap_or("Error");
    ApiError::new(status, code, title, detail, uri.path(), request_id.as_ref())
}

fn unavailable(error: &sqlx::Error, uri: &Uri, request_id: &RequestIdExtension) -> ApiError {
    tracing::warn!(%error, "agent profiles");
    problem(
        StatusCode::SERVICE_UNAVAILABLE,
        "unavailable",
        "Try again in a moment.",
        uri,
        request_id,
    )
}

fn invalid(detail: &'static str, uri: &Uri, request_id: &RequestIdExtension) -> ApiError {
    problem(
        StatusCode::UNPROCESSABLE_ENTITY,
        "validation_failed",
        detail,
        uri,
        request_id,
    )
}

fn not_found(uri: &Uri, request_id: &RequestIdExtension) -> ApiError {
    problem(
        StatusCode::NOT_FOUND,
        "not_found",
        "Not found.",
        uri,
        request_id,
    )
}

fn forbidden(uri: &Uri, request_id: &RequestIdExtension) -> ApiError {
    problem(
        StatusCode::FORBIDDEN,
        "forbidden",
        "Only owners and admins can manage agents.",
        uri,
        request_id,
    )
}

/// The caller as a member (not a guest) of the workspace; 404 for others and when the agent is off.
async fn member(
    state: &AgentState,
    headers: &HeaderMap,
    workspace_id: &str,
    uri: &Uri,
    request_id: &RequestIdExtension,
) -> Result<(Id, orbit_domain::Actor), ApiError> {
    let session = authenticate_session(
        &state.identity,
        state.cookie_mode,
        headers,
        uri.path(),
        request_id.as_ref(),
    )
    .await?;
    if state.config.is_none() {
        return Err(problem(
            StatusCode::NOT_FOUND,
            "agent_disabled",
            "The agent is not set up on this server.",
            uri,
            request_id,
        ));
    }
    let workspace_id: Id = workspace_id
        .parse()
        .map_err(|_| not_found(uri, request_id))?;
    let actor = membership::actor(state.database().pool(), workspace_id, session.user.id)
        .await
        .map_err(|error| unavailable(&error, uri, request_id))?
        .ok_or_else(|| not_found(uri, request_id))?;
    Ok((workspace_id, actor))
}

/// The caller as an owner or admin of the workspace.
async fn manager(
    state: &AgentState,
    headers: &HeaderMap,
    workspace_id: &str,
    uri: &Uri,
    request_id: &RequestIdExtension,
) -> Result<(Id, orbit_domain::Actor), ApiError> {
    let (workspace_id, actor) = member(state, headers, workspace_id, uri, request_id).await?;
    if !actor.can(Permission::WorkspaceUpdate) {
        return Err(forbidden(uri, request_id));
    }
    Ok((workspace_id, actor))
}

#[derive(Serialize)]
struct ChannelView {
    id: String,
    name: String,
    kind: String,
}

#[derive(Serialize)]
struct AgentView {
    id: String,
    user_id: String,
    name: String,
    avatar_url: Option<String>,
    instructions: String,
    tools: Vec<String>,
    model: Option<String>,
    thinking: Option<String>,
    answer_level: String,
    all_channels: bool,
    channels: Vec<ChannelView>,
    /// The integrations it may use (ticked on its card).
    integration_ids: Vec<String>,
    created_at: i64,
    updated_at: i64,
}

/// The default agent's card. Its name and avatar are its bot user's, the same in every workspace.
#[derive(Serialize)]
struct DefaultAgentView {
    user_id: String,
    name: String,
    avatar_url: Option<String>,
    instructions: String,
    model: Option<String>,
    thinking: Option<String>,
    answer_level: String,
    integration_ids: Vec<String>,
    /// Installation admins: the name and avatar are the installation's.
    can_rename: bool,
}

#[derive(Serialize)]
struct IntegrationOption {
    id: String,
    name: String,
    kind: String,
}

#[derive(Serialize)]
struct ToolView {
    name: &'static str,
    group: &'static str,
    writes: bool,
}

async fn agent_views(
    database: &Database,
    workspace_id: Id,
    only: Option<Id>,
) -> Result<Vec<AgentView>, sqlx::Error> {
    let mut query = SqlBuilder::new(
        "SELECT p.id, p.user_id, u.display_name, a.updated_at AS avatar_at, p.instructions, p.tools, \
         p.model, p.thinking, p.answer_level, p.all_channels, p.created_at, p.updated_at FROM agent_profiles p \
         JOIN users u ON u.id = p.user_id LEFT JOIN user_avatars a ON a.user_id = p.user_id \
         WHERE p.deleted_at IS NULL AND p.workspace_id = ",
    );
    query.push_bind(workspace_id.to_string());
    if let Some(only) = only {
        query.push(" AND p.id = ");
        query.push_bind(only.to_string());
    }
    query.push(" ORDER BY lower(u.display_name), p.id");
    let rows = query.build().fetch_all(database.pool()).await?;
    let mut views = Vec::with_capacity(rows.len());
    for row in rows {
        let id: String = row.get("id");
        let user_id: String = row.get("user_id");
        let channels = sqlx::query(
            "SELECT c.id, c.name, c.kind FROM agent_profile_channels a \
             JOIN chat_conversations c ON c.id = a.conversation_id WHERE a.agent_id = $1 ORDER BY c.name",
        )
        .bind(&id)
        .fetch_all(database.pool())
        .await?
        .iter()
        .map(|channel| ChannelView {
            id: channel.get("id"),
            name: channel.get("name"),
            kind: channel.get("kind"),
        })
        .collect();
        let integration_ids = sqlx::query_scalar(
            "SELECT integration_id FROM agent_profile_integrations WHERE agent_id = $1 ORDER BY integration_id",
        )
        .bind(&id)
        .fetch_all(database.pool())
        .await?;
        views.push(AgentView {
            avatar_url: user_id.parse().ok().and_then(|user: Id| {
                crate::auth_routes::avatar_url(user, row.get::<Option<i64>, _>("avatar_at"))
            }),
            id,
            user_id,
            name: row.get("display_name"),
            instructions: row.get("instructions"),
            tools: serde_json::from_str(&row.get::<String, _>("tools")).unwrap_or_default(),
            model: row.get("model"),
            thinking: row.get("thinking"),
            answer_level: row.get("answer_level"),
            all_channels: row.get::<bool, _>("all_channels"),
            channels,
            integration_ids,
            created_at: row.get("created_at"),
            updated_at: row.get("updated_at"),
        });
    }
    Ok(views)
}

/// Every live agent of the workspace, the tool catalog and the models to pick from. Every member
/// reads it (the routine form picks an agent); only owners and admins change it.
async fn list_agents(
    State(state): State<AgentState>,
    Path(workspace_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
) -> Result<Json<Value>, ApiError> {
    let (workspace_id, actor) = member(&state, &headers, &workspace_id, &uri, &request_id).await?;
    let agents = agent_views(state.database(), workspace_id, None)
        .await
        .map_err(|error| unavailable(&error, &uri, &request_id))?;
    let can_manage = actor.can(Permission::WorkspaceUpdate);
    let can_rename = state
        .identity
        .is_installation_admin(actor.user_id)
        .await
        .unwrap_or(false);
    let default_agent = default_view(state.database(), workspace_id, can_rename)
        .await
        .map_err(|error| unavailable(&error, &uri, &request_id))?;
    let integrations = if can_manage {
        let mut conn = state
            .database()
            .pool()
            .acquire()
            .await
            .map_err(|error| unavailable(&error, &uri, &request_id))?;
        workspace_integrations(&mut conn, workspace_id)
            .await
            .map_err(|error| unavailable(&error, &uri, &request_id))?
    } else {
        Vec::new()
    };
    let tools: Vec<ToolView> = tool_catalog()
        .map(|(name, group, writes)| ToolView {
            name,
            group,
            writes,
        })
        .collect();
    let models = if can_manage {
        super::settings::sidecar_models(&state).await
    } else {
        None
    };
    Ok(Json(json!({
        "default_agent": default_agent,
        "agents": agents,
        "integrations": integrations,
        "can_manage": can_manage,
        "tools": tools,
        "models": models,
        "thinking_levels": THINKING_LEVELS,
        "max_instructions_chars": MAX_INSTRUCTIONS_CHARS,
        "max_agents": MAX_AGENTS,
    })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentBody {
    name: Option<String>,
    instructions: Option<String>,
    tools: Option<Vec<String>>,
    /// `null` or absent on create: the installation's model. On update, absent keeps it.
    #[serde(default, with = "double_option")]
    model: Option<Option<String>>,
    #[serde(default, with = "double_option")]
    thinking: Option<Option<String>>,
    /// `short`, `normal` or `thorough`; absent keeps it (`normal` on create).
    answer_level: Option<String>,
    all_channels: Option<bool>,
    channel_ids: Option<Vec<Id>>,
    /// The integrations it may use; absent on update keeps them.
    integration_ids: Option<Vec<String>>,
}

/// Tells an absent field (`None`) from `null` (`Some(None)`).
mod double_option {
    use serde::{Deserialize, Deserializer};

    pub fn deserialize<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
    where
        D: Deserializer<'de>,
        T: Deserialize<'de>,
    {
        Option::<T>::deserialize(deserializer).map(Some)
    }
}

enum SaveError {
    Invalid(&'static str),
    Taken,
    NotFound,
    Forbidden,
    Limit,
    /// The default agent's name or avatar: the installation's, so installation admins only.
    NotInstallationAdmin,
    Database(sqlx::Error),
}

impl From<sqlx::Error> for SaveError {
    fn from(error: sqlx::Error) -> Self {
        Self::Database(error)
    }
}

fn save_failure(error: SaveError, uri: &Uri, request_id: &RequestIdExtension) -> ApiError {
    match error {
        SaveError::Invalid(detail) => invalid(detail, uri, request_id),
        SaveError::Taken => problem(
            StatusCode::CONFLICT,
            "agent_name_taken",
            "Someone in Orbit already has this name. Mentions of the agent would reach them, so pick another name.",
            uri,
            request_id,
        ),
        SaveError::NotFound => not_found(uri, request_id),
        SaveError::Forbidden => forbidden(uri, request_id),
        SaveError::Limit => problem(
            StatusCode::CONFLICT,
            "limit_reached",
            "This workspace has 20 agents. Delete one first.",
            uri,
            request_id,
        ),
        SaveError::NotInstallationAdmin => problem(
            StatusCode::FORBIDDEN,
            "installation_admin_required",
            "The default agent's name and avatar are the same in every workspace, so only an installation administrator changes them.",
            uri,
            request_id,
        ),
        SaveError::Database(error) => unavailable(&error, uri, request_id),
    }
}

/// The model and thinking level, checked against what the sidecar offers.
async fn checked_model(
    state: &AgentState,
    model: Option<Option<String>>,
    thinking: Option<Option<String>>,
    uri: &Uri,
    request_id: &RequestIdExtension,
) -> Result<(Option<Option<String>>, Option<Option<String>>), ApiError> {
    let thinking = thinking.map(|level| level.filter(|level| !level.is_empty()));
    if let Some(Some(level)) = &thinking
        && !THINKING_LEVELS.contains(&level.as_str())
    {
        return Err(invalid(
            "The thinking level must be minimal, low, medium, high, xhigh or max.",
            uri,
            request_id,
        ));
    }
    let model = model.map(|model| {
        model
            .map(|model| model.trim().to_owned())
            .filter(|model| !model.is_empty())
    });
    if let Some(Some(model)) = &model {
        let available = super::settings::sidecar_models(state)
            .await
            .ok_or_else(|| {
                problem(
                    StatusCode::BAD_GATEWAY,
                    "agent_offline",
                    "The agent service is offline, so the model cannot be checked. Try again when it runs.",
                    uri,
                    request_id,
                )
            })?;
        if !available.models.iter().any(|option| &option.id == model) {
            return Err(invalid(
                "The agent does not know this model.",
                uri,
                request_id,
            ));
        }
    }
    Ok((model, thinking))
}

/// Whether another account has this name (any case, trimmed): `@Name` must reach one account.
pub(crate) async fn name_taken(
    conn: &mut AnyConnection,
    own: Option<Id>,
    name: &str,
) -> Result<bool, sqlx::Error> {
    let names =
        sqlx::query_scalar::<_, String>("SELECT DISTINCT display_name FROM users WHERE id <> $1")
            .bind(own.map(|id| id.to_string()).unwrap_or_default())
            .fetch_all(&mut *conn)
            .await?;
    Ok(names.iter().any(|other| same_name(other, name)))
}

/// Joins the agent's bot user to the listed channels and takes it out of channels it left. The
/// channels must be in the workspace, and a private one must have the caller in it.
async fn set_channels(
    conn: &mut AnyConnection,
    workspace_id: Id,
    actor: Id,
    agent: &LiveAgent,
    channel_ids: &[Id],
    now: TimestampMillis,
) -> Result<(), SaveError> {
    if channel_ids.len() > MAX_CHANNELS {
        return Err(SaveError::Invalid(
            "An agent can have at most 100 channels.",
        ));
    }
    for channel in channel_ids {
        let visible = sqlx::query_scalar::<_, Flag>(
            "SELECT EXISTS (SELECT 1 FROM chat_conversations c WHERE c.id = $1 AND c.workspace_id = $2 \
             AND c.kind <> 'dm' AND (c.kind = 'public' OR EXISTS (SELECT 1 FROM chat_members m \
             WHERE m.conversation_id = c.id AND m.user_id = $3)))",
        )
        .bind(channel.to_string())
        .bind(workspace_id.to_string())
        .bind(actor.to_string())
        .fetch_one(&mut *conn)
        .await?
        .0;
        if !visible {
            return Err(SaveError::Invalid(
                "One of the channels does not exist or is not yours to pick.",
            ));
        }
    }
    let old: Vec<String> = sqlx::query_scalar(
        "SELECT conversation_id FROM agent_profile_channels WHERE agent_id = $1",
    )
    .bind(agent.profile_id.to_string())
    .fetch_all(&mut *conn)
    .await?;
    for gone in old.iter().filter(|id| {
        !channel_ids
            .iter()
            .any(|channel| channel.to_string() == **id)
    }) {
        sqlx::query(
            "DELETE FROM agent_profile_channels WHERE agent_id = $1 AND conversation_id = $2",
        )
        .bind(agent.profile_id.to_string())
        .bind(gone)
        .execute(&mut *conn)
        .await?;
        sqlx::query("DELETE FROM chat_members WHERE conversation_id = $1 AND user_id = $2")
            .bind(gone)
            .bind(agent.user_id.to_string())
            .execute(&mut *conn)
            .await?;
    }
    for channel in channel_ids {
        sqlx::query(
            "INSERT INTO agent_profile_channels (agent_id, conversation_id) VALUES ($1, $2) \
             ON CONFLICT (agent_id, conversation_id) DO NOTHING",
        )
        .bind(agent.profile_id.to_string())
        .bind(channel.to_string())
        .execute(&mut *conn)
        .await?;
        // In with everything read, as a member brought in by a mention (`inputs::record`).
        sqlx::query(
            "INSERT INTO chat_members (conversation_id, user_id, workspace_id, notify, last_read_message_id, \
             read_count, joined_at) SELECT c.id, $2, c.workspace_id, 'mentions', NULL, c.message_count, $3 \
             FROM chat_conversations c WHERE c.id = $1 ON CONFLICT (conversation_id, user_id) DO NOTHING",
        )
        .bind(channel.to_string())
        .bind(agent.user_id.to_string())
        .bind(now.as_millis())
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

async fn create_agent(
    State(state): State<AgentState>,
    Path(workspace_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
    ApiJson(body): ApiJson<AgentBody>,
) -> Result<Json<Value>, ApiError> {
    let (workspace_id, actor) = manager(&state, &headers, &workspace_id, &uri, &request_id).await?;
    let name = checked_name(body.name.as_deref().unwrap_or_default())
        .map_err(|detail| invalid(detail, &uri, &request_id))?;
    let instructions = checked_instructions(body.instructions.as_deref().unwrap_or_default())
        .map_err(|detail| invalid(detail, &uri, &request_id))?;
    let tools = checked_tools(body.tools.as_deref().unwrap_or_default())
        .map_err(|detail| invalid(detail, &uri, &request_id))?;
    let answer_level = checked_answer_level(body.answer_level.as_deref())
        .map_err(|detail| invalid(detail, &uri, &request_id))?
        .unwrap_or_else(|| "normal".to_owned());
    let (model, thinking) =
        checked_model(&state, body.model, body.thinking, &uri, &request_id).await?;
    // The hash of a random secret that is thrown away: no password signs in as the agent.
    let password_hash = PasswordService::default()
        .hash(&generate_opaque_token())
        .map_err(|_| {
            unavailable(
                &sqlx::Error::Protocol("agent password".to_owned()),
                &uri,
                &request_id,
            )
        })?;
    let database = state.database();
    let request = request_id_value(request_id.as_ref()).to_owned();
    let created = async {
        let mut tx = database.immediate_transaction().await?;
        let allowed = membership::actor(&mut *tx, workspace_id, actor.user_id)
            .await?
            .is_some_and(|actor| actor.can(Permission::WorkspaceUpdate));
        if !allowed {
            return Err(SaveError::Forbidden);
        }
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM agent_profiles WHERE workspace_id = $1 AND deleted_at IS NULL",
        )
        .bind(workspace_id.to_string())
        .fetch_one(&mut *tx)
        .await?;
        if count >= MAX_AGENTS {
            return Err(SaveError::Limit);
        }
        if name_taken(&mut tx, None, &name).await? {
            return Err(SaveError::Taken);
        }
        let now = TimestampMillis::now();
        let user_id = Id::new_v7();
        let profile_id = Id::new_v7();
        let email = format!("agent-{user_id}@orbit.invalid");
        sqlx::query(
            "INSERT INTO users (id, email, normalized_email, display_name, password_hash, created_at, updated_at) \
             VALUES ($1, $2, $2, $3, $4, $5, $5)",
        )
        .bind(user_id.to_string())
        .bind(&email)
        .bind(&name)
        .bind(&password_hash)
        .bind(now.as_millis())
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO memberships (id, workspace_id, user_id, role, version, created_at, updated_at) \
             VALUES ($1, $2, $3, 'member', 0, $4, $4)",
        )
        .bind(Id::new_v7().to_string())
        .bind(workspace_id.to_string())
        .bind(user_id.to_string())
        .bind(now.as_millis())
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO agent_profiles (id, workspace_id, user_id, instructions, tools, model, thinking, \
             all_channels, created_by, created_at, updated_at, answer_level) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $10, $11)",
        )
        .bind(profile_id.to_string())
        .bind(workspace_id.to_string())
        .bind(user_id.to_string())
        .bind(&instructions)
        .bind(serde_json::to_string(&tools).unwrap_or_else(|_| "[]".to_owned()))
        .bind(model.clone().flatten())
        .bind(thinking.clone().flatten())
        .bind(i64::from(body.all_channels.unwrap_or(false)))
        .bind(actor.user_id.to_string())
        .bind(now.as_millis())
        .bind(&answer_level)
        .execute(&mut *tx)
        .await?;
        let agent = LiveAgent {
            profile_id,
            user_id,
            all_channels: body.all_channels.unwrap_or(false),
        };
        set_channels(
            &mut tx,
            workspace_id,
            actor.user_id,
            &agent,
            body.channel_ids.as_deref().unwrap_or_default(),
            now,
        )
        .await?;
        let integrations = checked_integrations(
            &mut tx,
            workspace_id,
            body.integration_ids.as_deref().unwrap_or_default(),
        )
        .await?;
        set_integrations(&mut tx, profile_id, &integrations).await?;
        audit::record(
            &mut tx,
            workspace_id,
            Some(actor.user_id),
            "agent_profile.create",
            AuditOutcome::Success,
            "agent_profile",
            Some(profile_id),
            &request,
            json!({ "name": name, "tools": tools.len(), "integrations": integrations.len() }),
            now,
        )
        .await?;
        tx.commit().await?;
        Ok::<_, SaveError>(profile_id)
    }
    .await
    .map_err(|error| save_failure(error, &uri, &request_id))?;
    one(&state, workspace_id, created, &uri, &request_id).await
}

async fn one(
    state: &AgentState,
    workspace_id: Id,
    profile_id: Id,
    uri: &Uri,
    request_id: &RequestIdExtension,
) -> Result<Json<Value>, ApiError> {
    let view = agent_views(state.database(), workspace_id, Some(profile_id))
        .await
        .map_err(|error| unavailable(&error, uri, request_id))?
        .into_iter()
        .next()
        .ok_or_else(|| not_found(uri, request_id))?;
    Ok(Json(json!(view)))
}

/// The live profile `agent_id` of the workspace, in a write transaction.
async fn load_live(
    conn: &mut AnyConnection,
    workspace_id: Id,
    agent_id: Id,
) -> Result<LiveAgent, SaveError> {
    let row = sqlx::query(
        "SELECT user_id, all_channels FROM agent_profiles WHERE id = $1 AND workspace_id = $2 \
         AND deleted_at IS NULL",
    )
    .bind(agent_id.to_string())
    .bind(workspace_id.to_string())
    .fetch_optional(&mut *conn)
    .await?
    .ok_or(SaveError::NotFound)?;
    Ok(LiveAgent {
        profile_id: agent_id,
        user_id: row
            .get::<String, _>("user_id")
            .parse()
            .map_err(|_| SaveError::NotFound)?,
        all_channels: row.get::<bool, _>("all_channels"),
    })
}

async fn update_agent(
    State(state): State<AgentState>,
    Path((workspace_id, agent_id)): Path<(String, String)>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
    ApiJson(body): ApiJson<AgentBody>,
) -> Result<Json<Value>, ApiError> {
    let (workspace_id, actor) = manager(&state, &headers, &workspace_id, &uri, &request_id).await?;
    let agent_id: Id = agent_id.parse().map_err(|_| not_found(&uri, &request_id))?;
    let name = body
        .name
        .as_deref()
        .map(checked_name)
        .transpose()
        .map_err(|detail| invalid(detail, &uri, &request_id))?;
    let instructions = body
        .instructions
        .as_deref()
        .map(checked_instructions)
        .transpose()
        .map_err(|detail| invalid(detail, &uri, &request_id))?;
    let tools = body
        .tools
        .as_deref()
        .map(checked_tools)
        .transpose()
        .map_err(|detail| invalid(detail, &uri, &request_id))?;
    let answer_level = checked_answer_level(body.answer_level.as_deref())
        .map_err(|detail| invalid(detail, &uri, &request_id))?;
    let (model, thinking) =
        checked_model(&state, body.model, body.thinking, &uri, &request_id).await?;
    let database = state.database();
    let request = request_id_value(request_id.as_ref()).to_owned();
    async {
        let mut tx = database.immediate_transaction().await?;
        let allowed = membership::actor(&mut *tx, workspace_id, actor.user_id)
            .await?
            .is_some_and(|actor| actor.can(Permission::WorkspaceUpdate));
        if !allowed {
            return Err(SaveError::Forbidden);
        }
        let mut agent = load_live(&mut tx, workspace_id, agent_id).await?;
        let now = TimestampMillis::now();
        if let Some(name) = &name {
            if name_taken(&mut tx, Some(agent.user_id), name).await? {
                return Err(SaveError::Taken);
            }
            sqlx::query(
                "UPDATE users SET display_name = $1, updated_at = $2, version = version + 1 WHERE id = $3",
            )
            .bind(name)
            .bind(now.as_millis())
            .bind(agent.user_id.to_string())
            .execute(&mut *tx)
            .await?;
        }
        let mut update = SqlBuilder::new("UPDATE agent_profiles SET updated_at = ");
        update.push_bind(now.as_millis());
        if let Some(instructions) = &instructions {
            update.push(", instructions = ");
            update.push_bind(instructions.clone());
        }
        if let Some(tools) = &tools {
            update.push(", tools = ");
            update.push_bind(serde_json::to_string(tools).unwrap_or_else(|_| "[]".to_owned()));
        }
        if let Some(model) = &model {
            update.push(", model = ");
            update.push_bind(model.clone());
        }
        if let Some(thinking) = &thinking {
            update.push(", thinking = ");
            update.push_bind(thinking.clone());
        }
        if let Some(level) = &answer_level {
            update.push(", answer_level = ");
            update.push_bind(level.clone());
        }
        if let Some(all) = body.all_channels {
            update.push(", all_channels = ");
            update.push_bind(i64::from(all));
            agent.all_channels = all;
        }
        update.push(" WHERE id = ");
        update.push_bind(agent_id.to_string());
        update.build().execute(&mut *tx).await?;
        if let Some(channel_ids) = &body.channel_ids {
            set_channels(&mut tx, workspace_id, actor.user_id, &agent, channel_ids, now).await?;
        }
        let integrations = match &body.integration_ids {
            Some(ids) => {
                let ids = checked_integrations(&mut tx, workspace_id, ids).await?;
                set_integrations(&mut tx, agent_id, &ids).await?;
                Some(ids.len())
            }
            None => None,
        };
        audit::record(
            &mut tx,
            workspace_id,
            Some(actor.user_id),
            "agent_profile.update",
            AuditOutcome::Success,
            "agent_profile",
            Some(agent_id),
            &request,
            json!({ "name": name, "tools": tools.as_ref().map(Vec::len), "integrations": integrations }),
            now,
        )
        .await?;
        tx.commit().await?;
        Ok::<_, SaveError>(())
    }
    .await
    .map_err(|error| save_failure(error, &uri, &request_id))?;
    one(&state, workspace_id, agent_id, &uri, &request_id).await
}

/// Deletes an agent: nobody can reach it any more, its routines pause, its waiting inputs go, and
/// its bot user is suspended and renamed "<name> (deleted)", so its messages and DMs stay readable
/// and its name is free again.
async fn delete_agent(
    State(state): State<AgentState>,
    Path((workspace_id, agent_id)): Path<(String, String)>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
) -> Result<StatusCode, ApiError> {
    let (workspace_id, actor) = manager(&state, &headers, &workspace_id, &uri, &request_id).await?;
    let agent_id: Id = agent_id.parse().map_err(|_| not_found(&uri, &request_id))?;
    let database = state.database();
    let request = request_id_value(request_id.as_ref()).to_owned();
    async {
        let mut tx = database.immediate_transaction().await?;
        let allowed = membership::actor(&mut *tx, workspace_id, actor.user_id)
            .await?
            .is_some_and(|actor| actor.can(Permission::WorkspaceUpdate));
        if !allowed {
            return Err(SaveError::Forbidden);
        }
        let agent = load_live(&mut tx, workspace_id, agent_id).await?;
        let now = TimestampMillis::now();
        sqlx::query("UPDATE agent_profiles SET deleted_at = $1, updated_at = $1 WHERE id = $2")
            .bind(now.as_millis())
            .bind(agent_id.to_string())
            .execute(&mut *tx)
            .await?;
        let name: String = sqlx::query_scalar("SELECT display_name FROM users WHERE id = $1")
            .bind(agent.user_id.to_string())
            .fetch_one(&mut *tx)
            .await?;
        sqlx::query(
            "UPDATE users SET display_name = $1, suspended_at = $2, updated_at = $2, version = version + 1 \
             WHERE id = $3",
        )
        .bind(format!("{name} (deleted)"))
        .bind(now.as_millis())
        .bind(agent.user_id.to_string())
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE agent_routines SET status = 'paused', paused_reason = $1, next_run_at = NULL, \
             pending_run_id = NULL, updated_at = $2 WHERE agent_user_id = $3",
        )
        .bind("Paused: its agent was deleted.")
        .bind(now.as_millis())
        .bind(agent.user_id.to_string())
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE agent_inputs SET acked_at = $1 WHERE agent_user_id = $2 AND acked_at IS NULL",
        )
        .bind(now.as_millis())
        .bind(agent.user_id.to_string())
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE agent_threads SET status = 'idle', waiting_for_user_id = NULL, updated_at = $1 \
             WHERE agent_user_id = $2",
        )
        .bind(now.as_millis())
        .bind(agent.user_id.to_string())
        .execute(&mut *tx)
        .await?;
        // Its sessions end now: a run that is going cannot act any more.
        sqlx::query(
            "DELETE FROM sessions WHERE id IN (SELECT session_id FROM agent_sessions WHERE agent_user_id = $1)",
        )
        .bind(agent.user_id.to_string())
        .execute(&mut *tx)
        .await?;
        audit::record(
            &mut tx,
            workspace_id,
            Some(actor.user_id),
            "agent_profile.delete",
            AuditOutcome::Success,
            "agent_profile",
            Some(agent_id),
            &request,
            json!({ "name": name }),
            now,
        )
        .await?;
        tx.commit().await?;
        Ok::<_, SaveError>(())
    }
    .await
    .map_err(|error| save_failure(error, &uri, &request_id))?;
    Ok(StatusCode::NO_CONTENT)
}

/// The bot user of a live agent of the workspace, for an owner or admin.
async fn agent_user(
    state: &AgentState,
    headers: &HeaderMap,
    workspace_id: &str,
    agent_id: &str,
    uri: &Uri,
    request_id: &RequestIdExtension,
) -> Result<Id, ApiError> {
    let (workspace_id, _) = manager(state, headers, workspace_id, uri, request_id).await?;
    let agent_id: Id = agent_id.parse().map_err(|_| not_found(uri, request_id))?;
    let mut conn = state
        .database()
        .pool()
        .acquire()
        .await
        .map_err(|error| unavailable(&error, uri, request_id))?;
    match load_live(&mut conn, workspace_id, agent_id).await {
        Ok(agent) => Ok(agent.user_id),
        Err(SaveError::Database(error)) => Err(unavailable(&error, uri, request_id)),
        Err(_) => Err(not_found(uri, request_id)),
    }
}

/// The agent's avatar: a PNG, JPEG or WebP image of at most 512 KiB in the multipart field `file`.
async fn upload_avatar(
    State(state): State<AgentState>,
    Path((workspace_id, agent_id)): Path<(String, String)>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
    multipart: Multipart,
) -> Result<StatusCode, ApiError> {
    let user = agent_user(
        &state,
        &headers,
        &workspace_id,
        &agent_id,
        &uri,
        &request_id,
    )
    .await?;
    set_avatar(&state, user, multipart, &uri, &request_id).await
}

async fn set_avatar(
    state: &AgentState,
    user: Id,
    mut multipart: Multipart,
    uri: &Uri,
    request_id: &RequestIdExtension,
) -> Result<StatusCode, ApiError> {
    let bad = || {
        problem(
            StatusCode::BAD_REQUEST,
            "invalid_multipart",
            "Send the image as the multipart field \"file\".",
            uri,
            request_id,
        )
    };
    let mut image = None;
    while let Some(field) = multipart.next_field().await.map_err(|_| bad())? {
        if field.name() == Some("file") {
            image = Some(field.bytes().await.map_err(|_| bad())?);
        }
    }
    let image = image.ok_or_else(bad)?;
    if image.len() > crate::auth_routes::MAX_AVATAR_BYTES {
        return Err(problem(
            StatusCode::PAYLOAD_TOO_LARGE,
            "avatar_too_large",
            "The image must be at most 512 KiB.",
            uri,
            request_id,
        ));
    }
    let mime_type = crate::auth_routes::avatar_mime_type(&image)
        .ok_or_else(|| invalid("Upload a PNG, JPEG or WebP image.", uri, request_id))?;
    state
        .identity
        .set_avatar_audited(
            state.chat.chat.uploads(),
            user,
            mime_type,
            &image,
            request_id_value(request_id.as_ref()),
            TimestampMillis::now(),
        )
        .await
        .map_err(|_| {
            problem(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                "The avatar could not be changed.",
                uri,
                request_id,
            )
        })?;
    Ok(StatusCode::NO_CONTENT)
}

async fn remove_avatar(
    State(state): State<AgentState>,
    Path((workspace_id, agent_id)): Path<(String, String)>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
) -> Result<StatusCode, ApiError> {
    let user = agent_user(
        &state,
        &headers,
        &workspace_id,
        &agent_id,
        &uri,
        &request_id,
    )
    .await?;
    clear_avatar(&state, user, &uri, &request_id).await
}

async fn clear_avatar(
    state: &AgentState,
    user: Id,
    uri: &Uri,
    request_id: &RequestIdExtension,
) -> Result<StatusCode, ApiError> {
    state
        .identity
        .remove_avatar_audited(
            user,
            request_id_value(request_id.as_ref()),
            TimestampMillis::now(),
        )
        .await
        .map_err(|_| {
            problem(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                "The avatar could not be changed.",
                uri,
                request_id,
            )
        })?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------------------------
// The default agent's card

async fn default_view(
    database: &Database,
    workspace_id: Id,
    can_rename: bool,
) -> Result<DefaultAgentView, sqlx::Error> {
    let mut conn = database.pool().acquire().await?;
    // A write transaction only for an installation whose bot does not exist yet.
    let user_id = match super::inputs::bot_user_id(&mut conn).await? {
        Some(id) => id,
        None => bot::ensure(database).await?,
    };
    let identity = sqlx::query(
        "SELECT u.display_name, a.updated_at AS avatar_at FROM users u \
         LEFT JOIN user_avatars a ON a.user_id = u.id WHERE u.id = $1",
    )
    .bind(user_id.to_string())
    .fetch_one(&mut *conn)
    .await?;
    let settings = sqlx::query(
        "SELECT instructions, model, thinking, answer_level FROM agent_default_profiles \
         WHERE workspace_id = $1",
    )
    .bind(workspace_id.to_string())
    .fetch_optional(&mut *conn)
    .await?;
    let access = integration_access(&mut conn, workspace_id, None).await?;
    let integration_ids = workspace_integrations(&mut conn, workspace_id)
        .await?
        .into_iter()
        .map(|integration| integration.id)
        .filter(|id| access.allows(id))
        .collect();
    Ok(DefaultAgentView {
        user_id: user_id.to_string(),
        name: identity.get("display_name"),
        avatar_url: crate::auth_routes::avatar_url(user_id, identity.get("avatar_at")),
        instructions: settings
            .as_ref()
            .map(|row| row.get("instructions"))
            .unwrap_or_default(),
        model: settings.as_ref().and_then(|row| row.get("model")),
        thinking: settings.as_ref().and_then(|row| row.get("thinking")),
        answer_level: settings.map_or_else(|| "normal".to_owned(), |row| row.get("answer_level")),
        integration_ids,
        can_rename,
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DefaultBody {
    /// Installation admins only.
    name: Option<String>,
    instructions: Option<String>,
    #[serde(default, with = "double_option")]
    model: Option<Option<String>>,
    #[serde(default, with = "double_option")]
    thinking: Option<Option<String>>,
    /// `short`, `normal` or `thorough`; absent keeps it.
    answer_level: Option<String>,
    /// The integrations it may use; the others are off for it, new ones on.
    integration_ids: Option<Vec<String>>,
}

/// Changes the default agent: its name (for the whole installation), and in this workspace its
/// instructions, model, thinking level and integrations. Absent fields stay.
async fn update_default(
    State(state): State<AgentState>,
    Path(workspace_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
    ApiJson(body): ApiJson<DefaultBody>,
) -> Result<Json<Value>, ApiError> {
    let (workspace_id, actor) = manager(&state, &headers, &workspace_id, &uri, &request_id).await?;
    let can_rename = state
        .identity
        .is_installation_admin(actor.user_id)
        .await
        .unwrap_or(false);
    let name = body
        .name
        .as_deref()
        .map(checked_name)
        .transpose()
        .map_err(|detail| invalid(detail, &uri, &request_id))?;
    let instructions = body
        .instructions
        .as_deref()
        .map(checked_instructions)
        .transpose()
        .map_err(|detail| invalid(detail, &uri, &request_id))?;
    let answer_level = checked_answer_level(body.answer_level.as_deref())
        .map_err(|detail| invalid(detail, &uri, &request_id))?;
    let (model, thinking) =
        checked_model(&state, body.model, body.thinking, &uri, &request_id).await?;
    let database = state.database();
    let bot = bot::ensure(database)
        .await
        .map_err(|error| unavailable(&error, &uri, &request_id))?;
    let request = request_id_value(request_id.as_ref()).to_owned();
    async {
        let mut tx = database.immediate_transaction().await?;
        let allowed = membership::actor(&mut *tx, workspace_id, actor.user_id)
            .await?
            .is_some_and(|actor| actor.can(Permission::WorkspaceUpdate));
        if !allowed {
            return Err(SaveError::Forbidden);
        }
        let now = TimestampMillis::now();
        if let Some(name) = &name {
            if !can_rename {
                return Err(SaveError::NotInstallationAdmin);
            }
            if name_taken(&mut tx, Some(bot), name).await? {
                return Err(SaveError::Taken);
            }
            sqlx::query(
                "UPDATE users SET display_name = $1, updated_at = $2, version = version + 1 WHERE id = $3",
            )
            .bind(name)
            .bind(now.as_millis())
            .bind(bot.to_string())
            .execute(&mut *tx)
            .await?;
            audit::record_global(
                &mut tx,
                Some(actor.user_id),
                "agent.renamed",
                AuditOutcome::Success,
                "user",
                Some(bot),
                &request,
                json!({ "name": name }),
                now,
            )
            .await?;
        }
        sqlx::query(
            "INSERT INTO agent_default_profiles (workspace_id, updated_by, updated_at) VALUES ($1, $2, $3) \
             ON CONFLICT (workspace_id) DO UPDATE SET updated_by = excluded.updated_by, updated_at = excluded.updated_at",
        )
        .bind(workspace_id.to_string())
        .bind(actor.user_id.to_string())
        .bind(now.as_millis())
        .execute(&mut *tx)
        .await?;
        let mut update = SqlBuilder::new("UPDATE agent_default_profiles SET updated_at = ");
        update.push_bind(now.as_millis());
        if let Some(instructions) = &instructions {
            update.push(", instructions = ");
            update.push_bind(instructions.clone());
        }
        if let Some(model) = &model {
            update.push(", model = ");
            update.push_bind(model.clone());
        }
        if let Some(thinking) = &thinking {
            update.push(", thinking = ");
            update.push_bind(thinking.clone());
        }
        if let Some(level) = &answer_level {
            update.push(", answer_level = ");
            update.push_bind(level.clone());
        }
        update.push(" WHERE workspace_id = ");
        update.push_bind(workspace_id.to_string());
        update.build().execute(&mut *tx).await?;
        let mut off = None;
        if let Some(ids) = &body.integration_ids {
            let on = checked_integrations(&mut tx, workspace_id, ids).await?;
            sqlx::query(
                "DELETE FROM agent_default_integrations_off WHERE integration_id IN \
                 (SELECT id FROM agent_integrations WHERE workspace_id = $1)",
            )
            .bind(workspace_id.to_string())
            .execute(&mut *tx)
            .await?;
            let ids: Vec<String> = workspace_integrations(&mut tx, workspace_id)
                .await?
                .into_iter()
                .map(|integration| integration.id)
                .filter(|id| !on.contains(id))
                .collect();
            for id in &ids {
                sqlx::query("INSERT INTO agent_default_integrations_off (integration_id) VALUES ($1)")
                    .bind(id)
                    .execute(&mut *tx)
                    .await?;
            }
            off = Some(ids.len());
        }
        audit::record(
            &mut tx,
            workspace_id,
            Some(actor.user_id),
            "agent_default.update",
            AuditOutcome::Success,
            "workspace",
            Some(workspace_id),
            &request,
            json!({
                "name": name,
                "instructions": instructions.as_ref().map(|text| text.chars().count()),
                "integrations_off": off,
            }),
            now,
        )
        .await?;
        tx.commit().await?;
        Ok::<_, SaveError>(())
    }
    .await
    .map_err(|error| save_failure(error, &uri, &request_id))?;
    let view = default_view(database, workspace_id, can_rename)
        .await
        .map_err(|error| unavailable(&error, &uri, &request_id))?;
    Ok(Json(json!(view)))
}

/// The default agent's bot user, for an owner or admin of the workspace who is an installation
/// admin: its avatar shows in every workspace.
async fn default_user(
    state: &AgentState,
    headers: &HeaderMap,
    workspace_id: &str,
    uri: &Uri,
    request_id: &RequestIdExtension,
) -> Result<Id, ApiError> {
    let (_, actor) = manager(state, headers, workspace_id, uri, request_id).await?;
    let admin = state
        .identity
        .is_installation_admin(actor.user_id)
        .await
        .unwrap_or(false);
    if !admin {
        return Err(save_failure(
            SaveError::NotInstallationAdmin,
            uri,
            request_id,
        ));
    }
    bot::ensure(state.database())
        .await
        .map_err(|error| unavailable(&error, uri, request_id))
}

async fn upload_default_avatar(
    State(state): State<AgentState>,
    Path(workspace_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
    multipart: Multipart,
) -> Result<StatusCode, ApiError> {
    let user = default_user(&state, &headers, &workspace_id, &uri, &request_id).await?;
    set_avatar(&state, user, multipart, &uri, &request_id).await
}

async fn remove_default_avatar(
    State(state): State<AgentState>,
    Path(workspace_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
) -> Result<StatusCode, ApiError> {
    let user = default_user(&state, &headers, &workspace_id, &uri, &request_id).await?;
    clear_avatar(&state, user, &uri, &request_id).await
}

#[cfg(test)]
mod tests {
    use axum::http::Method;

    use super::{IntegrationAccess, checked_tools, drive_tool_allowed, tool_catalog, tools_allow};

    fn tools(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn tools_are_known_once_in_catalog_order_and_notes_bring_summaries() {
        assert_eq!(
            checked_tools(&tools(&[
                "memory_note",
                "create_page",
                "read_page",
                "read_page"
            ])),
            Ok(tools(&[
                "read_page",
                "create_page",
                "memory_note",
                "memory_summarize"
            ]))
        );
        assert!(checked_tools(&tools(&["delete_everything"])).is_err());
        assert!(checked_tools(&tools(&[""])).is_err());
        assert_eq!(checked_tools(&[]), Ok(Vec::new()));
        assert_eq!(tool_catalog().count(), 59);
    }

    #[test]
    fn routes_open_only_for_the_tools_that_need_them() {
        let writer = tools(&["read_page", "create_page", "update_page"]);
        for (method, rest) in [
            (Method::GET, ""),
            (Method::GET, "members"),
            (Method::GET, "pages"),
            (Method::GET, "pages/abc"),
            (Method::POST, "pages"),
            (Method::PATCH, "pages/abc"),
            // Its skills: the skill routes check the agent themselves.
            (Method::GET, "agent-tools/skills"),
            (Method::GET, "agent-tools/skills/pdf-forms/files/a.md"),
        ] {
            assert!(tools_allow(&writer, &method, rest), "{method} {rest}");
        }
        for (method, rest) in [
            (Method::GET, "tasks"),
            (Method::POST, "tasks"),
            (Method::GET, "projects"),
            (Method::PUT, "pages/abc/files/def/content"),
            (Method::GET, "chat/threads/abc"),
            (Method::GET, "chat/conversations"),
            (Method::POST, "agent-memory/team"),
            (Method::GET, "agent-tools"),
            (Method::POST, "agent-tools/call"),
            (Method::POST, "agent-tools/drive"),
            (Method::POST, "agent-tools/scheduled"),
            (Method::POST, "agent-tools/shopify-image"),
            (Method::POST, "agent-tools/skills"),
            (Method::POST, "routines"),
            (Method::POST, "scheduled-messages"),
            (Method::DELETE, "pages/abc"),
            (Method::GET, "something-new"),
        ] {
            assert!(!tools_allow(&writer, &method, rest), "{method} {rest}");
        }
        // Drive files on pages: each write needs its own tool.
        let files = tools(&["list_page_drive_files", "insert_drive_file"]);
        assert!(tools_allow(&files, &Method::GET, "pages/abc/drive-files"));
        assert!(tools_allow(&files, &Method::GET, "pages/drive-file-uses/n"));
        assert!(tools_allow(&files, &Method::POST, "pages/abc/drive-files"));
        assert!(!tools_allow(
            &files,
            &Method::PUT,
            "pages/abc/drive-files/b"
        ));
        assert!(!tools_allow(
            &files,
            &Method::DELETE,
            "pages/abc/drive-files/b"
        ));
        assert!(!tools_allow(
            &files,
            &Method::POST,
            "pages/abc/drive-files/resolve"
        ));
        assert!(!tools_allow(
            &writer,
            &Method::POST,
            "pages/abc/drive-files"
        ));
        let tasks = tools(&["get_task", "comment_on_task"]);
        assert!(tools_allow(&tasks, &Method::GET, "tasks/abc"));
        assert!(tools_allow(&tasks, &Method::POST, "tasks/abc/comments"));
        assert!(!tools_allow(&tasks, &Method::PATCH, "tasks/abc"));
        assert!(!tools_allow(&tasks, &Method::POST, "tasks"));
        assert!(!tools_allow(
            &tasks,
            &Method::POST,
            "projects/abc/label-rules"
        ));
        // Doc links: reading them comes with the task and page tools, changing them needs its own tool.
        assert!(tools_allow(&tasks, &Method::GET, "tasks/abc/linked-docs"));
        assert!(!tools_allow(&tasks, &Method::POST, "tasks/abc/linked-docs"));
        let linker = tools(&["link_doc_to_task"]);
        assert!(tools_allow(&linker, &Method::POST, "tasks/abc/linked-docs"));
        assert!(!tools_allow(
            &linker,
            &Method::DELETE,
            "tasks/abc/linked-docs/p"
        ));
        let unlinker = tools(&["unlink_doc_from_task"]);
        assert!(tools_allow(
            &unlinker,
            &Method::DELETE,
            "tasks/abc/linked-docs/p"
        ));
        assert!(tools_allow(&writer, &Method::GET, "pages/p/linked-tasks"));
        // Label rules: reading them needs a task tool, making one needs its own.
        let rules = tools(&["create_label_rule"]);
        assert!(tools_allow(
            &rules,
            &Method::GET,
            "projects/abc/label-rules"
        ));
        assert!(tools_allow(
            &rules,
            &Method::POST,
            "projects/abc/label-rules"
        ));
        assert!(!tools_allow(
            &rules,
            &Method::PATCH,
            "projects/abc/label-rules/r"
        ));
        assert!(!tools_allow(
            &rules,
            &Method::DELETE,
            "projects/abc/label-rules/r"
        ));
        assert!(!tools_allow(
            &rules,
            &Method::PUT,
            "projects/abc/label-rule-settings"
        ));
        // Routines and send later are made only through `/agent-tools/scheduled`.
        let scheduler = tools(&["send_later"]);
        assert!(tools_allow(
            &scheduler,
            &Method::POST,
            "agent-tools/scheduled"
        ));
        assert!(tools_allow(&scheduler, &Method::GET, "scheduled-messages"));
        assert!(!tools_allow(
            &scheduler,
            &Method::POST,
            "scheduled-messages"
        ));
        assert!(!tools_allow(&scheduler, &Method::GET, "routines"));
        // A Drive image to Shopify has its own route, apart from use_integration.
        let shopify = tools(&["drive_image_to_shopify"]);
        assert!(tools_allow(
            &shopify,
            &Method::POST,
            "agent-tools/shopify-image"
        ));
        assert!(!tools_allow(&shopify, &Method::POST, "agent-tools/call"));
        // Nothing at all: only the workspace and its members.
        assert!(tools_allow(&[], &Method::GET, "members"));
        assert!(!tools_allow(&[], &Method::GET, "pages"));
    }

    #[test]
    fn new_integrations_are_on_for_the_default_agent_and_off_for_custom_agents() {
        let default = IntegrationAccess::AllBut(tools(&["tiktok"]));
        assert!(!default.allows("tiktok"));
        assert!(default.allows("shopify"));
        assert!(default.allows("added-later"));
        let custom = IntegrationAccess::Only(tools(&["shopify"]));
        assert!(custom.allows("shopify"));
        assert!(!custom.allows("tiktok"));
        assert!(!custom.allows("added-later"));
    }

    #[test]
    fn drive_tools_are_checked_one_by_one() {
        let reader = tools(&["drive_list", "drive_read"]);
        assert!(drive_tool_allowed(&reader, "drive_list"));
        assert!(drive_tool_allowed(&reader, "drive_read"));
        assert!(!drive_tool_allowed(&reader, "drive_trash"));
        assert!(!drive_tool_allowed(&reader, "drive_upload_text"));
        assert!(!drive_tool_allowed(&reader, "drive_fetch"));
        let media = tools(&["view_image", "drive_save_text"]);
        assert!(drive_tool_allowed(&media, "drive_fetch"));
        assert!(drive_tool_allowed(&media, "drive_upload_text"));
        assert!(!drive_tool_allowed(&media, "drive_upload_begin"));
    }
}
