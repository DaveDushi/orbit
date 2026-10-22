//! `workspace_subscriptions` and `stripe_events` (Orbit Cloud billing, see `crate::billing`).

use orbit_platform::{Database, Id, Row, TimestampMillis};
use serde_json::json;

use crate::audit::{self, AuditOutcome};
use crate::billing::{SubscriptionStatus, WorkspaceAccess, access};

/// A workspace's billing row as stored.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct WorkspaceSubscription {
    pub stripe_customer_id: Option<String>,
    pub plan: Option<String>,
    /// `None` without a subscription, or for a status this server does not know.
    pub status: Option<SubscriptionStatus>,
    pub cancel_at_period_end: bool,
    /// Unix milliseconds.
    pub current_period_end: Option<i64>,
}

impl WorkspaceSubscription {
    #[must_use]
    pub fn access(&self) -> WorkspaceAccess {
        access(self.status)
    }
}

/// A subscription as Stripe reports it, mapped to Orbit's names.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubscriptionSnapshot {
    pub customer_id: String,
    pub subscription_id: String,
    pub plan: Option<String>,
    /// Stripe's status string, stored as is (an unknown one reads as read-only).
    pub status: String,
    pub cancel_at_period_end: bool,
    pub current_period_end: Option<i64>,
}

/// The access of one workspace; one primary-key lookup, run by the read-only guard on each write.
pub async fn workspace_access(
    database: &Database,
    workspace_id: Id,
) -> Result<WorkspaceAccess, sqlx::Error> {
    let status: Option<Option<String>> =
        sqlx::query_scalar("SELECT status FROM workspace_subscriptions WHERE workspace_id = $1")
            .bind(workspace_id.to_string())
            .fetch_optional(database.pool())
            .await?;
    Ok(access(
        status
            .flatten()
            .as_deref()
            .and_then(SubscriptionStatus::parse),
    ))
}

pub async fn subscription(
    database: &Database,
    workspace_id: Id,
) -> Result<WorkspaceSubscription, sqlx::Error> {
    let row = sqlx::query(
        "SELECT stripe_customer_id, plan, status, cancel_at_period_end, current_period_end \
         FROM workspace_subscriptions WHERE workspace_id = $1",
    )
    .bind(workspace_id.to_string())
    .fetch_optional(database.pool())
    .await?;
    Ok(row.map_or_else(WorkspaceSubscription::default, |row| {
        WorkspaceSubscription {
            stripe_customer_id: Some(row.get("stripe_customer_id")),
            plan: row.get("plan"),
            status: row
                .get::<Option<String>, _>("status")
                .as_deref()
                .and_then(SubscriptionStatus::parse),
            cancel_at_period_end: row.get("cancel_at_period_end"),
            current_period_end: row.get("current_period_end"),
        }
    }))
}

/// Saves the workspace's Stripe customer unless it already has one; returns the one it has.
pub async fn save_customer(
    database: &Database,
    workspace_id: Id,
    customer_id: &str,
    now: TimestampMillis,
) -> Result<String, sqlx::Error> {
    sqlx::query(
        "INSERT INTO workspace_subscriptions (workspace_id, stripe_customer_id, updated_at) \
         VALUES ($1, $2, $3) ON CONFLICT (workspace_id) DO NOTHING",
    )
    .bind(workspace_id.to_string())
    .bind(customer_id)
    .bind(now.as_millis())
    .execute(database.pool())
    .await?;
    sqlx::query_scalar(
        "SELECT stripe_customer_id FROM workspace_subscriptions WHERE workspace_id = $1",
    )
    .bind(workspace_id.to_string())
    .fetch_one(database.pool())
    .await
}

/// Records a webhook event; `false` when it was already recorded (a redelivery).
pub async fn record_event(
    database: &Database,
    event_id: &str,
    now: TimestampMillis,
) -> Result<bool, sqlx::Error> {
    const KEEP_MILLIS: i64 = 30 * 24 * 60 * 60 * 1000;
    let inserted = sqlx::query(
        "INSERT INTO stripe_events (event_id, received_at) VALUES ($1, $2) \
         ON CONFLICT (event_id) DO NOTHING",
    )
    .bind(event_id)
    .bind(now.as_millis())
    .execute(database.pool())
    .await?
    .rows_affected()
        == 1;
    sqlx::query("DELETE FROM stripe_events WHERE received_at < $1")
        .bind(now.as_millis() - KEEP_MILLIS)
        .execute(database.pool())
        .await?;
    Ok(inserted)
}

/// Forgets an event, so Stripe's next delivery of it is processed again.
pub async fn forget_event(database: &Database, event_id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM stripe_events WHERE event_id = $1")
        .bind(event_id)
        .execute(database.pool())
        .await
        .map(|_| ())
}

/// Stores the subscription of a workspace and audits a status change. A workspace that has a
/// different subscription keeps it unless this one grants access: a late event about an old,
/// cancelled subscription must not lock a workspace that subscribed again. Returns `false` when
/// the workspace no longer exists.
pub async fn apply_subscription(
    database: &Database,
    workspace_id: Id,
    snapshot: &SubscriptionSnapshot,
    now: TimestampMillis,
) -> Result<bool, sqlx::Error> {
    let grants_access =
        access(SubscriptionStatus::parse(&snapshot.status)) == WorkspaceAccess::Full;
    let mut transaction = database.immediate_transaction().await?;
    let exists: Option<String> = sqlx::query_scalar("SELECT id FROM workspaces WHERE id = $1")
        .bind(workspace_id.to_string())
        .fetch_optional(&mut *transaction)
        .await?;
    if exists.is_none() {
        transaction.rollback().await?;
        return Ok(false);
    }
    let previous: Option<Option<String>> =
        sqlx::query_scalar("SELECT status FROM workspace_subscriptions WHERE workspace_id = $1")
            .bind(workspace_id.to_string())
            .fetch_optional(&mut *transaction)
            .await?;
    let previous = previous.flatten();
    let written = sqlx::query(
        "INSERT INTO workspace_subscriptions (workspace_id, stripe_customer_id, \
         stripe_subscription_id, plan, status, cancel_at_period_end, current_period_end, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
         ON CONFLICT (workspace_id) DO UPDATE SET stripe_customer_id = excluded.stripe_customer_id, \
         stripe_subscription_id = excluded.stripe_subscription_id, plan = excluded.plan, \
         status = excluded.status, cancel_at_period_end = excluded.cancel_at_period_end, \
         current_period_end = excluded.current_period_end, updated_at = excluded.updated_at \
         WHERE workspace_subscriptions.stripe_subscription_id IS NULL \
         OR workspace_subscriptions.stripe_subscription_id = excluded.stripe_subscription_id \
         OR $9 = 1",
    )
    .bind(workspace_id.to_string())
    .bind(&snapshot.customer_id)
    .bind(&snapshot.subscription_id)
    .bind(snapshot.plan.as_deref())
    .bind(&snapshot.status)
    .bind(i64::from(snapshot.cancel_at_period_end))
    .bind(snapshot.current_period_end)
    .bind(now.as_millis())
    .bind(i64::from(grants_access))
    .execute(&mut *transaction)
    .await?
    .rows_affected()
        == 1;
    if written && previous.as_deref() != Some(snapshot.status.as_str()) {
        audit::record(
            &mut transaction,
            workspace_id,
            None,
            "billing.subscription_status_changed",
            AuditOutcome::Success,
            "workspace",
            Some(workspace_id),
            "stripe-webhook",
            json!({
                "from": previous,
                "to": snapshot.status,
                "plan": snapshot.plan,
            }),
            now,
        )
        .await?;
    }
    transaction.commit().await?;
    Ok(true)
}
