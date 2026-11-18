//! Support mailbox routes (docs/support.md). Session routes: a project's mailbox (owners and
//! admins), a task's emails and reply, and a retry of a failed reply; access is checked in
//! `repositories::support` for each id. One public route takes inbound mail:
//! `POST /api/v1/hooks/support/{mailbox_id}/{secret}` with the raw message (RFC 5322) as the body.
//! The secret in the path is the credential, so error bodies name only `…/{mailbox_id}`, and
//! nothing of the message is logged.

use axum::body::to_bytes;
use axum::extract::{Extension, Path, Request, State};
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use orbit_platform::{Id, RequestId, TimestampMillis, UploadService};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::billing::WorkspaceAccess;
use crate::billing::guard::READ_ONLY_DETAIL;
use crate::repositories::billing;
use crate::repositories::support::{
    MailboxInput, MailboxRecord, MailboxWithSecret, MessageRecord, Received, SupportRepository,
    TicketView,
};
use crate::support::inbound::{self, MAX_RAW_BYTES, ParseError};
use crate::support::outbound::SupportSender;
use crate::task_routes::{
    ApiError, ApiJson, TaskState, parse_id, request_id_value, scope, task_problem,
};

#[derive(Clone)]
pub struct SupportState {
    tasks: TaskState,
    uploads: UploadService,
    app_key: Option<[u8; 32]>,
    /// The origin of inbound URLs (`ORBIT_PUBLIC_HOOKS_URL`, else the public origin).
    hooks_origin: String,
    /// Local SMTP hosts and any port are allowed (a local mail catcher).
    development: bool,
}

impl SupportState {
    #[must_use]
    pub fn new(
        tasks: TaskState,
        uploads: UploadService,
        app_key: Option<[u8; 32]>,
        hooks_origin: &str,
        development: bool,
    ) -> Self {
        Self {
            tasks,
            uploads,
            app_key,
            hooks_origin: hooks_origin.trim_end_matches('/').to_owned(),
            development,
        }
    }

    fn repository(&self) -> SupportRepository {
        SupportRepository::new(self.tasks.identity.database().clone())
    }
}

pub fn support_router(state: SupportState) -> Router {
    Router::new()
        .route(
            "/api/v1/workspaces/{workspace_id}/projects/{project_id}/support-mailbox",
            get(get_support_mailbox)
                .post(create_support_mailbox)
                .put(update_support_mailbox)
                .delete(delete_support_mailbox),
        )
        .route(
            "/api/v1/workspaces/{workspace_id}/projects/{project_id}/support-mailbox/rotate",
            post(rotate_support_mailbox),
        )
        .route(
            "/api/v1/workspaces/{workspace_id}/projects/{project_id}/support-mailbox/test",
            post(test_support_mailbox),
        )
        .route(
            "/api/v1/workspaces/{workspace_id}/tasks/{task_id}/support",
            get(get_task_support),
        )
        .route(
            "/api/v1/workspaces/{workspace_id}/tasks/{task_id}/support/replies",
            post(create_support_reply),
        )
        .route(
            "/api/v1/workspaces/{workspace_id}/support/messages/{message_id}/retry",
            post(retry_support_message),
        )
        .route(
            "/api/v1/hooks/support/{mailbox_id}/{secret}",
            post(receive_support_email),
        )
        .with_state(state)
}

/// What an owner or admin sets for a mailbox.
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SupportMailboxBody {
    /// The support address customers write to (for example `support@example.com`).
    address: String,
    /// The sender name of replies, for example `Acme Support`.
    #[serde(default)]
    display_name: String,
    /// The SMTP server replies are sent with; without one, the mailbox only receives.
    #[schema(required = false)]
    smtp_host: Option<String>,
    #[schema(required = false)]
    smtp_port: Option<u16>,
    /// `tls`, `starttls`, or `none` (development only).
    #[schema(required = false)]
    smtp_security: Option<String>,
    #[schema(required = false)]
    smtp_username: Option<String>,
    /// Absent keeps the saved password; an empty string removes it.
    #[schema(required = false)]
    smtp_password: Option<String>,
    /// A reply moves the ticket to Pending (default `true`).
    #[schema(required = false)]
    move_to_pending: Option<bool>,
    /// New tickets are assigned to this member.
    #[schema(required = false)]
    default_assignee_id: Option<String>,
}

/// A mailbox. `inbound_url` holds the secret and is returned only when the mailbox is made or
/// its URL is rotated.
#[derive(Serialize, ToSchema)]
pub(crate) struct SupportMailboxResponse {
    mailbox: Option<MailboxRecord>,
    #[schema(required = false)]
    #[serde(skip_serializing_if = "Option::is_none")]
    inbound_url: Option<String>,
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SupportReplyBody {
    /// Plain text; it is emailed to the ticket's customer.
    body: String,
}

fn mailbox_path(workspace: &str, project: &str) -> String {
    format!("/api/v1/workspaces/{workspace}/projects/{project}/support-mailbox")
}

fn app_key_required(instance: &str, request_id: Option<&Extension<RequestId>>) -> ApiError {
    ApiError::new(
        StatusCode::CONFLICT,
        "app_key_required",
        "App key required",
        "Support mailboxes need ORBIT_APP_KEY on the server to encrypt the SMTP password and sign replies.",
        instance,
        request_id,
    )
}

fn with_secret(created: MailboxWithSecret) -> SupportMailboxResponse {
    SupportMailboxResponse {
        inbound_url: Some(format!(
            "{}/{}",
            created.mailbox.inbound_url_base, created.secret
        )),
        mailbox: Some(created.mailbox),
    }
}

fn mailbox_input(
    body: SupportMailboxBody,
    instance: &str,
    request_id: Option<&Extension<RequestId>>,
) -> Result<MailboxInput, ApiError> {
    let default_assignee_id = body
        .default_assignee_id
        .filter(|id| !id.is_empty())
        .map(|id| {
            id.parse::<Id>().map_err(|_| {
                task_problem(
                    crate::repositories::tasks::TaskError::Invalid {
                        field: "default_assignee_id",
                    },
                    instance,
                    request_id,
                )
            })
        })
        .transpose()?;
    Ok(MailboxInput {
        address: body.address,
        display_name: body.display_name,
        smtp_host: body.smtp_host,
        smtp_port: body.smtp_port,
        smtp_security: body.smtp_security,
        smtp_username: body.smtp_username,
        smtp_password: body.smtp_password,
        move_to_pending: body.move_to_pending.unwrap_or(true),
        default_assignee_id,
    })
}

/// The project's mailbox (`mailbox` is `null` without one). Owners and admins.
#[utoipa::path(get, path = "/api/v1/workspaces/{workspace_id}/projects/{project_id}/support-mailbox", params(("workspace_id" = String, Path), ("project_id" = String, Path)), responses((status = 200, body = SupportMailboxResponse)))]
pub(crate) async fn get_support_mailbox(
    State(state): State<SupportState>,
    Path((workspace, project)): Path<(String, String)>,
    headers: HeaderMap,
    request_id: Option<Extension<RequestId>>,
) -> Result<Json<SupportMailboxResponse>, ApiError> {
    let instance = mailbox_path(&workspace, &project);
    let (workspace_id, actor_id) = scope(
        &state.tasks,
        &headers,
        &workspace,
        &instance,
        request_id.as_ref(),
    )
    .await?;
    let project_id = parse_id(&project, &instance, request_id.as_ref())?;
    state
        .repository()
        .mailbox(workspace_id, project_id, actor_id, &state.hooks_origin)
        .await
        .map(|mailbox| {
            Json(SupportMailboxResponse {
                mailbox,
                inbound_url: None,
            })
        })
        .map_err(|error| task_problem(error, instance, request_id.as_ref()))
}

/// Connects a mailbox to the project. Adds the statuses Open, Pending, Solved and Closed when the
/// project lacks them. The answer holds the inbound URL, shown only now.
#[utoipa::path(post, path = "/api/v1/workspaces/{workspace_id}/projects/{project_id}/support-mailbox", params(("workspace_id" = String, Path), ("project_id" = String, Path)), request_body = SupportMailboxBody, responses((status = 201, body = SupportMailboxResponse)))]
pub(crate) async fn create_support_mailbox(
    State(state): State<SupportState>,
    Path((workspace, project)): Path<(String, String)>,
    headers: HeaderMap,
    request_id: Option<Extension<RequestId>>,
    ApiJson(body): ApiJson<SupportMailboxBody>,
) -> Result<Response, ApiError> {
    let instance = mailbox_path(&workspace, &project);
    let (workspace_id, actor_id) = scope(
        &state.tasks,
        &headers,
        &workspace,
        &instance,
        request_id.as_ref(),
    )
    .await?;
    let project_id = parse_id(&project, &instance, request_id.as_ref())?;
    let key = state
        .app_key
        .ok_or_else(|| app_key_required(&instance, request_id.as_ref()))?;
    let input = mailbox_input(body, &instance, request_id.as_ref())?;
    state
        .repository()
        .create_mailbox(
            workspace_id,
            project_id,
            actor_id,
            &input,
            &key,
            state.development,
            &state.hooks_origin,
            request_id_value(request_id.as_ref()),
            TimestampMillis::now(),
        )
        .await
        .map(|created| (StatusCode::CREATED, Json(with_secret(created))).into_response())
        .map_err(|error| task_problem(error, instance, request_id.as_ref()))
}

/// Changes the project's mailbox; its inbound URL stays.
#[utoipa::path(put, path = "/api/v1/workspaces/{workspace_id}/projects/{project_id}/support-mailbox", params(("workspace_id" = String, Path), ("project_id" = String, Path)), request_body = SupportMailboxBody, responses((status = 200, body = SupportMailboxResponse)))]
pub(crate) async fn update_support_mailbox(
    State(state): State<SupportState>,
    Path((workspace, project)): Path<(String, String)>,
    headers: HeaderMap,
    request_id: Option<Extension<RequestId>>,
    ApiJson(body): ApiJson<SupportMailboxBody>,
) -> Result<Json<SupportMailboxResponse>, ApiError> {
    let instance = mailbox_path(&workspace, &project);
    let (workspace_id, actor_id) = scope(
        &state.tasks,
        &headers,
        &workspace,
        &instance,
        request_id.as_ref(),
    )
    .await?;
    let project_id = parse_id(&project, &instance, request_id.as_ref())?;
    let key = state
        .app_key
        .ok_or_else(|| app_key_required(&instance, request_id.as_ref()))?;
    let input = mailbox_input(body, &instance, request_id.as_ref())?;
    state
        .repository()
        .update_mailbox(
            workspace_id,
            project_id,
            actor_id,
            &input,
            &key,
            state.development,
            &state.hooks_origin,
            request_id_value(request_id.as_ref()),
            TimestampMillis::now(),
        )
        .await
        .map(|mailbox| {
            Json(SupportMailboxResponse {
                mailbox: Some(mailbox),
                inbound_url: None,
            })
        })
        .map_err(|error| task_problem(error, instance, request_id.as_ref()))
}

/// Disconnects the mailbox. Its tickets and emails stay.
#[utoipa::path(delete, path = "/api/v1/workspaces/{workspace_id}/projects/{project_id}/support-mailbox", params(("workspace_id" = String, Path), ("project_id" = String, Path)), responses((status = 204)))]
pub(crate) async fn delete_support_mailbox(
    State(state): State<SupportState>,
    Path((workspace, project)): Path<(String, String)>,
    headers: HeaderMap,
    request_id: Option<Extension<RequestId>>,
) -> Result<StatusCode, ApiError> {
    let instance = mailbox_path(&workspace, &project);
    let (workspace_id, actor_id) = scope(
        &state.tasks,
        &headers,
        &workspace,
        &instance,
        request_id.as_ref(),
    )
    .await?;
    let project_id = parse_id(&project, &instance, request_id.as_ref())?;
    state
        .repository()
        .delete_mailbox(
            workspace_id,
            project_id,
            actor_id,
            request_id_value(request_id.as_ref()),
            TimestampMillis::now(),
        )
        .await
        .map(|()| StatusCode::NO_CONTENT)
        .map_err(|error| task_problem(error, instance, request_id.as_ref()))
}

/// A new inbound URL; the old one stops working at once.
#[utoipa::path(post, path = "/api/v1/workspaces/{workspace_id}/projects/{project_id}/support-mailbox/rotate", params(("workspace_id" = String, Path), ("project_id" = String, Path)), responses((status = 200, body = SupportMailboxResponse)))]
pub(crate) async fn rotate_support_mailbox(
    State(state): State<SupportState>,
    Path((workspace, project)): Path<(String, String)>,
    headers: HeaderMap,
    request_id: Option<Extension<RequestId>>,
) -> Result<Json<SupportMailboxResponse>, ApiError> {
    let instance = format!("{}/rotate", mailbox_path(&workspace, &project));
    let (workspace_id, actor_id) = scope(
        &state.tasks,
        &headers,
        &workspace,
        &instance,
        request_id.as_ref(),
    )
    .await?;
    let project_id = parse_id(&project, &instance, request_id.as_ref())?;
    state
        .repository()
        .rotate_mailbox(
            workspace_id,
            project_id,
            actor_id,
            &state.hooks_origin,
            request_id_value(request_id.as_ref()),
            TimestampMillis::now(),
        )
        .await
        .map(|rotated| Json(with_secret(rotated)))
        .map_err(|error| task_problem(error, instance, request_id.as_ref()))
}

/// The result of a connection test: `message` says what failed and how to fix it.
#[derive(Serialize, ToSchema)]
#[schema(as = SupportSmtpTest)]
pub(crate) struct SmtpTestResponse {
    ok: bool,
    message: String,
}

/// Signs in to the mailbox's saved SMTP server and quits, without sending an email. Owners and
/// admins. Answers 200 with `ok: false` and the reason when the server refuses (never the password).
#[utoipa::path(post, path = "/api/v1/workspaces/{workspace_id}/projects/{project_id}/support-mailbox/test", params(("workspace_id" = String, Path), ("project_id" = String, Path)), responses((status = 200, body = SmtpTestResponse)))]
pub(crate) async fn test_support_mailbox(
    State(state): State<SupportState>,
    Path((workspace, project)): Path<(String, String)>,
    headers: HeaderMap,
    request_id: Option<Extension<RequestId>>,
) -> Result<Json<SmtpTestResponse>, ApiError> {
    let instance = format!("{}/test", mailbox_path(&workspace, &project));
    let (workspace_id, actor_id) = scope(
        &state.tasks,
        &headers,
        &workspace,
        &instance,
        request_id.as_ref(),
    )
    .await?;
    let project_id = parse_id(&project, &instance, request_id.as_ref())?;
    let settings = state
        .repository()
        .smtp_settings(workspace_id, project_id, actor_id)
        .await
        .map_err(|error| task_problem(error, &instance, request_id.as_ref()))?;
    let sender = SupportSender::new(
        state.tasks.identity.database().clone(),
        state.app_key,
        state.development,
    );
    Ok(Json(match sender.test_connection(&settings).await {
        Ok(()) => SmtpTestResponse {
            ok: true,
            message: match settings.username.as_deref() {
                Some(user) => format!("Signed in to {} as {user}.", settings.host),
                None => format!("Connected to {} (no user name, no sign-in).", settings.host),
            },
        },
        Err(message) => SmtpTestResponse { ok: false, message },
    }))
}

/// The emails of a task and, for a support ticket, its customer and other tickets.
#[utoipa::path(get, path = "/api/v1/workspaces/{workspace_id}/tasks/{task_id}/support", params(("workspace_id" = String, Path), ("task_id" = String, Path)), responses((status = 200, body = TicketView)))]
pub(crate) async fn get_task_support(
    State(state): State<SupportState>,
    Path((workspace, task)): Path<(String, String)>,
    headers: HeaderMap,
    request_id: Option<Extension<RequestId>>,
) -> Result<Json<TicketView>, ApiError> {
    let instance = format!("/api/v1/workspaces/{workspace}/tasks/{task}/support");
    let (workspace_id, actor_id) = scope(
        &state.tasks,
        &headers,
        &workspace,
        &instance,
        request_id.as_ref(),
    )
    .await?;
    let task_id = parse_id(&task, &instance, request_id.as_ref())?;
    state
        .repository()
        .ticket(workspace_id, actor_id, task_id)
        .await
        .map(Json)
        .map_err(|error| task_problem(error, instance, request_id.as_ref()))
}

/// Emails a reply to the ticket's customer. It is queued at once and sent by a background job;
/// the ticket moves to Pending when its mailbox says so. 409 for a task that is no ticket or whose
/// mailbox has no SMTP server.
#[utoipa::path(post, path = "/api/v1/workspaces/{workspace_id}/tasks/{task_id}/support/replies", params(("workspace_id" = String, Path), ("task_id" = String, Path)), request_body = SupportReplyBody, responses((status = 201, body = MessageRecord)))]
pub(crate) async fn create_support_reply(
    State(state): State<SupportState>,
    Path((workspace, task)): Path<(String, String)>,
    headers: HeaderMap,
    request_id: Option<Extension<RequestId>>,
    ApiJson(body): ApiJson<SupportReplyBody>,
) -> Result<Response, ApiError> {
    let instance = format!("/api/v1/workspaces/{workspace}/tasks/{task}/support/replies");
    let (workspace_id, actor_id) = scope(
        &state.tasks,
        &headers,
        &workspace,
        &instance,
        request_id.as_ref(),
    )
    .await?;
    let task_id = parse_id(&task, &instance, request_id.as_ref())?;
    let key = state
        .app_key
        .ok_or_else(|| app_key_required(&instance, request_id.as_ref()))?;
    state
        .repository()
        .reply(
            workspace_id,
            actor_id,
            task_id,
            &body.body,
            &key,
            request_id_value(request_id.as_ref()),
            TimestampMillis::now(),
        )
        .await
        .map(|message| (StatusCode::CREATED, Json(message)).into_response())
        .map_err(|error| task_problem(error, instance, request_id.as_ref()))
}

/// Sends a failed reply again.
#[utoipa::path(post, path = "/api/v1/workspaces/{workspace_id}/support/messages/{message_id}/retry", params(("workspace_id" = String, Path), ("message_id" = String, Path)), responses((status = 200, body = MessageRecord)))]
pub(crate) async fn retry_support_message(
    State(state): State<SupportState>,
    Path((workspace, message)): Path<(String, String)>,
    headers: HeaderMap,
    request_id: Option<Extension<RequestId>>,
) -> Result<Json<MessageRecord>, ApiError> {
    let instance = format!("/api/v1/workspaces/{workspace}/support/messages/{message}/retry");
    let (workspace_id, actor_id) = scope(
        &state.tasks,
        &headers,
        &workspace,
        &instance,
        request_id.as_ref(),
    )
    .await?;
    let message_id = parse_id(&message, &instance, request_id.as_ref())?;
    state
        .repository()
        .retry(
            workspace_id,
            actor_id,
            message_id,
            request_id_value(request_id.as_ref()),
            TimestampMillis::now(),
        )
        .await
        .map(Json)
        .map_err(|error| task_problem(error, instance, request_id.as_ref()))
}

type HookOk = (
    StatusCode,
    [(axum::http::HeaderName, &'static str); 1],
    &'static str,
);

fn hook_answer(text: &'static str) -> HookOk {
    (
        StatusCode::OK,
        [(CONTENT_TYPE, "text/plain; charset=utf-8")],
        text,
    )
}

/// Files a raw email (RFC 5322, the body as received) for the mailbox. No session: the secret in
/// the path is the credential; an unknown mailbox and a wrong secret both answer 404. Answers
/// `ok` for a filed email or a redelivery, and `dropped` for an auto-reply or bounce (so the
/// sender does not retry). 413 above 36 MB, 422 for a body that is no email.
#[utoipa::path(post, path = "/api/v1/hooks/support/{mailbox_id}/{secret}", params(("mailbox_id" = String, Path), ("secret" = String, Path)), request_body(content = String, content_type = "message/rfc822"), responses((status = 200, description = "`ok` or `dropped`", content_type = "text/plain", body = String)))]
pub(crate) async fn receive_support_email(
    State(state): State<SupportState>,
    Path((mailbox, secret)): Path<(String, String)>,
    request_id: Option<Extension<RequestId>>,
    request: Request,
) -> Result<HookOk, ApiError> {
    let instance = format!("/api/v1/hooks/support/{mailbox}");
    let request_id = request_id.as_ref();
    let not_found = || {
        ApiError::new(
            StatusCode::NOT_FOUND,
            "support_mailbox_not_found",
            "Mailbox not found",
            "No mailbox has this URL.",
            &instance,
            request_id,
        )
    };
    let mailbox_id: Id = mailbox.parse().map_err(|_| not_found())?;
    let repository = state.repository();
    let target = repository
        .find_mailbox(mailbox_id, &secret)
        .await
        .map_err(|error| task_problem(error, &instance, request_id))?
        .ok_or_else(not_found)?;
    let key = state
        .app_key
        .ok_or_else(|| app_key_required(&instance, request_id))?;
    let access = billing::write_access(state.tasks.identity.database(), target.workspace_id)
        .await
        .map_err(|error| task_problem(error.into(), &instance, request_id))?;
    if access == WorkspaceAccess::ReadOnly {
        return Err(ApiError::new(
            StatusCode::PAYMENT_REQUIRED,
            "workspace_read_only",
            "Workspace is read-only",
            READ_ONLY_DETAIL,
            &instance,
            request_id,
        ));
    }
    let raw = to_bytes(request.into_body(), MAX_RAW_BYTES)
        .await
        .map_err(|_| {
            ApiError::new(
                StatusCode::PAYLOAD_TOO_LARGE,
                "email_too_large",
                "Email too large",
                "An email can be at most 36 MB.",
                &instance,
                request_id,
            )
        })?;
    // Parsing is CPU work on untrusted input: off the async workers.
    let email = tokio::task::spawn_blocking(move || inbound::parse(&raw))
        .await
        .map_err(|_| {
            task_problem(
                crate::repositories::tasks::TaskError::Conflict,
                &instance,
                request_id,
            )
        })?
        .map_err(|error| {
            let detail = match error {
                ParseError::NotEmail => "The body is not an email (RFC 5322).",
                ParseError::NoSender => "The email has no valid From address.",
            };
            ApiError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "invalid_email",
                "Invalid email",
                detail,
                &instance,
                request_id,
            )
        })?;
    if email.automatic {
        tracing::info!(%mailbox_id, "support email dropped: automatic reply or bounce");
        return Ok(hook_answer("dropped"));
    }
    let received = repository
        .receive(
            &state.uploads,
            &target,
            &email,
            &key,
            request_id_value(request_id),
        )
        .await
        .map_err(|error| task_problem(error, &instance, request_id))?;
    match received {
        Received::Created(task_id) => {
            tracing::info!(%mailbox_id, %task_id, "support email made a ticket")
        }
        Received::Added(task_id) => {
            tracing::info!(%mailbox_id, %task_id, "support email added to a ticket")
        }
        Received::Duplicate => tracing::info!(%mailbox_id, "support email was already filed"),
    }
    Ok(hook_answer("ok"))
}
