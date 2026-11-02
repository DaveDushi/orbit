//! Custom web apps of a workspace: one place to open them, with one login. Orbit does not host
//! them. Owners and admins set them up (`/workspace-apps`); members with an allowed role list them
//! and open a same-origin wrapper document (`/apps/{slug}/frame`) that has its own CSP, so app
//! addresses never enter the global policy. Every other member gets a 404. `oidc` is "Sign in with
//! Orbit". Design and grilling: `.ai/apps-design.md`. Like the agent's integration routes, these
//! are not in the OpenAPI contract.

pub mod oidc;

use std::sync::Arc;

use axum::extract::{Extension, Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use orbit_domain::{Actor, Permission, WorkspaceRole};
use orbit_platform::{Database, Id, RequestId, Row, TimestampMillis, generate_opaque_token};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{Any, Transaction};
use url::{Host, Url};

use crate::audit::{self, AuditOutcome};
use crate::auth_routes::CookieMode;
use crate::repositories::identity::IdentityRepository;
use crate::repositories::membership;
use crate::task_routes::{ApiError, authenticate_session};

type RequestIdExtension = Option<Extension<RequestId>>;

const ROLES: [&str; 3] = ["owner", "admin", "member"];
const ICONS: [&str; 8] = [
    "category", "code", "chart", "cloud", "book", "driver", "cpu", "link",
];
const MAX_APPS: i64 = 100;
const MAX_REDIRECT_URIS: usize = 10;
/// The client id of every app starts with this; `/oauth/register` only makes `orbit-client-…`.
pub(crate) const CLIENT_PREFIX: &str = "orbit-app-";
/// What the app's frame may do. `allow-same-origin` is safe only because the app's origin is never
/// Orbit's: the URL rules refuse that, and the frame route checks it again. The app's sign-in tabs
/// are normal tabs (`allow-popups-to-escape-sandbox`): a sandboxed tab cannot load a page with
/// `Cross-Origin-Opener-Policy`, which Orbit's own sign-in pages send.
const SANDBOX: &str = "allow-scripts allow-same-origin allow-forms allow-popups allow-popups-to-escape-sandbox allow-downloads";

#[derive(Clone)]
pub struct AppsState {
    pub(crate) database: Database,
    pub(crate) identity: Arc<IdentityRepository>,
    pub(crate) cookie_mode: CookieMode,
    pub(crate) origin: String,
    pub(crate) app_key: Option<[u8; 32]>,
    allow_http: bool,
}

impl AppsState {
    /// `allow_http` is for a development server only: production apps must use HTTPS.
    pub fn new(
        identity: Arc<IdentityRepository>,
        cookie_mode: CookieMode,
        origin: &str,
        app_key: Option<[u8; 32]>,
        allow_http: bool,
    ) -> Self {
        Self {
            database: identity.database().clone(),
            identity,
            cookie_mode,
            origin: origin.trim_end_matches('/').to_owned(),
            app_key,
            allow_http,
        }
    }
}

pub fn router(state: AppsState) -> Router {
    let admin = "/api/v1/workspaces/{workspace_id}/workspace-apps";
    let apps = "/api/v1/workspaces/{workspace_id}/apps";
    Router::new()
        .route(apps, get(visible))
        .route(&format!("{apps}/{{slug}}/frame"), get(frame))
        .route(admin, get(list).post(create))
        .route(&format!("{admin}/{{app_id}}"), patch(update).delete(remove))
        .route(
            &format!("{admin}/{{app_id}}/rotate-secret"),
            post(rotate_secret),
        )
        .with_state(state.clone())
        .merge(oidc::router(state))
}

/// Whether a query string or form body names a client of a workspace app.
pub(crate) fn is_app_client(encoded: &str) -> bool {
    url::form_urlencoded::parse(encoded.as_bytes())
        .any(|(name, value)| name == "client_id" && value.starts_with(CLIENT_PREFIX))
}

// ---------------------------------------------------------------------------------------------
// Errors.

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

fn not_found(uri: &Uri, request_id: &RequestIdExtension) -> ApiError {
    problem(
        StatusCode::NOT_FOUND,
        "not_found",
        "Not found.",
        uri,
        request_id,
    )
}

fn unavailable(uri: &Uri, request_id: &RequestIdExtension) -> impl Fn(sqlx::Error) -> ApiError {
    let uri = uri.clone();
    let request_id = request_id.clone();
    move |error| {
        tracing::warn!(%error, "workspace apps");
        problem(
            StatusCode::SERVICE_UNAVAILABLE,
            "unavailable",
            "Try again in a moment.",
            &uri,
            &request_id,
        )
    }
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

// ---------------------------------------------------------------------------------------------
// URL rules.

/// A host that is safe to write into a CSP header and an HTML attribute. The URL parser lets `;`,
/// `,` and `'` through in a host, and `;` would start a new CSP directive.
fn plain_host(url: &Url) -> bool {
    match url.host() {
        Some(Host::Domain(domain)) => {
            !domain.is_empty()
                && domain
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
        }
        Some(_) => true,
        None => false,
    }
}

fn same_origin(url: &Url, origin: &str) -> bool {
    Url::parse(origin).is_ok_and(|orbit| orbit.origin() == url.origin())
}

/// Checks an address an app is opened at, and returns it normalized.
pub fn check_app_url(raw: &str, orbit_origin: &str, allow_http: bool) -> Result<Url, &'static str> {
    let url = Url::parse(raw.trim()).map_err(|_| "Enter an absolute URL, with https://.")?;
    if raw.len() > 2048 {
        return Err("The URL is too long.");
    }
    if !(url.scheme() == "https" || (allow_http && url.scheme() == "http")) {
        return Err("The URL must start with https://.");
    }
    if !plain_host(&url) {
        return Err("The URL has no valid host name.");
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("The URL must not contain a user name or a password.");
    }
    if same_origin(&url, orbit_origin) {
        return Err("An app cannot run on Orbit's own address. Give it its own host name.");
    }
    Ok(url)
}

/// Checks a redirect URI of an app's sign-in client. It is compared exactly at sign-in.
pub fn check_redirect_uri(
    raw: &str,
    orbit_origin: &str,
    allow_http: bool,
) -> Result<(), &'static str> {
    let url = check_app_url(raw, orbit_origin, allow_http).map_err(
        |_| "Each redirect URI must be an https:// address that is not on Orbit's own origin.",
    )?;
    if url.as_str() != raw {
        return Err(
            "Write each redirect URI in its normal form, with a path (for example https://app.example.com/callback).",
        );
    }
    if url.fragment().is_some()
        || url
            .query_pairs()
            .any(|(name, _)| matches!(name.as_ref(), "code" | "state" | "iss" | "error"))
    {
        return Err("A redirect URI has no fragment and no code, state, iss or error parameter.");
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// The wrapper document.

/// Escapes text for HTML content and double-quoted attributes.
fn escape_html(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            other => escaped.push(other),
        }
    }
    escaped
}

/// The CSP of the wrapper document. With sign-in on, the frame also passes through Orbit's own
/// `/oauth/authorize` (a redirect, or the framed `login_required` page with `frame-ancestors 'self'`;
/// every other Orbit answer has `frame-ancestors 'none'`) and
/// through the origins of the redirect URIs. `None` when an address is not safe to write.
pub fn frame_csp(app: &Url, redirect_uris: &[String], sign_in: bool) -> Option<String> {
    let mut sources = vec![app.origin().ascii_serialization()];
    if !plain_host(app) {
        return None;
    }
    if sign_in {
        sources.push("'self'".to_owned());
        for redirect in redirect_uris {
            let redirect = Url::parse(redirect).ok().filter(plain_host)?;
            let origin = redirect.origin().ascii_serialization();
            if !sources.contains(&origin) {
                sources.push(origin);
            }
        }
    }
    let sources = sources.join(" ");
    Some(format!(
        "default-src 'none'; style-src 'unsafe-inline'; frame-src {sources}; frame-ancestors 'self'; base-uri 'none'; form-action 'none'"
    ))
}

/// The script-free document with one full-size frame of the app. The frame has `data-src`, not
/// `src`: the Orbit page (same origin) first listens to the frame's `load` and to the
/// `securitypolicyviolation` of a refused navigation, then sets `src` (`appReady.ts`).
pub fn frame_html(name: &str, app: &Url) -> String {
    let name = escape_html(name);
    let src = escape_html(app.as_str());
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>{name}</title>\
         <style>html,body{{margin:0;height:100%}}iframe{{display:block;border:0;width:100%;height:100%}}</style></head>\
         <body><iframe data-src=\"{src}\" title=\"{name}\" sandbox=\"{SANDBOX}\" referrerpolicy=\"strict-origin\"></iframe></body></html>"
    )
}

// ---------------------------------------------------------------------------------------------
// Storage.

#[derive(Clone)]
pub(crate) struct WorkspaceApp {
    pub(crate) id: String,
    pub(crate) workspace_id: String,
    pub(crate) slug: String,
    pub(crate) name: String,
    pub(crate) icon: String,
    pub(crate) url: String,
    pub(crate) allowed_roles: Vec<String>,
    pub(crate) open_mode: String,
    pub(crate) client_id: Option<String>,
    pub(crate) secret_hash: Option<Vec<u8>>,
    pub(crate) redirect_uris: Vec<String>,
    pub(crate) created_at: i64,
    pub(crate) updated_at: i64,
}

pub(crate) const COLUMNS: &str = "id, workspace_id, slug, name, icon, url, allowed_roles, open_mode, \
     oidc_client_id, oidc_secret_hash, oidc_redirect_uris, created_at, updated_at";

fn strings(json: &str) -> Vec<String> {
    serde_json::from_str(json).unwrap_or_default()
}

impl WorkspaceApp {
    pub(crate) fn from_row(row: &sqlx::any::AnyRow) -> Self {
        Self {
            id: row.get("id"),
            workspace_id: row.get("workspace_id"),
            slug: row.get("slug"),
            name: row.get("name"),
            icon: row.get("icon"),
            url: row.get("url"),
            allowed_roles: strings(row.get("allowed_roles")),
            open_mode: row.get("open_mode"),
            client_id: row.get("oidc_client_id"),
            secret_hash: row.get("oidc_secret_hash"),
            redirect_uris: strings(row.get("oidc_redirect_uris")),
            created_at: row.get("created_at"),
            updated_at: row.get("updated_at"),
        }
    }

    pub(crate) fn allows(&self, role: WorkspaceRole) -> bool {
        self.allowed_roles.iter().any(|name| name == role.as_str())
    }
}

async fn load_all(database: &Database, workspace_id: Id) -> Result<Vec<WorkspaceApp>, sqlx::Error> {
    let rows = sqlx::query(sqlx::AssertSqlSafe(format!(
        "SELECT {COLUMNS} FROM workspace_apps WHERE workspace_id = $1 ORDER BY lower(name), id"
    )))
    .bind(workspace_id.to_string())
    .fetch_all(database.pool())
    .await?;
    Ok(rows.iter().map(WorkspaceApp::from_row).collect())
}

/// Ends a removed member's sign-ins to the workspace's apps, in the transaction of the removal.
pub(crate) async fn revoke_member(
    tx: &mut Transaction<'_, Any>,
    workspace_id: Id,
    user_id: Id,
) -> Result<(), sqlx::Error> {
    for table in ["app_oidc_tokens", "app_oidc_codes"] {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "DELETE FROM {table} WHERE user_id = $1 AND app_id IN \
             (SELECT id FROM workspace_apps WHERE workspace_id = $2)"
        )))
        .bind(user_id.to_string())
        .bind(workspace_id.to_string())
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

async fn revoke_app(tx: &mut Transaction<'_, Any>, app_id: &str) -> Result<(), sqlx::Error> {
    for table in ["app_oidc_tokens", "app_oidc_codes"] {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "DELETE FROM {table} WHERE app_id = $1"
        )))
        .bind(app_id)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Who asks.

/// A person (never an agent session) who is a member of the live workspace; otherwise a 404.
async fn member(
    state: &AppsState,
    headers: &HeaderMap,
    workspace_id: &str,
    uri: &Uri,
    request_id: &RequestIdExtension,
) -> Result<(Actor, Id), ApiError> {
    let session = authenticate_session(
        &state.identity,
        state.cookie_mode,
        headers,
        uri.path(),
        request_id.as_ref(),
    )
    .await?;
    let agent: Option<String> =
        sqlx::query_scalar("SELECT session_id FROM agent_sessions WHERE session_id = $1")
            .bind(session.id.to_string())
            .fetch_optional(state.database.pool())
            .await
            .map_err(unavailable(uri, request_id))?;
    if agent.is_some() {
        return Err(problem(
            StatusCode::FORBIDDEN,
            "agent_scope",
            "The agent cannot use this route.",
            uri,
            request_id,
        ));
    }
    let workspace_id: Id = workspace_id
        .parse()
        .map_err(|_| not_found(uri, request_id))?;
    let actor = membership::actor(state.database.pool(), workspace_id, session.user.id)
        .await
        .map_err(unavailable(uri, request_id))?
        .ok_or_else(|| not_found(uri, request_id))?;
    Ok((actor, workspace_id))
}

async fn manager(
    state: &AppsState,
    headers: &HeaderMap,
    workspace_id: &str,
    uri: &Uri,
    request_id: &RequestIdExtension,
) -> Result<(Id, Id), ApiError> {
    let (actor, workspace_id) = member(state, headers, workspace_id, uri, request_id).await?;
    if !actor.can(Permission::AppsManage) {
        return Err(problem(
            StatusCode::FORBIDDEN,
            "forbidden",
            "Only workspace owners and admins can set up apps.",
            uri,
            request_id,
        ));
    }
    Ok((actor.user_id, workspace_id))
}

// ---------------------------------------------------------------------------------------------
// For members.

#[derive(Serialize)]
struct AppView {
    slug: String,
    name: String,
    icon: String,
    url: String,
    open_mode: String,
}

async fn visible(
    State(state): State<AppsState>,
    Path(workspace_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
) -> Result<Json<Value>, ApiError> {
    let (actor, workspace_id) = member(&state, &headers, &workspace_id, &uri, &request_id).await?;
    let apps = load_all(&state.database, workspace_id)
        .await
        .map_err(unavailable(&uri, &request_id))?;
    let items: Vec<AppView> = apps
        .into_iter()
        .filter(|app| app.allows(actor.role))
        .map(|app| AppView {
            slug: app.slug,
            name: app.name,
            icon: app.icon,
            url: app.url,
            open_mode: app.open_mode,
        })
        .collect();
    Ok(Json(json!({ "items": items })))
}

async fn frame(
    State(state): State<AppsState>,
    Path((workspace_id, slug)): Path<(String, String)>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
) -> Result<Response, ApiError> {
    let (actor, workspace_id) = member(&state, &headers, &workspace_id, &uri, &request_id).await?;
    let row = sqlx::query(sqlx::AssertSqlSafe(format!(
        "SELECT {COLUMNS} FROM workspace_apps WHERE workspace_id = $1 AND slug = $2"
    )))
    .bind(workspace_id.to_string())
    .bind(&slug)
    .fetch_optional(state.database.pool())
    .await
    .map_err(unavailable(&uri, &request_id))?
    .ok_or_else(|| not_found(&uri, &request_id))?;
    let app = WorkspaceApp::from_row(&row);
    if !app.allows(actor.role) || app.open_mode != "embed" {
        return Err(not_found(&uri, &request_id));
    }
    // Checked again: the public origin of the server may have changed since the app was saved.
    let url = check_app_url(&app.url, &state.origin, state.allow_http)
        .map_err(|_| not_found(&uri, &request_id))?;
    let policy = frame_csp(&url, &app.redirect_uris, app.client_id.is_some())
        .and_then(|policy| HeaderValue::from_str(&policy).ok())
        .ok_or_else(|| not_found(&uri, &request_id))?;
    let mut response = frame_html(&app.name, &url).into_response();
    let response_headers = response.headers_mut();
    response_headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    response_headers.insert(header::CONTENT_SECURITY_POLICY, policy);
    response_headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}

// ---------------------------------------------------------------------------------------------
// For owners and admins.

#[derive(Serialize)]
struct SignIn {
    client_id: String,
    redirect_uris: Vec<String>,
    issuer: String,
    discovery_url: String,
}

#[derive(Serialize)]
struct ManagedApp {
    id: String,
    slug: String,
    name: String,
    icon: String,
    url: String,
    allowed_roles: Vec<String>,
    open_mode: String,
    sign_in: Option<SignIn>,
    created_at: i64,
    updated_at: i64,
    /// Only in the answer that made the secret: Orbit keeps a hash.
    #[serde(skip_serializing_if = "Option::is_none")]
    client_secret: Option<String>,
}

impl ManagedApp {
    fn of(state: &AppsState, app: WorkspaceApp, client_secret: Option<String>) -> Self {
        let sign_in = app.client_id.map(|client_id| SignIn {
            client_id,
            redirect_uris: app.redirect_uris,
            issuer: state.origin.clone(),
            discovery_url: format!("{}/.well-known/openid-configuration", state.origin),
        });
        Self {
            id: app.id,
            slug: app.slug,
            name: app.name,
            icon: app.icon,
            url: app.url,
            allowed_roles: app.allowed_roles,
            open_mode: app.open_mode,
            sign_in,
            created_at: app.created_at,
            updated_at: app.updated_at,
            client_secret,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    name: Option<String>,
    slug: Option<String>,
    icon: Option<String>,
    url: Option<String>,
    allowed_roles: Option<Vec<String>>,
    open_mode: Option<String>,
    /// Non-empty turns sign-in on (and makes the client), empty turns it off.
    redirect_uris: Option<Vec<String>>,
}

fn slug_from(name: &str) -> String {
    let mut slug = String::new();
    for character in name.chars() {
        let character = character.to_ascii_lowercase();
        if character.is_ascii_alphanumeric() {
            slug.push(character);
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug: String = slug.trim_end_matches('-').chars().take(40).collect();
    slug.trim_end_matches('-').to_owned()
}

fn valid_slug(slug: &str) -> bool {
    (1..=40).contains(&slug.len())
        && slug
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && !slug.starts_with('-')
        && !slug.ends_with('-')
}

fn new_secret() -> (String, Vec<u8>) {
    let secret = format!("orbit-app-s-{}", generate_opaque_token());
    let hash = Sha256::digest(secret.as_bytes()).to_vec();
    (secret, hash)
}

/// Applies the input, and returns the new client secret if this change made one.
fn apply(
    state: &AppsState,
    app: &mut WorkspaceApp,
    input: Input,
    uri: &Uri,
    request_id: &RequestIdExtension,
) -> Result<Option<String>, ApiError> {
    if let Some(name) = input.name {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 80 {
            return Err(invalid(
                "Give the app a name of up to 80 characters.",
                uri,
                request_id,
            ));
        }
        name.clone_into(&mut app.name);
    }
    if let Some(slug) = input.slug {
        if !valid_slug(&slug) {
            return Err(invalid(
                "The address may have lowercase letters, digits and hyphens.",
                uri,
                request_id,
            ));
        }
        app.slug = slug;
    }
    if app.slug.is_empty() {
        app.slug = slug_from(&app.name);
        if app.slug.is_empty() {
            return Err(invalid("Give the app an address.", uri, request_id));
        }
    }
    if let Some(icon) = input.icon {
        if !ICONS.contains(&icon.as_str()) {
            return Err(invalid("Choose one of the icons.", uri, request_id));
        }
        app.icon = icon;
    }
    if let Some(url) = input.url {
        app.url = check_app_url(&url, &state.origin, state.allow_http)
            .map_err(|detail| invalid(detail, uri, request_id))?
            .to_string();
    }
    if app.url.is_empty() {
        return Err(invalid("Enter the address of the app.", uri, request_id));
    }
    if let Some(roles) = input.allowed_roles {
        let mut chosen: Vec<String> = ROLES
            .iter()
            .filter(|role| roles.iter().any(|name| name == *role))
            .map(|role| (*role).to_owned())
            .collect();
        if chosen.is_empty() || roles.iter().any(|name| !ROLES.contains(&name.as_str())) {
            return Err(invalid("Choose at least one known role.", uri, request_id));
        }
        chosen.dedup();
        app.allowed_roles = chosen;
    }
    if let Some(mode) = input.open_mode {
        if !matches!(mode.as_str(), "embed" | "new_tab") {
            return Err(invalid("Choose embed or new_tab.", uri, request_id));
        }
        app.open_mode = mode;
    }
    let mut secret = None;
    if let Some(uris) = input.redirect_uris {
        if uris.len() > MAX_REDIRECT_URIS {
            return Err(invalid(
                "An app has at most 10 redirect URIs.",
                uri,
                request_id,
            ));
        }
        for redirect in &uris {
            check_redirect_uri(redirect, &state.origin, state.allow_http)
                .map_err(|detail| invalid(detail, uri, request_id))?;
        }
        if uris.is_empty() {
            app.client_id = None;
            app.secret_hash = None;
        } else if app.client_id.is_none() {
            if state.app_key.is_none() {
                return Err(problem(
                    StatusCode::CONFLICT,
                    "app_key_missing",
                    "Sign-in needs ORBIT_APP_KEY on the server, to keep its signing key.",
                    uri,
                    request_id,
                ));
            }
            let (plain, hash) = new_secret();
            app.client_id = Some(format!("{CLIENT_PREFIX}{}", Id::new_v7()));
            app.secret_hash = Some(hash);
            secret = Some(plain);
        }
        app.redirect_uris = uris;
    }
    Ok(secret)
}

/// Writes the app and its audit record. `Ok(false)`: the address is taken or the limit is reached.
async fn save(
    state: &AppsState,
    actor: Id,
    workspace_id: Id,
    app: &WorkspaceApp,
    create: bool,
    action: &str,
    request_id: &RequestIdExtension,
) -> Result<bool, sqlx::Error> {
    let mut tx = state.database.immediate_transaction().await?;
    if create {
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM workspace_apps WHERE workspace_id = $1")
                .bind(workspace_id.to_string())
                .fetch_one(&mut *tx)
                .await?;
        if count >= MAX_APPS {
            return Ok(false);
        }
    }
    let taken: Option<String> = sqlx::query_scalar(
        "SELECT id FROM workspace_apps WHERE workspace_id = $1 AND slug = $2 AND id <> $3",
    )
    .bind(workspace_id.to_string())
    .bind(&app.slug)
    .bind(&app.id)
    .fetch_optional(&mut *tx)
    .await?;
    if taken.is_some() {
        return Ok(false);
    }
    sqlx::query(
        "INSERT INTO workspace_apps (id, workspace_id, slug, name, icon, url, allowed_roles, \
         open_mode, oidc_client_id, oidc_secret_hash, oidc_redirect_uris, created_by, created_at, \
         updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14) \
         ON CONFLICT (id) DO UPDATE SET slug = excluded.slug, name = excluded.name, \
         icon = excluded.icon, url = excluded.url, allowed_roles = excluded.allowed_roles, \
         open_mode = excluded.open_mode, oidc_client_id = excluded.oidc_client_id, \
         oidc_secret_hash = excluded.oidc_secret_hash, \
         oidc_redirect_uris = excluded.oidc_redirect_uris, updated_at = excluded.updated_at",
    )
    .bind(&app.id)
    .bind(workspace_id.to_string())
    .bind(&app.slug)
    .bind(&app.name)
    .bind(&app.icon)
    .bind(&app.url)
    .bind(serde_json::to_string(&app.allowed_roles).unwrap_or_else(|_| "[]".to_owned()))
    .bind(&app.open_mode)
    .bind(&app.client_id)
    .bind(&app.secret_hash)
    .bind(serde_json::to_string(&app.redirect_uris).unwrap_or_else(|_| "[]".to_owned()))
    .bind(actor.to_string())
    .bind(app.created_at)
    .bind(app.updated_at)
    .execute(&mut *tx)
    .await?;
    if app.client_id.is_none() {
        revoke_app(&mut tx, &app.id).await?;
    }
    audit_app(&mut tx, workspace_id, actor, action, app, request_id).await?;
    tx.commit().await?;
    Ok(true)
}

async fn audit_app(
    tx: &mut Transaction<'_, Any>,
    workspace_id: Id,
    actor: Id,
    action: &str,
    app: &WorkspaceApp,
    request_id: &RequestIdExtension,
) -> Result<(), sqlx::Error> {
    audit::record(
        tx,
        workspace_id,
        Some(actor),
        action,
        AuditOutcome::Success,
        "workspace_app",
        app.id.parse().ok(),
        request_id
            .as_ref()
            .map_or("workspace-app", |Extension(id)| id.as_str()),
        json!({"slug": app.slug, "sign_in": app.client_id.is_some()}),
        TimestampMillis::now(),
    )
    .await
}

async fn load_one(
    state: &AppsState,
    workspace_id: Id,
    app_id: &str,
    uri: &Uri,
    request_id: &RequestIdExtension,
) -> Result<WorkspaceApp, ApiError> {
    let row = sqlx::query(sqlx::AssertSqlSafe(format!(
        "SELECT {COLUMNS} FROM workspace_apps WHERE workspace_id = $1 AND id = $2"
    )))
    .bind(workspace_id.to_string())
    .bind(app_id)
    .fetch_optional(state.database.pool())
    .await
    .map_err(unavailable(uri, request_id))?
    .ok_or_else(|| not_found(uri, request_id))?;
    Ok(WorkspaceApp::from_row(&row))
}

fn conflict(uri: &Uri, request_id: &RequestIdExtension) -> ApiError {
    problem(
        StatusCode::CONFLICT,
        "workspace_app_conflict",
        "Another app uses this address, or the workspace has 100 apps.",
        uri,
        request_id,
    )
}

async fn list(
    State(state): State<AppsState>,
    Path(workspace_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
) -> Result<Json<Value>, ApiError> {
    let (_, workspace_id) = manager(&state, &headers, &workspace_id, &uri, &request_id).await?;
    let apps = load_all(&state.database, workspace_id)
        .await
        .map_err(unavailable(&uri, &request_id))?;
    let items: Vec<ManagedApp> = apps
        .into_iter()
        .map(|app| ManagedApp::of(&state, app, None))
        .collect();
    Ok(Json(
        json!({ "items": items, "can_sign_in": state.app_key.is_some() }),
    ))
}

async fn create(
    State(state): State<AppsState>,
    Path(workspace_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
    Json(input): Json<Input>,
) -> Result<(StatusCode, Json<ManagedApp>), ApiError> {
    let (actor, workspace_id) = manager(&state, &headers, &workspace_id, &uri, &request_id).await?;
    let now = TimestampMillis::now().as_millis();
    let mut app = WorkspaceApp {
        id: Id::new_v7().to_string(),
        workspace_id: workspace_id.to_string(),
        slug: String::new(),
        name: String::new(),
        icon: ICONS[0].to_owned(),
        url: String::new(),
        allowed_roles: ROLES.iter().map(|role| (*role).to_owned()).collect(),
        open_mode: "embed".to_owned(),
        client_id: None,
        secret_hash: None,
        redirect_uris: Vec::new(),
        created_at: now,
        updated_at: now,
    };
    if input.name.is_none() {
        return Err(invalid("Give the app a name.", &uri, &request_id));
    }
    let secret = apply(&state, &mut app, input, &uri, &request_id)?;
    if secret.is_some() {
        oidc::ensure_signing_key(&state)
            .await
            .map_err(unavailable(&uri, &request_id))?;
    }
    if !save(
        &state,
        actor,
        workspace_id,
        &app,
        true,
        "app.created",
        &request_id,
    )
    .await
    .map_err(unavailable(&uri, &request_id))?
    {
        return Err(conflict(&uri, &request_id));
    }
    Ok((
        StatusCode::CREATED,
        Json(ManagedApp::of(&state, app, secret)),
    ))
}

async fn update(
    State(state): State<AppsState>,
    Path((workspace_id, app_id)): Path<(String, String)>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
    Json(input): Json<Input>,
) -> Result<Json<ManagedApp>, ApiError> {
    let (actor, workspace_id) = manager(&state, &headers, &workspace_id, &uri, &request_id).await?;
    let mut app = load_one(&state, workspace_id, &app_id, &uri, &request_id).await?;
    let secret = apply(&state, &mut app, input, &uri, &request_id)?;
    app.updated_at = TimestampMillis::now().as_millis();
    if secret.is_some() {
        oidc::ensure_signing_key(&state)
            .await
            .map_err(unavailable(&uri, &request_id))?;
    }
    if !save(
        &state,
        actor,
        workspace_id,
        &app,
        false,
        "app.updated",
        &request_id,
    )
    .await
    .map_err(unavailable(&uri, &request_id))?
    {
        return Err(conflict(&uri, &request_id));
    }
    Ok(Json(ManagedApp::of(&state, app, secret)))
}

async fn remove(
    State(state): State<AppsState>,
    Path((workspace_id, app_id)): Path<(String, String)>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
) -> Result<StatusCode, ApiError> {
    let (actor, workspace_id) = manager(&state, &headers, &workspace_id, &uri, &request_id).await?;
    let app = load_one(&state, workspace_id, &app_id, &uri, &request_id).await?;
    let result = async {
        let mut tx = state.database.immediate_transaction().await?;
        // Its codes and tokens go with it (ON DELETE CASCADE).
        sqlx::query("DELETE FROM workspace_apps WHERE id = $1")
            .bind(&app.id)
            .execute(&mut *tx)
            .await?;
        audit_app(
            &mut tx,
            workspace_id,
            actor,
            "app.deleted",
            &app,
            &request_id,
        )
        .await?;
        tx.commit().await
    }
    .await;
    result.map_err(unavailable(&uri, &request_id))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn rotate_secret(
    State(state): State<AppsState>,
    Path((workspace_id, app_id)): Path<(String, String)>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
) -> Result<Json<ManagedApp>, ApiError> {
    let (actor, workspace_id) = manager(&state, &headers, &workspace_id, &uri, &request_id).await?;
    let mut app = load_one(&state, workspace_id, &app_id, &uri, &request_id).await?;
    if app.client_id.is_none() {
        return Err(invalid(
            "This app has no sign-in client. Add a redirect URI first.",
            &uri,
            &request_id,
        ));
    }
    let (secret, hash) = new_secret();
    app.secret_hash = Some(hash);
    app.updated_at = TimestampMillis::now().as_millis();
    let result = async {
        let mut tx = state.database.immediate_transaction().await?;
        sqlx::query(
            "UPDATE workspace_apps SET oidc_secret_hash = $1, updated_at = $2 WHERE id = $3",
        )
        .bind(&app.secret_hash)
        .bind(app.updated_at)
        .bind(&app.id)
        .execute(&mut *tx)
        .await?;
        // A rotation usually means a leak: what the old secret obtained ends too.
        revoke_app(&mut tx, &app.id).await?;
        audit_app(
            &mut tx,
            workspace_id,
            actor,
            "app.secret_rotated",
            &app,
            &request_id,
        )
        .await?;
        tx.commit().await
    }
    .await;
    result.map_err(unavailable(&uri, &request_id))?;
    Ok(Json(ManagedApp::of(&state, app, Some(secret))))
}

#[cfg(test)]
mod tests {
    use url::Url;

    use super::{check_app_url, check_redirect_uri, frame_csp, frame_html, slug_from};

    const ORBIT: &str = "https://orbit.example.com";

    #[test]
    fn app_urls_are_https_without_credentials_and_never_on_orbits_origin() {
        assert_eq!(
            check_app_url("HTTPS://App.Example.com:443/x?y=1", ORBIT, false)
                .unwrap()
                .as_str(),
            "https://app.example.com/x?y=1"
        );
        for bad in [
            "app.example.com",
            "/relative",
            "http://app.example.com",
            "javascript:alert(1)",
            "data:text/html,x",
            "https://user:pw@app.example.com",
            "https://user@app.example.com",
            "https://orbit.example.com/anything",
            "https://ORBIT.example.com:443/",
            "https://a;script-src*.example.com/",
            "https://a'b.example.com/",
        ] {
            assert!(check_app_url(bad, ORBIT, false).is_err(), "{bad}");
        }
        assert!(check_app_url("http://127.0.0.1:4202/", "http://127.0.0.1:4201", true).is_ok());
        assert!(check_app_url("http://127.0.0.1:4201/", "http://127.0.0.1:4201", true).is_err());
    }

    #[test]
    fn redirect_uris_are_exact_and_carry_no_response_parameters() {
        assert!(check_redirect_uri("https://app.example.com/cb", ORBIT, false).is_ok());
        for bad in [
            "https://app.example.com",
            "https://App.example.com/cb",
            "https://app.example.com/cb#x",
            "https://app.example.com/cb?code=1",
            "https://orbit.example.com/cb",
            "http://app.example.com/cb",
        ] {
            assert!(check_redirect_uri(bad, ORBIT, false).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_wrapper_escapes_html_and_allows_only_the_apps_origin() {
        let app = Url::parse("https://app.example.com/a?x=\"><script>&y='z").unwrap();
        let html = frame_html("<b>\"Bad\" & 'name'</b>", &app);
        assert!(!html.contains("<script"));
        assert!(!html.contains("<b>"));
        assert!(html.contains("&lt;b&gt;&quot;Bad&quot; &amp; &#39;name&#39;&lt;/b&gt;"));
        assert!(html.contains("x=%22%3E%3Cscript%3E&amp;y=%27z"), "{html}");
        // The Orbit page sets `src` after it listens: the document itself starts nothing.
        assert!(
            html.contains("<iframe data-src=\"https://app.example.com/a?"),
            "{html}"
        );
        assert!(!html.contains(" src="), "{html}");
        assert_eq!(
            frame_csp(&app, &[], false).unwrap(),
            "default-src 'none'; style-src 'unsafe-inline'; frame-src https://app.example.com; frame-ancestors 'self'; base-uri 'none'; form-action 'none'"
        );
    }

    #[test]
    fn sign_in_adds_orbit_and_the_redirect_origins_to_the_frame_sources() {
        let app = Url::parse("https://app.example.com/").unwrap();
        let policy = frame_csp(
            &app,
            &[
                "https://app.example.com/cb".to_owned(),
                "https://auth.example.com/cb".to_owned(),
            ],
            true,
        )
        .unwrap();
        assert!(
            policy.contains("frame-src https://app.example.com 'self' https://auth.example.com;")
        );
        // A stored address with a CSP separator in the host is refused, not written.
        let unsafe_host = Url::parse("https://a;b.example.com/").unwrap();
        assert!(frame_csp(&unsafe_host, &[], false).is_none());
    }

    #[test]
    fn slugs_come_from_names() {
        assert_eq!(slug_from("My  Report App!"), "my-report-app");
        assert_eq!(slug_from("!!!"), "");
    }
}
