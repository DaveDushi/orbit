//! The Orbit API as the desktop app sees it: the `/api/v1` types it reads, the HTTP client, the
//! workspace sockets, the rules of the exe.dev gateway in front of a hosted server, and a fixture
//! backend for headless runs, and the update check. No UI: everything here is plain `cargo test`.
//! The types follow `packages/api/src/generated/openapi.json`; `tests/contract.rs` checks them.

pub mod api;
pub mod backoff;
pub mod cookies;
pub mod fixture;
pub mod gateway;
pub mod live;
pub mod secret;
pub mod server;
pub mod socket;
pub mod types;
pub mod update;

pub use api::{ApiError, Client, LoginOutcome, Problem, Session, classify_response, needs_browser_mode};
pub use cookies::{GatewayCookie, GatewayCookies, HandOff, HandOffError, hand_off_line, parse_hand_off};
pub use fixture::{Fixture, FixtureError, SidebarRow, sidebar};
pub use gateway::{GatewayReply, classify};
pub use live::{ChatEvent, Ended, LiveFrame, WorkspaceEvent, WorkspaceEventKind, parse_chat_event};
pub use secret::Secret;
pub use server::{ServerUrl, ServerUrlError};
pub use socket::{Socket, SocketConfig, SocketEvent, SocketKind};
pub use types::{
    AuthUser, ConversationKind, ConversationRecord, ConversationStateRecord, MemberRecord, MessageKind, MessagePage, MessageRecord,
    NotifyLevel, ReactionRecord, SetupStatus, ThreadPage, WorkspaceRecord,
};

/// Sent on every request; the server answers `409 contract_mismatch` when it differs
/// (`apps/server/src/openapi.rs`, `CONTRACT_ID`).
pub const CONTRACT_ID: &str = "orbit-api-v1";
