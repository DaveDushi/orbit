//! Orbit Cloud billing (`ORBIT_CLOUD=true`): one flat-price Stripe subscription per workspace.
//! A workspace whose subscription does not grant access is read-only (see [`access`]): reads work,
//! writes answer 402 `workspace_read_only` ([`guard`]). On a self-hosted server none of this is
//! mounted.

pub mod guard;
pub(crate) mod routes;
pub mod stripe;
mod sync;

use std::sync::Arc;

use orbit_platform::{BillingConfig, Database};
use serde::Serialize;
use utoipa::ToSchema;

pub use routes::router;
pub use sync::{SYNC_JOB, register_jobs};

/// A Stripe subscription status (`subscription.status`).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SubscriptionStatus {
    Incomplete,
    IncompleteExpired,
    Trialing,
    Active,
    PastDue,
    Canceled,
    Unpaid,
    Paused,
}

impl SubscriptionStatus {
    pub const ALL: [Self; 8] = [
        Self::Incomplete,
        Self::IncompleteExpired,
        Self::Trialing,
        Self::Active,
        Self::PastDue,
        Self::Canceled,
        Self::Unpaid,
        Self::Paused,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Incomplete => "incomplete",
            Self::IncompleteExpired => "incomplete_expired",
            Self::Trialing => "trialing",
            Self::Active => "active",
            Self::PastDue => "past_due",
            Self::Canceled => "canceled",
            Self::Unpaid => "unpaid",
            Self::Paused => "paused",
        }
    }

    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|status| status.as_str() == value)
    }
}

/// What members of a workspace may do.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceAccess {
    Full,
    ReadOnly,
}

/// The only rule that maps a subscription to access. `past_due` keeps full access while Stripe
/// retries the card; no subscription, or any other status, is read-only.
#[must_use]
pub const fn access(status: Option<SubscriptionStatus>) -> WorkspaceAccess {
    match status {
        Some(
            SubscriptionStatus::Active | SubscriptionStatus::Trialing | SubscriptionStatus::PastDue,
        ) => WorkspaceAccess::Full,
        Some(
            SubscriptionStatus::Incomplete
            | SubscriptionStatus::IncompleteExpired
            | SubscriptionStatus::Canceled
            | SubscriptionStatus::Unpaid
            | SubscriptionStatus::Paused,
        )
        | None => WorkspaceAccess::ReadOnly,
    }
}

/// Billing for the routes, the guard and the sync job.
#[derive(Clone)]
pub struct Billing {
    inner: Arc<Inner>,
}

struct Inner {
    config: BillingConfig,
    database: Database,
    stripe: stripe::StripeClient,
    public_origin: String,
}

impl Billing {
    #[must_use]
    pub fn new(config: BillingConfig, database: Database, public_origin: &str) -> Self {
        let stripe = stripe::StripeClient::new(&config);
        Self {
            inner: Arc::new(Inner {
                config,
                database,
                stripe,
                public_origin: public_origin.trim_end_matches('/').to_owned(),
            }),
        }
    }

    pub(crate) fn config(&self) -> &BillingConfig {
        &self.inner.config
    }

    pub(crate) fn database(&self) -> &Database {
        &self.inner.database
    }

    pub(crate) fn stripe(&self) -> &stripe::StripeClient {
        &self.inner.stripe
    }

    pub(crate) fn public_origin(&self) -> &str {
        &self.inner.public_origin
    }
}

#[cfg(test)]
mod tests {
    use super::{SubscriptionStatus, WorkspaceAccess, access};

    #[test]
    fn only_paying_subscriptions_grant_full_access() {
        let table: Vec<(Option<&str>, WorkspaceAccess)> = [None]
            .into_iter()
            .chain(SubscriptionStatus::ALL.map(Some))
            .map(|status| (status.map(SubscriptionStatus::as_str), access(status)))
            .collect();

        assert_eq!(
            table,
            [
                (None, WorkspaceAccess::ReadOnly),
                (Some("incomplete"), WorkspaceAccess::ReadOnly),
                (Some("incomplete_expired"), WorkspaceAccess::ReadOnly),
                (Some("trialing"), WorkspaceAccess::Full),
                (Some("active"), WorkspaceAccess::Full),
                (Some("past_due"), WorkspaceAccess::Full),
                (Some("canceled"), WorkspaceAccess::ReadOnly),
                (Some("unpaid"), WorkspaceAccess::ReadOnly),
                (Some("paused"), WorkspaceAccess::ReadOnly),
            ]
        );
    }

    #[test]
    fn statuses_round_trip_and_unknown_ones_are_rejected() {
        for status in SubscriptionStatus::ALL {
            assert_eq!(SubscriptionStatus::parse(status.as_str()), Some(status));
        }
        assert_eq!(SubscriptionStatus::parse("ended"), None);
    }
}
