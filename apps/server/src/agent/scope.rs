//! Agent sessions act as one person, but only on the work of one workspace. A request that carries
//! an agent token (`Authorization: Bearer orbit-agent.…`) passes this layer only for routes of
//! that workspace that are not about accounts, access or administration. Everything after the
//! layer is the normal session path, so every permission rule of the person still applies.

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::Response;
use orbit_platform::{Database, Problem, RequestId, Row};
use sha2::{Digest, Sha256};

/// Every agent session token starts with this. Normal session tokens are base64url and never
/// contain a `.`.
pub const TOKEN_PREFIX: &str = "orbit-agent.";

/// Workspace route segments an agent session never reaches: credentials, integrations, access,
/// data export, administration, the agent's own team instructions and its integrations. Drive too:
/// the agent uses it only through `/agent-tools/drive`, which asks the driver before it moves
/// anything to the trash or makes a public link (`super::drive`).
const DENIED: [&str; 16] = [
    "agent",
    "agent-integrations",
    "agents",
    "api-tokens",
    "apps",
    "audit",
    "drive",
    "exports",
    "github",
    "imports",
    "invitations",
    "live",
    "restore",
    "storage",
    "transfer-ownership",
    "workspace-apps",
];

/// Whether an agent session of `workspace_id` may make this request.
#[must_use]
pub fn allows(workspace_id: &str, method: &Method, path: &str) -> bool {
    let Some(rest) = path
        .strip_prefix("/api/v1/workspaces/")
        .and_then(|rest| rest.strip_prefix(workspace_id))
    else {
        return false;
    };
    let rest = rest.trim_end_matches('/');
    if rest.is_empty() {
        // The workspace itself: read it, never rename or delete it.
        return method == Method::GET;
    }
    let Some(rest) = rest.strip_prefix('/') else {
        // Another workspace whose id starts with this one.
        return false;
    };
    let segment = rest.split('/').next().unwrap_or_default();
    if DENIED.contains(&segment) {
        return false;
    }
    // The member list is readable; invitations, roles and removals are not for the agent.
    if segment == "members" {
        return method == Method::GET;
    }
    // The agent lists routines, reminders and scheduled messages here, and makes them only through
    // `/agent-tools/scheduled`, which asks the driver before anything posts as them
    // (`crate::scheduled`); only people change, pause or delete them (Settings, the composer).
    if segment == "routines" || segment == "scheduled-messages" {
        return method == Method::GET;
    }
    // The agent notes and reads memory; only people delete it.
    if segment == "agent-memory" {
        return method != Method::DELETE;
    }
    // Chat files: the agent reads their content, and never uploads or downloads them.
    if let Some(file) = rest.strip_prefix("chat/files") {
        return method == Method::GET
            && file
                .strip_prefix('/')
                .and_then(|file| file.strip_suffix("/content"))
                .is_some_and(|id| !id.is_empty() && !id.contains('/'));
    }
    true
}

/// Whether a custom agent with these tools may make this request (after [`allows`]): only the
/// routes its tools need (`profiles::route_need`).
#[must_use]
pub fn custom_allows(workspace_id: &str, tools: &[String], method: &Method, path: &str) -> bool {
    let rest = path
        .strip_prefix("/api/v1/workspaces/")
        .and_then(|rest| rest.strip_prefix(workspace_id))
        .unwrap_or_default()
        .trim_end_matches('/');
    let rest = rest.strip_prefix('/').unwrap_or(rest);
    super::profiles::tools_allow(tools, method, rest)
}

/// The layer: checks agent tokens, leaves every other request alone. A custom agent's session
/// also passes only for the routes of its tools, and carries them for the Drive route.
pub async fn layer(State(database): State<Database>, mut request: Request, next: Next) -> Response {
    let Some(token) = orbit_platform::bearer_token(request.headers())
        .filter(|token| token.starts_with(TOKEN_PREFIX))
        .map(str::to_owned)
    else {
        return next.run(request).await;
    };
    let hash: [u8; 32] = Sha256::digest(token.as_bytes()).into();
    let row = sqlx::query(
        "SELECT agent_sessions.workspace_id, agent_sessions.agent_user_id, p.tools, p.deleted_at \
         FROM sessions JOIN agent_sessions ON agent_sessions.session_id = sessions.id \
         LEFT JOIN agent_profiles p ON p.user_id = agent_sessions.agent_user_id \
         WHERE sessions.token_hash = $1 AND sessions.revoked_at IS NULL",
    )
    .bind(hash.to_vec())
    .fetch_optional(database.pool())
    .await;
    let row = match row {
        Ok(Some(row)) => row,
        Ok(None) => {
            return refuse(
                &request,
                StatusCode::UNAUTHORIZED,
                "authentication_required",
                "The agent session is not valid.",
            );
        }
        Err(error) => {
            tracing::warn!(%error, "agent session scope");
            return refuse(
                &request,
                StatusCode::SERVICE_UNAVAILABLE,
                "unavailable",
                "Try again in a moment.",
            );
        }
    };
    let workspace_id: String = row.get("workspace_id");
    // A custom agent: its tools, or none at all once it is deleted.
    let custom = row.get::<Option<String>, _>("agent_user_id").map(|_| {
        match row.get::<Option<i64>, _>("deleted_at") {
            None => row
                .get::<Option<String>, _>("tools")
                .and_then(|tools| serde_json::from_str::<Vec<String>>(&tools).ok())
                .unwrap_or_default(),
            Some(_) => Vec::new(),
        }
    });
    let path = request.uri().path().to_owned();
    let method = request.method().clone();
    let allowed = allows(&workspace_id, &method, &path)
        && custom
            .as_ref()
            .is_none_or(|tools| custom_allows(&workspace_id, tools, &method, &path));
    if !allowed {
        return refuse(
            &request,
            StatusCode::FORBIDDEN,
            "agent_scope",
            if custom.is_some() {
                "This agent's tools do not include this."
            } else {
                "The agent cannot use this route."
            },
        );
    }
    if let Some(tools) = custom {
        request
            .extensions_mut()
            .insert(super::profiles::AgentTools(std::sync::Arc::new(tools)));
    }
    next.run(request).await
}

fn refuse(request: &Request, status: StatusCode, code: &str, detail: &str) -> Response {
    let request_id = request
        .extensions()
        .get::<RequestId>()
        .cloned()
        .unwrap_or_else(RequestId::new);
    let problem = Problem {
        type_uri: format!("https://docs.orbit.dev/problems/{code}"),
        title: status.canonical_reason().unwrap_or("Error").to_owned(),
        status: status.as_u16(),
        code: code.to_owned(),
        detail: detail.to_owned(),
        instance: request.uri().path().to_owned(),
        request_id: request_id.to_string(),
        errors: None,
        conflict: None,
    };
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/problem+json")
        .body(Body::from(serde_json::to_vec(&problem).unwrap_or_default()))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use axum::http::Method;

    use super::allows;

    const WORKSPACE: &str = "0199c3a0-0000-7000-8000-000000000001";

    fn path(rest: &str) -> String {
        format!("/api/v1/workspaces/{WORKSPACE}{rest}")
    }

    #[test]
    fn work_routes_of_its_workspace_are_allowed() {
        for (method, rest) in [
            (Method::GET, "/tasks"),
            (Method::POST, "/tasks"),
            (Method::PATCH, "/tasks/abc"),
            (Method::GET, "/pages/search"),
            (Method::PATCH, "/pages/abc"),
            (Method::GET, "/pages/abc/files"),
            (Method::GET, "/pages/abc/files/def/content"),
            (Method::PUT, "/pages/abc/files/def/content"),
            (Method::POST, "/tasks/abc/comments"),
            (Method::GET, "/chat/threads/abc"),
            (Method::GET, "/chat/files/abc/content"),
            (Method::GET, "/chat/conversations/abc/files"),
            (Method::GET, "/members"),
            (Method::GET, ""),
            (Method::POST, "/agent-memory/team"),
            (Method::GET, "/agent-memory/private/wake"),
            (Method::GET, "/agent-tools"),
            (Method::POST, "/agent-tools/call"),
            (Method::POST, "/agent-tools/drive"),
            (Method::POST, "/agent-tools/scheduled"),
            (Method::GET, "/routines"),
            (Method::GET, "/scheduled-messages"),
        ] {
            assert!(allows(WORKSPACE, &method, &path(rest)), "{method} {rest}");
        }
    }

    #[test]
    fn account_access_and_admin_routes_are_refused() {
        for (method, rest) in [
            (Method::POST, "/api-tokens"),
            (Method::GET, "/api-tokens"),
            (Method::GET, "/audit"),
            (Method::POST, "/invitations"),
            (Method::PATCH, "/members/abc"),
            (Method::DELETE, "/members/abc"),
            (Method::POST, "/transfer-ownership"),
            (Method::PUT, "/storage/bucket"),
            (Method::POST, "/imports/notion"),
            (Method::POST, "/exports"),
            (Method::GET, "/exports/abc/download"),
            (Method::PATCH, ""),
            (Method::DELETE, ""),
            (Method::GET, "/agent"),
            (Method::PUT, "/agent"),
            (Method::DELETE, "/agent-memory/team/memories/3"),
            (Method::DELETE, "/agent-memory/private/summaries/0-1"),
            (Method::GET, "/agent-integrations"),
            (Method::POST, "/agent-integrations/abc/test"),
            (Method::GET, "/apps"),
            (Method::GET, "/apps/demo/frame"),
            (Method::GET, "/workspace-apps"),
            (Method::POST, "/workspace-apps/abc/rotate-secret"),
            (Method::GET, "/agents"),
            (Method::POST, "/agents"),
            (Method::PATCH, "/agents/abc"),
            (Method::GET, "/chat/files/abc"),
            (Method::POST, "/chat/files"),
            (Method::GET, "/chat/files"),
            (Method::PUT, "/chat/files/abc/content"),
            (Method::GET, "/chat/files//content"),
            (Method::GET, "/chat/files/abc/def/content"),
            (Method::GET, "/chat/filesabc/content"),
            (Method::GET, "/drive/nodes"),
            (Method::POST, "/drive/nodes/abc/trash"),
            (Method::POST, "/drive/nodes/abc/shares"),
            (Method::DELETE, "/drive/nodes/abc"),
            (Method::POST, "/drive/trash/empty"),
            (Method::POST, "/routines"),
            (Method::POST, "/scheduled-messages"),
            (Method::PATCH, "/routines/abc"),
            (Method::DELETE, "/routines/abc"),
            (Method::POST, "/routines/abc"),
            (Method::PATCH, "/scheduled-messages/abc"),
            (Method::DELETE, "/scheduled-messages/abc"),
        ] {
            assert!(!allows(WORKSPACE, &method, &path(rest)), "{method} {rest}");
        }
        for other in [
            "/api/v1/auth/me",
            "/api/v1/auth/password",
            "/api/v1/admin/users",
            "/api/v1/admin/agent",
            "/api/v1/admin/agent/model",
            "/api/v1/workspaces",
            "/api/v1/workspaces/0199c3a0-0000-7000-8000-000000000002/tasks",
            "/api/v1/workspaces/0199c3a0-0000-7000-8000-0000000000012/tasks",
            "/api/internal/agent/inputs",
            "/mcp",
        ] {
            assert!(!allows(WORKSPACE, &Method::GET, other), "{other}");
        }
    }
}
