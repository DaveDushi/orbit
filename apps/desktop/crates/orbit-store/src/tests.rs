use super::*;

fn open() -> (tempfile::TempDir, PathBuf, Store) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("orbit.sqlite");
    let store = Store::open(&path).unwrap();
    (dir, path, store)
}

#[test]
fn messages_leave_in_queue_order_one_head_per_conversation() {
    let (_dir, _path, store) = open();
    let a = store.enqueue("general", None, "first").unwrap();
    let b = store.enqueue("general", None, "second").unwrap();
    let c = store.enqueue("design", Some("root-1"), "reply").unwrap();

    let bodies = |items: Vec<OutboxItem>| items.into_iter().map(|i| i.body).collect::<Vec<_>>();
    assert_eq!(bodies(store.pending().unwrap()), ["first", "second", "reply"]);
    assert_eq!(bodies(store.heads().unwrap()), ["first", "reply"], "second waits behind first");

    assert!(store.acknowledge(&a.nonce).unwrap());
    assert_eq!(bodies(store.heads().unwrap()), ["second", "reply"]);
    assert!(store.acknowledge(&c.nonce).unwrap());
    assert!(store.acknowledge(&b.nonce).unwrap());
    assert!(store.pending().unwrap().is_empty());
}

#[test]
fn a_retry_keeps_the_nonce_so_the_server_can_dedupe_it() {
    let (_dir, path, store) = open();
    let sent = store.enqueue("general", None, "hello").unwrap();
    let other = store.enqueue("general", None, "hello").unwrap();
    assert_ne!(sent.nonce, other.nonce, "the same text twice is two messages");
    assert!(sent.nonce.starts_with("desk-") && sent.nonce.len() == 37, "{}", sent.nonce);

    // The response was lost: count the attempt, quit, start again.
    store.record_attempt(&sent.nonce).unwrap();
    drop(store);
    let store = Store::open(&path).unwrap();
    let head = &store.heads().unwrap()[0];
    assert_eq!((&head.nonce, head.attempts), (&sent.nonce, 1));

    // The reply and the live echo both confirm it; only the first removes it.
    assert!(store.acknowledge(&sent.nonce).unwrap());
    assert!(!store.acknowledge(&sent.nonce).unwrap());
    assert_eq!(store.heads().unwrap()[0].nonce, other.nonce);
}

#[cfg(unix)]
#[test]
fn the_file_is_private_and_sign_out_deletes_it() {
    use std::os::unix::fs::PermissionsExt;
    let (_dir, path, store) = open();
    store.enqueue("general", None, "secret").unwrap();
    assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    drop(store);

    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    drop(Store::open(&path).unwrap());
    assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600, "reopening narrows it");

    delete(&path).unwrap();
    assert!(!path.exists());
    delete(&path).unwrap();
}
