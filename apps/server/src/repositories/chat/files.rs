//! Files of chat messages. The bytes go through the shared `UploadService` (size limits, content
//! sniffing, per-workspace dedup, quarantine); a `chat_message_files` row is a message's
//! reference to a blob. A file larger than the chat limit goes to the conversation's Drive folder
//! instead (browser to bucket), and a `chat_drive_files` row refers to it. A file is uploaded first, and then it is its uploader's alone; the send
//! of the message attaches it. From then on it is readable by everyone who can read the
//! conversation.

use orbit_platform::Row;
use orbit_platform::{AuthorizedAttachment, BlobDownload, Flag, Id, StagedUpload, UploadError};
use sqlx::AnyConnection;
use sqlx::any::AnyRow;

use super::{
    ChatError, ChatFileRecord, ChatRepository, MESSAGE_SELECT, MessageRecord, hydrate, id_list,
    in_ids, load_access, load_actor, parse_id, parse_optional_id, reachable_dm,
};
use crate::previews::{
    FilePreview, PREVIEW_COLUMNS, PreviewOutput, Source, preview_kind, queue_in,
};

/// Files of one message.
pub(super) const MAX_MESSAGE_FILES: usize = 10;
const LIST_LIMIT: i64 = 100;
const MAX_DIMENSION: u32 = 100_000;

const FILE_COLUMNS: &str = "f.id, f.workspace_id, f.message_id, f.file_name, f.mime_type, f.size_bytes, f.width, \
     f.height, EXISTS (SELECT 1 FROM attachment_blobs b WHERE b.id = f.blob_id AND b.deleted_at IS NOT NULL) AS deleted, \
     CAST(NULL AS TEXT) AS drive_node_id";
/// A file's preview is the one of its blob and kind.
const FILE_FROM: &str = "chat_message_files f LEFT JOIN file_previews pv ON pv.source_blob_id = f.blob_id \
     AND pv.kind = CASE WHEN f.mime_type LIKE 'video/%' THEN 'video' ELSE 'document' END";
/// A file in Drive is deleted for chat once it is in the trash or deleted forever.
const DRIVE_FILE_COLUMNS: &str = "f.id, f.workspace_id, f.message_id, f.file_name, f.mime_type, f.size_bytes, \
     CAST(NULL AS BIGINT) AS width, CAST(NULL AS BIGINT) AS height, \
     (n.id IS NULL OR n.trashed_at IS NOT NULL) AS deleted, f.node_id AS drive_node_id";
const DRIVE_FILE_FROM: &str = "chat_drive_files f LEFT JOIN drive_nodes n ON n.id = f.node_id \
     LEFT JOIN file_previews pv ON pv.drive_blob_id = n.blob_id";

/// `c` is a conversation that `me` (a membership of the file's workspace) can read. A guest
/// reads no public channel, and nobody reads a direct message the guest can no longer reach.
fn conversation_readable() -> String {
    format!(
        "((c.kind = 'public' AND me.role <> 'guest') \
         OR (EXISTS (SELECT 1 FROM chat_members cm \
                     WHERE cm.conversation_id = c.id AND cm.user_id = me.user_id) \
             AND {}))",
        reachable_dm()
    )
}

/// The rows of `f` that `$1` may open: its uploader while it waits for its message, then everyone who can read
/// the conversation. Binds `$1` (the actor), `$2` (the file) and `$3` (the workspace).
fn readable_file() -> String {
    format!(
        "JOIN workspaces w ON w.id = f.workspace_id AND w.deleted_at IS NULL \
         JOIN memberships me ON me.workspace_id = f.workspace_id AND me.user_id = $1 \
         LEFT JOIN chat_conversations c ON c.id = f.conversation_id \
         WHERE f.id = $2 AND f.workspace_id = $3 AND ( \
             (f.message_id IS NULL AND f.uploaded_by = me.user_id) OR {})",
        conversation_readable()
    )
}

/// A file of a sent message, with where its bytes are.
#[derive(Clone, Debug)]
pub struct SentChatFile {
    pub file: ChatFileRecord,
    pub message_id: Id,
    pub conversation_id: Id,
    pub storage_key: String,
}

/// Where the bytes of a chat file are, for a reader.
#[derive(Clone, Debug)]
pub enum ChatFileSource {
    /// In the attachment store.
    Stored {
        blob_id: Id,
        storage_key: String,
        mime_type: String,
        file_name: String,
        size_bytes: i64,
        /// An admin deleted the blob.
        deleted: bool,
    },
    /// In the Drive bucket; `drive_blob_id` and `key` are `None` once the Drive file is in the trash or gone.
    Drive {
        drive_blob_id: Option<Id>,
        key: Option<String>,
        file_name: String,
    },
}

fn file_from_row(row: &AnyRow) -> Result<ChatFileRecord, ChatError> {
    let id = parse_id(row.get("id"))?;
    let workspace_id = parse_id(row.get("workspace_id"))?;
    let url = format!("/api/v1/workspaces/{workspace_id}/chat/files/{id}");
    let file_name: String = row.get("file_name");
    let mime_type: String = row.get("mime_type");
    let preview =
        crate::previews::preview_from_row(row, &mime_type, &file_name, &format!("{url}/preview"))?;
    Ok(ChatFileRecord {
        id,
        url,
        file_name,
        mime_type,
        size_bytes: row.get("size_bytes"),
        width: row.get("width"),
        height: row.get("height"),
        deleted: row.get::<Flag, _>("deleted").0,
        drive_node_id: parse_optional_id(row.get("drive_node_id"))?,
        preview,
    })
}

/// Puts the files of each message into its record: one query per kind of file for the whole list.
pub(super) async fn load_files(
    conn: &mut AnyConnection,
    messages: &mut [MessageRecord],
) -> Result<(), ChatError> {
    let ids = id_list(messages.iter().map(|message| message.id));
    let mut rows = Vec::new();
    for (columns, from) in [
        (FILE_COLUMNS, FILE_FROM),
        (DRIVE_FILE_COLUMNS, DRIVE_FILE_FROM),
    ] {
        rows.extend(
            sqlx::query(sqlx::AssertSqlSafe(format!(
                "SELECT {columns}, {PREVIEW_COLUMNS} FROM {from} \
                 WHERE f.message_id {} ORDER BY f.created_at, f.id",
                in_ids(&*conn, 1)
            )))
            .bind(&ids)
            .fetch_all(&mut *conn)
            .await?,
        );
    }
    for row in rows {
        let message_id = parse_id(row.get("message_id"))?;
        if let Some(message) = messages.iter_mut().find(|message| message.id == message_id) {
            message.attachments.push(file_from_row(&row)?);
        }
    }
    Ok(())
}

/// Attaches uploaded files to a new message. Each must be the author's own upload in the
/// conversation's workspace that no message has yet; a file kept in Drive must be in the conversation's own
/// Drive folder.
pub(super) async fn attach_files(
    conn: &mut AnyConnection,
    message: &mut MessageRecord,
    file_ids: &[Id],
) -> Result<(), ChatError> {
    if file_ids.is_empty() {
        return Ok(());
    }
    let mut attached = 0;
    for (table, rule) in [
        ("chat_message_files", ""),
        (
            "chat_drive_files",
            " AND EXISTS (SELECT 1 FROM drive_nodes n WHERE n.id = chat_drive_files.node_id \
              AND n.chat_conversation_id = $2)",
        ),
    ] {
        attached += sqlx::query(sqlx::AssertSqlSafe(format!(
            "UPDATE {table} SET message_id = $1, conversation_id = $2 \
             WHERE id {} AND uploaded_by = $4 AND message_id IS NULL \
             AND workspace_id = (SELECT workspace_id FROM chat_conversations WHERE id = $5){rule}",
            in_ids(&*conn, 3)
        )))
        .bind(message.id.to_string())
        .bind(message.conversation_id.to_string())
        .bind(id_list(file_ids.iter().copied()))
        .bind(message.author_id.to_string())
        .bind(message.conversation_id.to_string())
        .execute(&mut *conn)
        .await?
        .rows_affected();
    }
    if usize::try_from(attached) != Ok(file_ids.len()) {
        return Err(ChatError::Invalid { field: "file_ids" });
    }
    load_files(conn, std::slice::from_mut(message)).await
}

/// Gives a forward the files of the message it copies: new rows that point at the same blobs (or Drive files),
/// in the same order, so they are read with the access of the forward's own conversation and
/// stay when the original goes. The forwarder is their uploader. Both messages are in one
/// workspace, and a message has at most [`MAX_MESSAGE_FILES`] files, so the copy has too.
pub(super) async fn copy_files(
    conn: &mut AnyConnection,
    source: &MessageRecord,
    message: &mut MessageRecord,
) -> Result<(), ChatError> {
    if source.attachments.is_empty() {
        return Ok(());
    }
    // New ids ascend, so `ORDER BY created_at, id` keeps the order of the original.
    for file in &source.attachments {
        let copy = if file.drive_node_id.is_some() || file_in_drive(conn, file.id).await? {
            "INSERT INTO chat_drive_files (id, workspace_id, message_id, conversation_id, node_id, \
             file_name, mime_type, size_bytes, uploaded_by, created_at) \
             SELECT $1, workspace_id, $2, $3, node_id, file_name, mime_type, size_bytes, \
             $4, $5 FROM chat_drive_files WHERE id = $6 AND message_id = $7"
        } else {
            "INSERT INTO chat_message_files (id, workspace_id, message_id, conversation_id, blob_id, \
             file_name, mime_type, size_bytes, width, height, uploaded_by, created_at) \
             SELECT $1, workspace_id, $2, $3, blob_id, file_name, mime_type, size_bytes, width, height, \
             $4, $5 FROM chat_message_files WHERE id = $6 AND message_id = $7"
        };
        sqlx::query(copy)
            .bind(Id::new_v7().to_string())
            .bind(message.id.to_string())
            .bind(message.conversation_id.to_string())
            .bind(message.author_id.to_string())
            .bind(message.created_at.as_millis())
            .bind(file.id.to_string())
            .bind(source.id.to_string())
            .execute(&mut *conn)
            .await?;
    }
    load_files(conn, std::slice::from_mut(message)).await
}

/// A Drive file whose node was deleted forever has no `drive_node_id` any more, but is still a Drive file.
async fn file_in_drive(conn: &mut AnyConnection, file_id: Id) -> Result<bool, ChatError> {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM chat_drive_files WHERE id = $1")
        .bind(file_id.to_string())
        .fetch_one(&mut *conn)
        .await?;
    Ok(count > 0)
}

impl ChatRepository {
    /// Succeeds for a member of the live workspace. Run before reading an upload body.
    pub async fn authorize_upload(&self, workspace_id: Id, actor_id: Id) -> Result<(), ChatError> {
        let mut conn = self.database.pool().acquire().await?;
        load_actor(&mut conn, workspace_id, actor_id).await?;
        Ok(())
    }

    /// Turns a staged upload into a chat file that waits for its message. `width` and `height`
    /// are the uploader's measure of an image. The caller discards the staged upload when this
    /// fails.
    pub async fn finalize_file(
        &self,
        workspace_id: Id,
        actor_id: Id,
        upload: &StagedUpload,
        width: Option<u32>,
        height: Option<u32>,
    ) -> Result<ChatFileRecord, ChatError> {
        if upload.workspace_id != workspace_id || upload.owner_id != actor_id {
            return Err(ChatError::NotFound);
        }
        let dimension =
            |value: Option<u32>| value.filter(|value| (1..=MAX_DIMENSION).contains(value));
        let finalization = self.uploads.begin_finalization().await;
        let now = self.database.database_now().await?;
        let mut tx = self.database.immediate_transaction().await?;
        load_actor(&mut tx, workspace_id, actor_id).await?;
        let blob = finalization
            .finalize_blob_in_transaction(&mut tx, upload, now)
            .await?;
        let id = Id::new_v7();
        let size = i64::try_from(upload.size_bytes).map_err(|_| UploadError::SizeOverflow)?;
        sqlx::query(
            "INSERT INTO chat_message_files (id, workspace_id, blob_id, file_name, mime_type, size_bytes, \
             width, height, uploaded_by, created_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
        )
        .bind(id.to_string())
        .bind(workspace_id.to_string())
        .bind(blob.id.to_string())
        .bind(&upload.display_name)
        .bind(&upload.detected_media_type)
        .bind(size)
        .bind(dimension(width).map(i64::from))
        .bind(dimension(height).map(i64::from))
        .bind(actor_id.to_string())
        .bind(now.as_millis())
        .execute(&mut *tx)
        .await?;
        if let Some(kind) = preview_kind(&upload.detected_media_type, &upload.display_name) {
            queue_in(&mut tx, workspace_id, Source::Stored(blob.id), kind, now).await?;
        }
        let file = load_file(&mut tx, FILE_COLUMNS, FILE_FROM, id).await?;
        tx.commit().await?;
        Ok(file)
    }

    /// Makes a big file that the actor just uploaded to a conversation's Drive folder a chat file that waits for
    /// its message (`file_ids` of the send). Only the uploader's own live file in a chat folder of a conversation
    /// they are a member of.
    pub async fn attach_drive_upload(
        &self,
        workspace_id: Id,
        actor_id: Id,
        node_id: Id,
    ) -> Result<ChatFileRecord, ChatError> {
        let now = self.database.database_now().await?;
        let mut tx = self.database.immediate_transaction().await?;
        load_actor(&mut tx, workspace_id, actor_id).await?;
        let node = sqlx::query(
            "SELECT n.name, n.mime, n.size FROM drive_nodes n \
             WHERE n.id = $1 AND n.workspace_id = $2 AND n.kind = 'file' AND n.trashed_at IS NULL \
             AND n.created_by = $3 AND n.chat_conversation_id IS NOT NULL \
             AND EXISTS (SELECT 1 FROM chat_members m WHERE m.conversation_id = n.chat_conversation_id \
                         AND m.user_id = $3)",
        )
        .bind(node_id.to_string())
        .bind(workspace_id.to_string())
        .bind(actor_id.to_string())
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(ChatError::NotFound)?;
        let id = Id::new_v7();
        sqlx::query(
            "INSERT INTO chat_drive_files (id, workspace_id, node_id, file_name, mime_type, size_bytes, \
             uploaded_by, created_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(id.to_string())
        .bind(workspace_id.to_string())
        .bind(node_id.to_string())
        .bind(node.get::<String, _>("name"))
        .bind(
            node.get::<Option<String>, _>("mime")
                .unwrap_or_else(|| "application/octet-stream".to_owned()),
        )
        .bind(node.get::<Option<i64>, _>("size").unwrap_or(0))
        .bind(actor_id.to_string())
        .bind(now.as_millis())
        .execute(&mut *tx)
        .await?;
        let file = load_file(&mut tx, DRIVE_FILE_COLUMNS, DRIVE_FILE_FROM, id).await?;
        tx.commit().await?;
        Ok(file)
    }

    /// Where a chat file's bytes are, for `actor_id`: its uploader while it waits for its message, then
    /// everyone who can read the conversation.
    pub async fn file_source(
        &self,
        workspace_id: Id,
        actor_id: Id,
        file_id: Id,
    ) -> Result<ChatFileSource, ChatError> {
        let stored = sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT b.id AS blob_id, b.storage_key, b.deleted_at, f.mime_type, f.file_name, f.size_bytes \
             FROM chat_message_files f \
             JOIN attachment_blobs b ON b.id = f.blob_id AND b.workspace_id = f.workspace_id {}",
            readable_file()
        )))
        .bind(actor_id.to_string())
        .bind(file_id.to_string())
        .bind(workspace_id.to_string())
        .fetch_optional(self.database.pool())
        .await?;
        if let Some(row) = stored {
            return Ok(ChatFileSource::Stored {
                blob_id: parse_id(row.get("blob_id"))?,
                storage_key: row.get("storage_key"),
                mime_type: row.get("mime_type"),
                file_name: row.get("file_name"),
                size_bytes: row.get("size_bytes"),
                deleted: row.get::<Option<i64>, _>("deleted_at").is_some(),
            });
        }
        let row = sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT f.file_name, b.id AS blob_id, b.r2_key FROM chat_drive_files f \
             LEFT JOIN drive_nodes n ON n.id = f.node_id AND n.trashed_at IS NULL \
             LEFT JOIN drive_blobs b ON b.id = n.blob_id AND b.status = 'ready' {}",
            readable_file()
        )))
        .bind(actor_id.to_string())
        .bind(file_id.to_string())
        .bind(workspace_id.to_string())
        .fetch_optional(self.database.pool())
        .await?
        .ok_or(ChatError::NotFound)?;
        Ok(ChatFileSource::Drive {
            drive_blob_id: parse_optional_id(row.get("blob_id"))?,
            key: row.get("r2_key"),
            file_name: row.get("file_name"),
        })
    }

    /// Opens a stored chat file for `actor_id` (see [`Self::file_source`]). A file kept in Drive is
    /// [`ChatError::NotFound`] here: it is sent from the bucket.
    pub async fn download_file(
        &self,
        workspace_id: Id,
        actor_id: Id,
        file_id: Id,
    ) -> Result<BlobDownload, ChatError> {
        let ChatFileSource::Stored {
            storage_key,
            mime_type,
            file_name,
            deleted,
            ..
        } = self.file_source(workspace_id, actor_id, file_id).await?
        else {
            return Err(ChatError::NotFound);
        };
        self.uploads
            .download(move || async move {
                Ok(AuthorizedAttachment {
                    storage_key,
                    media_type: mime_type,
                    display_name: file_name,
                    deleted,
                })
            })
            .await
            .map_err(ChatError::Upload)
    }

    /// The preview of a chat file `actor_id` can open; with `request`, queues it first when the file has none
    /// (a file from before previews). `None` for a file that gets no preview.
    pub async fn file_preview(
        &self,
        workspace_id: Id,
        actor_id: Id,
        file_id: Id,
        request: bool,
    ) -> Result<Option<FilePreview>, ChatError> {
        let source = self.file_source(workspace_id, actor_id, file_id).await?;
        let mut tx = self.database.immediate_transaction().await?;
        let (columns, from) = match source {
            ChatFileSource::Stored { .. } => (FILE_COLUMNS, FILE_FROM),
            ChatFileSource::Drive { .. } => (DRIVE_FILE_COLUMNS, DRIVE_FILE_FROM),
        };
        let mut file = load_file(&mut tx, columns, from, file_id).await?;
        let source = match source {
            ChatFileSource::Stored {
                blob_id,
                deleted: false,
                ..
            } => Some(Source::Stored(blob_id)),
            ChatFileSource::Drive {
                drive_blob_id: Some(blob_id),
                ..
            } => Some(Source::Drive(blob_id)),
            _ => None,
        };
        if request
            && let (Some(preview), Some(source)) = (&file.preview, source)
            && preview.status == crate::previews::PreviewStatus::None
        {
            let now = self.database.database_now().await?;
            queue_in(&mut tx, workspace_id, source, preview.kind, now).await?;
            file = load_file(&mut tx, columns, from, file_id).await?;
        }
        tx.commit().await?;
        Ok(file.preview)
    }

    /// The preview output `role` (`play`, `poster`, `pdf`) of a chat file `actor_id` can open.
    pub async fn preview_output(
        &self,
        workspace_id: Id,
        actor_id: Id,
        file_id: Id,
        role: &str,
    ) -> Result<(PreviewOutput, String), ChatError> {
        let (source, name) = match self.file_source(workspace_id, actor_id, file_id).await? {
            ChatFileSource::Stored {
                blob_id,
                deleted: false,
                file_name,
                ..
            } => (Source::Stored(blob_id), file_name),
            ChatFileSource::Drive {
                drive_blob_id: Some(blob_id),
                file_name,
                ..
            } => (Source::Drive(blob_id), file_name),
            _ => return Err(ChatError::NotFound),
        };
        let mut conn = self.database.pool().acquire().await?;
        let output = crate::previews::output(&mut conn, source, role)
            .await?
            .ok_or(ChatError::NotFound)?;
        Ok((output, name))
    }

    /// A file of a sent message that `actor_id` can read: the conversation rule of
    /// [`Self::file_source`], but never an upload that waits for its message, and never a file
    /// of a deleted message. Every other file is [`ChatError::NotFound`].
    pub async fn sent_file(
        &self,
        workspace_id: Id,
        actor_id: Id,
        file_id: Id,
    ) -> Result<SentChatFile, ChatError> {
        let row = sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT {FILE_COLUMNS}, {PREVIEW_COLUMNS}, f.conversation_id, b.storage_key FROM {FILE_FROM} \
             JOIN attachment_blobs b ON b.id = f.blob_id AND b.workspace_id = f.workspace_id \
             JOIN workspaces w ON w.id = f.workspace_id AND w.deleted_at IS NULL \
             JOIN memberships me ON me.workspace_id = f.workspace_id AND me.user_id = $1 \
             JOIN chat_messages m ON m.id = f.message_id AND m.deleted_at IS NULL \
             JOIN chat_conversations c ON c.id = f.conversation_id \
             WHERE f.id = $2 AND f.workspace_id = $3 AND {}",
            conversation_readable()
        )))
        .bind(actor_id.to_string())
        .bind(file_id.to_string())
        .bind(workspace_id.to_string())
        .fetch_optional(self.database.pool())
        .await?
        .ok_or(ChatError::NotFound)?;
        Ok(SentChatFile {
            file: file_from_row(&row)?,
            message_id: parse_id(row.get("message_id"))?,
            conversation_id: parse_id(row.get("conversation_id"))?,
            storage_key: row.get("storage_key"),
        })
    }

    /// The messages of a conversation that have files, newest first.
    pub async fn list_files(
        &self,
        workspace_id: Id,
        actor_id: Id,
        conversation_id: Id,
    ) -> Result<Vec<MessageRecord>, ChatError> {
        let mut tx = self.database.pool().begin().await?;
        load_access(&mut tx, workspace_id, actor_id, conversation_id).await?;
        let rows = sqlx::query(sqlx::AssertSqlSafe(format!(
            "{MESSAGE_SELECT} WHERE m.id IN (SELECT message_id FROM (SELECT message_id FROM chat_message_files \
             WHERE conversation_id = $1 UNION SELECT message_id FROM chat_drive_files WHERE conversation_id = $1) \
             files ORDER BY message_id DESC LIMIT $2) ORDER BY m.id DESC"
        )))
        .bind(conversation_id.to_string())
        .bind(LIST_LIMIT)
        .fetch_all(&mut *tx)
        .await?;
        hydrate(&mut tx, &rows).await
    }
}

/// One chat file with its preview, by id (no access check: the caller made or checked it).
async fn load_file(
    conn: &mut AnyConnection,
    columns: &str,
    from: &str,
    file_id: Id,
) -> Result<ChatFileRecord, ChatError> {
    let row = sqlx::query(sqlx::AssertSqlSafe(format!(
        "SELECT {columns}, {PREVIEW_COLUMNS} FROM {from} WHERE f.id = $1"
    )))
    .bind(file_id.to_string())
    .fetch_optional(&mut *conn)
    .await?
    .ok_or(ChatError::NotFound)?;
    file_from_row(&row)
}
