//! Webhook bots (step 10): bots that answer. An owner or admin makes a bot in Settings → Agentic →
//! Bots: a name, an avatar, an https URL and the channels where it may be used. Each bot is a bot
//! user of its own, like an agent profile (step 6), so people mention it and DM it like a person.
//!
//! A message for a bot (a mention in one of its channels, any message in a DM of the person and the
//! bot, a mention in a group DM with it) is written as a delivery in the transaction of the message
//! ([`record`]). The dispatcher ([`run_service`]) sends it to the bot's URL as a signed JSON POST
//! ([`sign`]). The bot answers within 30 seconds: in the body of its HTTP answer, or later with one
//! POST to the reply URL of the delivery (single use, `routes::post_reply`). Without an answer at
//! the deadline the bot posts a short note instead ([`sweep`]).
//!
//! A bot only posts plain chat messages as its bot user: it never gets the agent's tools or any
//! person's permissions. Guests cannot reach a bot. Calls go through the agent's guarded client
//! (`crate::agent::outbound`): https to public addresses only (checked after DNS resolution, for
//! every address), no redirects, no proxy, a timeout and a size limit. Hosts in
//! `ORBIT_WEBHOOK_BOT_TEST_HOSTS` may use plain HTTP and local addresses: for tests only.
//!
//! Bot authors: `docs/webhook-bots.md`.

pub(crate) mod routes;

use std::sync::{Arc, LazyLock};
use std::time::Duration;

use axum::http::{HeaderMap, HeaderName, HeaderValue};
use hmac::{Hmac, Mac};
use orbit_platform::{Flag, Id, Row, SqlBuilder, TimestampMillis, generate_opaque_token};
use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::AnyConnection;
use tokio::sync::{Notify, Semaphore};
use tokio_util::sync::CancellationToken;

use crate::agent::outbound::{Outbound, OutboundError, TestHosts};
use crate::chat_routes::ChatState;
use crate::push::PushService;
use crate::repositories::chat::{ChatError, MESSAGE_MAX_CHARS, MessageRecord, SendInput, cut};

pub use routes::router;

/// How long a bot has to answer a message.
pub const REPLY_WINDOW: Duration = Duration::from_secs(30);
/// How far a signature's timestamp may be from the bot's clock (the bot checks it).
pub const REPLAY_WINDOW_SECS: i64 = 5 * 60;
/// Messages one bot gets per minute; more get one short note and are not sent.
pub const RATE_PER_MINUTE: i64 = 30;
/// Bots one message reaches.
const MAX_BOTS_PER_MESSAGE: usize = 5;
/// The most Orbit reads of a bot's HTTP answer, and of a reply sent to the reply URL.
pub const MAX_ANSWER_BYTES: usize = 64 * 1024;
/// Deliveries one server sends at the same time. More wait.
const MAX_SENDS: usize = 32;
const USER_AGENT: &str = concat!("Orbit-Bots/", env!("CARGO_PKG_VERSION"));

pub const SIGNATURE_HEADER: &str = "x-orbit-signature";
pub const TIMESTAMP_HEADER: &str = "x-orbit-timestamp";
pub const DELIVERY_HEADER: &str = "x-orbit-delivery";

/// Wakes this server's dispatcher after a message for a bot is committed. Other servers find it
/// on their next round (each second).
static WAKE: LazyLock<Notify> = LazyLock::new(Notify::new);

pub(crate) fn wake() {
    WAKE.notify_one();
}

/// What the bot routes and the dispatcher need.
#[derive(Clone)]
pub struct BotState {
    pub(crate) chat: ChatState,
    /// Encrypts the signing secrets; without it bots cannot be made or reached.
    pub(crate) app_key: Option<[u8; 32]>,
    pub(crate) outbound: Outbound,
    /// The origin of the reply URLs (`ORBIT_PUBLIC_ORIGIN`).
    pub(crate) public_origin: Arc<str>,
    pub(crate) window: Duration,
    sends: Arc<Semaphore>,
}

impl BotState {
    /// Test hosts come from `ORBIT_WEBHOOK_BOT_TEST_HOSTS` (empty unless set).
    #[must_use]
    pub fn new(chat: ChatState, app_key: Option<[u8; 32]>, public_origin: &str) -> Self {
        Self {
            chat,
            app_key,
            outbound: Outbound::with_user_agent(
                TestHosts::from_var("ORBIT_WEBHOOK_BOT_TEST_HOSTS"),
                USER_AGENT,
            ),
            public_origin: Arc::from(public_origin.trim_end_matches('/')),
            window: REPLY_WINDOW,
            sends: Arc::new(Semaphore::new(MAX_SENDS)),
        }
    }

    /// Lets bots reach these hosts over plain HTTP and on local addresses: for tests only.
    #[must_use]
    pub fn with_test_hosts(mut self, hosts: &[&str]) -> Self {
        self.outbound =
            Outbound::with_user_agent(TestHosts::new(hosts.iter().copied()), USER_AGENT);
        self
    }

    /// A shorter answer window than 30 seconds: for tests only.
    #[must_use]
    pub fn with_reply_window(mut self, window: Duration) -> Self {
        self.window = window;
        self
    }

    pub(crate) fn database(&self) -> &orbit_platform::Database {
        self.chat.identity.database()
    }
}

// ---------------------------------------------------------------------------------------------
// Signatures

/// The signature of a delivery: `v1=` and the hex HMAC-SHA256 of `v1:{timestamp}:{body}` with the
/// bot's signing secret. `timestamp` is Unix seconds, sent in `X-Orbit-Timestamp`.
#[must_use]
pub fn sign(secret: &str, timestamp: i64, body: &[u8]) -> String {
    let mut mac =
        Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC takes a key of any length");
    mac.update(format!("v1:{timestamp}:").as_bytes());
    mac.update(body);
    let hex: String = mac
        .finalize()
        .into_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("v1={hex}")
}

/// Why a bot refuses a delivery ([`verify`]).
#[derive(Debug, Eq, PartialEq)]
pub enum VerifyError {
    /// The timestamp is not a number, or more than 5 minutes from `now`: a replay or a bad clock.
    Stale,
    /// The signature is missing, malformed or wrong.
    BadSignature,
}

/// What a bot does with each delivery (`docs/webhook-bots.md` has the same in JavaScript and
/// Python): the timestamp is within 5 minutes of `now` (Unix seconds), and the signature matches,
/// compared in constant time.
pub fn verify(
    secret: &str,
    timestamp: &str,
    body: &[u8],
    signature: &str,
    now: i64,
) -> Result<(), VerifyError> {
    let sent: i64 = timestamp.trim().parse().map_err(|_| VerifyError::Stale)?;
    if (now - sent).abs() > REPLAY_WINDOW_SECS {
        return Err(VerifyError::Stale);
    }
    let hex = signature
        .strip_prefix("v1=")
        .ok_or(VerifyError::BadSignature)?;
    if hex.len() != 64 {
        return Err(VerifyError::BadSignature);
    }
    let mut expected = [0_u8; 32];
    for (index, byte) in expected.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16)
            .map_err(|_| VerifyError::BadSignature)?;
    }
    let mut mac =
        Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC takes a key of any length");
    mac.update(format!("v1:{sent}:").as_bytes());
    mac.update(body);
    mac.verify_slice(&expected)
        .map_err(|_| VerifyError::BadSignature)
}

/// A new signing secret: `obs_` and 43 URL-safe characters (256 random bits).
pub(crate) fn new_secret() -> String {
    format!("obs_{}", generate_opaque_token())
}

pub(crate) fn token_hash(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

// ---------------------------------------------------------------------------------------------
// Recording, in the transaction of the message

/// Whether `user_id` is a bot of any kind: the default agent, a custom agent or a webhook bot.
/// Bots never start agents or other bots, so two bots cannot answer each other forever.
pub(crate) async fn is_bot_user(
    conn: &mut AnyConnection,
    user_id: Id,
) -> Result<bool, sqlx::Error> {
    Ok(sqlx::query_scalar::<_, Flag>(
        "SELECT EXISTS (SELECT 1 FROM webhook_bots WHERE user_id = $1) \
         OR EXISTS (SELECT 1 FROM agent_profiles WHERE user_id = $1) \
         OR EXISTS (SELECT 1 FROM agent_bot WHERE user_id = $1)",
    )
    .bind(user_id.to_string())
    .fetch_one(&mut *conn)
    .await?
    .0)
}

/// Writes a delivery for each live bot that `message` is for. Called by `send_message` in its
/// transaction, after the message row; `true` when it wrote one ([`wake`] after the commit).
/// Never fails for a reason of its own: on Postgres an error would end the whole transaction and
/// lose the message.
pub(crate) async fn record(
    conn: &mut AnyConnection,
    workspace_id: Id,
    conversation_id: Id,
    is_dm: bool,
    message: &MessageRecord,
) -> Result<bool, sqlx::Error> {
    if message.mentions.user_ids.is_empty() && !is_dm {
        return Ok(false);
    }
    // The live bots of the workspace that are in this conversation.
    let rows = sqlx::query(
        "SELECT b.id, b.user_id FROM webhook_bots b JOIN chat_members m \
         ON m.user_id = b.user_id AND m.conversation_id = $2 \
         WHERE b.workspace_id = $1 AND b.deleted_at IS NULL",
    )
    .bind(workspace_id.to_string())
    .bind(conversation_id.to_string())
    .fetch_all(&mut *conn)
    .await?;
    let mut members: Vec<(Id, Id)> = rows
        .iter()
        .filter_map(|row| {
            Some((
                row.get::<String, _>("id").parse().ok()?,
                row.get::<String, _>("user_id").parse().ok()?,
            ))
        })
        .collect();
    let mut targets: Vec<(Id, Id)> = Vec::new();
    // A DM of the person and the bot: every message, answered in the main list. A group DM:
    // mentions only.
    let dm_of_two = is_dm
        && !members.is_empty()
        && sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM chat_members WHERE conversation_id = $1",
        )
        .bind(conversation_id.to_string())
        .fetch_one(&mut *conn)
        .await?
            == 2;
    if dm_of_two && members.len() == 1 {
        targets.append(&mut members);
    }
    let mut outside: Vec<Id> = Vec::new();
    if targets.is_empty() {
        for user in &message.mentions.user_ids {
            if let Some(found) = members.iter().find(|(_, bot_user)| bot_user == user) {
                if !targets.contains(found) {
                    targets.push(*found);
                }
            } else if !is_dm {
                outside.push(*user);
            }
        }
    }
    // A mentioned bot that is not in the channel: refused below, if it is a bot of the workspace.
    let mut refused: Vec<(Id, Id)> = Vec::new();
    if !outside.is_empty() {
        let mut query = SqlBuilder::new(
            "SELECT id, user_id FROM webhook_bots WHERE deleted_at IS NULL AND workspace_id = ",
        );
        query.push_bind(workspace_id.to_string());
        query.push(" AND user_id IN (");
        let mut ids = query.separated(", ");
        for id in outside.iter().take(MAX_BOTS_PER_MESSAGE) {
            ids.push_bind(id.to_string());
        }
        query.push(")");
        refused = query
            .build()
            .fetch_all(&mut *conn)
            .await?
            .iter()
            .filter_map(|row| {
                Some((
                    row.get::<String, _>("id").parse().ok()?,
                    row.get::<String, _>("user_id").parse().ok()?,
                ))
            })
            .collect();
    }
    if targets.is_empty() && refused.is_empty() {
        return Ok(false);
    }
    if is_bot_user(conn, message.author_id).await? {
        return Ok(false);
    }
    // A bot never answers a guest: a guest's mention or DM reaches nothing.
    let guest = sqlx::query_scalar::<_, Flag>(
        "SELECT EXISTS (SELECT 1 FROM memberships WHERE workspace_id = $1 AND user_id = $2 \
         AND role = 'guest')",
    )
    .bind(workspace_id.to_string())
    .bind(message.author_id.to_string())
    .fetch_one(&mut *conn)
    .await?
    .0;
    if guest {
        return Ok(false);
    }
    let (_, root) = crate::agent::inputs::thread_of(message, conversation_id, dm_of_two);
    let now = TimestampMillis::now().as_millis();
    let mut written = false;
    let all = targets
        .iter()
        .map(|target| (*target, true))
        .chain(refused.iter().map(|target| (*target, false)))
        .take(MAX_BOTS_PER_MESSAGE);
    for ((bot_id, _), in_conversation) in all {
        let allowed = in_conversation
            && (is_dm
                || sqlx::query_scalar::<_, Flag>(
                    "SELECT EXISTS (SELECT 1 FROM webhook_bot_channels WHERE bot_id = $1 \
                     AND conversation_id = $2)",
                )
                .bind(bot_id.to_string())
                .bind(conversation_id.to_string())
                .fetch_one(&mut *conn)
                .await?
                .0);
        let (status, finished) = if allowed {
            let recent: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM webhook_bot_deliveries WHERE bot_id = $1 AND created_at > $2 \
                 AND status NOT IN ('refused', 'limited')",
            )
            .bind(bot_id.to_string())
            .bind(now - 60_000)
            .fetch_one(&mut *conn)
            .await?;
            if recent >= RATE_PER_MINUTE {
                // One note a minute: the later ones are finished at once, silently.
                let noted = sqlx::query_scalar::<_, Flag>(
                    "SELECT EXISTS (SELECT 1 FROM webhook_bot_deliveries WHERE bot_id = $1 \
                     AND status = 'limited' AND created_at > $2)",
                )
                .bind(bot_id.to_string())
                .bind(now - 60_000)
                .fetch_one(&mut *conn)
                .await?
                .0;
                ("limited", noted.then_some(now))
            } else {
                ("pending", None)
            }
        } else {
            ("refused", None)
        };
        sqlx::query(
            "INSERT INTO webhook_bot_deliveries (id, bot_id, workspace_id, conversation_id, message_id, \
             root_message_id, author_id, status, created_at, expires_at, finished_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
        )
        .bind(Id::new_v7().to_string())
        .bind(bot_id.to_string())
        .bind(workspace_id.to_string())
        .bind(conversation_id.to_string())
        .bind(message.id.to_string())
        .bind(root.map(|root| root.to_string()))
        .bind(message.author_id.to_string())
        .bind(status)
        .bind(now)
        .bind(now + i64::try_from(REPLY_WINDOW.as_millis()).unwrap_or(30_000))
        .bind(finished)
        .execute(&mut *conn)
        .await?;
        written |= finished.is_none();
    }
    Ok(written)
}

// ---------------------------------------------------------------------------------------------
// Dispatch

/// A delivery this server took, with what its POST needs.
#[derive(Clone)]
struct Claimed {
    id: Id,
    status: String,
    bot_id: Id,
    bot_user: Id,
    bot_name: String,
    workspace_id: Id,
    conversation_id: Id,
    root: Option<Id>,
    message_id: Id,
    author_id: Id,
    expires_at: i64,
    /// The reply token of a `sending` delivery (only in memory, and in the POST).
    token: Option<String>,
}

/// The JSON a bot gets. Field names are part of the bot contract (`docs/webhook-bots.md`).
#[derive(Serialize)]
pub struct Payload {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub delivery_id: String,
    pub bot: PayloadRef,
    pub workspace: PayloadRef,
    pub conversation: PayloadConversation,
    /// The thread the answer goes to; null in a DM of the person and the bot.
    pub thread_id: Option<String>,
    pub message: PayloadMessage,
    pub author: PayloadRef,
    pub reply: PayloadReply,
}

#[derive(Serialize)]
pub struct PayloadRef {
    pub id: String,
    pub name: String,
}

#[derive(Serialize)]
pub struct PayloadConversation {
    pub id: String,
    /// The channel name; for a DM, the names of its people.
    pub name: String,
    /// `public`, `private` or `dm`.
    pub kind: String,
}

#[derive(Serialize)]
pub struct PayloadMessage {
    pub id: String,
    /// Markdown; mentions are tokens: `<@user_id>`, `<#conversation_id>`.
    pub text: String,
    /// Unix milliseconds.
    pub created_at: i64,
}

#[derive(Serialize)]
pub struct PayloadReply {
    /// POST `{"text": "…"}` here once, before `expires_at`.
    pub url: String,
    pub token: String,
    /// Unix milliseconds.
    pub expires_at: i64,
}

/// Takes the deliveries that wait for this server: new ones (each gets its reply token here) and
/// refused or rate-limited ones that still need their note. Each is taken by one server only.
async fn claim(state: &BotState) -> Result<Vec<Claimed>, sqlx::Error> {
    let now = TimestampMillis::now().as_millis();
    let window = i64::try_from(state.window.as_millis()).unwrap_or(30_000);
    let rows = sqlx::query(
        "SELECT id, status FROM webhook_bot_deliveries WHERE (status = 'pending' AND expires_at > $1) \
         OR (status IN ('refused', 'limited') AND finished_at IS NULL) ORDER BY created_at LIMIT 50",
    )
    .bind(now)
    .fetch_all(state.database().pool())
    .await?;
    let mut claimed = Vec::new();
    for row in rows {
        let id: String = row.get("id");
        let status: String = row.get("status");
        let (token, taken) = if status == "pending" {
            let token = generate_opaque_token();
            let taken = sqlx::query(
                "UPDATE webhook_bot_deliveries SET status = 'sending', reply_token_hash = $1, \
                 expires_at = CASE WHEN expires_at < $2 THEN expires_at ELSE $2 END \
                 WHERE id = $3 AND status = 'pending'",
            )
            .bind(token_hash(&token))
            .bind(now + window)
            .bind(&id)
            .execute(state.database().pool())
            .await?
            .rows_affected();
            (Some(token), taken)
        } else {
            let taken = sqlx::query(
                "UPDATE webhook_bot_deliveries SET finished_at = $1 WHERE id = $2 AND finished_at IS NULL",
            )
            .bind(now)
            .bind(&id)
            .execute(state.database().pool())
            .await?
            .rows_affected();
            (None, taken)
        };
        if taken == 0 {
            continue;
        }
        match load(state, &id).await? {
            Some(mut delivery) => {
                delivery.token = token;
                claimed.push(delivery);
            }
            None => {
                finish(
                    state,
                    &id,
                    &["sending"],
                    "failed",
                    None,
                    Some("bot_deleted"),
                )
                .await?;
            }
        }
    }
    Ok(claimed)
}

/// A delivery with its live bot; `None` when the bot was deleted.
async fn load(state: &BotState, id: &str) -> Result<Option<Claimed>, sqlx::Error> {
    let row = sqlx::query(
        "SELECT d.id, d.status, d.bot_id, d.workspace_id, d.conversation_id, d.root_message_id, \
         d.message_id, d.author_id, d.expires_at, b.user_id, u.display_name FROM webhook_bot_deliveries d \
         JOIN webhook_bots b ON b.id = d.bot_id JOIN users u ON u.id = b.user_id \
         WHERE d.id = $1 AND b.deleted_at IS NULL",
    )
    .bind(id)
    .fetch_optional(state.database().pool())
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let id = |column: &str| row.get::<String, _>(column).parse::<Id>().ok();
    let parsed = (|| {
        Some(Claimed {
            id: id("id")?,
            status: row.get("status"),
            bot_id: id("bot_id")?,
            bot_user: id("user_id")?,
            bot_name: row.get("display_name"),
            workspace_id: id("workspace_id")?,
            conversation_id: id("conversation_id")?,
            root: row
                .get::<Option<String>, _>("root_message_id")
                .and_then(|root| root.parse().ok()),
            message_id: id("message_id")?,
            author_id: id("author_id")?,
            expires_at: row.get("expires_at"),
            token: None,
        })
    })();
    Ok(parsed)
}

/// Moves a delivery from one of `from` to `to`; `true` when this call did it (so it alone posts
/// the answer or the note).
async fn finish(
    state: &BotState,
    id: &str,
    from: &[&str],
    to: &str,
    http_status: Option<u16>,
    error: Option<&str>,
) -> Result<bool, sqlx::Error> {
    let mut query = SqlBuilder::new("UPDATE webhook_bot_deliveries SET status = ");
    query.push_bind(to.to_owned());
    query.push(", reply_token_hash = NULL, finished_at = ");
    query.push_bind(TimestampMillis::now().as_millis());
    if let Some(status) = http_status {
        query.push(", http_status = ");
        query.push_bind(i64::from(status));
    }
    if let Some(error) = error {
        query.push(", error = ");
        query.push_bind(error.to_owned());
    }
    query.push(" WHERE id = ");
    query.push_bind(id.to_owned());
    query.push(" AND status IN (");
    let mut states = query.separated(", ");
    for status in from {
        states.push_bind((*status).to_owned());
    }
    query.push(")");
    Ok(query
        .build()
        .execute(state.database().pool())
        .await?
        .rows_affected()
        == 1)
}

/// Sends one delivery and handles the bot's HTTP answer.
async fn deliver(state: BotState, delivery: Claimed) {
    let result = match delivery.status.as_str() {
        "refused" => refuse(&state, &delivery).await,
        "limited" => {
            post_note(
                &state,
                &delivery,
                "_No answer: this bot gets too many messages. Try again in a minute._",
            )
            .await
        }
        _ => send(&state, &delivery).await,
    };
    if let Err(error) = result {
        tracing::warn!(%error, delivery = %delivery.id, "webhook bot delivery");
    }
}

/// What the bot's HTTP answer means.
#[derive(Debug, Eq, PartialEq)]
pub enum Answer {
    /// The reply, posted now.
    Reply(String),
    /// No reply in the body: the bot answers later through the reply URL.
    Later,
}

/// A 2xx body: `{"text": "…"}` with text is the reply; an empty body, `{}` or blank text means
/// later. `None` for a body that is not JSON, or a `text` that is not a string.
#[must_use]
pub fn parse_answer(body: &[u8]) -> Option<Answer> {
    if body.iter().all(u8::is_ascii_whitespace) {
        return Some(Answer::Later);
    }
    let value: serde_json::Value = serde_json::from_slice(body).ok()?;
    match value.get("text") {
        None | Some(serde_json::Value::Null) => Some(Answer::Later),
        Some(serde_json::Value::String(text)) if text.trim().is_empty() => Some(Answer::Later),
        Some(serde_json::Value::String(text)) => Some(Answer::Reply(text.clone())),
        Some(_) => None,
    }
}

/// A reply as Orbit posts it: trimmed, at most 4,000 characters, and `@channel`/`@here` as plain
/// text (a bot never notifies a whole channel).
#[must_use]
pub fn clean_reply(text: &str) -> String {
    let text = text
        .replace("\r\n", "\n")
        .replace("<!channel>", "@channel")
        .replace("<!here>", "@here");
    cut(text.trim(), MESSAGE_MAX_CHARS)
}

async fn send(state: &BotState, delivery: &Claimed) -> Result<(), sqlx::Error> {
    let id = delivery.id.to_string();
    let Some(key) = state.app_key else {
        return note_failure(state, delivery, None, "no_app_key").await;
    };
    let bot = sqlx::query("SELECT url, signing_secret FROM webhook_bots WHERE id = $1")
        .bind(delivery.bot_id.to_string())
        .fetch_one(state.database().pool())
        .await?;
    let url: String = bot.get("url");
    let Ok(secret) =
        crate::secret_box::decrypt_secret(&key, &bot.get::<Vec<u8>, _>("signing_secret"))
    else {
        return note_failure(state, delivery, None, "secret_unreadable").await;
    };
    let body = payload(state, delivery).await?;
    let body = serde_json::to_vec(&body).unwrap_or_default();
    let timestamp = TimestampMillis::now().as_millis() / 1000;
    let mut headers = HeaderMap::new();
    let mut put = |name: &'static str, value: String| {
        if let Ok(value) = HeaderValue::from_str(&value) {
            headers.insert(HeaderName::from_static(name), value);
        }
    };
    put(SIGNATURE_HEADER, sign(&secret, timestamp, &body));
    put(TIMESTAMP_HEADER, timestamp.to_string());
    put(DELIVERY_HEADER, id.clone());
    let Ok(url) = url::Url::parse(&url) else {
        return note_failure(state, delivery, None, "invalid_url").await;
    };
    let Ok(_permit) = state.sends.acquire().await else {
        return Ok(());
    };
    let left = delivery.expires_at - TimestampMillis::now().as_millis();
    if left <= 0 {
        return expire(state, delivery).await;
    }
    let timeout = Duration::from_millis(u64::try_from(left).unwrap_or(0));
    let answer = state
        .outbound
        .post_bytes(&url, body, headers, timeout, MAX_ANSWER_BYTES)
        .await;
    match answer {
        Ok((status, body)) if (200..300).contains(&status) => match parse_answer(&body) {
            Some(Answer::Reply(text)) => {
                if finish(state, &id, &["sending"], "answered", Some(status), None).await? {
                    post_reply(state, delivery, &text).await?;
                }
                Ok(())
            }
            Some(Answer::Later) => {
                sqlx::query(
                    "UPDATE webhook_bot_deliveries SET http_status = $1 WHERE id = $2 AND status = 'sending'",
                )
                .bind(i64::from(status))
                .bind(&id)
                .execute(state.database().pool())
                .await?;
                Ok(())
            }
            None => note_failure(state, delivery, Some(status), "invalid_answer").await,
        },
        Ok((status, _)) => note_failure(state, delivery, Some(status), "http_status").await,
        Err(OutboundError::Network(reason)) if reason == "timeout" => expire(state, delivery).await,
        Err(OutboundError::TooLarge) => {
            note_failure(state, delivery, None, "answer_too_large").await
        }
        Err(OutboundError::Refused(_)) => note_failure(state, delivery, None, "blocked_url").await,
        Err(OutboundError::Network(reason)) => {
            let reason = if reason.contains("local or private") {
                "blocked_address"
            } else {
                "unreachable"
            };
            note_failure(state, delivery, None, reason).await
        }
        Err(_) => note_failure(state, delivery, None, "failed").await,
    }
}

async fn payload(state: &BotState, delivery: &Claimed) -> Result<Payload, sqlx::Error> {
    let row = sqlx::query(
        "SELECT w.name AS workspace_name, c.name AS conversation_name, c.kind, m.body, m.created_at, \
         u.display_name AS author_name FROM workspaces w, chat_conversations c, chat_messages m, users u \
         WHERE w.id = $1 AND c.id = $2 AND m.id = $3 AND u.id = $4",
    )
    .bind(delivery.workspace_id.to_string())
    .bind(delivery.conversation_id.to_string())
    .bind(delivery.message_id.to_string())
    .bind(delivery.author_id.to_string())
    .fetch_one(state.database().pool())
    .await?;
    let token = delivery.token.clone().unwrap_or_default();
    Ok(Payload {
        kind: "message",
        delivery_id: delivery.id.to_string(),
        bot: PayloadRef {
            id: delivery.bot_id.to_string(),
            name: delivery.bot_name.clone(),
        },
        workspace: PayloadRef {
            id: delivery.workspace_id.to_string(),
            name: row.get("workspace_name"),
        },
        conversation: PayloadConversation {
            id: delivery.conversation_id.to_string(),
            name: row
                .get::<Option<String>, _>("conversation_name")
                .unwrap_or_default(),
            kind: row.get("kind"),
        },
        thread_id: delivery.root.map(|root| root.to_string()),
        message: PayloadMessage {
            id: delivery.message_id.to_string(),
            text: row.get("body"),
            created_at: row.get("created_at"),
        },
        author: PayloadRef {
            id: delivery.author_id.to_string(),
            name: row.get("author_name"),
        },
        reply: PayloadReply {
            url: format!(
                "{}/api/v1/hooks/bots/{}/replies/{token}",
                state.public_origin, delivery.bot_id
            ),
            token,
            expires_at: delivery.expires_at,
        },
    })
}

/// No answer in time: the note, once.
async fn expire(state: &BotState, delivery: &Claimed) -> Result<(), sqlx::Error> {
    if finish(
        state,
        &delivery.id.to_string(),
        &["pending", "sending"],
        "late",
        None,
        Some("timeout"),
    )
    .await?
    {
        post_note(
            state,
            delivery,
            "_No answer: the bot did not reply within 30 seconds._",
        )
        .await?;
    }
    Ok(())
}

/// The bot refused the message or could not be reached: the note, once.
async fn note_failure(
    state: &BotState,
    delivery: &Claimed,
    http_status: Option<u16>,
    reason: &str,
) -> Result<(), sqlx::Error> {
    tracing::info!(delivery = %delivery.id, reason, http_status, "webhook bot did not answer");
    if finish(
        state,
        &delivery.id.to_string(),
        &["pending", "sending"],
        "failed",
        http_status,
        Some(reason),
    )
    .await?
    {
        post_note(
            state,
            delivery,
            "_No answer: the bot could not be reached._",
        )
        .await?;
    }
    Ok(())
}

/// Posts `body` as the bot where the delivery's answer goes. `nonce` makes a retry post once.
async fn post_as_bot(
    state: &BotState,
    delivery: &Claimed,
    body: String,
    nonce: String,
) -> Result<Option<MessageRecord>, sqlx::Error> {
    let chat = &state.chat;
    let input = SendInput {
        conversation_id: delivery.conversation_id,
        thread_root_id: delivery.root,
        reply_to_id: None,
        body,
        sticker_id: None,
        file_ids: Vec::new(),
        online: chat.hub.online(delivery.workspace_id),
        also_in_channel: false,
        nonce,
        page_context: None,
    };
    match chat
        .publish(
            delivery.workspace_id,
            delivery.bot_user,
            chat.chat
                .send_message(delivery.workspace_id, delivery.bot_user, input),
        )
        .await
    {
        Ok((written, _)) => {
            PushService::of(state.database()).chat_message(&written.value);
            Ok(Some(written.value))
        }
        Err(ChatError::Unavailable(error)) => Err(error),
        Err(error) => {
            // The bot left the channel, the channel was archived or the thread deleted.
            tracing::info!(%error, delivery = %delivery.id, "webhook bot message not posted");
            Ok(None)
        }
    }
}

async fn post_reply(
    state: &BotState,
    delivery: &Claimed,
    text: &str,
) -> Result<Option<MessageRecord>, sqlx::Error> {
    let text = clean_reply(text);
    if text.is_empty() {
        return Ok(None);
    }
    post_as_bot(state, delivery, text, format!("bot:{}", delivery.id)).await
}

async fn post_note(state: &BotState, delivery: &Claimed, note: &str) -> Result<(), sqlx::Error> {
    post_as_bot(
        state,
        delivery,
        note.to_owned(),
        format!("bot-note:{}", delivery.id),
    )
    .await?;
    Ok(())
}

/// A mention in a channel that is not the bot's: the bot tells the author in a DM where it works,
/// as an agent does (step 6). The message is not sent to the bot.
async fn refuse(state: &BotState, delivery: &Claimed) -> Result<(), sqlx::Error> {
    let chat = &state.chat;
    let (channel, channels) = {
        let mut conn = state.database().pool().acquire().await?;
        let channel: Option<String> =
            sqlx::query_scalar("SELECT name FROM chat_conversations WHERE id = $1")
                .bind(delivery.conversation_id.to_string())
                .fetch_optional(&mut *conn)
                .await?
                .flatten();
        let channels: Vec<String> = sqlx::query_scalar(
            "SELECT c.name FROM webhook_bot_channels b JOIN chat_conversations c ON c.id = b.conversation_id \
             WHERE b.bot_id = $1 AND c.archived_at IS NULL ORDER BY c.name",
        )
        .bind(delivery.bot_id.to_string())
        .fetch_all(&mut *conn)
        .await?;
        (channel.unwrap_or_default(), channels)
    };
    let dm = match chat
        .publish(
            delivery.workspace_id,
            delivery.bot_user,
            chat.chat.open_dm(
                delivery.workspace_id,
                delivery.bot_user,
                vec![delivery.author_id],
            ),
        )
        .await
    {
        Ok((written, _)) => written.value.id,
        Err(ChatError::Unavailable(error)) => return Err(error),
        Err(error) => {
            tracing::info!(%error, "webhook bot refusal DM");
            return Ok(());
        }
    };
    let list = if channels.is_empty() {
        "only in direct messages".to_owned()
    } else {
        format!(
            "only in {} and in direct messages",
            channels
                .iter()
                .map(|name| format!("#{name}"))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    let body = format!(
        "I can't answer in #{channel}: I work {list}. Ask me here, or ask an owner or admin to add \
         that channel in Settings → Agentic → Bots."
    );
    let refusal = Claimed {
        conversation_id: dm,
        root: None,
        ..delivery.clone()
    };
    post_as_bot(
        state,
        &refusal,
        body,
        format!("bot-refusal:{}", delivery.id),
    )
    .await?;
    Ok(())
}

/// Posts the note of every delivery whose 30 seconds are over without an answer (a bot that took
/// the message but never replied, or a delivery a stopped server could not send).
pub async fn sweep(state: &BotState) -> Result<usize, sqlx::Error> {
    let ids: Vec<String> = sqlx::query_scalar(
        "SELECT id FROM webhook_bot_deliveries WHERE status IN ('pending', 'sending') \
         AND expires_at <= $1 ORDER BY expires_at LIMIT 100",
    )
    .bind(TimestampMillis::now().as_millis())
    .fetch_all(state.database().pool())
    .await?;
    let mut noted = 0;
    for id in ids {
        match load(state, &id).await? {
            Some(delivery) => {
                expire(state, &delivery).await?;
                noted += 1;
            }
            None => {
                finish(
                    state,
                    &id,
                    &["pending", "sending"],
                    "failed",
                    None,
                    Some("bot_deleted"),
                )
                .await?;
            }
        }
    }
    Ok(noted)
}

/// Sends every delivery that waits and handles the answers in the HTTP bodies; returns how many.
/// The service does the same in the background; tests call it to run one round.
pub async fn dispatch(state: &BotState) -> Result<usize, sqlx::Error> {
    let claimed = claim(state).await?;
    let count = claimed.len();
    let tasks: Vec<_> = claimed
        .into_iter()
        .map(|delivery| tokio::spawn(deliver(state.clone(), delivery)))
        .collect();
    for task in tasks {
        let _ = task.await;
    }
    Ok(count)
}

/// The dispatcher: sends new deliveries as soon as they are committed (or within a second when
/// another server took the message) and posts the notes of deliveries without an answer. A
/// database error waits for the next round; it never stops the server.
pub async fn run_service(state: BotState, shutdown: CancellationToken) -> Result<(), String> {
    let mut tasks = tokio::task::JoinSet::new();
    let mut last_cleanup = std::time::Instant::now();
    loop {
        tokio::select! {
            () = shutdown.cancelled() => break,
            () = WAKE.notified() => {}
            () = tokio::time::sleep(Duration::from_secs(1)) => {}
        }
        match claim(&state).await {
            Ok(claimed) => {
                for delivery in claimed {
                    tasks.spawn(deliver(state.clone(), delivery));
                }
            }
            Err(error) => tracing::warn!(%error, "webhook bot deliveries"),
        }
        if let Err(error) = sweep(&state).await {
            tracing::warn!(%error, "webhook bot deadlines");
        }
        if last_cleanup.elapsed() >= Duration::from_secs(60 * 60) {
            last_cleanup = std::time::Instant::now();
            if let Err(error) = forget_old(&state).await {
                tracing::warn!(%error, "webhook bot delivery cleanup");
            }
        }
        while tasks.try_join_next().is_some() {}
    }
    tasks.abort_all();
    Ok(())
}

/// Deletes finished deliveries older than 30 days: the rate limit needs the last minute, and the
/// settings show the 20 newest.
pub async fn forget_old(state: &BotState) -> Result<u64, sqlx::Error> {
    Ok(sqlx::query(
        "DELETE FROM webhook_bot_deliveries WHERE created_at < $1 AND status NOT IN ('pending', 'sending')",
    )
    .bind(TimestampMillis::now().as_millis() - 30 * 24 * 60 * 60 * 1000)
    .execute(state.database().pool())
    .await?
    .rows_affected())
}

/// Takes the delivery of `bot_id` with this reply token while it waits for an answer, once.
pub(crate) async fn use_reply_token(
    state: &BotState,
    bot_id: Id,
    token: &str,
) -> Result<Option<Id>, sqlx::Error> {
    let id: Option<String> = sqlx::query_scalar(
        "SELECT id FROM webhook_bot_deliveries WHERE reply_token_hash = $1 AND bot_id = $2 \
         AND status = 'sending' AND expires_at > $3",
    )
    .bind(token_hash(token))
    .bind(bot_id.to_string())
    .bind(TimestampMillis::now().as_millis())
    .fetch_optional(state.database().pool())
    .await?;
    let Some(id) = id else {
        return Ok(None);
    };
    if !finish(state, &id, &["sending"], "answered", None, None).await? {
        return Ok(None);
    }
    Ok(id.parse().ok())
}

/// Posts the async reply of a delivery that [`use_reply_token`] took.
pub(crate) async fn post_async_reply(
    state: &BotState,
    delivery_id: Id,
    text: &str,
) -> Result<Option<MessageRecord>, sqlx::Error> {
    let Some(delivery) = load(state, &delivery_id.to_string()).await? else {
        return Ok(None);
    };
    post_reply(state, &delivery, text).await
}

/// The JSON a delivery list shows to admins: never the payload or the token.
pub(crate) fn delivery_view(row: &sqlx::any::AnyRow) -> serde_json::Value {
    json!({
        "id": row.get::<String, _>("id"),
        "status": row.get::<String, _>("status"),
        "http_status": row.get::<Option<i64>, _>("http_status"),
        "error": row.get::<Option<String>, _>("error"),
        "created_at": row.get::<i64, _>("created_at"),
        "finished_at": row.get::<Option<i64>, _>("finished_at"),
    })
}

#[cfg(test)]
mod tests {
    use super::{Answer, VerifyError, clean_reply, parse_answer, sign, verify};

    #[test]
    fn a_signature_verifies_only_with_its_secret_body_and_time() {
        let body = br#"{"type":"message"}"#;
        let signature = sign("obs_secret", 1_700_000_000, body);
        assert!(signature.starts_with("v1=") && signature.len() == 67);
        assert_eq!(
            verify("obs_secret", "1700000000", body, &signature, 1_700_000_100),
            Ok(())
        );
        assert_eq!(
            verify("obs_other", "1700000000", body, &signature, 1_700_000_100),
            Err(VerifyError::BadSignature)
        );
        assert_eq!(
            verify("obs_secret", "1700000000", b"{}", &signature, 1_700_000_100),
            Err(VerifyError::BadSignature)
        );
        // The timestamp is signed: another one with the same signature fails.
        assert_eq!(
            verify("obs_secret", "1700000001", body, &signature, 1_700_000_100),
            Err(VerifyError::BadSignature)
        );
        for bad in ["", "v1=", "v0=00", &signature.replace("v1=", "v2=")] {
            assert_eq!(
                verify("obs_secret", "1700000000", body, bad, 1_700_000_000),
                Err(VerifyError::BadSignature),
                "{bad}"
            );
        }
    }

    #[test]
    fn a_signature_older_or_newer_than_five_minutes_is_a_replay() {
        let body = b"{}";
        let signature = sign("obs_secret", 1_700_000_000, body);
        assert!(verify("obs_secret", "1700000000", body, &signature, 1_700_000_300).is_ok());
        assert!(verify("obs_secret", "1700000000", body, &signature, 1_699_999_700).is_ok());
        assert_eq!(
            verify("obs_secret", "1700000000", body, &signature, 1_700_000_301),
            Err(VerifyError::Stale)
        );
        assert_eq!(
            verify("obs_secret", "1700000000", body, &signature, 1_699_999_699),
            Err(VerifyError::Stale)
        );
        assert_eq!(
            verify("obs_secret", "soon", body, &signature, 1_700_000_000),
            Err(VerifyError::Stale)
        );
    }

    #[test]
    fn an_answer_body_is_a_reply_later_or_invalid() {
        assert_eq!(
            parse_answer(br#"{"text":"Hi"}"#),
            Some(Answer::Reply("Hi".to_owned()))
        );
        for later in [
            &b""[..],
            b"  \n",
            b"{}",
            br#"{"text":null}"#,
            br#"{"text":"  "}"#,
        ] {
            assert_eq!(parse_answer(later), Some(Answer::Later));
        }
        assert!(parse_answer(b"<html>").is_none());
        assert!(parse_answer(br#"{"text":5}"#).is_none());
    }

    #[test]
    fn a_reply_never_notifies_a_whole_channel_and_is_cut() {
        assert_eq!(
            clean_reply(" <!channel> and <!here> \r\n"),
            "@channel and @here"
        );
        assert_eq!(clean_reply(&"a".repeat(5_000)).chars().count(), 4_000);
    }
}
