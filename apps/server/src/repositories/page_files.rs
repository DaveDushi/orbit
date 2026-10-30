//! Files attached to docs pages (editor images and files, uploaded covers). The bytes go through
//! the shared `UploadService` (size limits, content sniffing, per-workspace dedup, quarantine);
//! a `page_files` row is the page's reference to a blob. Access follows the page: a teamspace
//! page's files are readable by every member, a private page's only by its owner, and a trashed
//! page's by nobody.
//!
//! `csv`, `tsv` and text files can change. Each change makes a new blob, moves the replaced one to
//! `page_file_versions` (a person can restore it), and needs the version it was based on, so a stale
//! save answers a conflict and overwrites nothing.

use orbit_platform::{
    AuthorizedAttachment, BlobDownload, Database, Flag, Id, Row, StagedUpload, TimestampMillis,
    UploadError, UploadService,
};
use serde::Serialize;
use serde_json::json;
use sqlx::{Any, Transaction};
use thiserror::Error;
use utoipa::ToSchema;

use super::pages::visible;
use crate::audit::{self, AuditOutcome};
use crate::file_parse::{self, FileKind, MAX_EDIT_BYTES};

/// A file attached to a page.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct PageFile {
    #[schema(value_type = String)]
    pub id: Id,
    #[schema(value_type = String)]
    pub page_id: Id,
    /// Same-origin download path (`/api/v1/workspaces/{workspace_id}/pages/{page_id}/files/{id}`);
    /// usable as an image `src`, a file link, or the page's `cover_url`.
    pub url: String,
    pub file_name: String,
    /// Detected from the file's bytes, not taken from the client.
    pub mime_type: String,
    pub size_bytes: u64,
    #[schema(value_type = String, format = DateTime)]
    pub created_at: TimestampMillis,
    /// Starts at 1; each save or restore adds one. Send it back as `expected_version`.
    pub version: u64,
    pub kind: FileKind,
    /// The kind can change here (`csv`, `tsv`, text), and the page is not locked.
    pub can_edit: bool,
}

/// One version of a page file, the current one too.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct PageFileVersion {
    pub version: u64,
    pub current: bool,
    pub size_bytes: u64,
    #[schema(value_type = Option<String>)]
    pub created_by: Option<Id>,
    pub created_by_name: Option<String>,
    #[schema(value_type = String, format = DateTime)]
    pub created_at: TimestampMillis,
}

/// A page file with what the routes need to read its bytes.
#[derive(Clone, Debug)]
pub struct PageFileRecord {
    pub file: PageFile,
    pub storage_key: String,
    pub deleted: bool,
    pub blob_id: String,
    pub locked: bool,
}

#[derive(Debug, Error)]
pub enum PageFileError {
    /// The page or file is missing, trashed, or not visible to the caller.
    #[error("page file target was not found")]
    NotFound,
    #[error(transparent)]
    Upload(#[from] UploadError),
    #[error("page file repository is unavailable")]
    Database(#[from] sqlx::Error),
    #[error("stale page file version")]
    VersionConflict { current: Box<PageFile> },
    #[error("this kind of file cannot change")]
    NotEditable,
    #[error("the file is too large to edit")]
    TooLargeToEdit,
    #[error("page is locked")]
    Locked,
    /// An admin deleted the stored file.
    #[error("the stored file was deleted")]
    Deleted,
}

/// The same-origin download path of a page file.
#[must_use]
pub fn page_file_url(workspace_id: Id, page_id: Id, file_id: Id) -> String {
    format!("/api/v1/workspaces/{workspace_id}/pages/{page_id}/files/{file_id}")
}

/// Parses a canonical page file path back into `(workspace_id, page_id, file_id)`. Anything that
/// is not exactly what [`page_file_url`] produces (query, fragment, other casing, extra
/// segments) is rejected.
#[must_use]
pub fn parse_page_file_url(url: &str) -> Option<(Id, Id, Id)> {
    let rest = url.strip_prefix("/api/v1/workspaces/")?;
    let mut parts = rest.split('/');
    let workspace_id: Id = parts.next()?.parse().ok()?;
    if parts.next()? != "pages" {
        return None;
    }
    let page_id: Id = parts.next()?.parse().ok()?;
    if parts.next()? != "files" {
        return None;
    }
    let file_id: Id = parts.next()?.parse().ok()?;
    if parts.next().is_some() || page_file_url(workspace_id, page_id, file_id) != url {
        return None;
    }
    Some((workspace_id, page_id, file_id))
}

#[derive(Clone)]
pub struct PageFileRepository {
    database: Database,
    uploads: UploadService,
}

impl PageFileRepository {
    #[must_use]
    pub fn new(database: Database, uploads: UploadService) -> Self {
        Self { database, uploads }
    }

    #[must_use]
    pub fn uploads(&self) -> &UploadService {
        &self.uploads
    }

    /// Succeeds when `actor_id` may attach files to the page: a member of the live workspace and
    /// the page is live and visible to them. Run before reading an upload body.
    pub async fn authorize_upload(
        &self,
        workspace_id: Id,
        page_id: Id,
        actor_id: Id,
    ) -> Result<(), PageFileError> {
        let mut connection = self.database.pool().acquire().await?;
        require_live_page(&mut connection, workspace_id, page_id, actor_id).await
    }

    /// Turns a staged upload into a file of the page (re-checking access in the same
    /// transaction) and records `page.file_added`. The caller discards the staged upload when
    /// this fails.
    pub async fn finalize(
        &self,
        workspace_id: Id,
        page_id: Id,
        actor_id: Id,
        upload: &StagedUpload,
        request_id: &str,
    ) -> Result<PageFile, PageFileError> {
        if upload.workspace_id != workspace_id || upload.owner_id != actor_id {
            return Err(PageFileError::NotFound);
        }
        let finalization = self.uploads.begin_finalization().await;
        let now = self.database.database_now().await?;
        let mut tx = self.database.immediate_transaction().await?;
        require_live_page(&mut tx, workspace_id, page_id, actor_id).await?;
        let blob = finalization
            .finalize_blob_in_transaction(&mut tx, upload, now)
            .await?;
        let id = Id::new_v7();
        let size = i64::try_from(upload.size_bytes).map_err(|_| UploadError::SizeOverflow)?;
        sqlx::query(
            "INSERT INTO page_files (id, workspace_id, page_id, blob_id, file_name, mime_type, \
             size_bytes, uploaded_by, created_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        )
        .bind(id.to_string())
        .bind(workspace_id.to_string())
        .bind(page_id.to_string())
        .bind(blob.id.to_string())
        .bind(&upload.display_name)
        .bind(&upload.detected_media_type)
        .bind(size)
        .bind(actor_id.to_string())
        .bind(now.as_millis())
        .execute(&mut *tx)
        .await?;
        audit::record(
            &mut tx,
            workspace_id,
            Some(actor_id),
            "page.file_added",
            AuditOutcome::Success,
            "page",
            Some(page_id),
            request_id,
            json!({"file_id": id}),
            now,
        )
        .await?;
        tx.commit().await?;
        Ok(PageFile {
            id,
            page_id,
            url: page_file_url(workspace_id, page_id, id),
            file_name: upload.display_name.clone(),
            mime_type: upload.detected_media_type.clone(),
            size_bytes: upload.size_bytes,
            created_at: now,
            version: 1,
            kind: file_parse::classify(&upload.display_name, &upload.detected_media_type),
            can_edit: file_parse::classify(&upload.display_name, &upload.detected_media_type)
                .editable(),
        })
    }

    /// Attaches bytes the server already holds (e.g. a file the Notion importer downloaded) to a
    /// page, acting as `actor_id`. Goes through the same path as an HTTP upload: the configured
    /// size limit, file-name validation, content sniffing (the stored `mime_type` is detected,
    /// never declared), blob dedup, and the page access check. Records `page.file_added`.
    pub async fn create_from_bytes(
        &self,
        workspace_id: Id,
        page_id: Id,
        actor_id: Id,
        file_name: &str,
        bytes: &[u8],
        request_id: &str,
    ) -> Result<PageFile, PageFileError> {
        self.authorize_upload(workspace_id, page_id, actor_id)
            .await?;
        let staged = self
            .uploads
            .stage(workspace_id, actor_id, file_name, bytes)
            .await?;
        match self
            .finalize(workspace_id, page_id, actor_id, &staged, request_id)
            .await
        {
            Ok(file) => Ok(file),
            Err(error) => {
                let _ = self.uploads.discard(&staged).await;
                Err(error)
            }
        }
    }

    /// Opens a page file for `actor_id` with the shared download safety metadata.
    pub async fn download(
        &self,
        workspace_id: Id,
        page_id: Id,
        file_id: Id,
        actor_id: Id,
    ) -> Result<BlobDownload, PageFileError> {
        let database = self.database.clone();
        self.uploads
            .download(move || async move {
                authorized_file(&database, workspace_id, page_id, file_id, actor_id)
                    .await
                    .map_err(upload_error)
            })
            .await
            .map_err(|error| match error {
                UploadError::Unauthorized => PageFileError::NotFound,
                error => PageFileError::Upload(error),
            })
    }
}

/// Reading and changing page files.
impl PageFileRepository {
    /// The files of a page, oldest first.
    pub async fn list(
        &self,
        workspace_id: Id,
        page_id: Id,
        actor_id: Id,
    ) -> Result<Vec<PageFile>, PageFileError> {
        let mut connection = self.database.pool().acquire().await?;
        require_live_page(&mut connection, workspace_id, page_id, actor_id).await?;
        let rows = sqlx::query(
            "SELECT page_files.*, pages.locked_at AS locked_at FROM page_files \
             JOIN pages ON pages.id = page_files.page_id \
             WHERE page_files.page_id = $1 AND page_files.workspace_id = $2 \
             ORDER BY page_files.created_at, page_files.id",
        )
        .bind(page_id.to_string())
        .bind(workspace_id.to_string())
        .fetch_all(&mut *connection)
        .await?;
        rows.iter()
            .map(|row| file_from_row(row, workspace_id))
            .collect()
    }

    /// The current state of a file the actor can see.
    pub async fn record(
        &self,
        workspace_id: Id,
        page_id: Id,
        file_id: Id,
        actor_id: Id,
    ) -> Result<PageFileRecord, PageFileError> {
        let mut connection = self.database.pool().acquire().await?;
        record_on(&mut connection, workspace_id, page_id, file_id, actor_id).await
    }

    /// Replaces the bytes of an editable file, as `actor_id`. Records `page.file_saved`. Bytes the
    /// file already has make no new version.
    #[allow(clippy::too_many_arguments)]
    pub async fn save(
        &self,
        workspace_id: Id,
        page_id: Id,
        file_id: Id,
        actor_id: Id,
        expected_version: u64,
        bytes: &[u8],
        request_id: &str,
    ) -> Result<PageFile, PageFileError> {
        let record = self
            .record(workspace_id, page_id, file_id, actor_id)
            .await?;
        check_change(&record, expected_version)?;
        if !record.file.kind.editable() {
            return Err(PageFileError::NotEditable);
        }
        if bytes.len() as u64 > MAX_EDIT_BYTES {
            return Err(PageFileError::TooLargeToEdit);
        }
        let staged = self
            .uploads
            .stage(workspace_id, actor_id, &record.file.file_name, bytes)
            .await?;
        let saved = self
            .save_staged(
                workspace_id,
                page_id,
                file_id,
                actor_id,
                expected_version,
                &staged,
                request_id,
            )
            .await;
        if saved.is_err() {
            let _ = self.uploads.discard(&staged).await;
        }
        saved
    }

    #[allow(clippy::too_many_arguments)]
    async fn save_staged(
        &self,
        workspace_id: Id,
        page_id: Id,
        file_id: Id,
        actor_id: Id,
        expected_version: u64,
        staged: &StagedUpload,
        request_id: &str,
    ) -> Result<PageFile, PageFileError> {
        let finalization = self.uploads.begin_finalization().await;
        let now = self.database.database_now().await?;
        let mut tx = self.database.immediate_transaction().await?;
        // Access, the lock and the version again, in the transaction that serializes writers.
        let record = record_on(&mut tx, workspace_id, page_id, file_id, actor_id).await?;
        check_change(&record, expected_version)?;
        let blob = finalization
            .finalize_blob_in_transaction(&mut tx, staged, now)
            .await?;
        if blob.id.to_string() == record.blob_id {
            tx.commit().await?;
            return Ok(record.file);
        }
        let size = i64::try_from(staged.size_bytes).map_err(|_| UploadError::SizeOverflow)?;
        let file = replace_current(
            &mut tx,
            &record,
            actor_id,
            &blob.id.to_string(),
            // The file stays what it was, whatever the new text looks like: text that starts like a
            // page or an image is still text to edit, and it still only downloads.
            &record.file.mime_type,
            size,
            now,
        )
        .await?;
        audit::record(
            &mut tx,
            workspace_id,
            Some(actor_id),
            "page.file_saved",
            AuditOutcome::Success,
            "page",
            Some(page_id),
            request_id,
            json!({"file_id": file_id, "version": file.version}),
            now,
        )
        .await?;
        tx.commit().await?;
        Ok(file)
    }

    /// The versions of a file, newest first (at most 200).
    pub async fn versions(
        &self,
        workspace_id: Id,
        page_id: Id,
        file_id: Id,
        actor_id: Id,
    ) -> Result<Vec<PageFileVersion>, PageFileError> {
        let record = self
            .record(workspace_id, page_id, file_id, actor_id)
            .await?;
        let rows = sqlx::query(
            "SELECT version, size_bytes, created_by, created_at, \
             (SELECT display_name FROM users WHERE users.id = created_by) AS created_by_name \
             FROM page_file_versions WHERE page_file_id = $1 AND workspace_id = $2 \
             ORDER BY version DESC LIMIT 199",
        )
        .bind(file_id.to_string())
        .bind(workspace_id.to_string())
        .fetch_all(self.database.pool())
        .await?;
        let current = sqlx::query(
            "SELECT COALESCE(updated_by, uploaded_by) AS created_by, \
             COALESCE(updated_at, created_at) AS created_at, \
             (SELECT display_name FROM users WHERE users.id = COALESCE(updated_by, uploaded_by)) AS created_by_name \
             FROM page_files WHERE id = $1",
        )
        .bind(file_id.to_string())
        .fetch_one(self.database.pool())
        .await?;
        let mut versions = vec![PageFileVersion {
            version: record.file.version,
            current: true,
            size_bytes: record.file.size_bytes,
            created_by: optional_id(current.try_get("created_by")?),
            created_by_name: current.try_get("created_by_name")?,
            created_at: TimestampMillis::from_millis(current.try_get("created_at")?),
        }];
        for row in rows {
            versions.push(PageFileVersion {
                version: u64::try_from(row.try_get::<i64, _>("version")?).unwrap_or(0),
                current: false,
                size_bytes: u64::try_from(row.try_get::<i64, _>("size_bytes")?).unwrap_or(0),
                created_by: optional_id(row.try_get("created_by")?),
                created_by_name: row.try_get("created_by_name")?,
                created_at: TimestampMillis::from_millis(row.try_get("created_at")?),
            });
        }
        Ok(versions)
    }

    /// Opens an old version for `actor_id`, with the shared download safety metadata.
    pub async fn download_version(
        &self,
        workspace_id: Id,
        page_id: Id,
        file_id: Id,
        version: u64,
        actor_id: Id,
    ) -> Result<BlobDownload, PageFileError> {
        let this = self.clone();
        self.uploads
            .download(move || async move {
                let record = this
                    .record(workspace_id, page_id, file_id, actor_id)
                    .await
                    .map_err(upload_error)?;
                if record.file.version == version {
                    return Ok(AuthorizedAttachment {
                        storage_key: record.storage_key,
                        media_type: record.file.mime_type,
                        display_name: record.file.file_name,
                        deleted: record.deleted,
                    });
                }
                let row = sqlx::query(
                    "SELECT attachment_blobs.storage_key AS storage_key, \
                     attachment_blobs.deleted_at AS deleted_at, \
                     page_file_versions.mime_type AS mime_type, page_file_versions.file_name AS file_name \
                     FROM page_file_versions JOIN attachment_blobs ON attachment_blobs.id = page_file_versions.blob_id \
                     WHERE page_file_versions.page_file_id = $1 AND page_file_versions.version = $2 \
                     AND page_file_versions.workspace_id = $3",
                )
                .bind(file_id.to_string())
                .bind(i64::try_from(version).unwrap_or(i64::MAX))
                .bind(workspace_id.to_string())
                .fetch_optional(this.database.pool())
                .await
                .map_err(UploadError::Database)?
                .ok_or(UploadError::Unauthorized)?;
                Ok(AuthorizedAttachment {
                    storage_key: row.try_get("storage_key").map_err(UploadError::Database)?,
                    media_type: row.try_get("mime_type").map_err(UploadError::Database)?,
                    display_name: row.try_get("file_name").map_err(UploadError::Database)?,
                    deleted: row
                        .try_get::<Option<i64>, _>("deleted_at")
                        .map_err(UploadError::Database)?
                        .is_some(),
                })
            })
            .await
            .map_err(|error| match error {
                UploadError::Unauthorized => PageFileError::NotFound,
                error => PageFileError::Upload(error),
            })
    }

    /// Puts an old version back as a new version, so the replaced one stays restorable. Records
    /// `page.file_restored`.
    pub async fn restore(
        &self,
        workspace_id: Id,
        page_id: Id,
        file_id: Id,
        version: u64,
        actor_id: Id,
        expected_version: u64,
        request_id: &str,
    ) -> Result<PageFile, PageFileError> {
        let now = self.database.database_now().await?;
        let mut tx = self.database.immediate_transaction().await?;
        let record = record_on(&mut tx, workspace_id, page_id, file_id, actor_id).await?;
        check_change(&record, expected_version)?;
        let row = sqlx::query(
            "SELECT page_file_versions.blob_id AS blob_id, page_file_versions.mime_type AS mime_type, \
             page_file_versions.size_bytes AS size_bytes, attachment_blobs.deleted_at AS deleted_at \
             FROM page_file_versions JOIN attachment_blobs ON attachment_blobs.id = page_file_versions.blob_id \
             WHERE page_file_versions.page_file_id = $1 AND page_file_versions.version = $2 \
             AND page_file_versions.workspace_id = $3",
        )
        .bind(file_id.to_string())
        .bind(i64::try_from(version).unwrap_or(i64::MAX))
        .bind(workspace_id.to_string())
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(PageFileError::NotFound)?;
        if row.try_get::<Option<i64>, _>("deleted_at")?.is_some() {
            return Err(PageFileError::Deleted);
        }
        let mime_type: String = row.try_get("mime_type")?;
        let file = replace_current(
            &mut tx,
            &record,
            actor_id,
            &row.try_get::<String, _>("blob_id")?,
            &mime_type,
            row.try_get("size_bytes")?,
            now,
        )
        .await?;
        audit::record(
            &mut tx,
            workspace_id,
            Some(actor_id),
            "page.file_restored",
            AuditOutcome::Success,
            "page",
            Some(page_id),
            request_id,
            json!({"file_id": file_id, "version": file.version, "restored": version}),
            now,
        )
        .await?;
        tx.commit().await?;
        Ok(file)
    }
}

fn upload_error(error: PageFileError) -> UploadError {
    match error {
        PageFileError::Database(error) => UploadError::Database(error),
        PageFileError::Upload(error) => error,
        _ => UploadError::Unauthorized,
    }
}

fn optional_id(value: Option<String>) -> Option<Id> {
    value.and_then(|id| id.parse().ok())
}

/// What every change of a file needs, in this order: the page open for edits, the right version,
/// and stored bytes that still exist.
fn check_change(record: &PageFileRecord, expected_version: u64) -> Result<(), PageFileError> {
    if record.locked {
        return Err(PageFileError::Locked);
    }
    if !record.file.kind.editable() {
        return Err(PageFileError::NotEditable);
    }
    if record.file.version != expected_version {
        return Err(PageFileError::VersionConflict {
            current: Box::new(record.file.clone()),
        });
    }
    if record.deleted {
        return Err(PageFileError::Deleted);
    }
    Ok(())
}

/// Moves the current state of the file to its versions and makes `blob_id` current.
#[allow(clippy::too_many_arguments)]
async fn replace_current(
    tx: &mut Transaction<'_, Any>,
    record: &PageFileRecord,
    actor_id: Id,
    blob_id: &str,
    mime_type: &str,
    size_bytes: i64,
    now: TimestampMillis,
) -> Result<PageFile, PageFileError> {
    let file = &record.file;
    let id = file.id.to_string();
    sqlx::query(
        "INSERT INTO page_file_versions (id, workspace_id, page_file_id, version, blob_id, file_name, \
         mime_type, size_bytes, created_by, created_at) \
         SELECT $1, workspace_id, id, version, blob_id, file_name, mime_type, size_bytes, \
         COALESCE(updated_by, uploaded_by), COALESCE(updated_at, created_at) \
         FROM page_files WHERE id = $2",
    )
    .bind(Id::new_v7().to_string())
    .bind(&id)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "UPDATE page_files SET blob_id = $1, mime_type = $2, size_bytes = $3, version = version + 1, \
         updated_by = $4, updated_at = $5 WHERE id = $6",
    )
    .bind(blob_id)
    .bind(mime_type)
    .bind(size_bytes)
    .bind(actor_id.to_string())
    .bind(now.as_millis())
    .bind(&id)
    .execute(&mut **tx)
    .await?;
    Ok(PageFile {
        mime_type: mime_type.to_owned(),
        size_bytes: u64::try_from(size_bytes).unwrap_or(0),
        version: file.version + 1,
        ..file.clone()
    })
}

fn file_from_row(row: &sqlx::any::AnyRow, workspace_id: Id) -> Result<PageFile, PageFileError> {
    let corrupt = || PageFileError::Database(sqlx::Error::Protocol("invalid page file id".into()));
    let id: Id = row
        .try_get::<String, _>("id")?
        .parse()
        .map_err(|_| corrupt())?;
    let page_id: Id = row
        .try_get::<String, _>("page_id")?
        .parse()
        .map_err(|_| corrupt())?;
    let file_name: String = row.try_get("file_name")?;
    let mime_type: String = row.try_get("mime_type")?;
    let kind = file_parse::classify(&file_name, &mime_type);
    let locked = row.try_get::<Option<i64>, _>("locked_at")?.is_some();
    Ok(PageFile {
        id,
        page_id,
        url: page_file_url(workspace_id, page_id, id),
        file_name,
        mime_type,
        size_bytes: u64::try_from(row.try_get::<i64, _>("size_bytes")?).unwrap_or(0),
        created_at: TimestampMillis::from_millis(row.try_get("created_at")?),
        version: u64::try_from(row.try_get::<i64, _>("version")?).unwrap_or(1),
        kind,
        can_edit: kind.editable() && !locked,
    })
}

async fn record_on(
    connection: &mut sqlx::AnyConnection,
    workspace_id: Id,
    page_id: Id,
    file_id: Id,
    actor_id: Id,
) -> Result<PageFileRecord, PageFileError> {
    let row = sqlx::query(sqlx::AssertSqlSafe(format!(
        "SELECT page_files.*, pages.locked_at AS locked_at, \
         attachment_blobs.storage_key AS storage_key, attachment_blobs.deleted_at AS deleted_at \
         FROM page_files \
         JOIN attachment_blobs ON attachment_blobs.id = page_files.blob_id \
         AND attachment_blobs.workspace_id = page_files.workspace_id \
         JOIN pages ON pages.id = page_files.page_id AND pages.workspace_id = page_files.workspace_id \
         JOIN workspaces ON workspaces.id = page_files.workspace_id \
         JOIN memberships ON memberships.workspace_id = page_files.workspace_id AND memberships.user_id = $1 \
         WHERE page_files.id = $2 AND page_files.page_id = $3 AND page_files.workspace_id = $4 \
         AND pages.deleted_at IS NULL AND workspaces.deleted_at IS NULL AND {visible}",
        visible = visible(5)
    )))
    .bind(actor_id.to_string())
    .bind(file_id.to_string())
    .bind(page_id.to_string())
    .bind(workspace_id.to_string())
    .bind(actor_id.to_string())
    .fetch_optional(connection)
    .await?
    .ok_or(PageFileError::NotFound)?;
    Ok(PageFileRecord {
        file: file_from_row(&row, workspace_id)?,
        storage_key: row.try_get("storage_key")?,
        deleted: row.try_get::<Option<i64>, _>("deleted_at")?.is_some(),
        blob_id: row.try_get("blob_id")?,
        locked: row.try_get::<Option<i64>, _>("locked_at")?.is_some(),
    })
}

/// True when the page has a file with this id (used to validate uploaded covers).
pub(super) async fn page_has_file(
    tx: &mut Transaction<'_, Any>,
    workspace_id: Id,
    page_id: Id,
    file_id: Id,
) -> Result<bool, sqlx::Error> {
    let Flag(found) = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM page_files WHERE id = $1 AND page_id = $2 AND workspace_id = $3)",
    )
    .bind(file_id.to_string())
    .bind(page_id.to_string())
    .bind(workspace_id.to_string())
    .fetch_one(&mut **tx)
    .await?;
    Ok(found)
}

async fn require_live_page(
    connection: &mut sqlx::AnyConnection,
    workspace_id: Id,
    page_id: Id,
    actor_id: Id,
) -> Result<(), PageFileError> {
    let Flag(allowed) = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT EXISTS(SELECT 1 FROM pages \
         JOIN workspaces ON workspaces.id = pages.workspace_id \
         JOIN memberships ON memberships.workspace_id = pages.workspace_id AND memberships.user_id = $1 \
         WHERE pages.id = $2 AND pages.workspace_id = $3 AND pages.deleted_at IS NULL \
         AND workspaces.deleted_at IS NULL AND {visible})",
        visible = visible(4)
    )))
    .bind(actor_id.to_string())
    .bind(page_id.to_string())
    .bind(workspace_id.to_string())
    .bind(actor_id.to_string())
    .fetch_one(connection)
    .await?;
    if allowed {
        Ok(())
    } else {
        Err(PageFileError::NotFound)
    }
}

async fn authorized_file(
    database: &Database,
    workspace_id: Id,
    page_id: Id,
    file_id: Id,
    actor_id: Id,
) -> Result<AuthorizedAttachment, PageFileError> {
    let row = sqlx::query(sqlx::AssertSqlSafe(format!(
        "SELECT attachment_blobs.storage_key AS storage_key, attachment_blobs.deleted_at AS deleted_at, \
         page_files.mime_type AS mime_type, page_files.file_name AS file_name FROM page_files \
         JOIN attachment_blobs ON attachment_blobs.id = page_files.blob_id \
         AND attachment_blobs.workspace_id = page_files.workspace_id \
         JOIN pages ON pages.id = page_files.page_id AND pages.workspace_id = page_files.workspace_id \
         JOIN workspaces ON workspaces.id = page_files.workspace_id \
         JOIN memberships ON memberships.workspace_id = page_files.workspace_id AND memberships.user_id = $1 \
         WHERE page_files.id = $2 AND page_files.page_id = $3 AND page_files.workspace_id = $4 \
         AND pages.deleted_at IS NULL AND workspaces.deleted_at IS NULL AND {visible}",
        visible = visible(5)
    )))
    .bind(actor_id.to_string())
    .bind(file_id.to_string())
    .bind(page_id.to_string())
    .bind(workspace_id.to_string())
    .bind(actor_id.to_string())
    .fetch_optional(database.pool())
    .await?
    .ok_or(PageFileError::NotFound)?;
    Ok(AuthorizedAttachment {
        storage_key: row.try_get("storage_key")?,
        media_type: row.try_get("mime_type")?,
        display_name: row.try_get("file_name")?,
        deleted: row.try_get::<Option<i64>, _>("deleted_at")?.is_some(),
    })
}

#[cfg(test)]
mod tests {
    use orbit_platform::Id;

    use super::{page_file_url, parse_page_file_url};

    #[test]
    fn page_file_urls_round_trip_strictly() {
        let [workspace, page, file]: [Id; 3] = std::array::from_fn(|_| Id::new_v7());
        let url = page_file_url(workspace, page, file);
        assert_eq!(parse_page_file_url(&url), Some((workspace, page, file)));
        for bad in [
            format!("{url}?x=1"),
            format!("{url}/"),
            format!("{url}#a"),
            url.to_uppercase(),
            format!("https://example.com{url}"),
            format!("/api/v1/workspaces/{workspace}/tasks/{page}/files/{file}"),
            format!("/api/v1/workspaces/{workspace}/pages/{page}/files/not-an-id"),
            format!("//api/v1/workspaces/{workspace}/pages/{page}/files/{file}"),
        ] {
            assert_eq!(parse_page_file_url(&bad), None, "{bad}");
        }
    }
}
