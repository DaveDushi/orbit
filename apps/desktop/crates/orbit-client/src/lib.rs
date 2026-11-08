//! The Orbit API as the desktop app sees it: the few `/api/v1` types it reads, the rules of the
//! exe.dev gateway in front of a hosted server, and a fixture backend for headless runs.
//! The types follow `packages/api/src/generated/openapi.json`; `tests/contract.rs` checks them.

pub mod fixture;
pub mod gateway;
pub mod types;

pub use fixture::{Fixture, FixtureError, SidebarRow};
pub use gateway::{GATEWAY_TOKEN_HEADER, GatewayReply, classify};
pub use types::{ConversationKind, ConversationRecord, ConversationStateRecord, NotifyLevel};

/// Sent on every request; the server answers `409 contract_mismatch` when it differs
/// (`apps/server/src/openapi.rs`, `CONTRACT_ID`).
pub const CONTRACT_ID: &str = "orbit-api-v1";
