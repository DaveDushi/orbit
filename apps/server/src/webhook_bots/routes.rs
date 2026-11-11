//! The bot routes. Owners and admins of a workspace (`workspace.update`) list, make, change and
//! delete its bots, rotate a bot's signing secret (shown once) and read its newest deliveries. A bot
//! answers with `POST /api/v1/hooks/bots/{bot_id}/replies/{token}` and no session: the single-use
//! token in the path is the credential, so error bodies never name it. Like the agent routes,
//! these are not in the OpenAPI contract.

use axum::body::to_bytes;
use axum::extract::{Extension, Multipart, Path, Request, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::routing::{get, patch, post, put};
use axum::{Json, Router};
use orbit_domain::Permission;
use orbit_platform::{
    Database, Flag, Id, PasswordService, RequestId, Row, TimestampMillis, generate_opaque_token,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::AnyConnection;

use super::{BotState, MAX_ANSWER_BYTES, new_secret};
use crate::agent::outbound::check_url;
use crate::agent::settings::checked_name;
use crate::audit::{self, AuditOutcome};
use crate::repositories::membership;
use crate::secret_box::encrypt_secret;
use crate::task_routes::{ApiError, ApiJson, authenticate_session, request_id_value};

type RequestIdExtension = Option<Extension<RequestId>>;

/// Bots one workspace may have (deleted ones do not count).
pub const MAX_BOTS: i64 = 20;
/// Channels one bot may list.
pub const MAX_CHANNELS: usize = 100;

pub fn router(state: BotState) -> Router {
    let base = "/api/v1/workspaces/{workspace_id}/bots";
    Router::new()
        .route(base, get(list_bots).post(create_bot))
        .route(
            &format!("{base}/{{bot_id}}"),
            patch(update_bot).delete(delete_bot),
        )
        .route(&format!("{base}/{{bot_id}}/secret"), post(rotate_secret))
        .route(
            &format!("{base}/{{bot_id}}/avatar"),
            put(upload_avatar).delete(remove_avatar),
        )
        .route(
            &format!("{base}/{{bot_id}}/deliveries"),
            get(list_deliveries),
        )
        .route(
            "/api/v1/hooks/bots/{bot_id}/replies/{token}",
            post(post_reply),
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
    tracing::warn!(%error, "webhook bots");
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
        "Only owners and admins can manage bots.",
        uri,
        request_id,
    )
}

/// The caller as an owner or admin of the workspace: 404 for people outside it and for guests, 403
/// for members.
async fn manager(
    state: &BotState,
    headers: &HeaderMap,
    workspace_id: &str,
    uri: &Uri,
    request_id: &RequestIdExtension,
) -> Result<(Id, Id), ApiError> {
    let session = authenticate_session(
        &state.chat.identity,
        state.chat.cookie_mode,
        headers,
        uri.path(),
        request_id.as_ref(),
    )
    .await?;
    let workspace_id: Id = workspace_id
        .parse()
        .map_err(|_| not_found(uri, request_id))?;
    let actor = membership::actor(state.database().pool(), workspace_id, session.user.id)
        .await
        .map_err(|error| unavailable(&error, uri, request_id))?
        .ok_or_else(|| not_found(uri, request_id))?;
    if !actor.can(Permission::WorkspaceUpdate) {
        return Err(forbidden(uri, request_id));
    }
    Ok((workspace_id, actor.user_id))
}

fn app_key(
    state: &BotState,
    uri: &Uri,
    request_id: &RequestIdExtension,
) -> Result<[u8; 32], ApiError> {
    state.app_key.ok_or_else(|| {
        problem(
            StatusCode::SERVICE_UNAVAILABLE,
            "app_key_missing",
            "Bots need the server's app key (ORBIT_APP_KEY).",
            uri,
            request_id,
        )
    })
}

#[derive(Serialize)]
struct ChannelView {
    id: String,
    name: String,
    kind: String,
}

#[derive(Serialize)]
struct BotView {
    id: String,
    user_id: String,
    name: String,
    avatar_url: Option<String>,
    url: String,
    channels: Vec<ChannelView>,
    created_at: i64,
    updated_at: i64,
    secret_rotated_at: Option<i64>,
}

async fn bot_views(
    database: &Database,
    workspace_id: Id,
    only: Option<Id>,
) -> Result<Vec<BotView>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT b.id, b.user_id, u.display_name, a.updated_at AS avatar_at, b.url, b.created_at, \
         b.updated_at, b.secret_rotated_at FROM webhook_bots b JOIN users u ON u.id = b.user_id \
         LEFT JOIN user_avatars a ON a.user_id = b.user_id \
         WHERE b.deleted_at IS NULL AND b.workspace_id = $1 AND ($2 = '' OR b.id = $2) \
         ORDER BY lower(u.display_name), b.id",
    )
    .bind(workspace_id.to_string())
    .bind(only.map(|id| id.to_string()).unwrap_or_default())
    .fetch_all(database.pool())
    .await?;
    let mut views = Vec::with_capacity(rows.len());
    for row in rows {
        let id: String = row.get("id");
        let user_id: String = row.get("user_id");
        let channels = sqlx::query(
            "SELECT c.id, c.name, c.kind FROM webhook_bot_channels b \
             JOIN chat_conversations c ON c.id = b.conversation_id WHERE b.bot_id = $1 ORDER BY c.name",
        )
        .bind(&id)
        .fetch_all(database.pool())
        .await?
        .iter()
        .map(|channel| ChannelView {
            id: channel.get("id"),
            name: channel.get::<Option<String>, _>("name").unwrap_or_default(),
            kind: channel.get("kind"),
        })
        .collect();
        views.push(BotView {
            avatar_url: user_id.parse().ok().and_then(|user: Id| {
                crate::auth_routes::avatar_url(user, row.get::<Option<i64>, _>("avatar_at"))
            }),
            id,
            user_id,
            name: row.get("display_name"),
            url: row.get("url"),
            channels,
            created_at: row.get("created_at"),
            updated_at: row.get("updated_at"),
            secret_rotated_at: row.get("secret_rotated_at"),
        });
    }
    Ok(views)
}

/// Every live bot of the workspace.
async fn list_bots(
    State(state): State<BotState>,
    Path(workspace_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
) -> Result<Json<Value>, ApiError> {
    let (workspace_id, _) = manager(&state, &headers, &workspace_id, &uri, &request_id).await?;
    let bots = bot_views(state.database(), workspace_id, None)
        .await
        .map_err(|error| unavailable(&error, &uri, &request_id))?;
    Ok(Json(json!({
        "bots": bots,
        "max_bots": MAX_BOTS,
        "reply_window_seconds": state.window.as_secs(),
    })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BotBody {
    name: Option<String>,
    url: Option<String>,
    channel_ids: Option<Vec<Id>>,
}

enum SaveError {
    Invalid(&'static str),
    Taken,
    NotFound,
    Forbidden,
    Limit,
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
            "bot_name_taken",
            "Someone in Orbit already has this name. Mentions of the bot would reach them, so pick another name.",
            uri,
            request_id,
        ),
        SaveError::NotFound => not_found(uri, request_id),
        SaveError::Forbidden => forbidden(uri, request_id),
        SaveError::Limit => problem(
            StatusCode::CONFLICT,
            "limit_reached",
            "This workspace has 20 bots. Delete one first.",
            uri,
            request_id,
        ),
        SaveError::Database(error) => unavailable(&error, uri, request_id),
    }
}

/// The URL as saved: https to a public host (an IP must be public; host names are checked again at
/// each call, after DNS), at most 2,048 characters.
fn checked_url(state: &BotState, raw: &str) -> Result<String, &'static str> {
    if raw.trim().len() > 2_048 {
        return Err("The URL may have at most 2,048 characters.");
    }
    check_url(raw, state.outbound.test_hosts())
        .map(|url| url.to_string())
        .map_err(|error| match error {
            crate::agent::outbound::OutboundError::Refused(detail) => detail,
            _ => "The URL is not valid.",
        })
}

/// Rechecks in the write transaction that the caller still manages the workspace.
async fn still_manager(
    conn: &mut AnyConnection,
    workspace_id: Id,
    actor: Id,
) -> Result<(), SaveError> {
    let allowed = membership::actor(&mut *conn, workspace_id, actor)
        .await?
        .is_some_and(|actor| actor.can(Permission::WorkspaceUpdate));
    if allowed {
        Ok(())
    } else {
        Err(SaveError::Forbidden)
    }
}

/// Joins the bot user to the listed channels and takes it out of channels it left. The channels
/// must be in the workspace, and a private one must have the caller in it.
async fn set_channels(
    conn: &mut AnyConnection,
    workspace_id: Id,
    actor: Id,
    bot_id: Id,
    bot_user: Id,
    channel_ids: &[Id],
    now: TimestampMillis,
) -> Result<(), SaveError> {
    if channel_ids.len() > MAX_CHANNELS {
        return Err(SaveError::Invalid("A bot can have at most 100 channels."));
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
    let old: Vec<String> =
        sqlx::query_scalar("SELECT conversation_id FROM webhook_bot_channels WHERE bot_id = $1")
            .bind(bot_id.to_string())
            .fetch_all(&mut *conn)
            .await?;
    for gone in old.iter().filter(|id| {
        !channel_ids
            .iter()
            .any(|channel| channel.to_string() == **id)
    }) {
        sqlx::query("DELETE FROM webhook_bot_channels WHERE bot_id = $1 AND conversation_id = $2")
            .bind(bot_id.to_string())
            .bind(gone)
            .execute(&mut *conn)
            .await?;
        sqlx::query("DELETE FROM chat_members WHERE conversation_id = $1 AND user_id = $2")
            .bind(gone)
            .bind(bot_user.to_string())
            .execute(&mut *conn)
            .await?;
    }
    for channel in channel_ids {
        sqlx::query(
            "INSERT INTO webhook_bot_channels (bot_id, conversation_id) VALUES ($1, $2) \
             ON CONFLICT (bot_id, conversation_id) DO NOTHING",
        )
        .bind(bot_id.to_string())
        .bind(channel.to_string())
        .execute(&mut *conn)
        .await?;
        // In with everything read, as an agent of the channel (`agent::profiles`).
        sqlx::query(
            "INSERT INTO chat_members (conversation_id, user_id, workspace_id, notify, last_read_message_id, \
             read_count, joined_at) SELECT c.id, $2, c.workspace_id, 'mentions', NULL, c.message_count, $3 \
             FROM chat_conversations c WHERE c.id = $1 ON CONFLICT (conversation_id, user_id) DO NOTHING",
        )
        .bind(channel.to_string())
        .bind(bot_user.to_string())
        .bind(now.as_millis())
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

/// Makes a bot. The answer has its signing secret, the only time Orbit shows it.
async fn create_bot(
    State(state): State<BotState>,
    Path(workspace_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
    ApiJson(body): ApiJson<BotBody>,
) -> Result<Json<Value>, ApiError> {
    let (workspace_id, actor) = manager(&state, &headers, &workspace_id, &uri, &request_id).await?;
    let key = app_key(&state, &uri, &request_id)?;
    let name = checked_name(body.name.as_deref().unwrap_or_default())
        .map_err(|detail| invalid(detail, &uri, &request_id))?;
    let url = checked_url(&state, body.url.as_deref().unwrap_or_default())
        .map_err(|detail| invalid(detail, &uri, &request_id))?;
    let secret = new_secret();
    let sealed = encrypt_secret(&key, &secret).map_err(|()| {
        unavailable(
            &sqlx::Error::Protocol("bot secret".to_owned()),
            &uri,
            &request_id,
        )
    })?;
    // The hash of a random secret that is thrown away: no password signs in as the bot.
    let password_hash = PasswordService::default()
        .hash(&generate_opaque_token())
        .map_err(|_| {
            unavailable(
                &sqlx::Error::Protocol("bot password".to_owned()),
                &uri,
                &request_id,
            )
        })?;
    let database = state.database();
    let request = request_id_value(request_id.as_ref()).to_owned();
    let created = async {
        let mut tx = database.immediate_transaction().await?;
        still_manager(&mut tx, workspace_id, actor).await?;
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM webhook_bots WHERE workspace_id = $1 AND deleted_at IS NULL",
        )
        .bind(workspace_id.to_string())
        .fetch_one(&mut *tx)
        .await?;
        if count >= MAX_BOTS {
            return Err(SaveError::Limit);
        }
        if crate::agent::profiles::name_taken(&mut tx, None, &name).await? {
            return Err(SaveError::Taken);
        }
        let now = TimestampMillis::now();
        let user_id = Id::new_v7();
        let bot_id = Id::new_v7();
        sqlx::query(
            "INSERT INTO users (id, email, normalized_email, display_name, password_hash, created_at, updated_at) \
             VALUES ($1, $2, $2, $3, $4, $5, $5)",
        )
        .bind(user_id.to_string())
        .bind(format!("bot-{user_id}@orbit.invalid"))
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
            "INSERT INTO webhook_bots (id, workspace_id, user_id, url, signing_secret, created_by, \
             created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $7)",
        )
        .bind(bot_id.to_string())
        .bind(workspace_id.to_string())
        .bind(user_id.to_string())
        .bind(&url)
        .bind(&sealed)
        .bind(actor.to_string())
        .bind(now.as_millis())
        .execute(&mut *tx)
        .await?;
        set_channels(
            &mut tx,
            workspace_id,
            actor,
            bot_id,
            user_id,
            body.channel_ids.as_deref().unwrap_or_default(),
            now,
        )
        .await?;
        audit::record(
            &mut tx,
            workspace_id,
            Some(actor),
            "webhook_bot.create",
            AuditOutcome::Success,
            "webhook_bot",
            Some(bot_id),
            &request,
            json!({ "name": name }),
            now,
        )
        .await?;
        tx.commit().await?;
        Ok::<_, SaveError>(bot_id)
    }
    .await
    .map_err(|error| save_failure(error, &uri, &request_id))?;
    let view = one(&state, workspace_id, created, &uri, &request_id).await?;
    Ok(Json(json!({ "bot": view, "signing_secret": secret })))
}

async fn one(
    state: &BotState,
    workspace_id: Id,
    bot_id: Id,
    uri: &Uri,
    request_id: &RequestIdExtension,
) -> Result<BotView, ApiError> {
    bot_views(state.database(), workspace_id, Some(bot_id))
        .await
        .map_err(|error| unavailable(&error, uri, request_id))?
        .into_iter()
        .next()
        .ok_or_else(|| not_found(uri, request_id))
}

/// The bot user of the live bot `bot_id` of the workspace.
async fn load_live(
    conn: &mut AnyConnection,
    workspace_id: Id,
    bot_id: Id,
) -> Result<Id, SaveError> {
    sqlx::query_scalar::<_, String>(
        "SELECT user_id FROM webhook_bots WHERE id = $1 AND workspace_id = $2 AND deleted_at IS NULL",
    )
    .bind(bot_id.to_string())
    .bind(workspace_id.to_string())
    .fetch_optional(&mut *conn)
    .await?
    .and_then(|user| user.parse().ok())
    .ok_or(SaveError::NotFound)
}

async fn update_bot(
    State(state): State<BotState>,
    Path((workspace_id, bot_id)): Path<(String, String)>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
    ApiJson(body): ApiJson<BotBody>,
) -> Result<Json<Value>, ApiError> {
    let (workspace_id, actor) = manager(&state, &headers, &workspace_id, &uri, &request_id).await?;
    let bot_id: Id = bot_id.parse().map_err(|_| not_found(&uri, &request_id))?;
    let name = body
        .name
        .as_deref()
        .map(checked_name)
        .transpose()
        .map_err(|detail| invalid(detail, &uri, &request_id))?;
    let url = body
        .url
        .as_deref()
        .map(|url| checked_url(&state, url))
        .transpose()
        .map_err(|detail| invalid(detail, &uri, &request_id))?;
    let database = state.database();
    let request = request_id_value(request_id.as_ref()).to_owned();
    async {
        let mut tx = database.immediate_transaction().await?;
        still_manager(&mut tx, workspace_id, actor).await?;
        let user_id = load_live(&mut tx, workspace_id, bot_id).await?;
        let now = TimestampMillis::now();
        if let Some(name) = &name {
            if crate::agent::profiles::name_taken(&mut tx, Some(user_id), name).await? {
                return Err(SaveError::Taken);
            }
            sqlx::query(
                "UPDATE users SET display_name = $1, updated_at = $2, version = version + 1 WHERE id = $3",
            )
            .bind(name)
            .bind(now.as_millis())
            .bind(user_id.to_string())
            .execute(&mut *tx)
            .await?;
        }
        sqlx::query(
            "UPDATE webhook_bots SET updated_at = $1, url = COALESCE($2, url) WHERE id = $3",
        )
        .bind(now.as_millis())
        .bind(url.clone())
        .bind(bot_id.to_string())
        .execute(&mut *tx)
        .await?;
        if let Some(channel_ids) = &body.channel_ids {
            set_channels(&mut tx, workspace_id, actor, bot_id, user_id, channel_ids, now).await?;
        }
        audit::record(
            &mut tx,
            workspace_id,
            Some(actor),
            "webhook_bot.update",
            AuditOutcome::Success,
            "webhook_bot",
            Some(bot_id),
            &request,
            json!({ "name": name, "url_changed": url.is_some() }),
            now,
        )
        .await?;
        tx.commit().await?;
        Ok::<_, SaveError>(())
    }
    .await
    .map_err(|error| save_failure(error, &uri, &request_id))?;
    Ok(Json(json!(
        one(&state, workspace_id, bot_id, &uri, &request_id).await?
    )))
}

/// A new signing secret, shown once. Deliveries are signed with it at once.
async fn rotate_secret(
    State(state): State<BotState>,
    Path((workspace_id, bot_id)): Path<(String, String)>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
) -> Result<Json<Value>, ApiError> {
    let (workspace_id, actor) = manager(&state, &headers, &workspace_id, &uri, &request_id).await?;
    let key = app_key(&state, &uri, &request_id)?;
    let bot_id: Id = bot_id.parse().map_err(|_| not_found(&uri, &request_id))?;
    let secret = new_secret();
    let sealed = encrypt_secret(&key, &secret).map_err(|()| {
        unavailable(
            &sqlx::Error::Protocol("bot secret".to_owned()),
            &uri,
            &request_id,
        )
    })?;
    let database = state.database();
    let request = request_id_value(request_id.as_ref()).to_owned();
    async {
        let mut tx = database.immediate_transaction().await?;
        still_manager(&mut tx, workspace_id, actor).await?;
        load_live(&mut tx, workspace_id, bot_id).await?;
        let now = TimestampMillis::now();
        sqlx::query(
            "UPDATE webhook_bots SET signing_secret = $1, secret_rotated_at = $2, updated_at = $2 WHERE id = $3",
        )
        .bind(&sealed)
        .bind(now.as_millis())
        .bind(bot_id.to_string())
        .execute(&mut *tx)
        .await?;
        audit::record(
            &mut tx,
            workspace_id,
            Some(actor),
            "webhook_bot.rotate_secret",
            AuditOutcome::Success,
            "webhook_bot",
            Some(bot_id),
            &request,
            json!({}),
            now,
        )
        .await?;
        tx.commit().await?;
        Ok::<_, SaveError>(())
    }
    .await
    .map_err(|error| save_failure(error, &uri, &request_id))?;
    Ok(Json(json!({ "signing_secret": secret })))
}

/// Deletes a bot: nobody can reach it any more, its waiting deliveries end without a note, and its
/// bot user is suspended and renamed "<name> (deleted)", so its messages and DMs stay readable
/// and its name is free again.
async fn delete_bot(
    State(state): State<BotState>,
    Path((workspace_id, bot_id)): Path<(String, String)>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
) -> Result<StatusCode, ApiError> {
    let (workspace_id, actor) = manager(&state, &headers, &workspace_id, &uri, &request_id).await?;
    let bot_id: Id = bot_id.parse().map_err(|_| not_found(&uri, &request_id))?;
    let database = state.database();
    let request = request_id_value(request_id.as_ref()).to_owned();
    async {
        let mut tx = database.immediate_transaction().await?;
        still_manager(&mut tx, workspace_id, actor).await?;
        let user_id = load_live(&mut tx, workspace_id, bot_id).await?;
        let now = TimestampMillis::now();
        sqlx::query("UPDATE webhook_bots SET deleted_at = $1, updated_at = $1 WHERE id = $2")
            .bind(now.as_millis())
            .bind(bot_id.to_string())
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "UPDATE webhook_bot_deliveries SET status = 'failed', error = 'bot_deleted', \
             reply_token_hash = NULL, finished_at = $1 WHERE bot_id = $2 AND status IN ('pending', 'sending')",
        )
        .bind(now.as_millis())
        .bind(bot_id.to_string())
        .execute(&mut *tx)
        .await?;
        let name: String = sqlx::query_scalar("SELECT display_name FROM users WHERE id = $1")
            .bind(user_id.to_string())
            .fetch_one(&mut *tx)
            .await?;
        sqlx::query(
            "UPDATE users SET display_name = $1, suspended_at = $2, updated_at = $2, version = version + 1 \
             WHERE id = $3",
        )
        .bind(format!("{name} (deleted)"))
        .bind(now.as_millis())
        .bind(user_id.to_string())
        .execute(&mut *tx)
        .await?;
        audit::record(
            &mut tx,
            workspace_id,
            Some(actor),
            "webhook_bot.delete",
            AuditOutcome::Success,
            "webhook_bot",
            Some(bot_id),
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

/// The bot's 20 newest deliveries: status, HTTP status and a short reason. Never the payload.
async fn list_deliveries(
    State(state): State<BotState>,
    Path((workspace_id, bot_id)): Path<(String, String)>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
) -> Result<Json<Value>, ApiError> {
    let (workspace_id, _) = manager(&state, &headers, &workspace_id, &uri, &request_id).await?;
    let bot_id: Id = bot_id.parse().map_err(|_| not_found(&uri, &request_id))?;
    let rows = sqlx::query(
        "SELECT d.id, d.status, d.http_status, d.error, d.created_at, d.finished_at \
         FROM webhook_bot_deliveries d JOIN webhook_bots b ON b.id = d.bot_id \
         WHERE d.bot_id = $1 AND b.workspace_id = $2 AND b.deleted_at IS NULL \
         ORDER BY d.created_at DESC LIMIT 20",
    )
    .bind(bot_id.to_string())
    .bind(workspace_id.to_string())
    .fetch_all(state.database().pool())
    .await
    .map_err(|error| unavailable(&error, &uri, &request_id))?;
    Ok(Json(json!({
        "deliveries": rows.iter().map(super::delivery_view).collect::<Vec<_>>(),
    })))
}

/// The bot user of a live bot of the workspace, for an owner or admin.
async fn bot_user(
    state: &BotState,
    headers: &HeaderMap,
    workspace_id: &str,
    bot_id: &str,
    uri: &Uri,
    request_id: &RequestIdExtension,
) -> Result<Id, ApiError> {
    let (workspace_id, _) = manager(state, headers, workspace_id, uri, request_id).await?;
    let bot_id: Id = bot_id.parse().map_err(|_| not_found(uri, request_id))?;
    let mut conn = state
        .database()
        .pool()
        .acquire()
        .await
        .map_err(|error| unavailable(&error, uri, request_id))?;
    match load_live(&mut conn, workspace_id, bot_id).await {
        Ok(user) => Ok(user),
        Err(SaveError::Database(error)) => Err(unavailable(&error, uri, request_id)),
        Err(_) => Err(not_found(uri, request_id)),
    }
}

/// The bot's avatar: a PNG, JPEG or WebP image of at most 512 KiB in the multipart field `file`.
async fn upload_avatar(
    State(state): State<BotState>,
    Path((workspace_id, bot_id)): Path<(String, String)>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
    mut multipart: Multipart,
) -> Result<StatusCode, ApiError> {
    let user = bot_user(&state, &headers, &workspace_id, &bot_id, &uri, &request_id).await?;
    let bad = || {
        problem(
            StatusCode::BAD_REQUEST,
            "invalid_multipart",
            "Send the image as the multipart field \"file\".",
            &uri,
            &request_id,
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
            &uri,
            &request_id,
        ));
    }
    let mime_type = crate::auth_routes::avatar_mime_type(&image)
        .ok_or_else(|| invalid("Upload a PNG, JPEG or WebP image.", &uri, &request_id))?;
    state
        .chat
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
                &uri,
                &request_id,
            )
        })?;
    Ok(StatusCode::NO_CONTENT)
}

async fn remove_avatar(
    State(state): State<BotState>,
    Path((workspace_id, bot_id)): Path<(String, String)>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
) -> Result<StatusCode, ApiError> {
    let user = bot_user(&state, &headers, &workspace_id, &bot_id, &uri, &request_id).await?;
    state
        .chat
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
                &uri,
                &request_id,
            )
        })?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct ReplyBody {
    text: String,
}

/// A bot's answer through the reply URL of a delivery: `{"text": "…"}`, once, within the 30
/// seconds. The token is checked after the body, so a bad body does not use it up.
async fn post_reply(
    State(state): State<BotState>,
    Path((bot_id, token)): Path<(String, String)>,
    request_id: RequestIdExtension,
    request: Request,
) -> Result<Json<Value>, ApiError> {
    // Never the token: it is in the path.
    let instance = format!("/api/v1/hooks/bots/{bot_id}/replies");
    let uri: Uri = instance.parse().unwrap_or_default();
    let gone = || {
        problem(
            StatusCode::NOT_FOUND,
            "reply_url_not_found",
            "This reply URL is not valid: it was used, it is older than 30 seconds, or its bot is gone.",
            &uri,
            &request_id,
        )
    };
    let bytes = to_bytes(request.into_body(), MAX_ANSWER_BYTES)
        .await
        .map_err(|_| {
            problem(
                StatusCode::PAYLOAD_TOO_LARGE,
                "reply_too_large",
                "A reply may have at most 64 KiB.",
                &uri,
                &request_id,
            )
        })?;
    let body: ReplyBody = serde_json::from_slice(&bytes)
        .map_err(|_| invalid("Send JSON with a \"text\" string.", &uri, &request_id))?;
    if body.text.trim().is_empty() {
        return Err(invalid("The text is empty.", &uri, &request_id));
    }
    let bot_id: Id = bot_id.parse().map_err(|_| gone())?;
    let delivery = super::use_reply_token(&state, bot_id, &token)
        .await
        .map_err(|error| unavailable(&error, &uri, &request_id))?
        .ok_or_else(gone)?;
    let message = super::post_async_reply(&state, delivery, &body.text)
        .await
        .map_err(|error| unavailable(&error, &uri, &request_id))?;
    match message {
        Some(message) => Ok(Json(
            json!({ "ok": true, "message_id": message.id.to_string() }),
        )),
        None => Err(problem(
            StatusCode::CONFLICT,
            "reply_not_posted",
            "The reply could not be posted: the bot left the conversation or the thread is gone.",
            &uri,
            &request_id,
        )),
    }
}
