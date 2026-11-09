//! Response types of `/api/v1`, by their OpenAPI schema names. Unknown fields are ignored and
//! unknown enum values read as `Unknown`, so a newer server does not break an older app.

use serde::{Deserialize, Serialize};

/// `ConversationKind`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConversationKind {
    Public,
    Private,
    Dm,
    #[serde(other)]
    Unknown,
}

/// `NotifyLevel`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NotifyLevel {
    All,
    Mentions,
    Muted,
    #[serde(other)]
    Unknown,
}

/// `ConversationRecord`: an item of `GET /api/v1/workspaces/{workspace_id}/chat/conversations`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConversationRecord {
    pub id: String,
    pub kind: ConversationKind,
    pub name: String,
    #[serde(default)]
    pub topic: String,
    #[serde(default)]
    pub category_id: Option<String>,
    #[serde(default)]
    pub position: i64,
    #[serde(default)]
    pub member_ids: Vec<String>,
    #[serde(default)]
    pub archived: bool,
    #[serde(default)]
    pub self_dm: bool,
    #[serde(default)]
    pub last_message_at: Option<String>,
}

/// `ConversationStateRecord`: an item of `GET /api/v1/workspaces/{workspace_id}/chat/states`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConversationStateRecord {
    pub conversation_id: String,
    #[serde(default)]
    pub unread_count: i64,
    #[serde(default)]
    pub mention_count: i64,
    pub notify: NotifyLevel,
    #[serde(default)]
    pub favorite: bool,
}

/// `SetupStatus`: `GET /api/v1/setup/status`, the probe that tells an Orbit server (and its gateway) apart.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SetupStatus {
    pub complete: bool,
}

/// `AuthUserResponse`: `GET /api/v1/auth/me` and the `user` of a sign-in.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AuthUser {
    pub id: String,
    pub email: String,
    pub display_name: String,
}

/// `WorkspaceRecord`: an item of `GET /api/v1/workspaces`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceRecord {
    pub id: String,
    pub name: String,
}

/// `MessageKind`: `pin`, `join` and `leave` are one-line system rows about `author_id`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageKind {
    Message,
    Pin,
    Join,
    Leave,
    #[serde(other)]
    Unknown,
}

/// `ReactionRecord`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReactionRecord {
    pub emoji: String,
    #[serde(default)]
    pub user_ids: Vec<String>,
}

/// `MessageRecord`: the fields the app reads. `body` is Markdown with mention tokens
/// (`<@user_id>`, `<#conversation_id>`, `<!channel>`, `<!here>`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MessageRecord {
    pub id: String,
    pub conversation_id: String,
    #[serde(default)]
    pub thread_root_id: Option<String>,
    pub kind: MessageKind,
    pub author_id: String,
    #[serde(default)]
    pub body: String,
    pub created_at: String,
    #[serde(default)]
    pub edited_at: Option<String>,
    #[serde(default)]
    pub deleted: bool,
    #[serde(default)]
    pub reactions: Vec<ReactionRecord>,
    #[serde(default)]
    pub also_in_channel: bool,
    /// The sender's nonce, so the sender replaces its local row.
    #[serde(default)]
    pub nonce: Option<String>,
    #[serde(default)]
    pub reply_count: i64,
    #[serde(default)]
    pub last_reply_at: Option<String>,
}

/// `MessagePage`: messages in ascending order; a null cursor means nothing more that way.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MessagePage {
    pub items: Vec<MessageRecord>,
    #[serde(default)]
    pub before: Option<String>,
    #[serde(default)]
    pub after: Option<String>,
}

/// `ThreadPage`: `GET …/chat/threads/{root_id}`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ThreadPage {
    pub root: MessageRecord,
    pub items: Vec<MessageRecord>,
    #[serde(default)]
    pub before: Option<String>,
    #[serde(default)]
    pub after: Option<String>,
}

/// `MemberRecord`: the fields the app reads (names of message authors).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MemberRecord {
    pub user_id: String,
    pub display_name: String,
}

/// `Page_MemberRecord`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MemberPage {
    pub items: Vec<MemberRecord>,
}

/// `ChatWrite_MessageRecord`: a send or an edit answers the changed message.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MessageWrite {
    pub result: MessageRecord,
}
