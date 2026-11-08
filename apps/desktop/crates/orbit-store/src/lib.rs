//! The desktop's local SQLite file (decision D5: mode 0600, not encrypted, deleted on sign-out).
//! M0 holds only the outbox: messages written while offline, sent later in order.
//!
//! Each entry gets its nonce once, when it is queued, and keeps it on every retry. The server
//! returns the first message for a repeated `(conversation, author, nonce)`
//! (`apps/server/src/repositories/chat/messages.rs`, `send_message` and `sent_before`; unique index
//! `chat_messages_nonce` in `migrations/0037_chat.sql`), so a retry after a lost response cannot post twice.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension, params};

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Sql(rusqlite::Error),
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self { Error::Io(e) }
}

impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self { Error::Sql(e) }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Io(e) => write!(f, "{e}"),
            Error::Sql(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for Error {}

/// A message waiting to be sent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutboxItem {
    /// Queue order; also the send order within a conversation.
    pub seq: i64,
    /// The `nonce` of `POST …/messages` (1–64 bytes on the server).
    pub nonce: String,
    pub conversation_id: String,
    pub thread_root_id: Option<String>,
    pub body: String,
    pub attempts: i64,
}

pub struct Store {
    conn: Connection,
}

impl Store {
    /// Opens the file, or makes it readable and writable by its owner only.
    pub fn open(path: &Path) -> Result<Self, Error> {
        restrict(path)?;
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS outbox (
                seq INTEGER PRIMARY KEY AUTOINCREMENT,
                nonce TEXT NOT NULL UNIQUE,
                conversation_id TEXT NOT NULL,
                thread_root_id TEXT,
                body TEXT NOT NULL,
                attempts INTEGER NOT NULL DEFAULT 0
            );",
        )?;
        Ok(Store { conn })
    }

    /// Queues a message with a new nonce (`desk-` and 32 random hex digits).
    pub fn enqueue(&self, conversation_id: &str, thread_root_id: Option<&str>, body: &str) -> Result<OutboxItem, Error> {
        let seq = self.conn.query_row(
            "INSERT INTO outbox (nonce, conversation_id, thread_root_id, body)
             VALUES ('desk-' || lower(hex(randomblob(16))), ?1, ?2, ?3) RETURNING seq",
            params![conversation_id, thread_root_id, body],
            |row| row.get(0),
        )?;
        self.item(seq).map(|item| item.expect("the row just inserted"))
    }

    /// Everything queued, in queue order.
    pub fn pending(&self) -> Result<Vec<OutboxItem>, Error> {
        let mut statement = self.conn.prepare(&format!("{SELECT} ORDER BY seq"))?;
        Ok(statement.query_map([], read_item)?.collect::<Result<_, _>>()?)
    }

    /// The oldest message of each conversation: the only one to send now, since a later one may
    /// not overtake it. Conversations are independent of each other.
    pub fn heads(&self) -> Result<Vec<OutboxItem>, Error> {
        let mut statement = self.conn.prepare(&format!(
            "{SELECT} WHERE seq IN (SELECT min(seq) FROM outbox GROUP BY conversation_id) ORDER BY seq"
        ))?;
        Ok(statement.query_map([], read_item)?.collect::<Result<_, _>>()?)
    }

    /// Counts a send that failed; the entry stays at the head with the same nonce.
    pub fn record_attempt(&self, nonce: &str) -> Result<(), Error> {
        self.conn.execute("UPDATE outbox SET attempts = attempts + 1 WHERE nonce = ?1", [nonce])?;
        Ok(())
    }

    /// Drops an entry the server confirmed. False when it was already gone (a second reply for the
    /// same nonce, or a live `message` event that echoed it first).
    pub fn acknowledge(&self, nonce: &str) -> Result<bool, Error> {
        Ok(self.conn.execute("DELETE FROM outbox WHERE nonce = ?1", [nonce])? == 1)
    }

    fn item(&self, seq: i64) -> Result<Option<OutboxItem>, Error> {
        Ok(self.conn.query_row(&format!("{SELECT} WHERE seq = ?1"), [seq], read_item).optional()?)
    }
}

/// Deletes the file and SQLite's side files, as sign-out does. Missing files are fine.
pub fn delete(path: &Path) -> std::io::Result<()> {
    for file in [path.to_owned(), side(path, "-journal"), side(path, "-wal"), side(path, "-shm")] {
        match std::fs::remove_file(&file) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
            _ => {}
        }
    }
    Ok(())
}

const SELECT: &str = "SELECT seq, nonce, conversation_id, thread_root_id, body, attempts FROM outbox";

fn read_item(row: &rusqlite::Row<'_>) -> rusqlite::Result<OutboxItem> {
    Ok(OutboxItem {
        seq: row.get(0)?,
        nonce: row.get(1)?,
        conversation_id: row.get(2)?,
        thread_root_id: row.get(3)?,
        body: row.get(4)?,
        attempts: row.get(5)?,
    })
}

fn side(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// Creates the file as 0600 before SQLite opens it, and narrows an existing one. SQLite gives its
/// journal files the database file's mode. Windows: the file inherits the user profile's ACL.
fn restrict(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        std::fs::OpenOptions::new().create(true).append(true).mode(0o600).open(path)?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

#[cfg(test)]
mod tests;
