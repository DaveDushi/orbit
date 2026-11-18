//! Support mailboxes and the emails of their tickets (docs/support.md). A ticket is a task in a
//! project with a connected mailbox: an inbound email finds its ticket or makes a new task in the
//! mailbox's Open status ([`SupportRepository::receive`]); a member's reply is queued as an
//! outbound message that `crate::support::outbound` sends with the mailbox's own SMTP server.
//!
//! Access: owners and admins (`integrations.manage`) set up mailboxes. Tickets are tasks, so their
//! emails, customer and reply follow the task's access (project visibility; guests never pass).
//! The inbound path finds the mailbox by id and secret only, never by the address it was sent to.

use orbit_domain::{Permission, StatusCategory};
use orbit_platform::{Database, Flag, Id, Job, JobStore, Row, TimestampMillis, UploadService};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::any::AnyRow;
use sqlx::{Any, Transaction};
use subtle::ConstantTimeEq;
use utoipa::ToSchema;

use super::instance_settings::SmtpSecurity;
use super::membership;
use super::project_access::{ProjectScope, user_sees_project, user_sees_task_sql};
use super::tasks::{
    TaskChanges, TaskError, TaskUpdate, parse_id, record_principal_mutation, require_access_tx,
    task_in_tx, update_task_in_tx,
};
use crate::audit::{self, AuditOutcome};
use crate::support::inbound::{InboundEmail, one_line, valid_address};
use crate::support::{html, outbound, thread};

/// The name of the service account that creates tickets and moves them on customer replies.
const SERVICE_ACCOUNT: &str = "Support email";
/// The statuses a mailbox needs; missing ones are added to the project when it is connected.
const SUPPORT_STATUSES: [(&str, StatusCategory, &str); 4] = [
    ("Open", StatusCategory::Unstarted, "#3b82f6"),
    ("Pending", StatusCategory::Started, "#f59e0b"),
    ("Solved", StatusCategory::Completed, "#22c55e"),
    ("Closed", StatusCategory::Cancelled, "#6b7280"),
];
/// SMTP ports a mailbox may use outside development: no other service can be reached through it.
const SMTP_PORTS: [u16; 4] = [25, 465, 587, 2525];
/// Characters of a reply.
const MAX_REPLY_CHARS: usize = 50_000;
/// Message-IDs kept in the `References` of a reply.
const MAX_REFERENCES: usize = 20;
/// Other tickets of a customer shown on the Customer card.
const OTHER_TICKETS: i64 = 10;

#[derive(Clone)]
pub struct SupportRepository {
    database: Database,
}

/// A project's mailbox, without its secrets.
#[derive(Clone, Debug, Serialize, ToSchema)]
#[schema(as = SupportMailbox)]
pub struct MailboxRecord {
    #[schema(value_type = String)]
    pub id: Id,
    #[schema(value_type = String)]
    pub project_id: Id,
    /// The support address customers write to; replies come from it.
    pub address: String,
    pub display_name: String,
    pub smtp_host: Option<String>,
    pub smtp_port: Option<u16>,
    /// `tls`, `starttls` or `none`.
    pub smtp_security: Option<String>,
    pub smtp_username: Option<String>,
    /// A password is saved (it is never returned).
    pub smtp_password_set: bool,
    /// A reply moves the ticket to Pending.
    pub move_to_pending: bool,
    /// New tickets are assigned to this member.
    #[schema(value_type = Option<String>)]
    pub default_assignee_id: Option<Id>,
    /// The URL of the inbound endpoint without its secret (`…/{secret}` is added to it).
    pub inbound_url_base: String,
    #[schema(value_type = String, format = DateTime)]
    pub created_at: TimestampMillis,
    #[schema(value_type = String, format = DateTime)]
    pub updated_at: TimestampMillis,
    #[schema(value_type = Option<String>, format = DateTime)]
    pub rotated_at: Option<TimestampMillis>,
    #[schema(value_type = Option<String>, format = DateTime)]
    pub last_received_at: Option<TimestampMillis>,
}

/// A mailbox with the secret of its inbound URL: only when it is made or rotated.
#[derive(Clone, Debug)]
pub struct MailboxWithSecret {
    pub mailbox: MailboxRecord,
    pub secret: String,
}

/// What an owner or admin sets. `smtp_password`: `None` keeps the saved one, `Some("")` clears it.
#[derive(Clone, Debug)]
pub struct MailboxInput {
    pub address: String,
    pub display_name: String,
    pub smtp_host: Option<String>,
    pub smtp_port: Option<u16>,
    pub smtp_security: Option<String>,
    pub smtp_username: Option<String>,
    pub smtp_password: Option<String>,
    pub move_to_pending: bool,
    pub default_assignee_id: Option<Id>,
}

/// The mailbox an inbound URL names, for filing its email.
#[derive(Clone, Debug)]
pub struct InboundTarget {
    pub mailbox_id: Id,
    pub workspace_id: Id,
    pub project_id: Id,
    pub created_by: Id,
    pub open_status_id: String,
    pub default_assignee_id: Option<Id>,
}

/// What happened to an inbound email.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Received {
    /// A new ticket.
    Created(Id),
    /// Added to an existing ticket.
    Added(Id),
    /// The same Message-ID came before (a redelivery).
    Duplicate,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[schema(as = SupportMessageAttachment)]
pub struct MessageAttachment {
    /// A task attachment of the ticket (`…/tasks/{task_id}/attachments/{id}/download`).
    #[schema(value_type = String)]
    pub id: Id,
    pub name: String,
    pub media_type: String,
    pub size: u64,
    /// Shown inside the HTML body (`cid:`), not in the list.
    pub inline: bool,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
#[schema(as = SupportMessage)]
pub struct MessageRecord {
    #[schema(value_type = String)]
    pub id: Id,
    #[schema(value_type = String)]
    pub task_id: Id,
    /// `in` from the customer, `out` a reply.
    pub direction: String,
    /// The member who wrote a reply.
    #[schema(value_type = Option<String>)]
    pub author_id: Option<Id>,
    pub from_address: String,
    pub from_name: String,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub subject: String,
    pub body_text: String,
    /// Sanitized HTML of an inbound email; show it only in a sandboxed frame.
    pub body_html: Option<String>,
    pub attachments: Vec<MessageAttachment>,
    /// Attachments left out by the size or count cap.
    pub skipped_attachments: u32,
    /// The provider reported failed SPF/DKIM/DMARC checks.
    pub unverified: bool,
    /// Replies: `queued`, `sent` or `failed`.
    pub send_state: Option<String>,
    /// Why the last send failed.
    pub send_error: Option<String>,
    #[schema(value_type = Option<String>, format = DateTime)]
    pub sent_at: Option<TimestampMillis>,
    #[schema(value_type = String, format = DateTime)]
    pub created_at: TimestampMillis,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
#[schema(as = SupportContact)]
pub struct ContactRecord {
    #[schema(value_type = String)]
    pub id: Id,
    pub email: String,
    pub name: String,
}

/// Another ticket of the same customer that the reader sees.
#[derive(Clone, Debug, Serialize, ToSchema)]
#[schema(as = SupportOtherTicket)]
pub struct OtherTicket {
    #[schema(value_type = String)]
    pub task_id: Id,
    #[schema(value_type = String)]
    pub project_id: Id,
    pub identifier: String,
    pub title: String,
    #[schema(value_type = String)]
    pub status_id: Id,
    #[schema(value_type = String, format = DateTime)]
    pub created_at: TimestampMillis,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
#[schema(as = SupportTicketInfo)]
pub struct TicketInfo {
    pub contact: ContactRecord,
    /// The mailbox address replies come from; `None` once the mailbox is disconnected.
    pub mailbox_address: Option<String>,
    /// A reply can be sent: the mailbox is connected and has an SMTP server.
    pub can_reply: bool,
    pub other_tickets: Vec<OtherTicket>,
}

/// The emails of a task. `ticket` is `None` for a task that is no support ticket.
#[derive(Clone, Debug, Serialize, ToSchema)]
#[schema(as = SupportTicket)]
pub struct TicketView {
    pub ticket: Option<TicketInfo>,
    pub messages: Vec<MessageRecord>,
}

/// A queued reply, as the send job needs it.
#[derive(Clone, Debug)]
pub struct Outgoing {
    pub message_id: Id,
    pub workspace_id: Id,
    pub mailbox_id: Id,
    pub from_address: String,
    pub from_name: String,
    pub to: String,
    pub subject: String,
    pub body: String,
    pub header_id: String,
    pub in_reply_to: Option<String>,
    pub references: String,
    pub smtp_host: Option<String>,
    pub smtp_port: Option<u16>,
    pub smtp_security: Option<String>,
    pub smtp_username: Option<String>,
    pub smtp_password: Option<Vec<u8>>,
}

fn time(row: &AnyRow, column: &str) -> Option<TimestampMillis> {
    row.get::<Option<i64>, _>(column)
        .map(TimestampMillis::from_millis)
}

fn optional_id(value: Option<String>) -> Result<Option<Id>, TaskError> {
    value.map(parse_id).transpose()
}

fn split_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .filter(|item| !item.is_empty())
        .map(str::to_owned)
        .collect()
}

const MAILBOX_COLUMNS: &str = "id, project_id, address, display_name, smtp_host, smtp_port, smtp_security, \
     smtp_username, smtp_password, pending_status_id, default_assignee_id, created_at, updated_at, \
     rotated_at, last_received_at";

fn mailbox_from_row(row: &AnyRow, hooks_origin: &str) -> Result<MailboxRecord, TaskError> {
    let id = parse_id(row.get("id"))?;
    Ok(MailboxRecord {
        id,
        project_id: parse_id(row.get("project_id"))?,
        address: row.get("address"),
        display_name: row.get("display_name"),
        smtp_host: row.get("smtp_host"),
        smtp_port: row
            .get::<Option<i64>, _>("smtp_port")
            .and_then(|port| u16::try_from(port).ok()),
        smtp_security: row.get("smtp_security"),
        smtp_username: row.get("smtp_username"),
        smtp_password_set: row.get::<Option<Vec<u8>>, _>("smtp_password").is_some(),
        move_to_pending: row.get::<Option<String>, _>("pending_status_id").is_some(),
        default_assignee_id: optional_id(row.get("default_assignee_id"))?,
        inbound_url_base: format!(
            "{}/api/v1/hooks/support/{id}",
            hooks_origin.trim_end_matches('/')
        ),
        created_at: TimestampMillis::from_millis(row.get("created_at")),
        updated_at: TimestampMillis::from_millis(row.get("updated_at")),
        rotated_at: time(row, "rotated_at"),
        last_received_at: time(row, "last_received_at"),
    })
}

const MESSAGE_COLUMNS: &str = "id, task_id, direction, author_id, from_address, from_name, to_addresses, \
     cc_addresses, subject, body_text, body_html, attachments_json, unverified, send_state, send_error, \
     sent_at, created_at";

/// What `attachments_json` holds: the attachments, and how many the caps left out.
#[derive(Default, Deserialize, Serialize)]
#[serde(default)]
struct StoredAttachments {
    items: Vec<MessageAttachment>,
    skipped: u32,
}

fn message_from_row(row: &AnyRow) -> Result<MessageRecord, TaskError> {
    let stored: StoredAttachments =
        serde_json::from_str(&row.get::<String, _>("attachments_json")).unwrap_or_default();
    Ok(MessageRecord {
        id: parse_id(row.get("id"))?,
        task_id: parse_id(row.get("task_id"))?,
        direction: row.get("direction"),
        author_id: optional_id(row.get("author_id"))?,
        from_address: row.get("from_address"),
        from_name: row.get("from_name"),
        to: split_list(&row.get::<String, _>("to_addresses")),
        cc: split_list(&row.get::<String, _>("cc_addresses")),
        subject: row.get("subject"),
        body_text: row.get("body_text"),
        body_html: row.get("body_html"),
        attachments: stored.items,
        skipped_attachments: stored.skipped,
        unverified: row.get::<i64, _>("unverified") != 0,
        send_state: row.get("send_state"),
        send_error: row.get("send_error"),
        sent_at: time(row, "sent_at"),
        created_at: TimestampMillis::from_millis(row.get("created_at")),
    })
}

fn new_secret() -> (String, [u8; 32]) {
    let secret = orbit_platform::generate_opaque_token();
    let hash = Sha256::digest(secret.as_bytes()).into();
    (secret, hash)
}

/// The domain of an address, for Message-IDs.
fn domain_of(address: &str) -> &str {
    address
        .rsplit_once('@')
        .map_or("orbit.invalid", |(_, domain)| domain)
}

/// An SMTP host a mailbox may use: a public name or address and an SMTP port. `development`
/// also takes local hosts and any port, for a local mail catcher.
pub(crate) fn smtp_host_allowed(host: &str, port: u16, development: bool) -> bool {
    if host.is_empty() || host.len() > 253 || host.contains(|c: char| c.is_whitespace() || c == '/')
    {
        return false;
    }
    if development {
        return port > 0;
    }
    if !SMTP_PORTS.contains(&port) {
        return false;
    }
    match host.trim_matches(['[', ']']).parse::<std::net::IpAddr>() {
        Ok(ip) => orbit_platform::is_public_ip(ip),
        Err(_) => {
            let name = host.to_ascii_lowercase();
            name.contains('.')
                && name != "localhost"
                && ![".localhost", ".local", ".internal", ".lan", ".home.arpa"]
                    .iter()
                    .any(|suffix| name.ends_with(suffix))
        }
    }
}

/// Checks and cleans `input`; the SMTP password is encrypted with `key`.
struct CleanInput {
    address: String,
    display_name: String,
    smtp_host: Option<String>,
    smtp_port: Option<u16>,
    smtp_security: Option<String>,
    smtp_username: Option<String>,
    /// `None` keeps the saved password.
    smtp_password: Option<Option<Vec<u8>>>,
}

fn clean_input(
    input: &MailboxInput,
    key: &[u8; 32],
    development: bool,
) -> Result<CleanInput, TaskError> {
    let address = input.address.trim().to_ascii_lowercase();
    if !valid_address(&address) {
        return Err(TaskError::Invalid { field: "address" });
    }
    let display_name = one_line(&input.display_name, 80);
    let host = input
        .smtp_host
        .as_deref()
        .map(str::trim)
        .filter(|host| !host.is_empty())
        .map(str::to_ascii_lowercase);
    let (smtp_host, smtp_port, smtp_security) = match host {
        None => (None, None, None),
        Some(host) => {
            let port = input
                .smtp_port
                .ok_or(TaskError::Invalid { field: "smtp_port" })?;
            if !smtp_host_allowed(&host, port, development) {
                return Err(TaskError::Invalid { field: "smtp_host" });
            }
            let security = input
                .smtp_security
                .as_deref()
                .and_then(SmtpSecurity::parse)
                .ok_or(TaskError::Invalid {
                    field: "smtp_security",
                })?;
            if security == SmtpSecurity::None && !development {
                return Err(TaskError::Invalid {
                    field: "smtp_security",
                });
            }
            (Some(host), Some(port), Some(security.as_str().to_owned()))
        }
    };
    let smtp_username = input
        .smtp_username
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned);
    if smtp_username
        .as_deref()
        .is_some_and(|name| name.len() > 320 || name.contains(['\r', '\n']))
    {
        return Err(TaskError::Invalid {
            field: "smtp_username",
        });
    }
    let smtp_password = match input.smtp_password.as_deref() {
        None => None,
        Some("") => Some(None),
        Some(password) if password.len() > 1024 => {
            return Err(TaskError::Invalid {
                field: "smtp_password",
            });
        }
        Some(password) => Some(Some(
            crate::secret_box::encrypt_secret(key, password).map_err(|()| TaskError::Conflict)?,
        )),
    };
    Ok(CleanInput {
        address,
        display_name,
        smtp_host,
        smtp_port,
        smtp_security,
        smtp_username,
        smtp_password,
    })
}

/// Owners and admins set up mailboxes, of a live project of their workspace.
async fn require_manager(
    tx: &mut Transaction<'_, Any>,
    workspace_id: Id,
    project_id: Id,
    actor_id: Id,
) -> Result<(), TaskError> {
    let actor = membership::actor(&mut **tx, workspace_id, actor_id)
        .await?
        .ok_or(TaskError::NotFound)?;
    let Flag(live) = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM projects WHERE id = $1 AND workspace_id = $2 AND deleted_at IS NULL)",
    )
    .bind(project_id.to_string())
    .bind(workspace_id.to_string())
    .fetch_one(&mut **tx)
    .await?;
    if !live {
        return Err(TaskError::NotFound);
    }
    if !actor.can(Permission::IntegrationsManage) {
        return Err(TaskError::Forbidden);
    }
    Ok(())
}

/// The default assignee must be an active member who sees the project.
async fn check_assignee(
    tx: &mut Transaction<'_, Any>,
    workspace_id: Id,
    project_id: Id,
    user_id: Option<Id>,
) -> Result<(), TaskError> {
    let Some(user_id) = user_id else {
        return Ok(());
    };
    let member = membership::actor(&mut **tx, workspace_id, user_id).await?;
    if member.is_none() || !user_sees_project(&mut **tx, project_id, user_id).await? {
        return Err(TaskError::Invalid {
            field: "default_assignee_id",
        });
    }
    Ok(())
}

/// The ids of the project's Open and Pending statuses; missing support statuses are added.
async fn ensure_statuses(
    tx: &mut Transaction<'_, Any>,
    workspace_id: Id,
    project_id: Id,
    actor_id: Id,
    request_id: &str,
    now: TimestampMillis,
) -> Result<(String, String), TaskError> {
    let mut ids = Vec::new();
    for (name, category, color) in SUPPORT_STATUSES {
        let existing: Option<String> = sqlx::query_scalar(
            "SELECT id FROM task_statuses WHERE workspace_id = $1 AND project_id = $2 \
             AND lower(name) = lower($3) ORDER BY position, id LIMIT 1",
        )
        .bind(workspace_id.to_string())
        .bind(project_id.to_string())
        .bind(name)
        .fetch_optional(&mut **tx)
        .await?;
        let id = match existing {
            Some(id) => id,
            None => {
                let id = Id::new_v7();
                let position: i64 = sqlx::query_scalar(
                    "SELECT COALESCE(MAX(position) + 1, 0) FROM task_statuses WHERE workspace_id = $1 AND project_id = $2",
                )
                .bind(workspace_id.to_string())
                .bind(project_id.to_string())
                .fetch_one(&mut **tx)
                .await?;
                sqlx::query(
                    "INSERT INTO task_statuses (id, workspace_id, project_id, name, description, color, category, \
                     position, version, created_at, updated_at) VALUES ($1, $2, $3, $4, '', $5, $6, $7, 0, $8, $8)",
                )
                .bind(id.to_string())
                .bind(workspace_id.to_string())
                .bind(project_id.to_string())
                .bind(name)
                .bind(color)
                .bind(category.as_str())
                .bind(position)
                .bind(now.as_millis())
                .execute(&mut **tx)
                .await?;
                audit::record(
                    tx,
                    workspace_id,
                    Some(actor_id),
                    "status.created",
                    AuditOutcome::Success,
                    "status",
                    Some(id),
                    request_id,
                    json!({}),
                    now,
                )
                .await?;
                id.to_string()
            }
        };
        ids.push(id);
    }
    Ok((ids[0].clone(), ids[1].clone()))
}

/// The status a ticket moves to when it is still in the project; else `None`.
async fn live_status(
    tx: &mut Transaction<'_, Any>,
    project_id: Id,
    status_id: Option<&str>,
) -> Result<Option<Id>, TaskError> {
    let Some(status_id) = status_id else {
        return Ok(None);
    };
    let found: Option<String> =
        sqlx::query_scalar("SELECT id FROM task_statuses WHERE id = $1 AND project_id = $2")
            .bind(status_id)
            .bind(project_id.to_string())
            .fetch_optional(&mut **tx)
            .await?;
    optional_id(found)
}

async fn status_category(
    tx: &mut Transaction<'_, Any>,
    status_id: Id,
) -> Result<String, TaskError> {
    Ok(
        sqlx::query_scalar("SELECT category FROM task_statuses WHERE id = $1")
            .bind(status_id.to_string())
            .fetch_optional(&mut **tx)
            .await?
            .unwrap_or_default(),
    )
}

async fn service_account_in_tx(
    tx: &mut Transaction<'_, Any>,
    workspace_id: Id,
    actor_id: Id,
    now: TimestampMillis,
) -> Result<Id, TaskError> {
    if let Some(id) = sqlx::query_scalar::<_, String>(
        "UPDATE service_accounts SET disabled_at = NULL WHERE workspace_id = $1 AND lower(name) = lower($2) RETURNING id",
    )
    .bind(workspace_id.to_string())
    .bind(SERVICE_ACCOUNT)
    .fetch_optional(&mut **tx)
    .await?
    {
        return parse_id(id);
    }
    let id = Id::new_v7();
    sqlx::query(
        "INSERT INTO service_accounts (id, workspace_id, name, created_by, created_at) VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(id.to_string())
    .bind(workspace_id.to_string())
    .bind(SERVICE_ACCOUNT)
    .bind(actor_id.to_string())
    .bind(now.as_millis())
    .execute(&mut **tx)
    .await?;
    Ok(id)
}

/// Moves a ticket to `status_id`, as `actor_id` or, with `service_account`, as that account.
#[allow(clippy::too_many_arguments)]
async fn move_ticket(
    tx: &mut Transaction<'_, Any>,
    workspace_id: Id,
    task_id: Id,
    status_id: Id,
    actor_id: Id,
    service_account: Option<Id>,
    request_id: &str,
    now: TimestampMillis,
) -> Result<(), TaskError> {
    let current = task_in_tx(tx, ProjectScope::All, workspace_id, task_id, false).await?;
    if current.status_id == status_id {
        return Ok(());
    }
    let update = TaskUpdate {
        id: task_id,
        expected_version: current.version,
        changes: TaskChanges {
            status_id: Some(status_id),
            ..TaskChanges::default()
        },
    };
    update_task_in_tx(
        tx,
        ProjectScope::All,
        workspace_id,
        actor_id,
        &update,
        request_id,
        now,
        service_account.is_none(),
        false,
    )
    .await?;
    let changes =
        json!({ "changes": { "status": { "from": current.status_id, "to": status_id } } });
    match service_account {
        Some(account) => {
            let mut metadata = changes;
            metadata["actor_service_account_id"] = json!(account);
            metadata["actor_service_account_name"] = json!(SERVICE_ACCOUNT);
            audit::record(
                tx,
                workspace_id,
                None,
                "task.updated",
                AuditOutcome::Success,
                "task",
                Some(task_id),
                request_id,
                metadata,
                now,
            )
            .await?;
        }
        None => {
            audit::record(
                tx,
                workspace_id,
                Some(actor_id),
                "task.updated",
                AuditOutcome::Success,
                "task",
                Some(task_id),
                request_id,
                changes,
                now,
            )
            .await?;
        }
    }
    Ok(())
}

/// An event that refreshes open clients (the change feed follows audit rows); not task activity.
#[allow(clippy::too_many_arguments)]
async fn record_message_event(
    tx: &mut Transaction<'_, Any>,
    workspace_id: Id,
    actor_id: Option<Id>,
    action: &str,
    message_id: Id,
    task_id: Id,
    request_id: &str,
    now: TimestampMillis,
) -> Result<(), TaskError> {
    audit::record(
        tx,
        workspace_id,
        actor_id,
        action,
        AuditOutcome::Success,
        "support_message",
        Some(message_id),
        request_id,
        json!({ "task_id": task_id }),
        now,
    )
    .await?;
    Ok(())
}

/// Inbox notifications (and so pushes) for the ticket's assignees and followers who see it, from
/// the customer's address (`actor_github_login` holds an external author; an email has an `@`).
async fn notify_people(
    tx: &mut Transaction<'_, Any>,
    workspace_id: Id,
    task_id: Id,
    message_id: Id,
    from_address: &str,
    now: TimestampMillis,
) -> Result<(), TaskError> {
    let recipients: Vec<String> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT users.id FROM users WHERE users.suspended_at IS NULL \
         AND (users.id IN (SELECT user_id FROM task_assignees WHERE task_id = $1) \
              OR users.id IN (SELECT user_id FROM task_followers WHERE task_id = $2)) \
         AND {} ORDER BY users.id",
        user_sees_task_sql("$3", "users.id")
    )))
    .bind(task_id.to_string())
    .bind(task_id.to_string())
    .bind(task_id.to_string())
    .fetch_all(&mut **tx)
    .await?;
    let actor: String = from_address.chars().take(100).collect();
    for recipient in recipients {
        sqlx::query(
            "INSERT INTO notifications (id, workspace_id, recipient_user_id, actor_user_id, actor_github_login, \
             kind, task_id, comment_id, dedupe_key, created_at) \
             VALUES ($1, $2, $3, NULL, $4, 'task_commented', $5, NULL, $6, $7) ON CONFLICT DO NOTHING",
        )
        .bind(Id::new_v7().to_string())
        .bind(workspace_id.to_string())
        .bind(&recipient)
        .bind(&actor)
        .bind(task_id.to_string())
        .bind(format!("{workspace_id}:{recipient}:support_message:{message_id}"))
        .bind(now.as_millis())
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

impl SupportRepository {
    #[must_use]
    pub fn new(database: Database) -> Self {
        Self { database }
    }

    /// The project's mailbox, or `None`. Owners and admins only.
    pub async fn mailbox(
        &self,
        workspace_id: Id,
        project_id: Id,
        actor_id: Id,
        hooks_origin: &str,
    ) -> Result<Option<MailboxRecord>, TaskError> {
        let mut tx = self.database.pool().begin().await?;
        require_manager(&mut tx, workspace_id, project_id, actor_id).await?;
        let row = sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT {MAILBOX_COLUMNS} FROM support_mailboxes WHERE workspace_id = $1 AND project_id = $2"
        )))
        .bind(workspace_id.to_string())
        .bind(project_id.to_string())
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;
        row.map(|row| mailbox_from_row(&row, hooks_origin))
            .transpose()
    }

    /// Connects a mailbox to the project (one per project) and adds the support statuses it
    /// lacks. Answers with the secret of the inbound URL, shown once.
    #[allow(clippy::too_many_arguments)]
    pub async fn create_mailbox(
        &self,
        workspace_id: Id,
        project_id: Id,
        actor_id: Id,
        input: &MailboxInput,
        key: &[u8; 32],
        development: bool,
        hooks_origin: &str,
        request_id: &str,
        now: TimestampMillis,
    ) -> Result<MailboxWithSecret, TaskError> {
        let clean = clean_input(input, key, development)?;
        let mut tx = self.database.immediate_transaction().await?;
        require_manager(&mut tx, workspace_id, project_id, actor_id).await?;
        check_assignee(&mut tx, workspace_id, project_id, input.default_assignee_id).await?;
        let Flag(taken) = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM support_mailboxes WHERE project_id = $1)",
        )
        .bind(project_id.to_string())
        .fetch_one(&mut *tx)
        .await?;
        if taken {
            return Err(TaskError::Conflict);
        }
        let (open, pending) =
            ensure_statuses(&mut tx, workspace_id, project_id, actor_id, request_id, now).await?;
        let id = Id::new_v7();
        let (secret, hash) = new_secret();
        sqlx::query(
            "INSERT INTO support_mailboxes (id, workspace_id, project_id, address, display_name, secret_hash, \
             smtp_host, smtp_port, smtp_security, smtp_username, smtp_password, open_status_id, pending_status_id, \
             default_assignee_id, created_by, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $16)",
        )
        .bind(id.to_string())
        .bind(workspace_id.to_string())
        .bind(project_id.to_string())
        .bind(&clean.address)
        .bind(&clean.display_name)
        .bind(hash.to_vec())
        .bind(clean.smtp_host.as_deref())
        .bind(clean.smtp_port.map(i64::from))
        .bind(clean.smtp_security.as_deref())
        .bind(clean.smtp_username.as_deref())
        .bind(clean.smtp_password.flatten())
        .bind(&open)
        .bind(input.move_to_pending.then_some(pending))
        .bind(input.default_assignee_id.map(|id| id.to_string()))
        .bind(actor_id.to_string())
        .bind(now.as_millis())
        .execute(&mut *tx)
        .await?;
        audit::record(
            &mut tx,
            workspace_id,
            Some(actor_id),
            "support_mailbox.created",
            AuditOutcome::Success,
            "support_mailbox",
            Some(id),
            request_id,
            json!({ "project_id": project_id, "address": clean.address }),
            now,
        )
        .await?;
        let mailbox = self
            .load_mailbox(&mut tx, workspace_id, project_id, hooks_origin)
            .await?;
        tx.commit().await?;
        Ok(MailboxWithSecret { mailbox, secret })
    }

    async fn load_mailbox(
        &self,
        tx: &mut Transaction<'_, Any>,
        workspace_id: Id,
        project_id: Id,
        hooks_origin: &str,
    ) -> Result<MailboxRecord, TaskError> {
        let row = sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT {MAILBOX_COLUMNS} FROM support_mailboxes WHERE workspace_id = $1 AND project_id = $2"
        )))
        .bind(workspace_id.to_string())
        .bind(project_id.to_string())
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(TaskError::NotFound)?;
        mailbox_from_row(&row, hooks_origin)
    }

    /// Changes the project's mailbox. The inbound secret stays.
    #[allow(clippy::too_many_arguments)]
    pub async fn update_mailbox(
        &self,
        workspace_id: Id,
        project_id: Id,
        actor_id: Id,
        input: &MailboxInput,
        key: &[u8; 32],
        development: bool,
        hooks_origin: &str,
        request_id: &str,
        now: TimestampMillis,
    ) -> Result<MailboxRecord, TaskError> {
        let clean = clean_input(input, key, development)?;
        let mut tx = self.database.immediate_transaction().await?;
        require_manager(&mut tx, workspace_id, project_id, actor_id).await?;
        check_assignee(&mut tx, workspace_id, project_id, input.default_assignee_id).await?;
        let current = self
            .load_mailbox(&mut tx, workspace_id, project_id, hooks_origin)
            .await?;
        let pending = if input.move_to_pending {
            Some(
                ensure_statuses(&mut tx, workspace_id, project_id, actor_id, request_id, now)
                    .await?
                    .1,
            )
        } else {
            None
        };
        // A new host or user without a new password drops the old one: it belongs to the old server.
        let server_changed =
            clean.smtp_host != current.smtp_host || clean.smtp_username != current.smtp_username;
        let password = match clean.smtp_password {
            Some(password) => password,
            None if server_changed => None,
            None => {
                sqlx::query_scalar("SELECT smtp_password FROM support_mailboxes WHERE id = $1")
                    .bind(current.id.to_string())
                    .fetch_one(&mut *tx)
                    .await?
            }
        };
        sqlx::query(
            "UPDATE support_mailboxes SET address = $1, display_name = $2, smtp_host = $3, smtp_port = $4, \
             smtp_security = $5, smtp_username = $6, smtp_password = $7, pending_status_id = $8, \
             default_assignee_id = $9, updated_at = $10 WHERE id = $11",
        )
        .bind(&clean.address)
        .bind(&clean.display_name)
        .bind(clean.smtp_host.as_deref())
        .bind(clean.smtp_port.map(i64::from))
        .bind(clean.smtp_security.as_deref())
        .bind(clean.smtp_username.as_deref())
        .bind(password)
        .bind(pending)
        .bind(input.default_assignee_id.map(|id| id.to_string()))
        .bind(now.as_millis())
        .bind(current.id.to_string())
        .execute(&mut *tx)
        .await?;
        audit::record(
            &mut tx,
            workspace_id,
            Some(actor_id),
            "support_mailbox.updated",
            AuditOutcome::Success,
            "support_mailbox",
            Some(current.id),
            request_id,
            json!({ "project_id": project_id, "address": clean.address }),
            now,
        )
        .await?;
        let mailbox = self
            .load_mailbox(&mut tx, workspace_id, project_id, hooks_origin)
            .await?;
        tx.commit().await?;
        Ok(mailbox)
    }

    /// A new inbound secret; the old URL stops working at once.
    pub async fn rotate_mailbox(
        &self,
        workspace_id: Id,
        project_id: Id,
        actor_id: Id,
        hooks_origin: &str,
        request_id: &str,
        now: TimestampMillis,
    ) -> Result<MailboxWithSecret, TaskError> {
        let mut tx = self.database.immediate_transaction().await?;
        require_manager(&mut tx, workspace_id, project_id, actor_id).await?;
        let current = self
            .load_mailbox(&mut tx, workspace_id, project_id, hooks_origin)
            .await?;
        let (secret, hash) = new_secret();
        sqlx::query("UPDATE support_mailboxes SET secret_hash = $1, rotated_at = $2 WHERE id = $3")
            .bind(hash.to_vec())
            .bind(now.as_millis())
            .bind(current.id.to_string())
            .execute(&mut *tx)
            .await?;
        audit::record(
            &mut tx,
            workspace_id,
            Some(actor_id),
            "support_mailbox.rotated",
            AuditOutcome::Success,
            "support_mailbox",
            Some(current.id),
            request_id,
            json!({ "project_id": project_id }),
            now,
        )
        .await?;
        let mailbox = self
            .load_mailbox(&mut tx, workspace_id, project_id, hooks_origin)
            .await?;
        tx.commit().await?;
        Ok(MailboxWithSecret { mailbox, secret })
    }

    /// Disconnects the mailbox. Its tickets and their emails stay; they can no longer be answered.
    pub async fn delete_mailbox(
        &self,
        workspace_id: Id,
        project_id: Id,
        actor_id: Id,
        request_id: &str,
        now: TimestampMillis,
    ) -> Result<(), TaskError> {
        let mut tx = self.database.immediate_transaction().await?;
        require_manager(&mut tx, workspace_id, project_id, actor_id).await?;
        let id: Option<String> = sqlx::query_scalar(
            "SELECT id FROM support_mailboxes WHERE workspace_id = $1 AND project_id = $2",
        )
        .bind(workspace_id.to_string())
        .bind(project_id.to_string())
        .fetch_optional(&mut *tx)
        .await?;
        let id = parse_id(id.ok_or(TaskError::NotFound)?)?;
        // Queued replies of the mailbox cannot be sent any more.
        sqlx::query(
            "UPDATE support_messages SET send_state = 'failed', send_error = 'The mailbox was disconnected.' \
             WHERE mailbox_id = $1 AND send_state = 'queued'",
        )
        .bind(id.to_string())
        .execute(&mut *tx)
        .await?;
        sqlx::query("DELETE FROM support_mailboxes WHERE id = $1")
            .bind(id.to_string())
            .execute(&mut *tx)
            .await?;
        audit::record(
            &mut tx,
            workspace_id,
            Some(actor_id),
            "support_mailbox.deleted",
            AuditOutcome::Success,
            "support_mailbox",
            Some(id),
            request_id,
            json!({ "project_id": project_id }),
            now,
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// The mailbox whose id and secret match, of a live project in a live workspace. `None` for
    /// an unknown id, a wrong secret or a deleted mailbox alike.
    pub async fn find_mailbox(
        &self,
        mailbox_id: Id,
        secret: &str,
    ) -> Result<Option<InboundTarget>, TaskError> {
        let row = sqlx::query(
            "SELECT m.workspace_id, m.project_id, m.created_by, m.secret_hash, m.open_status_id, \
             m.default_assignee_id FROM support_mailboxes m \
             JOIN workspaces w ON w.id = m.workspace_id JOIN projects p ON p.id = m.project_id \
             WHERE m.id = $1 AND w.deleted_at IS NULL AND p.deleted_at IS NULL",
        )
        .bind(mailbox_id.to_string())
        .fetch_optional(self.database.pool())
        .await?;
        let supplied: [u8; 32] = Sha256::digest(secret.as_bytes()).into();
        let Some(row) = row.filter(|row| {
            bool::from(
                row.get::<Vec<u8>, _>("secret_hash")
                    .as_slice()
                    .ct_eq(&supplied),
            )
        }) else {
            return Ok(None);
        };
        Ok(Some(InboundTarget {
            mailbox_id,
            workspace_id: parse_id(row.get("workspace_id"))?,
            project_id: parse_id(row.get("project_id"))?,
            created_by: parse_id(row.get("created_by"))?,
            open_status_id: row.get("open_status_id"),
            default_assignee_id: optional_id(row.get("default_assignee_id"))?,
        }))
    }

    /// The ticket of `email`: a signed Message-ID of one of Orbit's replies, an earlier email of
    /// the thread, or the subject tag when the sender is the ticket's customer. Only tickets of
    /// this mailbox; a trashed task or one in a Cancelled (Closed) status is no match.
    async fn thread_task(
        tx: &mut Transaction<'_, Any>,
        target: &InboundTarget,
        email: &InboundEmail,
        key: &[u8; 32],
    ) -> Result<Option<Id>, TaskError> {
        let ids: Vec<&String> = email
            .in_reply_to
            .iter()
            .chain(email.references.iter().rev())
            .collect();
        let mut candidates: Vec<Id> = ids
            .iter()
            .filter_map(|id| thread::task_of(key, id))
            .collect();
        for id in &ids {
            if let Some(task) = sqlx::query_scalar::<_, String>(
                "SELECT task_id FROM support_messages WHERE workspace_id = $1 AND mailbox_id = $2 \
                 AND message_id = $3 ORDER BY created_at DESC LIMIT 1",
            )
            .bind(target.workspace_id.to_string())
            .bind(target.mailbox_id.to_string())
            .bind(id.as_str())
            .fetch_optional(&mut **tx)
            .await?
            {
                candidates.push(parse_id(task)?);
            }
        }
        if let Some((key, number)) = thread::subject_tag(&email.subject)
            && let Some(task) = sqlx::query_scalar::<_, String>(
                "SELECT support_tickets.task_id FROM support_tickets \
                 JOIN tasks ON tasks.id = support_tickets.task_id \
                 JOIN projects ON projects.id = tasks.project_id \
                 JOIN support_contacts ON support_contacts.id = support_tickets.contact_id \
                 WHERE support_tickets.mailbox_id = $1 AND upper(projects.project_key) = $2 \
                 AND tasks.number = $3 AND support_contacts.email = $4",
            )
            .bind(target.mailbox_id.to_string())
            .bind(&key)
            .bind(number)
            .bind(&email.from_address)
            .fetch_optional(&mut **tx)
            .await?
        {
            candidates.push(parse_id(task)?);
        }
        for task_id in candidates {
            let row = sqlx::query(
                "SELECT task_statuses.category FROM support_tickets \
                 JOIN tasks ON tasks.id = support_tickets.task_id \
                 JOIN task_statuses ON task_statuses.id = tasks.status_id \
                 WHERE support_tickets.task_id = $1 AND support_tickets.mailbox_id = $2 \
                 AND tasks.workspace_id = $3 AND tasks.deleted_at IS NULL",
            )
            .bind(task_id.to_string())
            .bind(target.mailbox_id.to_string())
            .bind(target.workspace_id.to_string())
            .fetch_optional(&mut **tx)
            .await?;
            if let Some(row) = row
                && row.get::<String, _>("category") != StatusCategory::Cancelled.as_str()
            {
                return Ok(Some(task_id));
            }
        }
        Ok(None)
    }

    /// Files an inbound email: a reply goes to its ticket (a Solved or Pending ticket moves back to
    /// Open), any other email makes a new ticket in Open. Attachments become task attachments of
    /// the ticket. The ticket's assignees and followers are notified. The caller dropped automatic
    /// mail (`InboundEmail::automatic`) already.
    pub async fn receive(
        &self,
        uploads: &UploadService,
        target: &InboundTarget,
        email: &InboundEmail,
        key: &[u8; 32],
        request_id: &str,
    ) -> Result<Received, TaskError> {
        // CPU work first, outside the write transaction: the type of each attachment, which ones
        // the HTML shows inline, and the cleaned HTML. Inline images get a placeholder of this
        // email (a random nonce) that the transaction swaps for their attachment's URL.
        let nonce = Id::new_v7().to_string();
        let placeholder = |index: usize| format!("/__orbit_inline__/{nonce}/{index}");
        let prepared: Vec<(String, Option<&String>)> = email
            .attachments
            .iter()
            .map(|attachment| {
                let media_type =
                    orbit_platform::detect_media_type(&attachment.bytes, &attachment.name)
                        .to_owned();
                let inline_cid = attachment.content_id.as_ref().filter(|cid| {
                    orbit_platform::INLINE_IMAGE_TYPES.contains(&media_type.as_str())
                        && email
                            .html
                            .as_deref()
                            .is_some_and(|html| html.contains(cid.as_str()))
                });
                (media_type, inline_cid)
            })
            .collect();
        let inline_images: std::collections::HashMap<String, String> = prepared
            .iter()
            .enumerate()
            .filter_map(|(index, (_, cid))| cid.map(|cid| (cid.clone(), placeholder(index))))
            .collect();
        let mut body_html = match email.html.clone() {
            Some(raw) => tokio::task::spawn_blocking(move || html::sanitize(&raw, inline_images))
                .await
                .map_err(|_| TaskError::Conflict)
                .map(Some)?
                .filter(|html| !html.trim().is_empty()),
            None => None,
        };

        let finalization = uploads.begin_finalization().await;
        let now = self.database.database_now().await?;
        let mut tx = self.database.immediate_transaction().await?;
        let Flag(known) = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM support_messages WHERE workspace_id = $1 AND mailbox_id = $2 AND message_id = $3)",
        )
        .bind(target.workspace_id.to_string())
        .bind(target.mailbox_id.to_string())
        .bind(&email.message_id)
        .fetch_one(&mut *tx)
        .await?;
        if known {
            return Ok(Received::Duplicate);
        }
        let account =
            service_account_in_tx(&mut tx, target.workspace_id, target.created_by, now).await?;
        sqlx::query(
            "INSERT INTO support_contacts (id, workspace_id, email, name, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $5) ON CONFLICT (workspace_id, email) DO UPDATE SET \
             name = CASE WHEN excluded.name <> '' THEN excluded.name ELSE support_contacts.name END, \
             updated_at = excluded.updated_at",
        )
        .bind(Id::new_v7().to_string())
        .bind(target.workspace_id.to_string())
        .bind(&email.from_address)
        .bind(&email.from_name)
        .bind(now.as_millis())
        .execute(&mut *tx)
        .await?;
        let contact_id: String = sqlx::query_scalar(
            "SELECT id FROM support_contacts WHERE workspace_id = $1 AND email = $2",
        )
        .bind(target.workspace_id.to_string())
        .bind(&email.from_address)
        .fetch_one(&mut *tx)
        .await?;

        let open = live_status(&mut tx, target.project_id, Some(&target.open_status_id)).await?;
        let existing = Self::thread_task(&mut tx, target, email, key).await?;
        let (task_id, created) = match existing {
            Some(task_id) => {
                let task = task_in_tx(
                    &mut tx,
                    ProjectScope::All,
                    target.workspace_id,
                    task_id,
                    false,
                )
                .await?;
                let category = status_category(&mut tx, task.status_id).await?;
                let pending: Option<String> = sqlx::query_scalar(
                    "SELECT pending_status_id FROM support_mailboxes WHERE id = $1",
                )
                .bind(target.mailbox_id.to_string())
                .fetch_one(&mut *tx)
                .await?;
                let waiting = pending.as_deref() == Some(task.status_id.to_string().as_str());
                // The open status of the ticket's own project (it may have moved).
                let reopen_to = if task.project_id == target.project_id {
                    open
                } else {
                    None
                };
                if (category == StatusCategory::Completed.as_str() || waiting)
                    && let Some(open) = reopen_to
                {
                    move_ticket(
                        &mut tx,
                        target.workspace_id,
                        task_id,
                        open,
                        target.created_by,
                        Some(account),
                        request_id,
                        now,
                    )
                    .await?;
                }
                (task_id, false)
            }
            None => {
                let status_id = match open {
                    Some(open) => open,
                    None => crate::repositories::task_relations::default_status_id_in_tx(
                        &mut tx,
                        target.workspace_id,
                        target.project_id,
                    )
                    .await?
                    .ok_or(TaskError::NotFound)?,
                };
                let task_id = Id::new_v7();
                let title = if email.subject.is_empty() {
                    format!("Email from {}", email.from_address)
                } else {
                    email.subject.clone()
                };
                let position: i64 = sqlx::query_scalar(
                    "SELECT COALESCE(MAX(position) + 1, 0) FROM tasks WHERE workspace_id = $1 AND project_id = $2 \
                     AND status_id = $3 AND deleted_at IS NULL",
                )
                .bind(target.workspace_id.to_string())
                .bind(target.project_id.to_string())
                .bind(status_id.to_string())
                .fetch_one(&mut *tx)
                .await?;
                sqlx::query(
                    "INSERT INTO tasks (id, workspace_id, project_id, status_id, title, description, priority, position, \
                     creator_id, creator_service_account_id, version, created_at, updated_at) \
                     VALUES ($1, $2, $3, $4, $5, '', 'none', $6, $7, $8, 0, $9, $9)",
                )
                .bind(task_id.to_string())
                .bind(target.workspace_id.to_string())
                .bind(target.project_id.to_string())
                .bind(status_id.to_string())
                .bind(&title)
                .bind(position)
                .bind(target.created_by.to_string())
                .bind(account.to_string())
                .bind(now.as_millis())
                .execute(&mut *tx)
                .await?;
                sqlx::query(
                    "INSERT INTO support_tickets (task_id, workspace_id, mailbox_id, contact_id, subject, created_at) \
                     VALUES ($1, $2, $3, $4, $5, $6)",
                )
                .bind(task_id.to_string())
                .bind(target.workspace_id.to_string())
                .bind(target.mailbox_id.to_string())
                .bind(&contact_id)
                .bind(&email.subject)
                .bind(now.as_millis())
                .execute(&mut *tx)
                .await?;
                record_principal_mutation(
                    &mut tx,
                    target.workspace_id,
                    target.created_by,
                    Some((account, SERVICE_ACCOUNT)),
                    "task.created",
                    "task",
                    task_id,
                    request_id,
                    now,
                )
                .await?;
                if let Some(assignee) = target.default_assignee_id {
                    assign_default(&mut tx, target, task_id, assignee, now).await?;
                }
                (task_id, true)
            }
        };

        // Attachments: task attachments of the ticket, owned by the mailbox's creator.
        let mut stored = StoredAttachments {
            items: Vec::new(),
            skipped: u32::try_from(email.skipped_attachments).unwrap_or(u32::MAX),
        };
        for (index, (attachment, (media_type, inline_cid))) in
            email.attachments.iter().zip(prepared).enumerate()
        {
            let blob = match finalization
                .finalize_bytes_in_transaction(&mut tx, target.workspace_id, &attachment.bytes, now)
                .await
            {
                Ok(blob) => blob,
                Err(orbit_platform::UploadError::StorageQuotaExceeded { .. }) => {
                    stored.skipped += 1;
                    continue;
                }
                Err(error) => {
                    tracing::warn!(%error, "support attachment could not be stored");
                    return Err(TaskError::Conflict);
                }
            };
            let id = Id::new_v7();
            let inline = inline_cid.is_some();
            sqlx::query(
                "INSERT INTO attachment_references (id, workspace_id, task_id, comment_id, owner_id, blob_id, \
                 display_name, media_type, byte_size, created_at, inline) \
                 VALUES ($1, $2, $3, NULL, $4, $5, $6, $7, $8, $9, $10)",
            )
            .bind(id.to_string())
            .bind(target.workspace_id.to_string())
            .bind(task_id.to_string())
            .bind(target.created_by.to_string())
            .bind(blob.id.to_string())
            .bind(&attachment.name)
            .bind(&media_type)
            .bind(i64::try_from(attachment.bytes.len()).unwrap_or(i64::MAX))
            .bind(now.as_millis())
            .bind(i64::from(inline))
            .execute(&mut *tx)
            .await?;
            if inline && let Some(html) = body_html.as_mut() {
                *html = html.replace(
                    &placeholder(index),
                    &format!(
                        "/api/v1/workspaces/{}/tasks/{task_id}/attachments/{id}/download",
                        target.workspace_id
                    ),
                );
            }
            stored.items.push(MessageAttachment {
                id,
                name: attachment.name.clone(),
                media_type,
                size: attachment.bytes.len() as u64,
                inline,
            });
        }

        // An inline image that was not stored (storage quota) loses its source.
        if let Some(html) = body_html.as_mut() {
            for index in 0..email.attachments.len() {
                *html = html.replace(&placeholder(index), "");
            }
        }

        let message_id = Id::new_v7();
        sqlx::query(
            "INSERT INTO support_messages (id, workspace_id, task_id, mailbox_id, direction, author_id, from_address, \
             from_name, to_addresses, cc_addresses, subject, message_id, in_reply_to, references_header, body_text, \
             body_html, attachments_json, unverified, send_state, created_at) \
             VALUES ($1, $2, $3, $4, 'in', NULL, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, NULL, $17)",
        )
        .bind(message_id.to_string())
        .bind(target.workspace_id.to_string())
        .bind(task_id.to_string())
        .bind(target.mailbox_id.to_string())
        .bind(&email.from_address)
        .bind(&email.from_name)
        .bind(email.to.join(","))
        .bind(email.cc.join(","))
        .bind(&email.subject)
        .bind(&email.message_id)
        .bind(email.in_reply_to.first().map(String::as_str))
        .bind(email.references.join(" "))
        .bind(&email.text)
        .bind(body_html)
        .bind(serde_json::to_string(&stored).map_err(|_| TaskError::Conflict)?)
        .bind(i64::from(email.unverified))
        .bind(now.as_millis())
        .execute(&mut *tx)
        .await?;
        sqlx::query("UPDATE support_mailboxes SET last_received_at = $1 WHERE id = $2")
            .bind(now.as_millis())
            .bind(target.mailbox_id.to_string())
            .execute(&mut *tx)
            .await?;
        notify_people(
            &mut tx,
            target.workspace_id,
            task_id,
            message_id,
            &email.from_address,
            now,
        )
        .await?;
        record_message_event(
            &mut tx,
            target.workspace_id,
            None,
            "support_message.received",
            message_id,
            task_id,
            request_id,
            now,
        )
        .await?;
        tx.commit().await?;
        drop(finalization);
        Ok(if created {
            Received::Created(task_id)
        } else {
            Received::Added(task_id)
        })
    }

    /// The emails of a task the actor sees, with its customer when it is a ticket.
    pub async fn ticket(
        &self,
        workspace_id: Id,
        actor_id: Id,
        task_id: Id,
    ) -> Result<TicketView, TaskError> {
        let mut tx = self.database.pool().begin().await?;
        let actor = require_access_tx_read(&mut tx, workspace_id, actor_id).await?;
        let scope = ProjectScope::of(actor);
        task_in_tx(&mut tx, scope, workspace_id, task_id, false).await?;
        let messages = sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT {MESSAGE_COLUMNS} FROM support_messages WHERE workspace_id = $1 AND task_id = $2 \
             ORDER BY created_at, id"
        )))
        .bind(workspace_id.to_string())
        .bind(task_id.to_string())
        .fetch_all(&mut *tx)
        .await?
        .iter()
        .map(message_from_row)
        .collect::<Result<Vec<_>, _>>()?;
        let row = sqlx::query(
            "SELECT support_contacts.id, support_contacts.email, support_contacts.name, \
             support_mailboxes.address, support_mailboxes.smtp_host FROM support_tickets \
             JOIN support_contacts ON support_contacts.id = support_tickets.contact_id \
             LEFT JOIN support_mailboxes ON support_mailboxes.id = support_tickets.mailbox_id \
             WHERE support_tickets.task_id = $1 AND support_tickets.workspace_id = $2",
        )
        .bind(task_id.to_string())
        .bind(workspace_id.to_string())
        .fetch_optional(&mut *tx)
        .await?;
        let ticket = match row {
            None => None,
            Some(row) => {
                let contact_id = parse_id(row.get("id"))?;
                let others = sqlx::query(sqlx::AssertSqlSafe(format!(
                    "SELECT tasks.id, tasks.project_id, tasks.number, tasks.title, tasks.status_id, tasks.created_at, \
                     projects.project_key FROM support_tickets JOIN tasks ON tasks.id = support_tickets.task_id \
                     JOIN projects ON projects.id = tasks.project_id \
                     WHERE support_tickets.contact_id = $1 AND support_tickets.workspace_id = $2 \
                     AND tasks.id <> $3 AND tasks.deleted_at IS NULL AND projects.deleted_at IS NULL AND {} \
                     ORDER BY tasks.created_at DESC LIMIT {OTHER_TICKETS}",
                    scope.project("tasks.project_id")
                )))
                .bind(contact_id.to_string())
                .bind(workspace_id.to_string())
                .bind(task_id.to_string())
                .fetch_all(&mut *tx)
                .await?
                .iter()
                .map(|row| {
                    Ok(OtherTicket {
                        task_id: parse_id(row.get("id"))?,
                        project_id: parse_id(row.get("project_id"))?,
                        identifier: format!(
                            "{}-{}",
                            row.get::<String, _>("project_key"),
                            row.get::<Option<i64>, _>("number").unwrap_or_default()
                        ),
                        title: row.get("title"),
                        status_id: parse_id(row.get("status_id"))?,
                        created_at: TimestampMillis::from_millis(row.get("created_at")),
                    })
                })
                .collect::<Result<Vec<_>, TaskError>>()?;
                let mailbox_address: Option<String> = row.get("address");
                Some(TicketInfo {
                    contact: ContactRecord {
                        id: contact_id,
                        email: row.get("email"),
                        name: row.get("name"),
                    },
                    can_reply: mailbox_address.is_some()
                        && row.get::<Option<String>, _>("smtp_host").is_some(),
                    mailbox_address,
                    other_tickets: others,
                })
            }
        };
        tx.commit().await?;
        Ok(TicketView { ticket, messages })
    }

    /// Queues a reply to the ticket's customer (plain text) and, when the mailbox says so, moves
    /// the ticket to Pending. The `support.send` job sends it.
    #[allow(clippy::too_many_arguments)]
    pub async fn reply(
        &self,
        workspace_id: Id,
        actor_id: Id,
        task_id: Id,
        body: &str,
        key: &[u8; 32],
        request_id: &str,
        now: TimestampMillis,
    ) -> Result<MessageRecord, TaskError> {
        let body = body.trim();
        if body.is_empty() || body.chars().count() > MAX_REPLY_CHARS {
            return Err(TaskError::Invalid { field: "body" });
        }
        let mut tx = self.database.immediate_transaction().await?;
        let actor = require_access_tx(&mut tx, workspace_id, actor_id).await?;
        let task = task_in_tx(
            &mut tx,
            ProjectScope::of(actor),
            workspace_id,
            task_id,
            false,
        )
        .await?;
        let row = sqlx::query(
            "SELECT support_tickets.subject, support_contacts.email, support_mailboxes.id AS mailbox_id, \
             support_mailboxes.address, support_mailboxes.display_name, support_mailboxes.pending_status_id, \
             support_mailboxes.smtp_host, projects.project_key FROM support_tickets \
             JOIN support_contacts ON support_contacts.id = support_tickets.contact_id \
             JOIN support_mailboxes ON support_mailboxes.id = support_tickets.mailbox_id \
             JOIN projects ON projects.id = $3 \
             WHERE support_tickets.task_id = $1 AND support_tickets.workspace_id = $2",
        )
        .bind(task_id.to_string())
        .bind(workspace_id.to_string())
        .bind(task.project_id.to_string())
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(TaskError::Conflict)?;
        if row.get::<Option<String>, _>("smtp_host").is_none() {
            return Err(TaskError::Conflict);
        }
        let to: String = row.get("email");
        if !valid_address(&to) {
            return Err(TaskError::Invalid { field: "to" });
        }
        let address: String = row.get("address");
        let identifier = format!("{}-{}", row.get::<String, _>("project_key"), task.number);
        let subject = thread::reply_subject(&row.get::<String, _>("subject"), &identifier);
        let last_inbound = sqlx::query(
            "SELECT message_id, references_header FROM support_messages WHERE task_id = $1 AND direction = 'in' \
             ORDER BY created_at DESC, id DESC LIMIT 1",
        )
        .bind(task_id.to_string())
        .fetch_optional(&mut *tx)
        .await?;
        let (in_reply_to, references) = match &last_inbound {
            Some(last) => {
                let parent: String = last.get("message_id");
                let mut chain: Vec<String> = last
                    .get::<String, _>("references_header")
                    .split_whitespace()
                    .map(str::to_owned)
                    .collect();
                chain.push(parent.clone());
                let keep = chain.len().saturating_sub(MAX_REFERENCES);
                (Some(parent), chain[keep..].join(" "))
            }
            None => (None, String::new()),
        };
        let mailbox_id = parse_id(row.get("mailbox_id"))?;
        let id = Id::new_v7();
        sqlx::query(
            "INSERT INTO support_messages (id, workspace_id, task_id, mailbox_id, direction, author_id, from_address, \
             from_name, to_addresses, cc_addresses, subject, message_id, in_reply_to, references_header, body_text, \
             body_html, attachments_json, unverified, send_state, created_at) \
             VALUES ($1, $2, $3, $4, 'out', $5, $6, $7, $8, '', $9, $10, $11, $12, $13, NULL, '{}', 0, 'queued', $14)",
        )
        .bind(id.to_string())
        .bind(workspace_id.to_string())
        .bind(task_id.to_string())
        .bind(mailbox_id.to_string())
        .bind(actor_id.to_string())
        .bind(&address)
        .bind(row.get::<String, _>("display_name"))
        .bind(&to)
        .bind(&subject)
        .bind(thread::message_id(key, task_id, domain_of(&address)))
        .bind(in_reply_to)
        .bind(references)
        .bind(body)
        .bind(now.as_millis())
        .execute(&mut *tx)
        .await?;
        enqueue_send(&mut tx, workspace_id, id, now).await?;
        if let Some(pending) = live_status(
            &mut tx,
            task.project_id,
            row.get::<Option<String>, _>("pending_status_id").as_deref(),
        )
        .await?
        {
            move_ticket(
                &mut tx,
                workspace_id,
                task_id,
                pending,
                actor_id,
                None,
                request_id,
                now,
            )
            .await?;
        }
        record_message_event(
            &mut tx,
            workspace_id,
            Some(actor_id),
            "support_message.created",
            id,
            task_id,
            request_id,
            now,
        )
        .await?;
        let message = load_message(&mut tx, workspace_id, id).await?;
        tx.commit().await?;
        Ok(message)
    }

    /// Queues a failed reply again.
    pub async fn retry(
        &self,
        workspace_id: Id,
        actor_id: Id,
        message_id: Id,
        request_id: &str,
        now: TimestampMillis,
    ) -> Result<MessageRecord, TaskError> {
        let mut tx = self.database.immediate_transaction().await?;
        let actor = require_access_tx(&mut tx, workspace_id, actor_id).await?;
        let message = load_message(&mut tx, workspace_id, message_id).await?;
        // The task's access decides, as for its other content.
        task_in_tx(
            &mut tx,
            ProjectScope::of(actor),
            workspace_id,
            message.task_id,
            false,
        )
        .await?;
        if message.send_state.as_deref() != Some("failed") {
            return Err(TaskError::Conflict);
        }
        let Flag(connected) = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM support_messages JOIN support_mailboxes ON support_mailboxes.id = support_messages.mailbox_id \
             WHERE support_messages.id = $1 AND support_mailboxes.smtp_host IS NOT NULL)",
        )
        .bind(message_id.to_string())
        .fetch_one(&mut *tx)
        .await?;
        if !connected {
            return Err(TaskError::Conflict);
        }
        sqlx::query(
            "UPDATE support_messages SET send_state = 'queued', send_error = NULL WHERE id = $1",
        )
        .bind(message_id.to_string())
        .execute(&mut *tx)
        .await?;
        enqueue_send(&mut tx, workspace_id, message_id, now).await?;
        record_message_event(
            &mut tx,
            workspace_id,
            Some(actor_id),
            "support_message.retried",
            message_id,
            message.task_id,
            request_id,
            now,
        )
        .await?;
        let message = load_message(&mut tx, workspace_id, message_id).await?;
        tx.commit().await?;
        Ok(message)
    }

    /// A queued reply with its mailbox's SMTP settings, for the send job; `None` once it is no
    /// longer queued.
    pub async fn outgoing(&self, message_id: Id) -> Result<Option<Outgoing>, TaskError> {
        let row = sqlx::query(
            "SELECT m.id, m.workspace_id, m.mailbox_id, m.from_address, m.from_name, m.to_addresses, m.subject, \
             m.body_text, m.message_id, m.in_reply_to, m.references_header, b.smtp_host, b.smtp_port, \
             b.smtp_security, b.smtp_username, b.smtp_password FROM support_messages m \
             JOIN support_mailboxes b ON b.id = m.mailbox_id \
             WHERE m.id = $1 AND m.direction = 'out' AND m.send_state = 'queued'",
        )
        .bind(message_id.to_string())
        .fetch_optional(self.database.pool())
        .await?;
        row.map(|row| {
            Ok(Outgoing {
                message_id,
                workspace_id: parse_id(row.get("workspace_id"))?,
                mailbox_id: parse_id(row.get("mailbox_id"))?,
                from_address: row.get("from_address"),
                from_name: row.get("from_name"),
                to: row.get("to_addresses"),
                subject: row.get("subject"),
                body: row.get("body_text"),
                header_id: row.get("message_id"),
                in_reply_to: row.get("in_reply_to"),
                references: row.get("references_header"),
                smtp_host: row.get("smtp_host"),
                smtp_port: row
                    .get::<Option<i64>, _>("smtp_port")
                    .and_then(|port| u16::try_from(port).ok()),
                smtp_security: row.get("smtp_security"),
                smtp_username: row.get("smtp_username"),
                smtp_password: row.get("smtp_password"),
            })
        })
        .transpose()
    }

    /// Records the result of a send attempt: `Ok` sent, `Err((text, last))` failed (`last`: no
    /// retry follows, so the reply shows "Not sent").
    pub async fn finish_send(
        &self,
        message_id: Id,
        result: Result<(), (String, bool)>,
    ) -> Result<(), TaskError> {
        let now = TimestampMillis::now();
        let mut tx = self.database.immediate_transaction().await?;
        let row = sqlx::query("SELECT workspace_id, task_id FROM support_messages WHERE id = $1 AND send_state = 'queued'")
            .bind(message_id.to_string())
            .fetch_optional(&mut *tx)
            .await?;
        let Some(row) = row else {
            return Ok(());
        };
        let workspace_id = parse_id(row.get("workspace_id"))?;
        let task_id = parse_id(row.get("task_id"))?;
        let action = match &result {
            Ok(()) => {
                sqlx::query("UPDATE support_messages SET send_state = 'sent', send_error = NULL, sent_at = $1 WHERE id = $2")
                    .bind(now.as_millis())
                    .bind(message_id.to_string())
                    .execute(&mut *tx)
                    .await?;
                "support_message.sent"
            }
            Err((error, last)) => {
                let error: String = error.chars().take(500).collect();
                sqlx::query(
                    "UPDATE support_messages SET send_state = $1, send_error = $2 WHERE id = $3",
                )
                .bind(if *last { "failed" } else { "queued" })
                .bind(error)
                .bind(message_id.to_string())
                .execute(&mut *tx)
                .await?;
                if !*last {
                    tx.commit().await?;
                    return Ok(());
                }
                "support_message.failed"
            }
        };
        record_message_event(
            &mut tx,
            workspace_id,
            None,
            action,
            message_id,
            task_id,
            "job",
            now,
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }
}

/// The actor for a read: guests and outsiders are `NotFound`, as for every task read.
async fn require_access_tx_read(
    tx: &mut Transaction<'_, Any>,
    workspace_id: Id,
    actor_id: Id,
) -> Result<orbit_domain::Actor, TaskError> {
    membership::actor(&mut **tx, workspace_id, actor_id)
        .await?
        .ok_or(TaskError::NotFound)
}

async fn load_message(
    tx: &mut Transaction<'_, Any>,
    workspace_id: Id,
    message_id: Id,
) -> Result<MessageRecord, TaskError> {
    let row = sqlx::query(sqlx::AssertSqlSafe(format!(
        "SELECT {MESSAGE_COLUMNS} FROM support_messages WHERE id = $1 AND workspace_id = $2"
    )))
    .bind(message_id.to_string())
    .bind(workspace_id.to_string())
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(TaskError::NotFound)?;
    message_from_row(&row)
}

async fn enqueue_send(
    tx: &mut Transaction<'_, Any>,
    workspace_id: Id,
    message_id: Id,
    now: TimestampMillis,
) -> Result<(), TaskError> {
    let job = Job::new(
        outbound::job_kind(),
        json!({ "message_id": message_id }),
        now,
    )
    .with_workspace(workspace_id.to_string());
    JobStore::enqueue_in(tx, &job)
        .await
        .map_err(|_| TaskError::Conflict)?;
    Ok(())
}

/// Assigns a new ticket to the mailbox's default assignee, when they can still be assigned.
async fn assign_default(
    tx: &mut Transaction<'_, Any>,
    target: &InboundTarget,
    task_id: Id,
    user_id: Id,
    now: TimestampMillis,
) -> Result<(), TaskError> {
    let membership: Option<String> = sqlx::query_scalar(
        "SELECT memberships.id FROM memberships JOIN users ON users.id = memberships.user_id \
         WHERE memberships.workspace_id = $1 AND memberships.user_id = $2 AND memberships.role <> 'guest' \
         AND users.suspended_at IS NULL",
    )
    .bind(target.workspace_id.to_string())
    .bind(user_id.to_string())
    .fetch_optional(&mut **tx)
    .await?;
    let Some(membership) = membership else {
        return Ok(());
    };
    if !user_sees_project(&mut **tx, target.project_id, user_id).await? {
        return Ok(());
    }
    sqlx::query("INSERT INTO task_assignees (task_id, membership_id, user_id) VALUES ($1, $2, $3)")
        .bind(task_id.to_string())
        .bind(membership)
        .bind(user_id.to_string())
        .execute(&mut **tx)
        .await?;
    super::task_followers::follow_in_tx(tx, task_id, &[user_id], now).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::smtp_host_allowed;

    #[test]
    fn smtp_hosts_must_be_public_smtp_servers_outside_development() {
        assert!(smtp_host_allowed("smtp.example.com", 587, false));
        assert!(smtp_host_allowed("smtp.example.com", 465, false));
        assert!(!smtp_host_allowed("smtp.example.com", 6379, false));
        assert!(!smtp_host_allowed("127.0.0.1", 25, false));
        assert!(!smtp_host_allowed("10.0.0.5", 587, false));
        assert!(!smtp_host_allowed("[::1]", 587, false));
        assert!(!smtp_host_allowed("localhost", 587, false));
        assert!(!smtp_host_allowed("mail.internal", 587, false));
        assert!(!smtp_host_allowed("smtp.example.com/x", 587, false));
        assert!(smtp_host_allowed("127.0.0.1", 1025, true));
        assert!(!smtp_host_allowed("", 1025, true));
    }
}
