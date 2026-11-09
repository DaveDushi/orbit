//! What the app shows, without any UI: the screens, the sidebar, the open conversation and thread,
//! messages on their way out, and the socket status. Network replies and live events change it
//! here; `app.rs` runs the network work and `view.rs` draws it. Everything here is plain
//! `cargo test`.

use std::collections::HashMap;

use orbit_client::live::Ended;
use orbit_client::{
    ApiError, ChatEvent, ConversationRecord, ConversationStateRecord, LiveFrame, MemberRecord, MessageKind, MessageRecord,
    SidebarRow, SocketEvent, SocketKind, WorkspaceEventKind, parse_chat_event, sidebar,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    /// Enter the server's address.
    Server,
    /// The gateway's sign-in window is open.
    Gateway,
    Login,
    TwoFactor,
    SignedIn,
}

/// The state of one socket, for the status line.
#[derive(Clone, Debug, PartialEq)]
pub enum Link {
    Off,
    Connecting,
    Connected,
    /// Next try in this many seconds.
    Retrying(u64),
    Ended(String),
}

/// Where a message of the list is on its way to the server.
#[derive(Clone, Debug, PartialEq)]
pub enum Delivery {
    Sent,
    /// Shown at once, with the nonce it is sent with.
    Sending,
    /// The send failed with this error; "Retry" sends it again with the same nonce.
    Failed(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub message: MessageRecord,
    pub delivery: Delivery,
}

impl Row {
    fn sent(message: MessageRecord) -> Self {
        Row { message, delivery: Delivery::Sent }
    }
}

/// The open thread: its root and replies.
#[derive(Clone, Debug, PartialEq)]
pub struct Thread {
    pub root_id: String,
    pub root: Option<MessageRecord>,
    pub replies: Vec<Row>,
    pub loading: bool,
}

/// A message to send now: the app posts it with this nonce.
#[derive(Clone, Debug, PartialEq)]
pub struct Outgoing {
    pub conversation_id: String,
    pub thread_root_id: Option<String>,
    pub nonce: String,
    pub body: String,
}

/// The longest body the server takes (`ChatSendBody.body`).
pub const MAX_BODY: usize = 4000;

/// The signed-in chat of one workspace.
#[derive(Clone, Debug, Default)]
pub struct Chat {
    /// The signed-in person's user id: their messages can be edited.
    pub me: String,
    pub workspace_id: String,
    pub workspace: String,
    pub conversations: Vec<ConversationRecord>,
    pub states: Vec<ConversationStateRecord>,
    /// Derived from `conversations` and `states`.
    pub rows: Vec<SidebarRow>,
    /// User id → display name.
    pub names: HashMap<String, String>,
    pub selected: Option<String>,
    /// The open conversation's main list, oldest first.
    pub messages: Vec<Row>,
    pub loading: bool,
    pub thread: Option<Thread>,
    /// The message the main composer edits, instead of sending a new one.
    pub editing: Option<String>,
    /// An error of the open conversation (a failed load or edit).
    pub error: Option<String>,
}

impl Chat {
    pub fn new(me: &str, workspace_id: &str, workspace: &str) -> Self {
        Chat { me: me.into(), workspace_id: workspace_id.into(), workspace: workspace.into(), ..Chat::default() }
    }

    pub fn set_sidebar(&mut self, conversations: Vec<ConversationRecord>, states: Vec<ConversationStateRecord>) {
        self.conversations = conversations;
        self.states = states;
        self.rebuild_rows();
        if self.selected.as_ref().is_some_and(|id| !self.rows.iter().any(|r| &r.id == id)) {
            self.close_conversation();
        }
    }

    pub fn set_members(&mut self, members: &[MemberRecord]) {
        self.names = members.iter().map(|m| (m.user_id.clone(), m.display_name.clone())).collect();
    }

    pub fn name_of(&self, user_id: &str) -> String {
        self.names.get(user_id).cloned().unwrap_or_else(|| "Unknown member".into())
    }

    pub fn open_row(&self) -> Option<&SidebarRow> {
        let id = self.selected.as_ref()?;
        self.rows.iter().find(|r| &r.id == id)
    }

    /// Opens a conversation. True when its messages must be loaded (it was not open already).
    pub fn select(&mut self, conversation_id: &str) -> bool {
        if self.selected.as_deref() == Some(conversation_id) {
            return false;
        }
        self.close_conversation();
        self.selected = Some(conversation_id.into());
        self.loading = true;
        true
    }

    fn close_conversation(&mut self) {
        self.selected = None;
        self.messages.clear();
        self.thread = None;
        self.editing = None;
        self.loading = false;
        self.error = None;
    }

    /// The first page of a conversation arrived. Messages still on their way out stay at the end.
    pub fn set_messages(&mut self, conversation_id: &str, items: Vec<MessageRecord>) {
        if self.selected.as_deref() != Some(conversation_id) {
            return;
        }
        let outgoing: Vec<Row> = self.messages.drain(..).filter(|r| r.delivery != Delivery::Sent).collect();
        self.messages = items.into_iter().filter(in_main_list).map(Row::sent).collect();
        for row in outgoing {
            let nonce = row.message.nonce.clone();
            if !self.messages.iter().any(|r| r.message.nonce.is_some() && r.message.nonce == nonce) {
                self.messages.push(row);
            }
        }
        self.loading = false;
        self.error = None;
    }

    pub fn load_failed(&mut self, conversation_id: &str, error: String) {
        if self.selected.as_deref() == Some(conversation_id) {
            self.loading = false;
            self.error = Some(error);
        }
    }

    /// Opens a thread of the open conversation. True when it must be loaded.
    pub fn open_thread(&mut self, root_id: &str) -> bool {
        if self.thread.as_ref().is_some_and(|t| t.root_id == root_id) {
            return false;
        }
        let root = self.messages.iter().find(|r| r.message.id == root_id).map(|r| r.message.clone());
        self.thread = Some(Thread { root_id: root_id.into(), root, replies: Vec::new(), loading: true });
        true
    }

    pub fn close_thread(&mut self) {
        self.thread = None;
    }

    pub fn set_thread(&mut self, root: MessageRecord, replies: Vec<MessageRecord>) {
        let Some(thread) = self.thread.as_mut().filter(|t| t.root_id == root.id) else { return };
        let outgoing: Vec<Row> = thread.replies.drain(..).filter(|r| r.delivery != Delivery::Sent).collect();
        thread.replies = replies.into_iter().map(Row::sent).collect();
        for row in outgoing {
            if !thread.replies.iter().any(|r| r.message.nonce.is_some() && r.message.nonce == row.message.nonce) {
                thread.replies.push(row);
            }
        }
        thread.root = Some(root);
        thread.loading = false;
    }

    /// Checks a draft and shows it at once as "sending". `None` with an error when it cannot go.
    /// `thread`: a reply in the open thread instead of a message in the conversation.
    pub fn queue_send(&mut self, text: &str, thread: bool, nonce: String, now: &str) -> Result<Outgoing, String> {
        let body = text.trim();
        if body.is_empty() {
            return Err("Write a message first.".into());
        }
        if body.chars().count() > MAX_BODY {
            return Err(format!("A message can have at most {MAX_BODY} characters."));
        }
        let conversation_id = self.selected.clone().ok_or("Pick a conversation first.")?;
        let thread_root_id = match thread {
            true => Some(self.thread.as_ref().ok_or("No thread is open.")?.root_id.clone()),
            false => None,
        };
        let message = MessageRecord {
            id: format!("local-{nonce}"),
            conversation_id: conversation_id.clone(),
            thread_root_id: thread_root_id.clone(),
            kind: MessageKind::Message,
            author_id: self.me.clone(),
            body: body.into(),
            created_at: now.into(),
            edited_at: None,
            deleted: false,
            reactions: Vec::new(),
            also_in_channel: false,
            nonce: Some(nonce.clone()),
            reply_count: 0,
            last_reply_at: None,
        };
        let row = Row { message, delivery: Delivery::Sending };
        match (&thread_root_id, self.thread.as_mut()) {
            (Some(_), Some(t)) => t.replies.push(row),
            _ => self.messages.push(row),
        }
        Ok(Outgoing { conversation_id, thread_root_id, nonce, body: body.into() })
    }

    /// The server's answer to a send.
    pub fn sent(&mut self, nonce: &str, result: Result<MessageRecord, String>) {
        match result {
            Ok(message) => self.upsert(message),
            Err(error) => {
                for row in self.rows_mut() {
                    if row.message.nonce.as_deref() == Some(nonce) && row.delivery != Delivery::Sent {
                        row.delivery = Delivery::Failed(error.clone());
                    }
                }
            }
        }
    }

    /// A failed message to send again, with its first nonce (the server sends it at most once).
    pub fn retry(&mut self, nonce: &str) -> Option<Outgoing> {
        let mut found = None;
        for row in self.rows_mut() {
            if row.message.nonce.as_deref() == Some(nonce) && matches!(row.delivery, Delivery::Failed(_)) {
                row.delivery = Delivery::Sending;
                found = Some(Outgoing {
                    conversation_id: row.message.conversation_id.clone(),
                    thread_root_id: row.message.thread_root_id.clone(),
                    nonce: nonce.into(),
                    body: row.message.body.clone(),
                });
            }
        }
        found
    }

    /// Starts editing one of the person's own sent messages; returns its body for the composer.
    pub fn start_edit(&mut self, message_id: &str) -> Option<String> {
        let row = self.messages.iter().find(|r| r.message.id == message_id)?;
        let own = row.message.author_id == self.me && row.message.kind == MessageKind::Message;
        (own && row.delivery == Delivery::Sent && !row.message.deleted).then(|| {
            self.editing = Some(message_id.into());
            row.message.body.clone()
        })
    }

    /// The person's newest own message (Up in an empty composer edits it).
    pub fn last_own_message(&self) -> Option<String> {
        self.messages.iter().rev()
            .find(|r| r.message.author_id == self.me && r.delivery == Delivery::Sent && r.message.kind == MessageKind::Message && !r.message.deleted)
            .map(|r| r.message.id.clone())
    }

    pub fn cancel_edit(&mut self) {
        self.editing = None;
    }

    /// Checks an edit. `Ok((message id, body))` to send; the composer keeps the text on error.
    pub fn finish_edit(&mut self, text: &str) -> Result<(String, String), String> {
        let id = self.editing.clone().ok_or("Nothing is being edited.")?;
        let body = text.trim();
        if body.is_empty() {
            return Err("A message cannot be empty. Delete it in the web app instead.".into());
        }
        if body.chars().count() > MAX_BODY {
            return Err(format!("A message can have at most {MAX_BODY} characters."));
        }
        self.editing = None;
        Ok((id, body.into()))
    }

    pub fn edited(&mut self, result: Result<MessageRecord, String>) {
        match result {
            Ok(message) => self.upsert(message),
            Err(error) => self.error = Some(format!("The edit was not saved: {error}")),
        }
    }

    /// A live chat event: each one carries its change, so nothing is refetched.
    pub fn apply(&mut self, event: ChatEvent) {
        match event {
            ChatEvent::MessageCreated { message } | ChatEvent::MessageUpdated { message } => self.upsert(message),
            ChatEvent::MessageDeleted { message_id, .. } => {
                self.messages.retain(|r| r.message.id != message_id);
                if let Some(thread) = &mut self.thread {
                    thread.replies.retain(|r| r.message.id != message_id);
                    if thread.root_id == message_id {
                        self.thread = None;
                    }
                }
                if self.editing.as_deref() == Some(message_id.as_str()) {
                    self.editing = None;
                }
            }
            ChatEvent::ConversationChanged { conversation } => {
                match self.conversations.iter_mut().find(|c| c.id == conversation.id) {
                    Some(old) => *old = conversation,
                    None => self.conversations.push(conversation),
                }
                self.rebuild_rows();
                if self.selected.as_ref().is_some_and(|id| !self.rows.iter().any(|r| &r.id == id)) {
                    self.close_conversation();
                }
            }
            ChatEvent::ConversationRemoved { conversation_id } => {
                self.conversations.retain(|c| c.id != conversation_id);
                self.states.retain(|s| s.conversation_id != conversation_id);
                self.rebuild_rows();
                if self.selected.as_deref() == Some(conversation_id.as_str()) {
                    self.close_conversation();
                }
            }
            ChatEvent::StateChanged { state } => {
                match self.states.iter_mut().find(|s| s.conversation_id == state.conversation_id) {
                    Some(old) => *old = state,
                    None => self.states.push(state),
                }
                self.rebuild_rows();
            }
        }
    }

    /// Puts a server copy of a message where it belongs: replaces the row with its id or with its
    /// nonce (the local copy of a send), or adds it in time order. Messages of other
    /// conversations are not kept (the sidebar's counts come from `state.changed`).
    fn upsert(&mut self, message: MessageRecord) {
        if self.selected.as_deref() != Some(message.conversation_id.as_str()) {
            return;
        }
        if let Some(thread) = &mut self.thread {
            if thread.root_id == message.id {
                thread.root = Some(message.clone());
            } else if message.thread_root_id.as_deref() == Some(thread.root_id.as_str()) {
                place(&mut thread.replies, message.clone());
            }
        }
        if in_main_list(&message) {
            place(&mut self.messages, message);
        }
    }

    fn rows_mut(&mut self) -> impl Iterator<Item = &mut Row> {
        let thread = self.thread.iter_mut().flat_map(|t| t.replies.iter_mut());
        self.messages.iter_mut().chain(thread)
    }

    fn rebuild_rows(&mut self) {
        self.rows = sidebar(&self.conversations, &self.states);
    }
}

/// Root messages and thread replies sent "also in channel" show in the main list.
fn in_main_list(message: &MessageRecord) -> bool {
    message.thread_root_id.is_none() || message.also_in_channel
}

fn place(rows: &mut Vec<Row>, message: MessageRecord) {
    let same = rows.iter().position(|r| {
        r.message.id == message.id || (message.nonce.is_some() && r.message.nonce == message.nonce && r.delivery != Delivery::Sent)
    });
    if let Some(at) = same {
        rows[at] = Row::sent(message);
        return;
    }
    // Sent rows in time order (RFC 3339 UTC text sorts by time); local ones stay last.
    let at = rows.iter().position(|r| r.delivery != Delivery::Sent || r.message.created_at > message.created_at).unwrap_or(rows.len());
    rows.insert(at, Row::sent(message));
}

/// What the app does after a socket event (the I/O and the backoff live in `orbit_client::socket`).
#[derive(Clone, Debug, PartialEq)]
pub enum SocketAction {
    None,
    /// The app missed events it cannot replay: refetch the sidebar and the open lists.
    Refetch,
    Chat(ChatEvent),
    /// The Orbit session ended (close 4401, or a refused upgrade): sign in again.
    SignedOut,
    /// The gateway wants its sign-in again.
    Gateway(ApiError),
}

/// Applies a socket event to the status line and says what else to do.
pub fn socket_event(live: &mut Link, events: &mut Link, kind: SocketKind, event: SocketEvent) -> SocketAction {
    let link = match kind {
        SocketKind::Live => live,
        SocketKind::Events => events,
    };
    match event {
        SocketEvent::Connecting { .. } => *link = Link::Connecting,
        SocketEvent::Connected => *link = Link::Connected,
        SocketEvent::Retrying { after, .. } => *link = Link::Retrying(after.as_secs_f64().ceil().max(1.0) as u64),
        SocketEvent::Live(LiveFrame::Resync) => return SocketAction::Refetch,
        SocketEvent::Live(LiveFrame::Event { event, .. }) => {
            // Typing, presence and kinds the app does not show yet are skipped.
            return parse_chat_event(&event).map_or(SocketAction::None, SocketAction::Chat);
        }
        SocketEvent::Live(_) => {}
        SocketEvent::Workspace(w) => {
            // Chat changes come on the live socket; the events socket matters when the
            // workspace list or everything changed.
            if w.kind == WorkspaceEventKind::ResyncRequired || w.workspaces_changed {
                return SocketAction::Refetch;
            }
        }
        SocketEvent::Ended(Ended::SignedOut) => return SocketAction::SignedOut,
        SocketEvent::Ended(Ended::Gateway(e)) => return SocketAction::Gateway(e),
        SocketEvent::Ended(Ended::Forbidden) => *link = Link::Ended("no access to this workspace".into()),
        SocketEvent::Ended(Ended::ContractMismatch) => *link = Link::Ended("update the desktop app".into()),
    }
    SocketAction::None
}

/// Mention tokens as readable Markdown: `<@user>` → **@Name**, `<#conversation>` → **#name**,
/// `<!channel>` / `<!here>` → **@channel** / **@here**. Unknown ids keep a neutral label.
pub fn display_body(body: &str, names: &HashMap<String, String>, channels: &[ConversationRecord]) -> String {
    let mut out = String::with_capacity(body.len());
    let mut rest = body;
    while let Some(start) = rest.find('<') {
        out.push_str(&rest[..start]);
        let tail = &rest[start..];
        let Some(end) = tail.find('>') else {
            out.push_str(tail);
            return out;
        };
        let token = &tail[1..end];
        let shown = match token.split_at_checked(1) {
            Some(("@", id)) if !id.is_empty() => Some(format!("**@{}**", names.get(id).map_or("unknown", String::as_str))),
            Some(("#", id)) if !id.is_empty() => {
                Some(format!("**#{}**", channels.iter().find(|c| c.id == id).map_or("unknown", |c| c.name.as_str())))
            }
            Some(("!", "channel")) => Some("**@channel**".into()),
            Some(("!", "here")) => Some("**@here**".into()),
            _ => None,
        };
        match shown {
            Some(shown) => out.push_str(&shown),
            None => out.push_str(&tail[..=end]),
        }
        rest = &tail[end + 1..];
    }
    out.push_str(rest);
    out
}

/// A message time for the list: "14:05" today, "Nov 8, 14:05" this year, else "Nov 8, 2025".
/// `at` is RFC 3339; the text shows in `now`'s time zone. An unreadable time shows as given.
pub fn short_time<Tz: chrono::TimeZone>(at: &str, now: &chrono::DateTime<Tz>) -> String
where
    Tz::Offset: std::fmt::Display,
{
    use chrono::Datelike;
    let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(at) else { return at.to_owned() };
    let local = parsed.with_timezone(&now.timezone());
    if local.date_naive() == now.date_naive() {
        local.format("%H:%M").to_string()
    } else if local.year() == now.year() {
        local.format("%b %-d, %H:%M").to_string()
    } else {
        local.format("%b %-d, %Y").to_string()
    }
}

/// What changes a row's look: when it differs, the list measures the row again.
pub fn row_signature(row: &Row) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let m = &row.message;
    (&m.id, &m.body, &m.edited_at, m.deleted, m.reply_count).hash(&mut h);
    for r in &m.reactions {
        (&r.emoji, r.user_ids.len()).hash(&mut h);
    }
    match &row.delivery {
        Delivery::Sent => 0u8.hash(&mut h),
        Delivery::Sending => 1u8.hash(&mut h),
        Delivery::Failed(e) => (2u8, e).hash(&mut h),
    }
    h.finish()
}

/// The first index where two lists of row signatures differ (the list keeps the rows before it).
pub fn first_change(old: &[u64], new: &[u64]) -> usize {
    old.iter().zip(new).take_while(|(a, b)| a == b).count()
}

#[cfg(test)]
mod tests;
