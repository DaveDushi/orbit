use std::path::Path;

use orbit_client::{ChatEvent, ConversationStateRecord, Fixture, MessageRecord, NotifyLevel};

use super::*;

const DESIGN: &str = "00000000-0000-4000-8000-0000000000a2";
const GENERAL: &str = "00000000-0000-4000-8000-0000000000a1";
const THREAD_ROOT: &str = "00000000-0000-4000-8000-000000000003";

fn demo() -> (Fixture, Chat) {
    let f = Fixture::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/demo")).unwrap();
    let mut chat = Chat::new(&f.me, "w1", &f.workspace_name);
    chat.set_sidebar(f.conversations.clone(), f.states.clone());
    chat.set_members(&f.members);
    (f, chat)
}

fn open(chat: &mut Chat, f: &Fixture, id: &str) {
    assert!(chat.select(id));
    chat.set_messages(id, f.messages.get(id).cloned().unwrap_or_default());
}

fn message(id: &str, conversation: &str, at: &str, body: &str) -> MessageRecord {
    MessageRecord {
        id: id.into(),
        conversation_id: conversation.into(),
        thread_root_id: None,
        kind: MessageKind::Message,
        author_id: "00000000-0000-4000-8000-000000000002".into(),
        body: body.into(),
        created_at: at.into(),
        edited_at: None,
        deleted: false,
        reactions: Vec::new(),
        also_in_channel: false,
        nonce: None,
        reply_count: 0,
        last_reply_at: None,
    }
}

fn bodies(rows: &[Row]) -> Vec<&str> {
    rows.iter().map(|r| r.message.body.as_str()).collect()
}

#[test]
fn a_late_page_of_another_conversation_is_dropped() {
    let (f, mut chat) = demo();
    assert!(chat.select(DESIGN));
    assert!(!chat.select(DESIGN), "already open: no second load");
    assert!(chat.select(GENERAL));
    chat.set_messages(DESIGN, f.messages[DESIGN].clone());
    assert!(chat.messages.is_empty() && chat.loading, "the #design reply came after the switch to #general");
    chat.set_messages(GENERAL, f.messages[GENERAL].clone());
    assert_eq!(chat.messages.len(), 2);
    assert!(!chat.loading);
}

#[test]
fn a_send_shows_at_once_and_the_servers_copy_replaces_it_by_nonce() {
    let (f, mut chat) = demo();
    open(&mut chat, &f, GENERAL);
    assert_eq!(chat.queue_send("   ", false, "n0".into(), "2026-11-09T10:00:00Z"), Err("Write a message first.".into()));
    let out = chat.queue_send("  hello  ", false, "n1".into(), "2026-11-09T10:00:00Z").unwrap();
    assert_eq!((out.body.as_str(), out.thread_root_id.as_deref()), ("hello", None));
    assert_eq!(chat.messages.last().unwrap().delivery, Delivery::Sending);

    // The live socket echoes it before the POST answers: same nonce, so one row, not two.
    let mut echo = message("m-new", GENERAL, "2026-11-09T10:00:01Z", "hello");
    echo.nonce = Some("n1".into());
    echo.author_id = chat.me.clone();
    chat.apply(ChatEvent::MessageCreated { message: echo.clone() });
    chat.sent("n1", Ok(echo));
    assert_eq!(bodies(&chat.messages), ["Welcome to **Northwind**. Office hours are on Thursday.", "Thanks Theo!", "hello"]);
    assert_eq!(chat.messages[2].delivery, Delivery::Sent);
}

#[test]
fn a_failed_send_keeps_its_nonce_for_the_retry() {
    let (f, mut chat) = demo();
    open(&mut chat, &f, GENERAL);
    chat.queue_send("offline text", false, "n2".into(), "2026-11-09T10:00:00Z").unwrap();
    chat.sent("n2", Err("Cannot reach the server".into()));
    assert_eq!(chat.messages.last().unwrap().delivery, Delivery::Failed("Cannot reach the server".into()));
    let again = chat.retry("n2").unwrap();
    assert_eq!((again.nonce.as_str(), again.body.as_str()), ("n2", "offline text"));
    assert_eq!(chat.messages.last().unwrap().delivery, Delivery::Sending);
    assert!(chat.retry("n2").is_none(), "only a failed message is sent again");
}

#[test]
fn a_reload_keeps_messages_still_on_their_way_out() {
    let (f, mut chat) = demo();
    open(&mut chat, &f, GENERAL);
    chat.queue_send("pending", false, "n3".into(), "2026-11-09T10:00:00Z").unwrap();
    // A resync refetches the page while the send is in flight.
    chat.set_messages(GENERAL, f.messages[GENERAL].clone());
    assert_eq!(chat.messages.last().unwrap().message.body, "pending");
    // Once the page has the server's copy, the local one goes.
    let mut copy = message("m-p", GENERAL, "2026-11-09T10:00:01Z", "pending");
    copy.nonce = Some("n3".into());
    let mut page = f.messages[GENERAL].clone();
    page.push(copy);
    chat.set_messages(GENERAL, page);
    assert_eq!(chat.messages.iter().filter(|r| r.message.body == "pending").count(), 1);
    assert_eq!(chat.messages.last().unwrap().delivery, Delivery::Sent);
}

#[test]
fn live_events_insert_in_time_order_and_update_and_delete() {
    let (f, mut chat) = demo();
    open(&mut chat, &f, GENERAL);
    chat.apply(ChatEvent::MessageCreated { message: message("late", GENERAL, "2026-11-08T08:05:00Z", "in between") });
    assert_eq!(bodies(&chat.messages)[1], "in between", "a replayed event lands by time, not at the end");
    let mut edited = message("late", GENERAL, "2026-11-08T08:05:00Z", "in between (edited)");
    edited.edited_at = Some("2026-11-09T00:00:00Z".into());
    chat.apply(ChatEvent::MessageUpdated { message: edited });
    assert_eq!(bodies(&chat.messages)[1], "in between (edited)");
    chat.apply(ChatEvent::MessageDeleted { conversation_id: GENERAL.into(), message_id: "late".into(), thread_root_id: None });
    assert_eq!(chat.messages.len(), 2);
    // Other conversations are not kept; their counts come from state.changed.
    chat.apply(ChatEvent::MessageCreated { message: message("x", DESIGN, "2026-11-09T00:00:00Z", "elsewhere") });
    assert_eq!(chat.messages.len(), 2);
    chat.apply(ChatEvent::StateChanged {
        state: ConversationStateRecord { conversation_id: DESIGN.into(), unread_count: 5, mention_count: 2, notify: NotifyLevel::All, favorite: false },
    });
    let design = chat.rows.iter().find(|r| r.id == DESIGN).unwrap();
    assert_eq!((design.unread, design.mentions), (5, 2));
}

#[test]
fn thread_replies_go_to_the_thread_and_only_also_in_channel_ones_to_the_list() {
    let (f, mut chat) = demo();
    open(&mut chat, &f, DESIGN);
    assert!(chat.open_thread(THREAD_ROOT));
    assert_eq!(chat.thread.as_ref().unwrap().root.as_ref().unwrap().reply_count, 3, "the root shows before the load");
    let root = chat.messages.iter().find(|r| r.message.id == THREAD_ROOT).unwrap().message.clone();
    chat.set_thread(root, f.threads[THREAD_ROOT].clone());
    let before = chat.messages.len();

    let mut reply = message("r4", DESIGN, "2026-11-09T09:00:00Z", "a reply");
    reply.thread_root_id = Some(THREAD_ROOT.into());
    chat.apply(ChatEvent::MessageCreated { message: reply.clone() });
    assert_eq!(chat.thread.as_ref().unwrap().replies.len(), 4);
    assert_eq!(chat.messages.len(), before);

    reply.id = "r5".into();
    reply.also_in_channel = true;
    chat.apply(ChatEvent::MessageCreated { message: reply });
    assert_eq!(chat.thread.as_ref().unwrap().replies.len(), 5);
    assert_eq!(chat.messages.len(), before + 1);

    let out = chat.queue_send("thread answer", true, "n4".into(), "2026-11-09T10:00:00Z").unwrap();
    assert_eq!(out.thread_root_id.as_deref(), Some(THREAD_ROOT));
    assert_eq!(chat.thread.as_ref().unwrap().replies.last().unwrap().delivery, Delivery::Sending);

    chat.apply(ChatEvent::MessageDeleted { conversation_id: DESIGN.into(), message_id: THREAD_ROOT.into(), thread_root_id: None });
    assert!(chat.thread.is_none(), "the root is gone");
}

#[test]
fn only_own_sent_messages_can_be_edited() {
    let (f, mut chat) = demo();
    open(&mut chat, &f, DESIGN);
    let own = chat.last_own_message().expect("Ada has a message in #design");
    let theirs = chat.messages.iter().find(|r| r.message.author_id != chat.me && r.message.kind == MessageKind::Message).unwrap().message.id.clone();
    assert!(chat.start_edit(&theirs).is_none());
    let body = chat.start_edit(&own).unwrap();
    assert!(body.starts_with("I moved the mock"));
    assert_eq!(chat.finish_edit("  "), Err("A message cannot be empty. Delete it in the web app instead.".into()));
    assert_eq!(chat.editing.as_deref(), Some(own.as_str()), "still editing after a refused edit");
    assert_eq!(chat.finish_edit(" new text "), Ok((own.clone(), "new text".into())));
    assert!(chat.editing.is_none());
    chat.edited(Err("HTTP 403".into()));
    assert_eq!(chat.error.as_deref(), Some("The edit was not saved: HTTP 403"));
}

#[test]
fn a_removed_or_archived_conversation_closes() {
    let (f, mut chat) = demo();
    open(&mut chat, &f, DESIGN);
    let mut archived = f.conversations.iter().find(|c| c.id == DESIGN).unwrap().clone();
    archived.archived = true;
    chat.apply(ChatEvent::ConversationChanged { conversation: archived });
    assert!(chat.selected.is_none() && chat.messages.is_empty());
    open(&mut chat, &f, GENERAL);
    chat.apply(ChatEvent::ConversationRemoved { conversation_id: GENERAL.into() });
    assert!(chat.selected.is_none());
    assert!(!chat.rows.iter().any(|r| r.id == GENERAL));
}

#[test]
fn bodies_over_the_servers_limit_are_refused_before_sending() {
    let (f, mut chat) = demo();
    open(&mut chat, &f, GENERAL);
    let long = "é".repeat(MAX_BODY + 1);
    assert!(chat.queue_send(&long, false, "n5".into(), "t").is_err());
    assert!(chat.queue_send(&"é".repeat(MAX_BODY), false, "n6".into(), "t").is_ok(), "the limit counts characters, not bytes");
}

#[test]
fn socket_events_drive_the_status_line_and_the_next_step() {
    use std::time::Duration;
    use orbit_client::live::Ended;
    use orbit_client::{LiveFrame, SocketEvent, SocketKind, WorkspaceEvent, WorkspaceEventKind};

    let (mut live, mut events) = (Link::Off, Link::Off);
    let mut on = |kind, event| socket_event(&mut live, &mut events, kind, event);
    assert_eq!(on(SocketKind::Live, SocketEvent::Connecting { attempt: 0 }), SocketAction::None);
    assert_eq!(on(SocketKind::Live, SocketEvent::Retrying { after: Duration::from_millis(1500), close_code: Some(1012) }), SocketAction::None);
    assert_eq!(on(SocketKind::Live, SocketEvent::Live(LiveFrame::Resync)), SocketAction::Refetch);
    let created = serde_json::json!({"type":"conversation.removed","conversation_id":"c9"});
    assert_eq!(
        on(SocketKind::Live, SocketEvent::Live(LiveFrame::Event { seq: Some(4), topic: "chat".into(), event: created })),
        SocketAction::Chat(ChatEvent::ConversationRemoved { conversation_id: "c9".into() })
    );
    let typing = serde_json::json!({"type":"typing","user_id":"u2"});
    assert_eq!(on(SocketKind::Live, SocketEvent::Live(LiveFrame::Event { seq: None, topic: "chat".into(), event: typing })), SocketAction::None);
    let changed = |kind, workspaces_changed| WorkspaceEvent { version: 1, kind, sequence: "1".into(), workspaces_changed, profile_changed: false, views_only: false };
    assert_eq!(on(SocketKind::Events, SocketEvent::Workspace(changed(WorkspaceEventKind::Changed, false))), SocketAction::None);
    assert_eq!(on(SocketKind::Events, SocketEvent::Workspace(changed(WorkspaceEventKind::Changed, true))), SocketAction::Refetch);
    assert_eq!(on(SocketKind::Events, SocketEvent::Workspace(changed(WorkspaceEventKind::ResyncRequired, false))), SocketAction::Refetch);
    assert_eq!(on(SocketKind::Events, SocketEvent::Ended(Ended::Forbidden)), SocketAction::None);
    assert_eq!(on(SocketKind::Live, SocketEvent::Ended(Ended::SignedOut)), SocketAction::SignedOut);
    assert_eq!(live, Link::Retrying(2), "1.5 s rounds up: never 'reconnecting in 0 s'");
    assert_eq!(events, Link::Ended("no access to this workspace".into()));
}

#[test]
fn mention_tokens_read_as_names() {
    let (f, chat) = demo();
    let body = "Hi <@00000000-0000-4000-8000-000000000004>, see <#00000000-0000-4000-8000-0000000000a2> and <!here>. <@nobody> a<b> c < d";
    assert_eq!(
        display_body(body, &chat.names, &f.conversations),
        "Hi **@Mira Okafor**, see **#design** and **@here**. **@unknown** a<b> c < d"
    );
}

#[test]
fn the_list_remeasures_from_the_first_changed_row() {
    let (f, mut chat) = demo();
    open(&mut chat, &f, GENERAL);
    let before: Vec<u64> = chat.messages.iter().map(row_signature).collect();
    chat.queue_send("x", false, "n7".into(), "t").unwrap();
    let after: Vec<u64> = chat.messages.iter().map(row_signature).collect();
    assert_eq!(first_change(&before, &after), 2, "an append keeps every row");
    chat.sent("n7", Err("down".into()));
    let failed: Vec<u64> = chat.messages.iter().map(row_signature).collect();
    assert_eq!(first_change(&after, &failed), 2, "the failed row is measured again");
}

#[test]
fn message_times_show_in_the_local_zone_and_shorten_by_age() {
    use chrono::{FixedOffset, TimeZone};
    let berlin = FixedOffset::east_opt(3600).unwrap();
    let now = berlin.with_ymd_and_hms(2026, 11, 9, 10, 0, 0).unwrap();
    assert_eq!(short_time("2026-11-09T08:05:00Z", &now), "09:05");
    assert_eq!(short_time("2026-11-08T23:30:00Z", &now), "00:30", "late UTC evening is today in Berlin");
    assert_eq!(short_time("2026-11-08T09:42:00Z", &now), "Nov 8, 10:42");
    assert_eq!(short_time("2025-12-24T12:00:00Z", &now), "Dec 24, 2025");
    assert_eq!(short_time("soon", &now), "soon");
}
