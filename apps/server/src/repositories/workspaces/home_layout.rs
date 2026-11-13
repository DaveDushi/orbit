//! Each person's Home cards (`home_layouts`): one ordered list of cards per user, which the web
//! and the iPhone app share. The catalog lives here; reads and saves drop unknown cards, repeated
//! cards and cards the caller's role may not use in the workspace. Each card's data still comes
//! from its own API with that API's checks.

use orbit_platform::{Id, Row, TimestampMillis};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::{WorkspaceError, WorkspaceRepository, require_chat_actor};

/// Every card, in picker order, with whether it shows chat data (the only cards a guest gets).
const CARDS: &[(&str, bool)] = &[
    ("my_tasks", false),
    ("due_soon", false),
    ("overdue", false),
    ("inbox", false),
    ("activity", false),
    ("unread_chat", true),
    ("recent_docs", false),
    ("recent_files", false),
    ("ask_donna", false),
    ("routines", false),
];

/// The layout of someone who never saved one (the cards the role may use, in this order).
const DEFAULT_CARDS: &[&str] = &["overdue", "due_soon", "inbox", "recent_docs", "unread_chat"];

const MIN_CARD_LIMIT: i64 = 3;
const MAX_CARD_LIMIT: i64 = 20;

/// One card of a Home layout.
#[derive(Clone, Debug, Deserialize, Serialize, ToSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
#[schema(as = HomeCard)]
pub struct HomeCard {
    /// `my_tasks`, `due_soon`, `overdue`, `inbox`, `activity`, `unread_chat`, `recent_docs`,
    /// `recent_files`, `ask_donna` or `routines`. Other ids are dropped.
    pub id: String,
    /// Rows the card shows, 3–20 (a value outside is moved into the range). Absent: 7.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(minimum = 3, maximum = 20)]
    pub limit: Option<i64>,
}

/// The caller's Home cards in one workspace.
#[derive(Clone, Debug, Serialize, ToSchema)]
#[schema(as = HomeLayout)]
pub struct HomeLayout {
    /// The cards in order, only those the caller may use here.
    pub cards: Vec<HomeCard>,
    /// Every card the caller may add here, in picker order.
    pub available: Vec<String>,
    /// `false` while the caller never saved a layout (the default shows).
    pub customized: bool,
}

fn chat_card(id: &str) -> Option<bool> {
    CARDS
        .iter()
        .find(|(card, _)| *card == id)
        .map(|(_, chat)| *chat)
}

/// Known cards the caller may use, each once, with the limit moved into its range.
fn allowed(cards: &[HomeCard], actor: orbit_domain::Actor) -> Vec<HomeCard> {
    let mut kept: Vec<HomeCard> = Vec::new();
    for card in cards {
        let usable = chat_card(&card.id).is_some_and(|chat| actor.can_use_home_card(chat));
        if usable && kept.iter().all(|other| other.id != card.id) {
            kept.push(HomeCard {
                id: card.id.clone(),
                limit: card
                    .limit
                    .map(|limit| limit.clamp(MIN_CARD_LIMIT, MAX_CARD_LIMIT)),
            });
        }
    }
    kept
}

fn decode(cards_json: &str) -> Vec<HomeCard> {
    // a row that no longer decodes falls back to the default rather than failing Home
    serde_json::from_str(cards_json).unwrap_or_default()
}

impl WorkspaceRepository {
    /// The caller's layout as `workspace_id` lets them see it; the default when none was saved.
    pub async fn home_layout(
        &self,
        workspace_id: Id,
        actor_id: Id,
    ) -> Result<HomeLayout, WorkspaceError> {
        let actor = require_chat_actor(self.database.pool(), workspace_id, actor_id).await?;
        let stored = sqlx::query("SELECT cards_json FROM home_layouts WHERE user_id = $1")
            .bind(actor_id.to_string())
            .fetch_optional(self.database.pool())
            .await?
            .map(|row| decode(row.get::<String, _>("cards_json").as_str()));
        Ok(layout_for(stored, actor))
    }

    /// Saves the caller's layout from `workspace_id` and returns it as read back. Cards the
    /// caller may not use here are dropped; saved cards that another workspace allows but this
    /// one does not (a guest here, a member elsewhere) stay, after the new ones.
    pub async fn save_home_layout(
        &self,
        workspace_id: Id,
        actor_id: Id,
        cards: &[HomeCard],
        now: TimestampMillis,
    ) -> Result<HomeLayout, WorkspaceError> {
        let mut transaction = self.database.immediate_transaction().await?;
        let actor = require_chat_actor(&mut *transaction, workspace_id, actor_id).await?;
        let mut kept = allowed(cards, actor);
        let stored = sqlx::query("SELECT cards_json FROM home_layouts WHERE user_id = $1")
            .bind(actor_id.to_string())
            .fetch_optional(&mut *transaction)
            .await?
            .map(|row| decode(row.get::<String, _>("cards_json").as_str()))
            .unwrap_or_default();
        for card in stored {
            let hidden_here =
                chat_card(&card.id).is_some_and(|chat| !actor.can_use_home_card(chat));
            if hidden_here && kept.iter().all(|other| other.id != card.id) {
                kept.push(card);
            }
        }
        let cards_json = serde_json::json!(kept).to_string();
        sqlx::query(
            "INSERT INTO home_layouts (user_id, cards_json, updated_at) VALUES ($1, $2, $3) \
             ON CONFLICT (user_id) DO UPDATE SET cards_json = excluded.cards_json, \
             updated_at = excluded.updated_at",
        )
        .bind(actor_id.to_string())
        .bind(cards_json)
        .bind(now.as_millis())
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(layout_for(Some(kept), actor))
    }
}

fn layout_for(stored: Option<Vec<HomeCard>>, actor: orbit_domain::Actor) -> HomeLayout {
    let customized = stored.is_some();
    let cards = stored.unwrap_or_else(|| {
        DEFAULT_CARDS
            .iter()
            .map(|id| HomeCard {
                id: (*id).to_owned(),
                limit: None,
            })
            .collect()
    });
    HomeLayout {
        cards: allowed(&cards, actor),
        available: CARDS
            .iter()
            .filter(|(_, chat)| actor.can_use_home_card(*chat))
            .map(|(id, _)| (*id).to_owned())
            .collect(),
        customized,
    }
}
