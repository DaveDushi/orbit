//! A fake backend for headless runs and goldens: a folder of JSON files shaped like the API's
//! responses, with synthetic data only.
//!
//! - `workspace.json`: `{ "name": "…", "me": "<user id of the signed-in person>" }`
//! - `conversations.json`: the body of `GET …/chat/conversations`
//! - `states.json`: the body of `GET …/chat/states`
//! - `members.json`: the body of `GET …/members`
//! - `messages.json`: `{ "<conversation id>": [MessageRecord, …] }`, oldest first
//! - `threads.json`: `{ "<root message id>": [MessageRecord, …] }`, the replies, oldest first

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::types::{ConversationKind, ConversationRecord, ConversationStateRecord, MemberPage, MemberRecord, MessageRecord, NotifyLevel};

#[derive(Debug)]
pub enum FixtureError {
    Read(PathBuf, std::io::Error),
    Parse(PathBuf, serde_json::Error),
}

impl std::fmt::Display for FixtureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FixtureError::Read(path, e) => write!(f, "cannot read {}: {e}", path.display()),
            FixtureError::Parse(path, e) => write!(f, "cannot parse {}: {e}", path.display()),
        }
    }
}

impl std::error::Error for FixtureError {}

#[derive(Deserialize)]
struct WorkspaceFile {
    name: String,
    #[serde(default)]
    me: String,
}

#[derive(Clone, Debug, Default)]
pub struct Fixture {
    pub workspace_name: String,
    /// The signed-in person's user id.
    pub me: String,
    pub conversations: Vec<ConversationRecord>,
    pub states: Vec<ConversationStateRecord>,
    pub members: Vec<MemberRecord>,
    pub messages: HashMap<String, Vec<MessageRecord>>,
    pub threads: HashMap<String, Vec<MessageRecord>>,
}

/// One line of the sidebar.
#[derive(Clone, Debug, PartialEq)]
pub struct SidebarRow {
    pub id: String,
    pub kind: ConversationKind,
    pub name: String,
    pub topic: String,
    pub unread: i64,
    pub mentions: i64,
    pub muted: bool,
}

impl Fixture {
    pub fn load(dir: &Path) -> Result<Self, FixtureError> {
        let workspace: WorkspaceFile = read(&dir.join("workspace.json"))?;
        Ok(Fixture {
            workspace_name: workspace.name,
            me: workspace.me,
            conversations: read(&dir.join("conversations.json"))?,
            states: read(&dir.join("states.json"))?,
            members: read::<MemberPage>(&dir.join("members.json"))?.items,
            messages: read(&dir.join("messages.json"))?,
            threads: read(&dir.join("threads.json"))?,
        })
    }

    pub fn sidebar(&self) -> Vec<SidebarRow> {
        sidebar(&self.conversations, &self.states)
    }
}

/// Channels by position, then direct messages, newest first. Archived ones are left out.
pub fn sidebar(conversations: &[ConversationRecord], states: &[ConversationStateRecord]) -> Vec<SidebarRow> {
    let mut open: Vec<&ConversationRecord> = conversations.iter().filter(|c| !c.archived).collect();
    open.sort_by(|a, b| {
        let dm = |c: &ConversationRecord| c.kind == ConversationKind::Dm;
        dm(a).cmp(&dm(b))
            .then_with(|| if dm(a) { b.last_message_at.cmp(&a.last_message_at) } else { a.position.cmp(&b.position) })
            .then_with(|| a.name.cmp(&b.name))
    });
    open.into_iter()
        .map(|c| {
            let state = states.iter().find(|s| s.conversation_id == c.id);
            SidebarRow {
                id: c.id.clone(),
                kind: c.kind,
                name: c.name.clone(),
                topic: c.topic.clone(),
                unread: state.map_or(0, |s| s.unread_count),
                mentions: state.map_or(0, |s| s.mention_count),
                muted: state.is_some_and(|s| s.notify == NotifyLevel::Muted),
            }
        })
        .collect()
}

fn read<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, FixtureError> {
    let bytes = std::fs::read(path).map_err(|e| FixtureError::Read(path.to_owned(), e))?;
    serde_json::from_slice(&bytes).map_err(|e| FixtureError::Parse(path.to_owned(), e))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn demo() -> Fixture {
        Fixture::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/demo")).unwrap()
    }

    #[test]
    fn the_demo_fixture_parses_and_orders_the_sidebar() {
        let rows = demo().sidebar();
        let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["general", "design", "releases", "random", "Mira Okafor", "Donna"]);
        let design = &rows[1];
        assert_eq!((design.unread, design.mentions), (4, 1));
        let fixture = demo();
        let root = fixture.messages[&design.id].iter().find(|m| m.reply_count > 0).expect("a thread root in #design");
        assert_eq!(fixture.threads[&root.id].len() as i64, root.reply_count);
        assert!(fixture.members.iter().any(|m| m.user_id == fixture.me));
        assert!(rows[3].muted, "random is muted");
    }

    #[test]
    fn unknown_fields_and_enum_values_from_a_newer_server_still_parse() {
        let json = r#"[{"id":"c1","kind":"voice","name":"x","future_field":{"a":1}}]"#;
        let parsed: Vec<ConversationRecord> = serde_json::from_str(json).unwrap();
        assert_eq!(parsed[0].kind, ConversationKind::Unknown);
        assert!(!parsed[0].archived);
    }
}
