//! Clicks on an integration's buttons. A click becomes a Slack `block_actions` payload for the
//! webhook's request URL (the route sends it, signed) with a `response_url`: a token that lets the
//! integration answer five times in thirty minutes by replacing or deleting the message, posting a
//! new one, or showing a reply to the member who clicked only.
//!
//! A click on a card of the team agent (a `ui` block) goes to no webhook: it becomes the agent's
//! input from the member who clicked, in the card's thread, without a chat message
//! ([`ChatRepository::queue_agent_action`]).

use orbit_platform::{Id, Row, TimestampMillis, generate_opaque_token};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use super::blocks::find_button;
use super::messages::{bot_message_parts, remove_message, rewrite_message, webhook_interactive};
use super::ui::find_ui;
use super::webhooks::{BotSender, HookTarget, decrypt};
use super::{
    BotPost, ChatError, ChatEvent, ChatRepository, ConversationKind, Events, MentionsRecord,
    MessageKind, MessageRecord, Written, finish, load_conversation, load_message,
    load_message_access, mentioned_user_ids, parse_id,
};
use crate::agent::inputs::{InputKind, bot_user_id, thread_of};

/// How long and how often an integration may answer one click (as Slack's `response_url`).
const RESPONSE_TTL_MS: i64 = 30 * 60 * 1000;
const RESPONSE_USES: i64 = 5;
/// The form values of one submit, as JSON.
const FORM_VALUES_MAX_BYTES: usize = 2048;
const FORM_VALUE_MAX_CHARS: usize = 2000;
/// Clicks and form submits on one card of the agent that reach the agent in 30 minutes.
const CARD_ACTIONS: i64 = 5;
const CARD_ACTIONS_WINDOW_MS: i64 = 30 * 60 * 1000;

/// A click or form submit that a member sent from a card of the team agent.
#[derive(Clone, Debug, serde::Serialize, utoipa::ToSchema)]
pub struct SentAction {
    pub block_id: String,
    /// The button or form.
    pub element_id: String,
    /// A form's values, as sent.
    #[schema(value_type = Option<Object>)]
    pub values: Option<Value>,
}

/// A click, ready to send: where, the secret to sign with, and the payload.
pub struct Click {
    pub webhook_id: Id,
    pub action_id: String,
    pub request_url: String,
    pub signing_secret: String,
    pub payload: Value,
}

/// What an answer through a `response_url` acts on.
pub struct ResponseTarget {
    pub target: HookTarget,
    /// The message with the button.
    pub message_id: Id,
    /// The member who clicked: the reader of an ephemeral reply.
    pub user_id: Id,
}

fn token_hash(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

/// The form values of a submit: only the form's fields, as text or a checkbox's true/false, and
/// the required ones filled in.
fn form_values(props: &Value, values: Option<&Map<String, Value>>) -> Result<Value, ChatError> {
    let invalid = ChatError::Invalid { field: "values" };
    let values = values.ok_or(ChatError::Invalid { field: "values" })?;
    if Value::Object(values.clone()).to_string().len() > FORM_VALUES_MAX_BYTES {
        return Err(invalid);
    }
    let fields = props.get("fields").and_then(Value::as_array);
    let field = |name: &str| {
        fields
            .into_iter()
            .flatten()
            .find(|field| field.get("name").and_then(Value::as_str) == Some(name))
    };
    if values.keys().any(|name| field(name).is_none()) {
        return Err(invalid);
    }
    for field in fields.into_iter().flatten() {
        let name = field
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let filled = match values.get(name) {
            None | Some(Value::Null) => false,
            Some(Value::Bool(_)) => true,
            Some(Value::String(text)) if text.chars().count() <= FORM_VALUE_MAX_CHARS => {
                !text.trim().is_empty()
            }
            Some(_) => return Err(invalid),
        };
        if !filled && field.get("required").and_then(Value::as_bool) == Some(true) {
            return Err(invalid);
        }
    }
    Ok(Value::Object(values.clone()))
}

impl ChatRepository {
    /// Queues a click by `actor_id` on a card of the team agent as the agent's input, in the
    /// card's thread. No chat message reports it: the clicker sees "Sent" on the card and then the
    /// agent's answer. Returns `false` when the message is not the agent's, so the click is an
    /// integration's. What the click does (the button's label and params) comes from the stored
    /// card; only form `values` come from the member, and they must fit the form's fields.
    pub async fn queue_agent_action(
        &self,
        workspace_id: Id,
        actor_id: Id,
        message_id: Id,
        block_id: &str,
        element_id: &str,
        values: Option<&Map<String, Value>>,
    ) -> Result<bool, ChatError> {
        let now = TimestampMillis::now();
        let mut tx = self.database.immediate_transaction().await?;
        let (access, message) =
            load_message_access(&mut tx, workspace_id, actor_id, message_id).await?;
        let agent = bot_user_id(&mut tx).await?;
        if message.bot.is_some() || agent != Some(message.author_id) {
            return Ok(false);
        }
        access.require_member()?;
        access.require_open()?;
        if message.deleted {
            return Err(ChatError::NotFound);
        }
        let card = message
            .blocks
            .as_deref()
            .and_then(|blocks| find_ui(blocks, block_id))
            .ok_or(ChatError::NotFound)?;
        if card.live {
            return Err(ChatError::Conflict(
                "The agent is still writing this card. Try again when it is done.",
            ));
        }
        let element = card
            .spec
            .get("elements")
            .and_then(|elements| elements.get(element_id))
            .ok_or(ChatError::NotFound)?;
        let props = element.get("props").cloned().unwrap_or_else(|| json!({}));
        let text = |key: &str| props.get(key).and_then(Value::as_str).unwrap_or_default();
        let mut action = match element.get("type").and_then(Value::as_str) {
            Some("Button") if text("action") == "reply" => {
                if values.is_some_and(|values| !values.is_empty()) {
                    return Err(ChatError::Invalid { field: "values" });
                }
                json!({
                    "type": "button",
                    "action": "reply",
                    "label": text("label"),
                    "params": props.get("params").cloned().unwrap_or(Value::Null),
                })
            }
            Some("Form") => {
                let label = Some(text("submit_label")).filter(|label| !label.is_empty());
                json!({
                    "type": "form",
                    "action": "submit_form",
                    "label": label.unwrap_or("Send"),
                    "values": form_values(&props, values)?,
                })
            }
            _ => return Err(ChatError::Invalid { field: "action_id" }),
        };
        action["card"] = json!({ "message_id": message.id.to_string(), "block_id": block_id, "title": card.title, "element_id": element_id });

        let recent: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM agent_inputs WHERE card_message_id = $1 AND created_at > $2",
        )
        .bind(message.id.to_string())
        .bind(now.as_millis() - CARD_ACTIONS_WINDOW_MS)
        .fetch_one(&mut *tx)
        .await?;
        if recent >= CARD_ACTIONS {
            return Err(ChatError::RateLimited(
                "This card took 5 clicks in the last 30 minutes. Write to the agent instead.",
            ));
        }
        let is_dm = access.conversation.kind == ConversationKind::Dm;
        let (thread_key, root) = thread_of(&message, message.conversation_id, is_dm);
        sqlx::query(
            "INSERT INTO agent_inputs (message_id, workspace_id, conversation_id, thread_key, \
             root_message_id, author_id, kind, created_at, card_message_id, action) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
        )
        .bind(Id::new_v7().to_string())
        .bind(workspace_id.to_string())
        .bind(message.conversation_id.to_string())
        .bind(thread_key.to_string())
        .bind(root.map(|id| id.to_string()))
        .bind(actor_id.to_string())
        .bind(InputKind::Action.as_str())
        .bind(now.as_millis())
        .bind(message.id.to_string())
        .bind(action.to_string())
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(true)
    }

    /// What `actor_id` sent from the agent's card on `message_id`, oldest first: the element
    /// (button or form) and, for a form, the values. The card shows those as sent.
    pub async fn sent_agent_actions(
        &self,
        workspace_id: Id,
        actor_id: Id,
        message_id: Id,
    ) -> Result<Vec<SentAction>, ChatError> {
        let mut conn = self.database.pool().acquire().await?;
        let (access, message) =
            load_message_access(&mut conn, workspace_id, actor_id, message_id).await?;
        access.require_member()?;
        if message.deleted {
            return Err(ChatError::NotFound);
        }
        let rows = sqlx::query(
            "SELECT action FROM agent_inputs WHERE card_message_id = $1 AND author_id = $2 \
             ORDER BY created_at, message_id",
        )
        .bind(message.id.to_string())
        .bind(actor_id.to_string())
        .fetch_all(&mut *conn)
        .await?;
        Ok(rows
            .iter()
            .filter_map(|row| {
                serde_json::from_str::<Value>(&row.get::<Option<String>, _>("action")?).ok()
            })
            .filter_map(|action| {
                Some(SentAction {
                    block_id: action["card"]["block_id"].as_str()?.to_owned(),
                    element_id: action["card"]["element_id"].as_str()?.to_owned(),
                    values: action.get("values").cloned().filter(Value::is_object),
                })
            })
            .collect())
    }

    /// Checks a click by `actor_id` and makes its payload and response token. Only the button's
    /// position comes from the member; its `value` is read from the stored message.
    #[allow(clippy::too_many_arguments)]
    pub async fn prepare_click(
        &self,
        workspace_id: Id,
        actor_id: Id,
        message_id: Id,
        block_id: &str,
        action_id: &str,
        key: &[u8; 32],
        public_origin: &str,
    ) -> Result<Click, ChatError> {
        let now = TimestampMillis::now();
        let mut tx = self.database.immediate_transaction().await?;
        let (access, message) =
            load_message_access(&mut tx, workspace_id, actor_id, message_id).await?;
        access.require_member()?;
        access.require_open()?;
        let (Some(bot), Some(blocks), false) = (&message.bot, &message.blocks, message.deleted)
        else {
            return Err(ChatError::NotFound);
        };
        let button = find_button(blocks, block_id, action_id).ok_or(ChatError::NotFound)?;
        let webhook = sqlx::query(
            "SELECT request_url, signing_secret FROM chat_webhooks \
             WHERE id = $1 AND revoked_at IS NULL AND request_url IS NOT NULL",
        )
        .bind(bot.webhook_id.to_string())
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(ChatError::Forbidden(
            "This integration does not take clicks.",
        ))?;
        let request_url: String = webhook.get("request_url");
        let signing_secret = decrypt(
            key,
            &webhook
                .get::<Option<Vec<u8>>, _>("signing_secret")
                .unwrap_or_default(),
        )?;
        let user = sqlx::query("SELECT display_name FROM users WHERE id = $1")
            .bind(actor_id.to_string())
            .fetch_one(&mut *tx)
            .await?;
        let display_name: String = user.get("display_name");
        let workspace_name: String =
            sqlx::query_scalar("SELECT name FROM workspaces WHERE id = $1")
                .bind(workspace_id.to_string())
                .fetch_one(&mut *tx)
                .await?;

        sqlx::query("DELETE FROM chat_action_responses WHERE expires_at < $1")
            .bind(now.as_millis())
            .execute(&mut *tx)
            .await?;
        let token = generate_opaque_token();
        sqlx::query(
            "INSERT INTO chat_action_responses (token_hash, webhook_id, message_id, conversation_id, \
             user_id, uses_left, expires_at, created_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(token_hash(&token))
        .bind(bot.webhook_id.to_string())
        .bind(message.id.to_string())
        .bind(message.conversation_id.to_string())
        .bind(actor_id.to_string())
        .bind(RESPONSE_USES)
        .bind(now.as_millis() + RESPONSE_TTL_MS)
        .bind(now.as_millis())
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;

        let seconds = |millis: i64| format!("{}.{:03}", millis / 1000, millis % 1000);
        let payload = json!({
            "type": "block_actions",
            "api_app_id": bot.webhook_id.to_string(),
            "token": "",
            "trigger_id": generate_opaque_token(),
            "team": { "id": workspace_id.to_string(), "domain": workspace_name },
            "user": {
                "id": actor_id.to_string(),
                "username": display_name,
                "name": display_name,
                "team_id": workspace_id.to_string(),
            },
            "channel": {
                "id": message.conversation_id.to_string(),
                "name": access.conversation.name,
            },
            "container": {
                "type": "message",
                "message_ts": message.id.to_string(),
                "channel_id": message.conversation_id.to_string(),
                "is_ephemeral": false,
            },
            "message": {
                "type": "message",
                "ts": message.id.to_string(),
                "text": message.body,
                "blocks": blocks,
            },
            "response_url": format!(
                "{}/api/v1/hooks/chat/{}/responses/{token}",
                public_origin.trim_end_matches('/'),
                bot.webhook_id
            ),
            "actions": [{
                "type": "button",
                "action_id": action_id,
                "block_id": block_id,
                "text": button.text,
                "value": button.value,
                "style": button.style,
                "action_ts": seconds(now.as_millis()),
            }],
        });
        Ok(Click {
            webhook_id: bot.webhook_id,
            action_id: action_id.to_owned(),
            request_url,
            signing_secret,
            payload,
        })
    }

    /// Spends one use of a response token. `None` for an unknown, used-up or expired token, or a
    /// deleted webhook.
    pub async fn use_response_token(
        &self,
        webhook_id: Id,
        token: &str,
    ) -> Result<Option<ResponseTarget>, ChatError> {
        let now = TimestampMillis::now();
        let mut tx = self.database.immediate_transaction().await?;
        let row = sqlx::query(
            "SELECT r.message_id, r.conversation_id, r.user_id, h.workspace_id, h.name, h.icon_url, \
             h.created_by FROM chat_action_responses r \
             JOIN chat_webhooks h ON h.id = r.webhook_id AND h.revoked_at IS NULL \
             JOIN workspaces w ON w.id = h.workspace_id AND w.deleted_at IS NULL \
             WHERE r.token_hash = $1 AND r.webhook_id = $2 AND r.uses_left > 0 AND r.expires_at >= $3",
        )
        .bind(token_hash(token))
        .bind(webhook_id.to_string())
        .bind(now.as_millis())
        .fetch_optional(&mut *tx)
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        sqlx::query(
            "UPDATE chat_action_responses SET uses_left = uses_left - 1 WHERE token_hash = $1",
        )
        .bind(token_hash(token))
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(Some(ResponseTarget {
            message_id: parse_id(row.get("message_id"))?,
            user_id: parse_id(row.get("user_id"))?,
            target: HookTarget {
                workspace_id: parse_id(row.get("workspace_id"))?,
                conversation_id: parse_id(row.get("conversation_id"))?,
                sender: BotSender {
                    webhook_id,
                    created_by: parse_id(row.get("created_by"))?,
                    name: row.get("name"),
                    icon_url: row.get("icon_url"),
                },
            },
        }))
    }

    /// The message with the button, if it is still this webhook's and in an open channel.
    async fn original(
        conn: &mut sqlx::AnyConnection,
        response: &ResponseTarget,
    ) -> Result<MessageRecord, ChatError> {
        let conversation = load_conversation(
            conn,
            response.target.workspace_id,
            response.target.conversation_id,
        )
        .await?
        .filter(|conversation| conversation.kind != ConversationKind::Dm)
        .ok_or(ChatError::NotFound)?;
        if conversation.archived_at.is_some() {
            return Err(ChatError::Forbidden("This channel is archived."));
        }
        load_message(conn, response.message_id)
            .await?
            .filter(|message| {
                !message.deleted
                    && message.bot.as_ref().map(|bot| bot.webhook_id)
                        == Some(response.target.sender.webhook_id)
            })
            .ok_or(ChatError::NotFound)
    }

    /// `replace_original`: the message gets the answer's text, blocks, name and icon. It is not
    /// marked as edited, and its mentions are counted again.
    pub async fn replace_bot_message(
        &self,
        response: &ResponseTarget,
        post: BotPost,
    ) -> Result<Written<MessageRecord>, ChatError> {
        let (body, bot, blocks) = bot_message_parts(post, &response.target.sender)?;
        let mut tx = self.database.immediate_transaction().await?;
        let message = Self::original(&mut tx, response).await?;
        sqlx::query(
            "UPDATE chat_messages SET bot_name = $1, bot_icon_url = $2, bot_icon_emoji = $3 WHERE id = $4",
        )
        .bind(&bot.name)
        .bind(bot.icon_url.as_deref())
        .bind(bot.icon_emoji.as_deref())
        .bind(message.id.to_string())
        .execute(&mut *tx)
        .await?;
        let (events, next) =
            rewrite_message(&mut tx, &message, &body, blocks.as_deref(), None).await?;
        finish(tx, events, next).await
    }

    /// `delete_original`: the integration owns its message, so no member permission applies.
    pub async fn delete_bot_message(
        &self,
        response: &ResponseTarget,
    ) -> Result<Written<()>, ChatError> {
        let now = TimestampMillis::now();
        let mut tx = self.database.immediate_transaction().await?;
        let message = Self::original(&mut tx, response).await?;
        let events = remove_message(
            &mut tx,
            response.target.workspace_id,
            response.target.sender.webhook_id,
            &message,
            now,
        )
        .await?;
        finish(tx, events, ()).await
    }

    /// `ephemeral`: a message that only the member who clicked sees, live, until they reload. It
    /// is not stored.
    pub async fn ephemeral_bot_message(
        &self,
        response: &ResponseTarget,
        post: BotPost,
    ) -> Result<Written<MessageRecord>, ChatError> {
        let (body, bot, blocks) = bot_message_parts(post, &response.target.sender)?;
        let mut conn = self.database.pool().acquire().await?;
        let interactive = webhook_interactive(&mut conn, bot.webhook_id).await?;
        let message = MessageRecord {
            id: Id::new_v7(),
            conversation_id: response.target.conversation_id,
            thread_root_id: None,
            kind: MessageKind::Message,
            author_id: response.target.sender.created_by,
            mentions: MentionsRecord {
                user_ids: mentioned_user_ids(&body),
                channel: false,
                here: false,
            },
            body,
            created_at: TimestampMillis::now(),
            edited_at: None,
            deleted: false,
            attachments: Vec::new(),
            reactions: Vec::new(),
            pinned: false,
            also_in_channel: false,
            nonce: None,
            reply_count: 0,
            last_reply_at: None,
            reply_user_ids: Vec::new(),
            last_reply: None,
            reply_to_id: None,
            reply_to: None,
            forwarded: None,
            sticker_id: None,
            sticker: None,
            bot: Some(super::BotRecord { interactive, ..bot }),
            blocks,
            last_reply_id: None,
        };
        let mut events = Events::default();
        events.user(
            response.user_id,
            ChatEvent::MessageEphemeral {
                message: message.clone(),
            },
        );
        Ok(events.written(message))
    }
}
