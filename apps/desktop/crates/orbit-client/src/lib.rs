//! The Orbit API as the desktop app sees it: the `/api/v1` types it reads, the HTTP client, the
//! workspace sockets, the rules of the exe.dev gateway in front of a hosted server, and a fixture
//! backend for headless runs. No Rune: everything here is plain `cargo test`.
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

pub use api::{ApiError, Client, LoginOutcome, Problem, Session, classify_response};
pub use cookies::{GatewayCookie, GatewayCookies, HandOff, HandOffError, hand_off_line, parse_hand_off};
pub use fixture::{Fixture, FixtureError, SidebarRow, sidebar};
pub use gateway::{GatewayReply, classify};
pub use live::{Ended, LiveFrame, WorkspaceEvent, WorkspaceEventKind};
pub use secret::Secret;
pub use server::{ServerUrl, ServerUrlError};
pub use socket::{Socket, SocketConfig, SocketEvent, SocketKind};
pub use types::{AuthUser, ConversationKind, ConversationRecord, ConversationStateRecord, NotifyLevel, SetupStatus, WorkspaceRecord};

/// Sent on every request; the server answers `409 contract_mismatch` when it differs
/// (`apps/server/src/openapi.rs`, `CONTRACT_ID`).
pub const CONTRACT_ID: &str = "orbit-api-v1";
