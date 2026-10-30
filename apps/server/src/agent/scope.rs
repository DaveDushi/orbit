//! Agent sessions act as one person, but only on the work of one workspace. A request that carries
//! an agent token (`Authorization: Bearer orbit-agent.…`) passes this layer only for routes of
//! that workspace that are not about accounts, access or administration. Everything after the
//! layer is the normal session path, so every permission rule of the person still applies.

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::Response;
use orbit_platform::{Database, Problem, RequestId};
use sha2::{Digest, Sha256};

/// Every agent session token starts with this. Normal session tokens are base64url and never
/// contain a `.`.
pub const TOKEN_PREFIX: &str = "orbit-agent.";

/// Workspace route segments an agent session never reaches: credentials, integrations, access,
/// data export, administration, the agent's own team instructions and its integrations.
const DENIED: [&str; 11] = [
    "agent",
    "agent-integrations",
    "api-tokens",
    "audit",
    "github",
    "imports",
    "invitations",
    "live",
    "restore",
    "storage",
    "transfer-ownership",
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
    // The agent notes and reads memory; only people delete it.
    if segment == "agent-memory" {
        return method != Method::DELETE;
    }
    true
}

/// The layer: checks agent tokens, leaves every other request alone.
pub async fn layer(State(database): State<Database>, request: Request, next: Next) -> Response {
    let Some(token) = orbit_platform::bearer_token(request.headers())
        .filter(|token| token.starts_with(TOKEN_PREFIX))
        .map(str::to_owned)
    else {
        return next.run(request).await;
    };
    let hash: [u8; 32] = Sha256::digest(token.as_bytes()).into();
    let workspace_id = sqlx::query_scalar::<_, String>(
        "SELECT agent_sessions.workspace_id FROM sessions \
         JOIN agent_sessions ON agent_sessions.session_id = sessions.id \
         WHERE sessions.token_hash = $1 AND sessions.revoked_at IS NULL",
    )
    .bind(hash.to_vec())
    .fetch_optional(database.pool())
    .await;
    match workspace_id {
        Ok(Some(workspace_id)) if allows(&workspace_id, request.method(), request.uri().path()) => {
            next.run(request).await
        }
        Ok(Some(_)) => refuse(
            &request,
            StatusCode::FORBIDDEN,
            "agent_scope",
            "The agent cannot use this route.",
        ),
        Ok(None) => refuse(
            &request,
            StatusCode::UNAUTHORIZED,
            "authentication_required",
            "The agent session is not valid.",
        ),
        Err(error) => {
            tracing::warn!(%error, "agent session scope");
            refuse(
                &request,
                StatusCode::SERVICE_UNAVAILABLE,
                "unavailable",
                "Try again in a moment.",
            )
        }
    }
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
            (Method::POST, "/tasks/abc/comments"),
            (Method::GET, "/chat/threads/abc"),
            (Method::GET, "/members"),
            (Method::GET, ""),
            (Method::POST, "/agent-memory/team"),
            (Method::GET, "/agent-memory/private/wake"),
            (Method::GET, "/agent-tools"),
            (Method::POST, "/agent-tools/call"),
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
            (Method::PATCH, ""),
            (Method::DELETE, ""),
            (Method::GET, "/agent"),
            (Method::PUT, "/agent"),
            (Method::DELETE, "/agent-memory/team/memories/3"),
            (Method::DELETE, "/agent-memory/private/summaries/0-1"),
            (Method::GET, "/agent-integrations"),
            (Method::POST, "/agent-integrations/abc/test"),
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
