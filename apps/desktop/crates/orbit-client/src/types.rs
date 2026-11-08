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
