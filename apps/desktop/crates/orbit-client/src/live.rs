//! The two sockets of a workspace, as data (same rules as `apps/mobile/src/data/live.ts`):
//!
//! - `GET /api/v1/workspaces/{id}/live?epoch=&after=` (`apps/server/src/live/socket.rs`): numbered
//!   chat events. It opens with `hello {epoch, seq}`, then replays what the client missed, or sends
//!   `resync` when it cannot (other epoch, or the event is out of the 1,000-event buffer). A text
//!   `ping` comes every heartbeat (25 s). It closes with 4401 (no session), 4403 (no access) and 1012
//!   (server restart).
//! - `GET /api/v1/workspaces/{id}/events?after=` (`apps/server/src/realtime.rs`): invalidations only,
//!   `workspace.changed` or `resync_required`, with a string `sequence`. It refuses a missing session
//!   (401) or access (403) with an HTTP status at the upgrade.
//!
//! Through the exe.dev gateway an upgrade can also get the gateway's 307 (sign in) or 401.

use serde::Deserialize;
use serde_json::Value;

use crate::api::{ApiError, classify_response};
use crate::types::{ConversationRecord, ConversationStateRecord, MessageRecord};

#[derive(Clone, Debug, PartialEq)]
pub enum LiveFrame {
    Hello { epoch: String, seq: u64 },
    Resync,
    Ping,
    /// A numbered event (`seq`) or an ephemeral one (typing, presence: no `seq`).
    Event { seq: Option<u64>, topic: String, event: Value },
}

/// `None` for frames the app has no use for and for garbage.
pub fn parse_live_frame(text: &str) -> Option<LiveFrame> {
    let frame: Value = serde_json::from_str(text).ok()?;
    match frame.get("type").and_then(Value::as_str) {
        Some("hello") => Some(LiveFrame::Hello {
            epoch: frame.get("epoch").and_then(Value::as_str).unwrap_or_default().to_owned(),
            seq: frame.get("seq").and_then(Value::as_u64).unwrap_or(0),
        }),
        Some("resync") => Some(LiveFrame::Resync),
        Some("ping") => Some(LiveFrame::Ping),
        _ => {
            let topic = frame.get("topic")?.as_str()?.to_owned();
            Some(LiveFrame::Event { seq: frame.get("seq").and_then(Value::as_u64), topic, event: frame.get("event").cloned().unwrap_or(Value::Null) })
        }
    }
}

/// A chat change from a live `Event` frame (`ChatEvent` in `openapi.json`): the kinds the app
/// applies to what it shows. Others (emoji, stickers, categories, threads) read as `None`.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(tag = "type")]
pub enum ChatEvent {
    #[serde(rename = "message.created")]
    MessageCreated { message: MessageRecord },
    /// Edits, reactions, pins and thread summary changes.
    #[serde(rename = "message.updated")]
    MessageUpdated { message: MessageRecord },
    #[serde(rename = "message.deleted")]
    MessageDeleted { conversation_id: String, message_id: String, #[serde(default)] thread_root_id: Option<String> },
    #[serde(rename = "conversation.changed")]
    ConversationChanged { conversation: ConversationRecord },
    /// The receiver lost access to it.
    #[serde(rename = "conversation.removed")]
    ConversationRemoved { conversation_id: String },
    #[serde(rename = "state.changed")]
    StateChanged { state: ConversationStateRecord },
}

/// The chat change in an event's `event` value, if it is one the app applies.
pub fn parse_chat_event(event: &Value) -> Option<ChatEvent> {
    ChatEvent::deserialize(event).ok()
}

/// Where the live socket resumes: the epoch and the number of the last event it got, so the server
/// replays what was missed. A new epoch starts the numbers again.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LiveCursor {
    epoch: Option<String>,
    hello_seq: u64,
    last_seq: Option<u64>,
}

impl LiveCursor {
    /// The query of the next connection; empty until a full state is known.
    pub fn query(&self) -> String {
        match (&self.epoch, self.last_seq) {
            (Some(epoch), Some(after)) => format!("?epoch={}&after={after}", encode(epoch)),
            _ => String::new(),
        }
    }

    pub fn read(&mut self, frame: &LiveFrame) {
        match frame {
            LiveFrame::Hello { epoch, seq } => {
                if self.epoch.as_deref() != Some(epoch.as_str()) {
                    self.last_seq = None;
                }
                self.epoch = Some(epoch.clone());
                self.hello_seq = *seq;
            }
            // The app refetches everything; it is now current as of the hello.
            LiveFrame::Resync => self.last_seq = Some(self.hello_seq),
            LiveFrame::Event { seq: Some(seq), .. } => self.last_seq = Some(*seq),
            _ => {}
        }
    }
}

/// A frame of the events socket.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct WorkspaceEvent {
    pub version: u8,
    pub kind: WorkspaceEventKind,
    pub sequence: String,
    #[serde(default)]
    pub workspaces_changed: bool,
    #[serde(default)]
    pub profile_changed: bool,
    #[serde(default)]
    pub views_only: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
pub enum WorkspaceEventKind {
    #[serde(rename = "workspace.changed")]
    Changed,
    /// The cursor was too old or unknown: refetch everything of the workspace.
    #[serde(rename = "resync_required")]
    ResyncRequired,
}

pub fn parse_workspace_event(text: &str) -> Option<WorkspaceEvent> {
    let event: WorkspaceEvent = serde_json::from_str(text).ok()?;
    let digits = !event.sequence.is_empty() && event.sequence.bytes().all(|b| b.is_ascii_digit())
        && (event.sequence == "0" || !event.sequence.starts_with('0'));
    (event.version == 1 && digits).then_some(event)
}

/// Where the events socket resumes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EventsCursor {
    after: Option<String>,
}

impl EventsCursor {
    pub fn query(&self) -> String {
        self.after.as_ref().map_or_else(String::new, |after| format!("?after={after}"))
    }

    pub fn read(&mut self, event: &WorkspaceEvent) {
        self.after = Some(event.sequence.clone());
    }
}

/// Why a socket stopped for good: the app must act before it can connect again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ended {
    /// The gateway wants its sign-in, or refused the cookies: open the sign-in window.
    Gateway(ApiError),
    /// The Orbit session ended: sign in to Orbit again.
    SignedOut,
    /// No access to this workspace (removed, or the workspace was deleted).
    Forbidden,
    /// Orbit refuses this app version.
    ContractMismatch,
}

/// What to do after a connection ends.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AfterClose {
    Reconnect,
    Stop(Ended),
}

/// After a close frame (`None`: the connection dropped without one).
pub fn after_close(code: Option<u16>) -> AfterClose {
    match code {
        Some(4401) => AfterClose::Stop(Ended::SignedOut),
        Some(4403) => AfterClose::Stop(Ended::Forbidden),
        // 1012 restart, 1013 overloaded, 4429 too many frames, a lost network: all retry with backoff.
        _ => AfterClose::Reconnect,
    }
}

/// After an upgrade refused with an HTTP response.
pub fn after_refused_upgrade(
    status: u16,
    location: Option<&str>,
    www_authenticate: Option<&str>,
    content_type: Option<&str>,
    body: &str,
) -> AfterClose {
    match classify_response(status, location, www_authenticate, content_type, body) {
        Err(e @ (ApiError::GatewaySignIn | ApiError::GatewayRejected)) => AfterClose::Stop(Ended::Gateway(e)),
        Err(ApiError::ContractMismatch) => AfterClose::Stop(Ended::ContractMismatch),
        Err(ApiError::SignedOut(_)) => AfterClose::Stop(Ended::SignedOut),
        // The events socket answers a bare 401 or 403.
        Err(ApiError::Status(401)) => AfterClose::Stop(Ended::SignedOut),
        Err(ApiError::Status(403) | ApiError::Problem(403, _)) => AfterClose::Stop(Ended::Forbidden),
        _ => AfterClose::Reconnect,
    }
}

/// Percent-encodes a query value (epochs are opaque strings).
fn encode(value: &str) -> String {
    value.bytes().map(|b| match b {
        b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
        _ => format!("%{b:02X}"),
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(text: &str) -> LiveFrame {
        parse_live_frame(text).unwrap()
    }

    #[test]
    fn the_cursor_resumes_after_the_last_numbered_event_of_its_epoch() {
        let mut cursor = LiveCursor::default();
        assert_eq!(cursor.query(), "", "first connection: no resume");
        cursor.read(&frame(r#"{"type":"hello","epoch":"e 1","seq":5,"presence":[]}"#));
        assert_eq!(cursor.query(), "", "a hello alone is not a known state");
        cursor.read(&frame(r#"{"type":"resync"}"#));
        assert_eq!(cursor.query(), "?epoch=e%201&after=5", "after a resync the app is current as of the hello");
        cursor.read(&frame(r#"{"seq":6,"topic":"chat","event":{"type":"message.created"}}"#));
        cursor.read(&frame(r#"{"topic":"chat","event":{"type":"typing"}}"#));
        cursor.read(&frame(r#"{"type":"ping"}"#));
        assert_eq!(cursor.query(), "?epoch=e%201&after=6", "ephemeral frames and pings do not move it");
        // Same epoch: replayed events continue the count.
        cursor.read(&frame(r#"{"type":"hello","epoch":"e 1","seq":9}"#));
        cursor.read(&frame(r#"{"seq":7,"topic":"chat","event":{}}"#));
        assert_eq!(cursor.query(), "?epoch=e%201&after=7");
        // A new epoch (the server's events were reset): numbers start again, so no resume until a resync.
        cursor.read(&frame(r#"{"type":"hello","epoch":"e2","seq":2}"#));
        assert_eq!(cursor.query(), "");
        cursor.read(&frame(r#"{"type":"resync"}"#));
        assert_eq!(cursor.query(), "?epoch=e2&after=2");
    }

    #[test]
    fn chat_events_parse_by_type_and_unknown_ones_are_none() {
        let message = r#"{"id":"m1","conversation_id":"c1","thread_root_id":null,"kind":"message","author_id":"u1",
            "body":"hi","mentions":{},"created_at":"2026-11-09T10:00:00Z","edited_at":null,"deleted":false,"attachments":[],
            "reactions":[{"emoji":"👍","user_ids":["u2"]}],"pinned":false,"also_in_channel":false,"nonce":"desk-1","reply_count":0,
            "last_reply_at":null,"reply_user_ids":[],"last_reply":null,"reply_to_id":null,"reply_to":null,"forwarded":null,
            "sticker_id":null,"sticker":null,"bot":null,"blocks":null,"future":1}"#;
        let created: Value = serde_json::from_str(&format!(r#"{{"type":"message.created","message":{message}}}"#)).unwrap();
        let Some(ChatEvent::MessageCreated { message }) = parse_chat_event(&created) else { panic!("{created}") };
        assert_eq!((message.id.as_str(), message.nonce.as_deref(), message.reactions.len()), ("m1", Some("desk-1"), 1));
        let deleted = serde_json::json!({"type":"message.deleted","conversation_id":"c1","message_id":"m1","thread_root_id":null});
        assert_eq!(parse_chat_event(&deleted), Some(ChatEvent::MessageDeleted { conversation_id: "c1".into(), message_id: "m1".into(), thread_root_id: None }));
        let state = serde_json::json!({"type":"state.changed","state":{"conversation_id":"c1","unread_count":3,"mention_count":1,"notify":"all","favorite":false}});
        assert!(matches!(parse_chat_event(&state), Some(ChatEvent::StateChanged { state }) if state.unread_count == 3));
        assert_eq!(parse_chat_event(&serde_json::json!({"type":"emoji.changed"})), None);
        assert_eq!(parse_chat_event(&serde_json::json!({"type":"message.created"})), None, "broken: no message");
    }

    #[test]
    fn unknown_and_broken_frames_are_skipped() {
        assert_eq!(parse_live_frame("nope"), None);
        assert_eq!(parse_live_frame(r#"{"type":"presence_future"}"#), None);
        assert_eq!(parse_live_frame(r#"{"type":"hello"}"#), Some(LiveFrame::Hello { epoch: String::new(), seq: 0 }));
    }

    #[test]
    fn events_resume_after_their_sequence_and_report_resyncs() {
        let mut cursor = EventsCursor::default();
        assert_eq!(cursor.query(), "");
        let event = parse_workspace_event(r#"{"version":1,"kind":"resync_required","sequence":"41","workspaces_changed":true,"profile_changed":true,"views_only":false}"#).unwrap();
        assert_eq!(event.kind, WorkspaceEventKind::ResyncRequired);
        cursor.read(&event);
        assert_eq!(cursor.query(), "?after=41");
        assert!(parse_workspace_event(r#"{"version":1,"kind":"workspace.changed","sequence":"042"}"#).is_none());
        assert!(parse_workspace_event(r#"{"version":2,"kind":"workspace.changed","sequence":"1"}"#).is_none());
        assert!(parse_workspace_event(r#"{"version":1,"kind":"other","sequence":"1"}"#).is_none());
    }

    #[test]
    fn close_codes_and_refused_upgrades_decide_retry_or_stop() {
        assert_eq!(after_close(Some(4401)), AfterClose::Stop(Ended::SignedOut));
        assert_eq!(after_close(Some(4403)), AfterClose::Stop(Ended::Forbidden));
        for code in [Some(1012), Some(1013), Some(1000), Some(4429), None] {
            assert_eq!(after_close(code), AfterClose::Reconnect, "{code:?}");
        }
        let login = Some("https://orbit.example.com/__exe.dev/login?redirect=%2F");
        assert_eq!(after_refused_upgrade(307, login, None, None, ""), AfterClose::Stop(Ended::Gateway(ApiError::GatewaySignIn)));
        assert_eq!(after_refused_upgrade(401, None, Some("Bearer, Basic"), Some("text/plain"), "x"), AfterClose::Stop(Ended::Gateway(ApiError::GatewayRejected)));
        assert_eq!(after_refused_upgrade(401, None, None, None, ""), AfterClose::Stop(Ended::SignedOut));
        assert_eq!(after_refused_upgrade(403, None, None, None, ""), AfterClose::Stop(Ended::Forbidden));
        let mismatch = r#"{"code":"contract_mismatch"}"#;
        assert_eq!(after_refused_upgrade(409, None, None, Some("application/problem+json"), mismatch), AfterClose::Stop(Ended::ContractMismatch));
        assert_eq!(after_refused_upgrade(502, None, None, Some("text/html"), ""), AfterClose::Reconnect);
        assert_eq!(after_refused_upgrade(503, None, None, None, ""), AfterClose::Reconnect);
    }
}
