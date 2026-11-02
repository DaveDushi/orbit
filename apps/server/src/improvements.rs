//! "Suggest a change": installation admins send change requests from the app, and an agent outside
//! Orbit (on the same host) reads them with `orbit improvements list` and reports back with
//! `orbit improvements update` (`docs/improvements.md`). Orbit only stores the requests: it never
//! runs anything from them. The text is untrusted input; clients show it as plain text.
//!
//! The routes are for installation admins only and only while the switch in Admin → General is on
//! (the list and the switch also work while it is off). They are under `/api/v1/admin`, so agent
//! sessions never reach them (`agent::scope`); API and MCP tokens are not sessions. Like the agent
//! routes, they are not in the OpenAPI contract (`packages/api/src/improvements.ts`).

use std::sync::Arc;

use axum::extract::{Extension, Path, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::routing::{get, post};
use axum::{Json, Router};
use orbit_platform::{Database, Id, RequestId, Row, TimestampMillis};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::audit::{self, AuditOutcome};
use crate::auth_routes::CookieMode;
use crate::repositories::identity::IdentityRepository;
use crate::task_routes::{ApiError, ApiJson, authenticate_session, request_id_value};

type RequestIdExtension = Option<Extension<RequestId>>;

pub const MAX_TEXT_CHARS: usize = 4_000;
pub const MAX_NOTE_CHARS: usize = 2_000;
pub const MAX_LINK_CHARS: usize = 500;
const MAX_PATH_CHARS: usize = 2_048;
const MAX_REVISION_CHARS: usize = 128;
/// The note of a request that an admin withdrew in the app.
pub const WITHDRAWN_NOTE: &str = "Withdrawn";

/// Where a request is. The agent sets every status but `new`; an admin can only withdraw a `new`
/// request (it becomes `declined`).
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// Sent, not picked up yet.
    New,
    /// The agent works on it.
    Building,
    /// Built; the owner reviews the screenshots.
    Review,
    /// Deployed.
    Live,
    /// Not built: refused by the agent or the owner, or withdrawn.
    Declined,
}

impl Status {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::New => "new",
            Self::Building => "building",
            Self::Review => "review",
            Self::Live => "live",
            Self::Declined => "declined",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        [
            Self::New,
            Self::Building,
            Self::Review,
            Self::Live,
            Self::Declined,
        ]
        .into_iter()
        .find(|status| status.as_str() == value)
    }
}

/// What the app knew when the request was sent.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RequestContext {
    /// The URL path of the page (no query, no host).
    pub path: String,
    /// The window size, `WIDTHxHEIGHT` in CSS pixels.
    pub viewport: String,
    /// The web build's revision (`development` on a dev server).
    pub revision: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Author {
    pub id: String,
    pub name: String,
    pub email: String,
}

/// One request, as the API and `orbit improvements list --json` show it. Keep this shape stable:
/// scripts read it (`docs/improvements.md`).
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ImprovementRequest {
    pub id: String,
    pub status: Status,
    /// Untrusted user input.
    pub text: String,
    pub context: RequestContext,
    pub author: Author,
    /// The agent's short status note.
    pub note: Option<String>,
    /// A branch, commit or address the agent gave.
    pub link: Option<String>,
    pub created_at: TimestampMillis,
    pub updated_at: TimestampMillis,
}

#[derive(Debug, thiserror::Error)]
pub enum ImprovementError {
    #[error("{0}")]
    Invalid(&'static str),
    #[error("no request with this id")]
    NotFound,
    #[error("only a new request can be withdrawn")]
    NotNew,
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

/// Line breaks as `\n`, no space around the text, no control characters but line breaks and tabs.
fn clean_text(raw: &str) -> Result<String, &'static str> {
    let text = raw.replace("\r\n", "\n");
    let text = text.trim();
    if text
        .chars()
        .any(|c| c.is_control() && c != '\n' && c != '\t')
    {
        return Err("The text may not contain control characters.");
    }
    Ok(text.to_owned())
}

/// The request text as stored: 1 to 4,000 characters.
pub fn checked_text(raw: &str) -> Result<String, &'static str> {
    let text = clean_text(raw)?;
    if text.is_empty() {
        return Err("Describe what should change.");
    }
    if text.chars().count() > MAX_TEXT_CHARS {
        return Err("A request may have at most 4,000 characters.");
    }
    Ok(text)
}

/// A note or link as stored: `None` when empty, else at most `max` characters.
fn checked_optional(
    raw: &str,
    max: usize,
    too_long: &'static str,
) -> Result<Option<String>, &'static str> {
    let text = clean_text(raw)?;
    if text.chars().count() > max {
        return Err(too_long);
    }
    Ok((!text.is_empty()).then_some(text))
}

pub fn checked_note(raw: &str) -> Result<Option<String>, &'static str> {
    checked_optional(
        raw,
        MAX_NOTE_CHARS,
        "A note may have at most 2,000 characters.",
    )
}

pub fn checked_link(raw: &str) -> Result<Option<String>, &'static str> {
    let link = checked_optional(
        raw,
        MAX_LINK_CHARS,
        "A link may have at most 500 characters.",
    )?;
    if link.as_deref().is_some_and(|link| link.contains('\n')) {
        return Err("A link is one line.");
    }
    Ok(link)
}

pub fn checked_context(context: RequestContext) -> Result<RequestContext, &'static str> {
    let path_ok = context.path.starts_with('/')
        && context.path.chars().count() <= MAX_PATH_CHARS
        && !context
            .path
            .chars()
            .any(|c| c.is_control() || c.is_whitespace());
    let viewport_ok = context
        .viewport
        .split_once('x')
        .is_some_and(|(width, height)| {
            [width, height].iter().all(|side| {
                (1..=5).contains(&side.len()) && side.bytes().all(|b| b.is_ascii_digit())
            })
        });
    let revision_ok = (1..=MAX_REVISION_CHARS).contains(&context.revision.len())
        && context
            .revision
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'));
    if path_ok && viewport_ok && revision_ok {
        Ok(context)
    } else {
        Err("The page context is not valid.")
    }
}

/// The query for requests with their authors, then `$tail` (a static string).
macro_rules! select {
    ($tail:literal) => {
        concat!(
            "SELECT improvement_requests.id, improvement_requests.status, improvement_requests.body, \
             improvement_requests.context_path, improvement_requests.context_viewport, \
             improvement_requests.context_revision, improvement_requests.status_note, improvement_requests.link, \
             improvement_requests.created_at, improvement_requests.updated_at, users.id AS author_id, \
             users.display_name AS author_name, users.email AS author_email \
             FROM improvement_requests JOIN users ON users.id = improvement_requests.author_id ",
            $tail
        )
    };
}

fn from_row(row: &sqlx::any::AnyRow) -> ImprovementRequest {
    let status: String = row.get("status");
    ImprovementRequest {
        id: row.get("id"),
        // The CHECK of the column allows only these.
        status: Status::parse(&status).unwrap_or(Status::New),
        text: row.get("body"),
        context: RequestContext {
            path: row.get("context_path"),
            viewport: row.get("context_viewport"),
            revision: row.get("context_revision"),
        },
        author: Author {
            id: row.get("author_id"),
            name: row.get("author_name"),
            email: row.get("author_email"),
        },
        note: row.get("status_note"),
        link: row.get("link"),
        created_at: TimestampMillis::from_millis(row.get("created_at")),
        updated_at: TimestampMillis::from_millis(row.get("updated_at")),
    }
}

/// Whether admins may send requests (Admin → General).
pub async fn enabled(database: &Database) -> Result<bool, sqlx::Error> {
    let flag = sqlx::query_scalar::<_, i64>(
        "SELECT improvement_requests_enabled FROM instance_settings WHERE id = 1",
    )
    .fetch_optional(database.pool())
    .await?;
    Ok(flag == Some(1))
}

pub async fn set_enabled(
    database: &Database,
    actor_id: Id,
    enabled: bool,
    request_id: &str,
) -> Result<(), sqlx::Error> {
    let now = TimestampMillis::now();
    let mut tx = database.immediate_transaction().await?;
    sqlx::query(
        "UPDATE instance_settings SET improvement_requests_enabled = $1, updated_at = $2 WHERE id = 1",
    )
    .bind(i64::from(enabled))
    .bind(now.as_millis())
    .execute(&mut *tx)
    .await?;
    audit::record_global(
        &mut tx,
        Some(actor_id),
        "instance.improvements_changed",
        AuditOutcome::Success,
        "instance",
        None,
        request_id,
        json!({ "enabled": enabled }),
        now,
    )
    .await?;
    tx.commit().await
}

/// Every request, newest first; only those with `status` when it is set.
pub async fn list(
    database: &Database,
    status: Option<Status>,
) -> Result<Vec<ImprovementRequest>, sqlx::Error> {
    let rows = match status {
        Some(status) => {
            sqlx::query(select!(
                "WHERE improvement_requests.status = $1 \
                 ORDER BY improvement_requests.created_at DESC, improvement_requests.id DESC"
            ))
            .bind(status.as_str())
            .fetch_all(database.pool())
            .await?
        }
        None => {
            sqlx::query(select!(
                "ORDER BY improvement_requests.created_at DESC, improvement_requests.id DESC"
            ))
            .fetch_all(database.pool())
            .await?
        }
    };
    Ok(rows.iter().map(from_row).collect())
}

async fn find(
    conn: &mut sqlx::AnyConnection,
    id: &str,
) -> Result<Option<ImprovementRequest>, sqlx::Error> {
    Ok(sqlx::query(select!("WHERE improvement_requests.id = $1"))
        .bind(id)
        .fetch_optional(conn)
        .await?
        .as_ref()
        .map(from_row))
}

/// Stores a new request of `author_id`. The text and context must be checked already.
pub async fn create(
    database: &Database,
    author_id: Id,
    text: &str,
    context: &RequestContext,
    request_id: &str,
) -> Result<ImprovementRequest, sqlx::Error> {
    let now = TimestampMillis::now();
    let id = Id::new_v7();
    let mut tx = database.immediate_transaction().await?;
    sqlx::query(
        "INSERT INTO improvement_requests (id, author_id, body, context_path, context_viewport, \
         context_revision, status, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, 'new', $7, $7)",
    )
    .bind(id.to_string())
    .bind(author_id.to_string())
    .bind(text)
    .bind(&context.path)
    .bind(&context.viewport)
    .bind(&context.revision)
    .bind(now.as_millis())
    .execute(&mut *tx)
    .await?;
    // Never the text: it is untrusted and can be long.
    audit::record_global(
        &mut tx,
        Some(author_id),
        "improvement.created",
        AuditOutcome::Success,
        "improvement_request",
        Some(id),
        request_id,
        json!({}),
        now,
    )
    .await?;
    let created = find(&mut tx, &id.to_string())
        .await?
        .ok_or(sqlx::Error::RowNotFound)?;
    tx.commit().await?;
    Ok(created)
}

/// Sets the status (the agent, through the CLI). `note` and `link`, when given, replace the stored
/// ones (an empty value clears them); when not given they stay.
pub async fn update(
    database: &Database,
    actor_id: Option<Id>,
    id: &str,
    status: Status,
    note: Option<Option<String>>,
    link: Option<Option<String>>,
    request_id: &str,
) -> Result<ImprovementRequest, ImprovementError> {
    let now = TimestampMillis::now();
    let mut tx = database.immediate_transaction().await?;
    let current = find(&mut tx, id).await?.ok_or(ImprovementError::NotFound)?;
    let note = note.unwrap_or(current.note.clone());
    let link = link.unwrap_or(current.link.clone());
    sqlx::query(
        "UPDATE improvement_requests SET status = $1, status_note = $2, link = $3, updated_at = $4 WHERE id = $5",
    )
    .bind(status.as_str())
    .bind(note)
    .bind(link)
    .bind(now.as_millis())
    .bind(id)
    .execute(&mut *tx)
    .await?;
    audit::record_global(
        &mut tx,
        actor_id,
        "improvement.status_changed",
        AuditOutcome::Success,
        "improvement_request",
        id.parse().ok(),
        request_id,
        json!({ "from": current.status, "to": status }),
        now,
    )
    .await?;
    let updated = find(&mut tx, id).await?.ok_or(ImprovementError::NotFound)?;
    tx.commit().await?;
    Ok(updated)
}

/// An admin takes back a request the agent has not picked up: it becomes `declined`, "Withdrawn".
pub async fn withdraw(
    database: &Database,
    actor_id: Id,
    id: &str,
    request_id: &str,
) -> Result<ImprovementRequest, ImprovementError> {
    let now = TimestampMillis::now();
    let mut tx = database.immediate_transaction().await?;
    let current = find(&mut tx, id).await?.ok_or(ImprovementError::NotFound)?;
    if current.status != Status::New {
        return Err(ImprovementError::NotNew);
    }
    sqlx::query(
        "UPDATE improvement_requests SET status = 'declined', status_note = $1, updated_at = $2 \
         WHERE id = $3 AND status = 'new'",
    )
    .bind(WITHDRAWN_NOTE)
    .bind(now.as_millis())
    .bind(id)
    .execute(&mut *tx)
    .await?;
    audit::record_global(
        &mut tx,
        Some(actor_id),
        "improvement.withdrawn",
        AuditOutcome::Success,
        "improvement_request",
        id.parse().ok(),
        request_id,
        json!({}),
        now,
    )
    .await?;
    let updated = find(&mut tx, id).await?.ok_or(ImprovementError::NotFound)?;
    tx.commit().await?;
    Ok(updated)
}

// ---------------------------------------------------------------------------------------------
// Routes.

#[derive(Clone)]
pub struct ImprovementState {
    identity: Arc<IdentityRepository>,
    cookie_mode: CookieMode,
}

impl ImprovementState {
    #[must_use]
    pub fn new(identity: Arc<IdentityRepository>, cookie_mode: CookieMode) -> Self {
        Self {
            identity,
            cookie_mode,
        }
    }

    fn database(&self) -> &Database {
        self.identity.database()
    }
}

pub fn router(state: ImprovementState) -> Router {
    Router::new()
        .route(
            "/api/v1/admin/improvements",
            get(list_route).post(create_route),
        )
        .route(
            "/api/v1/admin/improvements/settings",
            get(read_settings).put(save_settings),
        )
        .route(
            "/api/v1/admin/improvements/{id}/withdraw",
            post(withdraw_route),
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
    tracing::warn!(%error, "improvement requests");
    problem(
        StatusCode::SERVICE_UNAVAILABLE,
        "unavailable",
        "Try again in a moment.",
        uri,
        request_id,
    )
}

/// The signed-in installation admin, or the error to answer with.
async fn installation_admin(
    state: &ImprovementState,
    headers: &HeaderMap,
    uri: &Uri,
    request_id: &RequestIdExtension,
) -> Result<Id, ApiError> {
    let session = authenticate_session(
        &state.identity,
        state.cookie_mode,
        headers,
        uri.path(),
        request_id.as_ref(),
    )
    .await?;
    let admin = state
        .identity
        .is_installation_admin(session.user.id)
        .await
        .map_err(|_| {
            problem(
                StatusCode::SERVICE_UNAVAILABLE,
                "unavailable",
                "Try again in a moment.",
                uri,
                request_id,
            )
        })?;
    if !admin {
        return Err(problem(
            StatusCode::FORBIDDEN,
            "installation_admin_required",
            "Only an installation administrator may do this.",
            uri,
            request_id,
        ));
    }
    Ok(session.user.id)
}

#[derive(Serialize)]
struct SettingsView {
    enabled: bool,
    max_text_chars: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SettingsBody {
    enabled: bool,
}

async fn read_settings(
    State(state): State<ImprovementState>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
) -> Result<Json<SettingsView>, ApiError> {
    installation_admin(&state, &headers, &uri, &request_id).await?;
    let enabled = enabled(state.database())
        .await
        .map_err(|error| unavailable(&error, &uri, &request_id))?;
    Ok(Json(SettingsView {
        enabled,
        max_text_chars: MAX_TEXT_CHARS,
    }))
}

async fn save_settings(
    State(state): State<ImprovementState>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
    ApiJson(body): ApiJson<SettingsBody>,
) -> Result<Json<SettingsView>, ApiError> {
    let admin = installation_admin(&state, &headers, &uri, &request_id).await?;
    set_enabled(
        state.database(),
        admin,
        body.enabled,
        request_id_value(request_id.as_ref()),
    )
    .await
    .map_err(|error| unavailable(&error, &uri, &request_id))?;
    Ok(Json(SettingsView {
        enabled: body.enabled,
        max_text_chars: MAX_TEXT_CHARS,
    }))
}

#[derive(Serialize)]
struct ListView {
    items: Vec<ImprovementRequest>,
}

async fn list_route(
    State(state): State<ImprovementState>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
) -> Result<Json<ListView>, ApiError> {
    installation_admin(&state, &headers, &uri, &request_id).await?;
    let items = list(state.database(), None)
        .await
        .map_err(|error| unavailable(&error, &uri, &request_id))?;
    Ok(Json(ListView { items }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateBody {
    text: String,
    context: RequestContext,
}

fn disabled(uri: &Uri, request_id: &RequestIdExtension) -> ApiError {
    problem(
        StatusCode::NOT_FOUND,
        "improvements_disabled",
        "Suggest a change is off on this server. Turn it on in Admin → General.",
        uri,
        request_id,
    )
}

async fn create_route(
    State(state): State<ImprovementState>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
    ApiJson(body): ApiJson<CreateBody>,
) -> Result<(StatusCode, Json<ImprovementRequest>), ApiError> {
    let admin = installation_admin(&state, &headers, &uri, &request_id).await?;
    if !enabled(state.database())
        .await
        .map_err(|error| unavailable(&error, &uri, &request_id))?
    {
        return Err(disabled(&uri, &request_id));
    }
    let invalid = |detail| {
        problem(
            StatusCode::UNPROCESSABLE_ENTITY,
            "validation_failed",
            detail,
            &uri,
            &request_id,
        )
    };
    let text = checked_text(&body.text).map_err(invalid)?;
    let context = checked_context(body.context).map_err(invalid)?;
    let created = create(
        state.database(),
        admin,
        &text,
        &context,
        request_id_value(request_id.as_ref()),
    )
    .await
    .map_err(|error| unavailable(&error, &uri, &request_id))?;
    Ok((StatusCode::CREATED, Json(created)))
}

async fn withdraw_route(
    State(state): State<ImprovementState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
) -> Result<Json<ImprovementRequest>, ApiError> {
    let admin = installation_admin(&state, &headers, &uri, &request_id).await?;
    withdraw(
        state.database(),
        admin,
        &id,
        request_id_value(request_id.as_ref()),
    )
    .await
    .map(Json)
    .map_err(|error| match error {
        ImprovementError::NotFound | ImprovementError::Invalid(_) => problem(
            StatusCode::NOT_FOUND,
            "improvement_not_found",
            "This request does not exist.",
            &uri,
            &request_id,
        ),
        ImprovementError::NotNew => problem(
            StatusCode::CONFLICT,
            "improvement_not_new",
            "The agent has picked up this request already, so it can no longer be withdrawn.",
            &uri,
            &request_id,
        ),
        ImprovementError::Database(error) => unavailable(&error, &uri, &request_id),
    })
}

// ---------------------------------------------------------------------------------------------
// `orbit improvements`: the agent's side. It runs on the host against the database, never over HTTP.

#[derive(Debug, clap::Subcommand)]
pub enum ImprovementsCommand {
    /// Prints the requests, newest first.
    List {
        /// Only requests with this status.
        #[arg(long, value_enum)]
        status: Option<Status>,
        /// A JSON array of requests (the shape is in docs/improvements.md).
        #[arg(long)]
        json: bool,
    },
    /// Sets a request's status, and its note and link when given (an empty value clears them).
    Update {
        id: String,
        #[arg(long, value_enum)]
        status: Status,
        /// A short note for the admins, at most 2,000 characters.
        #[arg(long)]
        note: Option<String>,
        /// A branch, commit or address, at most 500 characters.
        #[arg(long)]
        link: Option<String>,
        /// Prints the updated request as JSON.
        #[arg(long)]
        json: bool,
    },
}

/// The audit log's request id for changes made with the CLI.
const CLI_REQUEST_ID: &str = "operator-cli";

/// Runs `orbit improvements …` on an open, migrated database and returns what to print.
pub async fn run_command(
    database: &Database,
    command: &ImprovementsCommand,
) -> Result<String, ImprovementError> {
    match command {
        ImprovementsCommand::List { status, json } => {
            let items = list(database, *status).await?;
            if *json {
                return Ok(serde_json::to_string_pretty(&items).unwrap_or_default());
            }
            if items.is_empty() {
                return Ok("no requests".to_owned());
            }
            Ok(items.iter().map(summary).collect::<Vec<_>>().join("\n"))
        }
        ImprovementsCommand::Update {
            id,
            status,
            note,
            link,
            json,
        } => {
            let note = note
                .as_deref()
                .map(checked_note)
                .transpose()
                .map_err(ImprovementError::Invalid)?;
            let link = link
                .as_deref()
                .map(checked_link)
                .transpose()
                .map_err(ImprovementError::Invalid)?;
            let updated = update(database, None, id, *status, note, link, CLI_REQUEST_ID).await?;
            Ok(if *json {
                serde_json::to_string_pretty(&updated).unwrap_or_default()
            } else {
                format!("{} is {}", updated.id, updated.status.as_str())
            })
        }
    }
}

/// One line for people: id, status, author and the start of the text. Scripts use `--json`.
fn summary(request: &ImprovementRequest) -> String {
    let first_line = request.text.lines().next().unwrap_or_default();
    let mut start: String = first_line.chars().take(80).collect();
    if start.len() < request.text.len() {
        start.push('…');
    }
    format!(
        "{}  {:<8}  {}  {}",
        request.id,
        request.status.as_str(),
        request.author.email,
        start
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> RequestContext {
        RequestContext {
            path: "/tasks".to_owned(),
            viewport: "1440x900".to_owned(),
            revision: "6af5609".to_owned(),
        }
    }

    #[test]
    fn text_has_one_to_4000_characters_and_no_control_characters() {
        assert_eq!(
            checked_text("  Make it pink\r\nplease \n").unwrap(),
            "Make it pink\nplease"
        );
        assert!(checked_text(" \n\t ").is_err());
        assert!(checked_text("a\u{0007}b").is_err());
        // Characters, not bytes.
        assert!(checked_text(&"é".repeat(MAX_TEXT_CHARS)).is_ok());
        assert!(checked_text(&"a".repeat(MAX_TEXT_CHARS + 1)).is_err());
    }

    #[test]
    fn note_and_link_are_optional_and_limited() {
        assert_eq!(checked_note("  ").unwrap(), None);
        assert_eq!(
            checked_note(&"n".repeat(MAX_NOTE_CHARS))
                .unwrap()
                .unwrap()
                .len(),
            MAX_NOTE_CHARS
        );
        assert!(checked_note(&"n".repeat(MAX_NOTE_CHARS + 1)).is_err());
        assert_eq!(
            checked_link(" feat/pink ").unwrap().as_deref(),
            Some("feat/pink")
        );
        assert!(checked_link("a\nb").is_err());
        assert!(checked_link(&"l".repeat(MAX_LINK_CHARS + 1)).is_err());
    }

    #[test]
    fn context_is_a_path_a_size_and_a_revision() {
        assert!(checked_context(context()).is_ok());
        for broken in [
            RequestContext {
                path: "https://evil.test/".to_owned(),
                ..context()
            },
            RequestContext {
                path: "/a b".to_owned(),
                ..context()
            },
            RequestContext {
                viewport: "1440".to_owned(),
                ..context()
            },
            RequestContext {
                viewport: "1440x-1".to_owned(),
                ..context()
            },
            RequestContext {
                revision: String::new(),
                ..context()
            },
            RequestContext {
                revision: "<script>".to_owned(),
                ..context()
            },
        ] {
            assert!(checked_context(broken.clone()).is_err(), "{broken:?}");
        }
    }

    #[test]
    fn statuses_are_the_five_of_the_check() {
        for status in ["new", "building", "review", "live", "declined"] {
            assert_eq!(Status::parse(status).unwrap().as_str(), status);
        }
        assert_eq!(Status::parse("done"), None);
        assert_eq!(serde_json::to_value(Status::Review).unwrap(), "review");
    }
}
