//! File previews (docs/file-previews.md): a playable MP4 (H.264/AAC, `+faststart`) and a poster of a video, a
//! PDF of an office file. A `file_previews` row belongs to one stored file version: a blob of the attachment store
//! (chat files) or a Drive blob, so every new version gets its own preview and the cache never goes stale.
//!
//! An upload queues its preview in its own transaction ([`queue_in`]); the `file.preview` job makes it, one at a
//! time on all servers, at low priority (`nice`, two ffmpeg threads). The outputs go where the source is: new
//! blobs of the attachment store for a chat file, objects next to the file in the Drive bucket for a Drive file.
//! A preview that cannot be made is `failed` (the converter refused the file) or `skipped` (larger than this
//! server converts, or the converter is not installed): clients then show the original or a download.

pub mod convert;
pub mod settings;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use orbit_platform::{
    AuthorizedAttachment, Database, Flag, Id, Job, JobError, JobKind, JobKindRegistrationError,
    JobPriority, JobStore, Row, TimestampMillis, UploadError, UploadService, Worker,
    detect_media_type,
};
use serde::Serialize;
use serde_json::{Value, json};
use sqlx::{Any, Transaction};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;
use utoipa::ToSchema;

use crate::drive::DriveService;
use convert::ToolError;
pub use settings::PreviewSettings;

pub const PREVIEW_JOB: &str = "file.preview";
pub const BACKFILL_JOB: &str = "file.preview.backfill";
/// How long a Drive file's presigned URL stays valid for ffmpeg.
const SOURCE_URL_LIFETIME: Duration = Duration::from_secs(2 * 60 * 60);
/// The bytes that the backfill reads to know a chat file's type.
const SNIFF_BYTES: u64 = 8 * 1024;
/// Chat files stored as `application/octet-stream` before Orbit knew videos and office files by their bytes.
const BACKFILL_EXTENSIONS: [&str; 20] = [
    "mov", "qt", "mp4", "m4v", "webm", "mkv", "avi", "3gp", "xlsx", "xlsm", "xls", "csv", "tsv",
    "docx", "doc", "pptx", "ppt", "odt", "ods", "odp",
];
const VIDEO_EXTENSIONS: [&str; 8] = ["mov", "qt", "mp4", "m4v", "webm", "mkv", "avi", "3gp"];
const DOCUMENT_EXTENSIONS: [&str; 13] = [
    "xlsx", "xlsm", "xls", "csv", "tsv", "docx", "doc", "pptx", "ppt", "odt", "ods", "odp", "rtf",
];

/// One preview at a time on all servers, at low priority; two retries for storage outages.
#[must_use]
pub fn job_kind() -> JobKind {
    JobKind::new(PREVIEW_JOB)
        .with_concurrency_limit(1)
        .with_retry_policy(
            3,
            vec![Duration::from_secs(60), Duration::from_secs(10 * 60)],
        )
}

fn backfill_kind() -> JobKind {
    JobKind::new(BACKFILL_JOB)
        .with_concurrency_limit(1)
        .with_retry_policy(3, vec![Duration::from_secs(5 * 60)])
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PreviewKind {
    /// A playable MP4 and a poster.
    Video,
    /// A PDF of an office file (LibreOffice).
    Document,
}

impl PreviewKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Video => "video",
            Self::Document => "document",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PreviewStatus {
    /// No preview was asked for yet (a file from before previews): `POST …/preview` queues one.
    None,
    /// Queued or being made.
    Pending,
    Ready,
    /// The converter could not read the file.
    Failed,
    /// Too large for this server, or the converter is not installed.
    Skipped,
}

impl PreviewStatus {
    fn parse(value: &str) -> Self {
        match value {
            "pending" => Self::Pending,
            "ready" => Self::Ready,
            "failed" => Self::Failed,
            "skipped" => Self::Skipped,
            _ => Self::None,
        }
    }
}

/// The preview of a file. Its URLs are same-origin paths; they are set when that file exists.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, ToSchema)]
pub struct FilePreview {
    pub kind: PreviewKind,
    pub status: PreviewStatus,
    /// An MP4 (H.264/AAC) that every browser plays; seekable (HTTP Range).
    #[schema(required = true)]
    pub play_url: Option<String>,
    /// A JPEG frame of a video.
    #[schema(required = true)]
    pub poster_url: Option<String>,
    /// A PDF of an office file.
    #[schema(required = true)]
    pub pdf_url: Option<String>,
}

/// The kind of preview a file gets, from its type and name: videos, and office files and CSV.
#[must_use]
pub fn preview_kind(mime: &str, name: &str) -> Option<PreviewKind> {
    let extension = extension_of(name);
    if mime.starts_with("video/") {
        return Some(PreviewKind::Video);
    }
    let office = mime.starts_with("application/vnd.openxmlformats-officedocument.")
        || mime.starts_with("application/vnd.oasis.opendocument.")
        || matches!(
            mime,
            "application/vnd.ms-excel"
                | "application/msword"
                | "application/vnd.ms-powerpoint"
                | "text/csv"
                | "text/tab-separated-values"
                | "application/rtf"
        );
    // Drive keeps the browser's type, which is empty (octet-stream) or vague for many files.
    let unknown = matches!(mime, "application/octet-stream" | "" | "text/plain");
    if office || (unknown && DOCUMENT_EXTENSIONS.contains(&extension.as_str())) {
        return Some(PreviewKind::Document);
    }
    if unknown && VIDEO_EXTENSIONS.contains(&extension.as_str()) {
        return Some(PreviewKind::Video);
    }
    None
}

fn extension_of(name: &str) -> String {
    name.rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .unwrap_or_default()
}

/// The file extension LibreOffice and ffmpeg read a source as.
fn input_extension(kind: PreviewKind, mime: &str, name: &str) -> &'static str {
    let extension = extension_of(name);
    let known = VIDEO_EXTENSIONS
        .iter()
        .chain(DOCUMENT_EXTENSIONS.iter())
        .find(|candidate| **candidate == extension);
    if let Some(known) = known {
        return known;
    }
    match (kind, mime) {
        (_, "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet") => "xlsx",
        (_, "application/vnd.openxmlformats-officedocument.wordprocessingml.document") => "docx",
        (_, "application/vnd.openxmlformats-officedocument.presentationml.presentation") => "pptx",
        (_, "application/vnd.oasis.opendocument.spreadsheet") => "ods",
        (_, "application/vnd.oasis.opendocument.text") => "odt",
        (_, "application/vnd.oasis.opendocument.presentation") => "odp",
        (_, "application/vnd.ms-excel") => "xls",
        (_, "application/msword") => "doc",
        (_, "application/vnd.ms-powerpoint") => "ppt",
        (_, "text/csv") => "csv",
        (_, "text/tab-separated-values") => "tsv",
        (PreviewKind::Video, "video/quicktime") => "mov",
        (PreviewKind::Video, "video/webm") => "webm",
        (PreviewKind::Video, "video/x-matroska") => "mkv",
        (PreviewKind::Video, _) => "mp4",
        (PreviewKind::Document, _) => "bin",
    }
}

/// The stored file a preview is made of.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Source {
    /// A blob of the attachment store (`attachment_blobs`).
    Stored(Id),
    /// A Drive blob (`drive_blobs`).
    Drive(Id),
}

impl Source {
    const fn column(self) -> &'static str {
        match self {
            Self::Stored(_) => "source_blob_id",
            Self::Drive(_) => "drive_blob_id",
        }
    }

    const fn id(self) -> Id {
        match self {
            Self::Stored(id) | Self::Drive(id) => id,
        }
    }
}

/// Records a pending preview of `source` and queues its job, in the caller's transaction, unless the source has
/// one already (a forward, the same bytes uploaded again).
pub async fn queue_in(
    transaction: &mut Transaction<'_, Any>,
    workspace_id: Id,
    source: Source,
    kind: PreviewKind,
    now: TimestampMillis,
) -> Result<(), sqlx::Error> {
    let id = Id::new_v7();
    let column = source.column();
    let inserted = sqlx::query(sqlx::AssertSqlSafe(format!(
        "INSERT INTO file_previews (id, workspace_id, {column}, kind, status, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, 'pending', $5, $5) ON CONFLICT ({column}, kind) DO NOTHING"
    )))
    .bind(id.to_string())
    .bind(workspace_id.to_string())
    .bind(source.id().to_string())
    .bind(kind.as_str())
    .bind(now.as_millis())
    .execute(&mut **transaction)
    .await?
    .rows_affected();
    if inserted == 1 {
        let job = Job::new(job_kind(), json!({ "preview_id": id.to_string() }), now)
            .with_workspace(workspace_id.to_string())
            .with_priority(JobPriority::Low);
        JobStore::enqueue_in(transaction, &job)
            .await
            .map_err(|error| sqlx::Error::Protocol(error.to_string()))?;
    }
    Ok(())
}

/// The columns of a preview joined as `pv` (`LEFT JOIN file_previews pv ON …`), for [`preview_from_row`].
pub const PREVIEW_COLUMNS: &str = "pv.status AS preview_status, \
     EXISTS (SELECT 1 FROM file_preview_outputs po WHERE po.preview_id = pv.id AND po.role = 'play') AS preview_play, \
     EXISTS (SELECT 1 FROM file_preview_outputs po WHERE po.preview_id = pv.id AND po.role = 'poster') AS preview_poster, \
     EXISTS (SELECT 1 FROM file_preview_outputs po WHERE po.preview_id = pv.id AND po.role = 'pdf') AS preview_pdf";

/// The preview of a file of type `mime` named `name`, from the [`PREVIEW_COLUMNS`] of its row; `None` for a file
/// that gets no preview. `base` is the path that the output roles are appended to (`{base}/play`).
pub fn preview_from_row(
    row: &sqlx::any::AnyRow,
    mime: &str,
    name: &str,
    base: &str,
) -> Result<Option<FilePreview>, sqlx::Error> {
    let Some(kind) = preview_kind(mime, name) else {
        return Ok(None);
    };
    let status = row
        .try_get::<Option<String>, _>("preview_status")?
        .map_or(PreviewStatus::None, |status| PreviewStatus::parse(&status));
    let url = |column: &str, role: &str| -> Result<Option<String>, sqlx::Error> {
        Ok(row
            .try_get::<Flag, _>(column)?
            .0
            .then(|| format!("{base}/{role}")))
    };
    Ok(Some(FilePreview {
        kind,
        status,
        play_url: url("preview_play", "play")?,
        poster_url: url("preview_poster", "poster")?,
        pdf_url: url("preview_pdf", "pdf")?,
    }))
}

/// A file a preview made.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PreviewOutput {
    /// In the attachment store.
    Stored {
        storage_key: String,
        mime_type: String,
        size_bytes: i64,
    },
    /// In the Drive bucket.
    Object { key: String, mime_type: String },
}

/// The output `role` (`play`, `poster`, `pdf`) of the preview of `source`.
pub async fn output(
    connection: &mut sqlx::AnyConnection,
    source: Source,
    role: &str,
) -> Result<Option<PreviewOutput>, sqlx::Error> {
    let row = sqlx::query(sqlx::AssertSqlSafe(format!(
        "SELECT o.blob_id, o.object_key, o.mime_type, o.size_bytes, b.storage_key FROM file_previews p \
         JOIN file_preview_outputs o ON o.preview_id = p.id \
         LEFT JOIN attachment_blobs b ON b.id = o.blob_id AND b.deleted_at IS NULL \
         WHERE p.{} = $1 AND o.role = $2",
        source.column()
    )))
    .bind(source.id().to_string())
    .bind(role)
    .fetch_optional(&mut *connection)
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let mime_type: String = row.try_get("mime_type")?;
    if let Some(key) = row.try_get::<Option<String>, _>("object_key")? {
        return Ok(Some(PreviewOutput::Object { key, mime_type }));
    }
    row.try_get::<Option<String>, _>("storage_key")?
        .map(|storage_key| -> Result<_, sqlx::Error> {
            Ok(PreviewOutput::Stored {
                storage_key,
                mime_type,
                size_bytes: row.try_get("size_bytes")?,
            })
        })
        .transpose()
}

#[derive(Debug, thiserror::Error)]
pub enum PreviewError {
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    Upload(#[from] UploadError),
    #[error(transparent)]
    Drive(#[from] crate::repositories::drive::DriveError),
    #[error(transparent)]
    Storage(#[from] orbit_platform::S3Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("the job stopped")]
    Cancelled,
}

/// The end of one preview job.
#[derive(Clone, Debug, Eq, PartialEq)]
struct Outcome {
    status: &'static str,
    detail: Option<String>,
}

impl Outcome {
    fn ready() -> Self {
        Self {
            status: "ready",
            detail: None,
        }
    }

    fn failed(detail: impl Into<String>) -> Self {
        Self {
            status: "failed",
            detail: Some(detail.into()),
        }
    }

    fn skipped(detail: impl Into<String>) -> Self {
        Self {
            status: "skipped",
            detail: Some(detail.into()),
        }
    }

    /// A converter error: a missing tool skips the preview, a stopped job tries again later, anything else
    /// fails it.
    fn of(error: ToolError) -> Result<Self, PreviewError> {
        match error {
            ToolError::Missing(tool) => Ok(Self::skipped(format!("{tool} is not installed"))),
            ToolError::Cancelled => Err(PreviewError::Cancelled),
            ToolError::Io(error) => Err(error.into()),
            error => Ok(Self::failed(error.to_string())),
        }
    }
}

/// One pending preview and its source.
struct Pending {
    id: Id,
    workspace_id: Id,
    source: Source,
    kind: PreviewKind,
}

/// Where the source bytes are.
struct SourceFile {
    name: String,
    mime: String,
    size: u64,
    /// The uploader, who owns the stored outputs of a chat file.
    owner_id: Id,
    location: SourceLocation,
}

enum SourceLocation {
    Stored { storage_key: String },
    Drive { key: String },
}

/// The job's own directory, removed when the job ends.
struct WorkDirectory(PathBuf);

impl WorkDirectory {
    async fn new(id: Id) -> Result<Self, std::io::Error> {
        let path = std::env::temp_dir()
            .join("orbit-previews")
            .join(id.to_string());
        let _ = tokio::fs::remove_dir_all(&path).await;
        tokio::fs::create_dir_all(&path).await?;
        Ok(Self(path))
    }
}

impl Drop for WorkDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[derive(Clone)]
pub struct PreviewService {
    database: Database,
    uploads: UploadService,
    drive: DriveService,
    settings: Arc<PreviewSettings>,
}

impl PreviewService {
    #[must_use]
    pub fn new(
        database: Database,
        uploads: UploadService,
        drive: DriveService,
        settings: PreviewSettings,
    ) -> Self {
        Self {
            database,
            uploads,
            drive,
            settings: Arc::new(settings),
        }
    }

    /// Makes a pending preview. A preview that is gone (its source was deleted) or done already is left as it is.
    pub async fn run(
        &self,
        preview_id: Id,
        cancel: &CancellationToken,
    ) -> Result<(), PreviewError> {
        let Some(pending) = self.pending(preview_id).await? else {
            return Ok(());
        };
        let Some(source) = self.source(&pending).await? else {
            return self
                .finish(preview_id, &Outcome::failed("the file is gone"))
                .await;
        };
        let directory = WorkDirectory::new(preview_id).await?;
        let outcome = match pending.kind {
            PreviewKind::Video => self.video(&pending, &source, &directory.0, cancel).await?,
            PreviewKind::Document => {
                self.document(&pending, &source, &directory.0, cancel)
                    .await?
            }
        };
        if let Some(detail) = &outcome.detail {
            tracing::info!(%preview_id, status = outcome.status, detail, "file preview not made");
        }
        self.finish(preview_id, &outcome).await
    }

    /// Marks a preview failed after its job's last try.
    pub async fn fail(&self, preview_id: Id, detail: &str) -> Result<(), PreviewError> {
        self.finish(preview_id, &Outcome::failed(detail)).await
    }

    async fn video(
        &self,
        pending: &Pending,
        source: &SourceFile,
        directory: &Path,
        cancel: &CancellationToken,
    ) -> Result<Outcome, PreviewError> {
        let input = self
            .input(pending, source, directory, PreviewKind::Video, true)
            .await?;
        let facts = match convert::probe(&self.settings, &input, cancel).await {
            Ok(facts) => facts,
            Err(error) => return Outcome::of(error),
        };
        let mut outcome = Outcome::ready();
        let play = directory.join("play.mp4");
        let too_long = !facts.remux_video()
            && facts
                .duration_seconds
                .is_some_and(|seconds| seconds > self.settings.transcode_max_seconds as f64);
        if source.size > self.settings.transcode_max_bytes || too_long {
            outcome = Outcome::skipped("larger than this server converts");
        } else if let Err(error) =
            convert::transcode(&self.settings, &facts, &input, &play, cancel).await
        {
            outcome = Outcome::of(error)?;
        }
        let made = outcome.status == "ready";
        let poster = directory.join("poster.jpg");
        let poster_input = if made {
            play.to_string_lossy().into_owned()
        } else {
            input
        };
        let poster_made = match convert::poster(
            &self.settings,
            &poster_input,
            facts.duration_seconds,
            &poster,
            cancel,
        )
        .await
        {
            Ok(()) => true,
            Err(ToolError::Cancelled) => return Err(PreviewError::Cancelled),
            Err(_) => false,
        };
        if made
            && let Some(failure) = self
                .store(pending, source, "play", &play, "video/mp4")
                .await?
        {
            outcome = failure;
        }
        if poster_made {
            self.store(pending, source, "poster", &poster, "image/jpeg")
                .await?;
        }
        Ok(outcome)
    }

    async fn document(
        &self,
        pending: &Pending,
        source: &SourceFile,
        directory: &Path,
        cancel: &CancellationToken,
    ) -> Result<Outcome, PreviewError> {
        if source.size > self.settings.document_max_bytes {
            return Ok(Outcome::skipped("larger than this server converts"));
        }
        if let Err(error) = convert::find_program(&self.settings.soffice) {
            return Outcome::of(error);
        }
        let input = PathBuf::from(
            self.input(pending, source, directory, PreviewKind::Document, false)
                .await?,
        );
        let pdf = match convert::document_to_pdf(&self.settings, directory, &input, cancel).await {
            Ok(pdf) => pdf,
            Err(error) => return Outcome::of(error),
        };
        Ok(self
            .store(pending, source, "pdf", &pdf, "application/pdf")
            .await?
            .unwrap_or_else(Outcome::ready))
    }

    /// The source for a converter: a local copy, or (for a Drive video, with `url_ok`) a presigned URL, so a
    /// large file is read as a stream and never copied to this server.
    async fn input(
        &self,
        pending: &Pending,
        source: &SourceFile,
        directory: &Path,
        kind: PreviewKind,
        url_ok: bool,
    ) -> Result<String, PreviewError> {
        let path = directory.join(format!(
            "input.{}",
            input_extension(kind, &source.mime, &source.name)
        ));
        match &source.location {
            SourceLocation::Stored { storage_key } => {
                let mut reader = self.open_stored(storage_key).await?;
                let mut file = tokio::fs::File::create(&path).await?;
                tokio::io::copy(&mut reader, &mut file).await?;
                file.flush().await?;
            }
            SourceLocation::Drive { key } => {
                let bucket = self.drive.bucket(pending.workspace_id)?;
                if url_ok {
                    return Ok(bucket.presign_get(key, SOURCE_URL_LIFETIME, None)?);
                }
                if !bucket.get_to_file(key, &path).await? {
                    return Err(PreviewError::Io(std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        "the Drive object is gone",
                    )));
                }
            }
        }
        Ok(path.to_string_lossy().into_owned())
    }

    async fn open_stored(
        &self,
        storage_key: &str,
    ) -> Result<orbit_platform::BlobReader, UploadError> {
        let storage_key = storage_key.to_owned();
        Ok(self
            .uploads
            .download(|| async move {
                Ok(AuthorizedAttachment {
                    storage_key,
                    media_type: "application/octet-stream".to_owned(),
                    display_name: "source".to_owned(),
                    deleted: false,
                })
            })
            .await?
            .reader)
    }

    /// Stores an output where the source is. Returns the outcome when the output cannot be kept (larger than
    /// the upload limit, over the workspace's storage quota).
    async fn store(
        &self,
        pending: &Pending,
        source: &SourceFile,
        role: &str,
        path: &Path,
        mime: &str,
    ) -> Result<Option<Outcome>, PreviewError> {
        let size = i64::try_from(tokio::fs::metadata(path).await?.len()).unwrap_or(i64::MAX);
        match &source.location {
            SourceLocation::Drive { key } => {
                let extension = path
                    .extension()
                    .map(|extension| extension.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let object_key = format!("{key}.{role}.{extension}");
                self.drive
                    .bucket(pending.workspace_id)?
                    .put_file(&object_key, path)
                    .await?;
                let mut transaction = self.database.immediate_transaction().await?;
                sqlx::query(
                    "INSERT INTO file_preview_outputs (preview_id, role, object_key, mime_type, size_bytes) \
                     VALUES ($1, $2, $3, $4, $5) ON CONFLICT (preview_id, role) DO NOTHING",
                )
                .bind(pending.id.to_string())
                .bind(role)
                .bind(&object_key)
                .bind(mime)
                .bind(size)
                .execute(&mut *transaction)
                .await?;
                transaction.commit().await?;
                Ok(None)
            }
            SourceLocation::Stored { .. } => {
                let name = format!(
                    "{role}.{}",
                    path.extension().unwrap_or_default().to_string_lossy()
                );
                let file = tokio::fs::File::open(path).await?;
                let staged = match self
                    .uploads
                    .stage(pending.workspace_id, source.owner_id, &name, file)
                    .await
                {
                    Ok(staged) => staged,
                    Err(UploadError::FileTooLarge { .. } | UploadError::RequestTooLarge { .. }) => {
                        return Ok(Some(Outcome::failed(
                            "the preview is larger than the upload limit",
                        )));
                    }
                    Err(error) => return Err(error.into()),
                };
                let finalization = self.uploads.begin_finalization().await;
                let now = self.database.database_now().await?;
                let stored = async {
                    let mut transaction = self.database.immediate_transaction().await?;
                    let blob = finalization
                        .finalize_blob_in_transaction(&mut transaction, &staged, now)
                        .await?;
                    sqlx::query(
                        "INSERT INTO file_preview_outputs (preview_id, role, blob_id, mime_type, size_bytes) \
                         VALUES ($1, $2, $3, $4, $5) ON CONFLICT (preview_id, role) DO NOTHING",
                    )
                    .bind(pending.id.to_string())
                    .bind(role)
                    .bind(blob.id.to_string())
                    .bind(&staged.detected_media_type)
                    .bind(size)
                    .execute(&mut *transaction)
                    .await?;
                    transaction.commit().await?;
                    Ok::<_, UploadError>(())
                }
                .await;
                drop(finalization);
                match stored {
                    Ok(()) => Ok(None),
                    Err(error) => {
                        let _ = self.uploads.discard(&staged).await;
                        match error {
                            UploadError::StorageQuotaExceeded { .. } => {
                                Ok(Some(Outcome::failed("the workspace storage is full")))
                            }
                            error => Err(error.into()),
                        }
                    }
                }
            }
        }
    }

    async fn pending(&self, preview_id: Id) -> Result<Option<Pending>, PreviewError> {
        let row = sqlx::query(
            "SELECT workspace_id, source_blob_id, drive_blob_id, kind FROM file_previews \
             WHERE id = $1 AND status = 'pending'",
        )
        .bind(preview_id.to_string())
        .fetch_optional(self.database.pool())
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let id = |value: String| {
            value
                .parse::<Id>()
                .map_err(|error| sqlx::Error::Decode(Box::new(error)))
        };
        let source = match (
            row.try_get::<Option<String>, _>("source_blob_id")?,
            row.try_get::<Option<String>, _>("drive_blob_id")?,
        ) {
            (Some(blob), _) => Source::Stored(id(blob)?),
            (None, Some(blob)) => Source::Drive(id(blob)?),
            (None, None) => return Ok(None),
        };
        Ok(Some(Pending {
            id: preview_id,
            workspace_id: id(row.try_get("workspace_id")?)?,
            source,
            kind: if row.try_get::<String, _>("kind")? == "video" {
                PreviewKind::Video
            } else {
                PreviewKind::Document
            },
        }))
    }

    async fn source(&self, pending: &Pending) -> Result<Option<SourceFile>, PreviewError> {
        let id = |value: String| {
            value
                .parse::<Id>()
                .map_err(|error| sqlx::Error::Decode(Box::new(error)))
        };
        match pending.source {
            Source::Stored(blob_id) => {
                let row = sqlx::query(
                    "SELECT b.storage_key, b.byte_size, f.file_name, f.mime_type, f.uploaded_by \
                     FROM attachment_blobs b JOIN chat_message_files f ON f.blob_id = b.id \
                     WHERE b.id = $1 AND b.deleted_at IS NULL ORDER BY f.created_at LIMIT 1",
                )
                .bind(blob_id.to_string())
                .fetch_optional(self.database.pool())
                .await?;
                row.map(|row| -> Result<_, PreviewError> {
                    Ok(SourceFile {
                        name: row.try_get("file_name")?,
                        mime: row.try_get("mime_type")?,
                        size: u64::try_from(row.try_get::<i64, _>("byte_size")?).unwrap_or(0),
                        owner_id: id(row.try_get("uploaded_by")?)?,
                        location: SourceLocation::Stored {
                            storage_key: row.try_get("storage_key")?,
                        },
                    })
                })
                .transpose()
            }
            Source::Drive(blob_id) => {
                let row = sqlx::query(
                    "SELECT b.r2_key, b.size, b.mime, n.name, b.created_by FROM drive_blobs b \
                     JOIN drive_nodes n ON n.blob_id = b.id WHERE b.id = $1 AND b.status = 'ready' LIMIT 1",
                )
                .bind(blob_id.to_string())
                .fetch_optional(self.database.pool())
                .await?;
                row.map(|row| -> Result<_, PreviewError> {
                    Ok(SourceFile {
                        name: row.try_get("name")?,
                        mime: row.try_get("mime")?,
                        size: u64::try_from(row.try_get::<i64, _>("size")?).unwrap_or(0),
                        owner_id: row
                            .try_get::<Option<String>, _>("created_by")?
                            .map(id)
                            .transpose()?
                            .unwrap_or_else(Id::new_v7),
                        location: SourceLocation::Drive {
                            key: row.try_get("r2_key")?,
                        },
                    })
                })
                .transpose()
            }
        }
    }

    async fn finish(&self, preview_id: Id, outcome: &Outcome) -> Result<(), PreviewError> {
        let now = self.database.database_now().await?;
        let mut transaction = self.database.immediate_transaction().await?;
        sqlx::query(
            "UPDATE file_previews SET status = $1, detail = $2, updated_at = $3 WHERE id = $4 AND status = 'pending'",
        )
        .bind(outcome.status)
        .bind(outcome.detail.as_deref().map(|detail| detail.chars().take(500).collect::<String>()))
        .bind(now.as_millis())
        .bind(preview_id.to_string())
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    /// Fixes chat files that were stored as `application/octet-stream` before Orbit knew videos and office files
    /// by their bytes, and queues a preview for every chat video that has none. Runs at every start; it does
    /// nothing the second time.
    pub async fn backfill(&self) -> Result<(u64, u64), PreviewError> {
        let rows = sqlx::query(
            "SELECT f.id, f.file_name, b.storage_key FROM chat_message_files f \
             JOIN attachment_blobs b ON b.id = f.blob_id AND b.deleted_at IS NULL \
             WHERE f.mime_type = 'application/octet-stream' ORDER BY f.created_at",
        )
        .fetch_all(self.database.pool())
        .await?;
        let mut fixed = 0;
        for row in rows {
            let name: String = row.try_get("file_name")?;
            if !BACKFILL_EXTENSIONS.contains(&extension_of(&name).as_str()) {
                continue;
            }
            let storage_key: String = row.try_get("storage_key")?;
            let mut head = Vec::new();
            let read = async {
                self.open_stored(&storage_key)
                    .await?
                    .take(SNIFF_BYTES)
                    .read_to_end(&mut head)
                    .await
                    .map_err(|source| {
                        UploadError::from(orbit_platform::BlobStoreError::Io {
                            path: PathBuf::from(&storage_key),
                            source,
                        })
                    })
            };
            if let Err(error) = read.await {
                tracing::warn!(%error, "a chat file could not be read to find its type");
                continue;
            }
            let detected = detect_media_type(&head, &name);
            if detected == "application/octet-stream" {
                continue;
            }
            let mut transaction = self.database.immediate_transaction().await?;
            fixed += sqlx::query(
                "UPDATE chat_message_files SET mime_type = $1 WHERE id = $2 AND mime_type = 'application/octet-stream'",
            )
            .bind(detected)
            .bind(row.try_get::<String, _>("id")?)
            .execute(&mut *transaction)
            .await?
            .rows_affected();
            transaction.commit().await?;
        }
        let videos = sqlx::query(
            "SELECT DISTINCT f.workspace_id, f.blob_id FROM chat_message_files f \
             JOIN attachment_blobs b ON b.id = f.blob_id AND b.deleted_at IS NULL \
             WHERE f.message_id IS NOT NULL AND f.mime_type LIKE 'video/%' \
             AND NOT EXISTS (SELECT 1 FROM file_previews p WHERE p.source_blob_id = f.blob_id AND p.kind = 'video')",
        )
        .fetch_all(self.database.pool())
        .await?;
        let mut queued = 0;
        for row in videos {
            let parse = |value: String| {
                value
                    .parse::<Id>()
                    .map_err(|error| sqlx::Error::Decode(Box::new(error)))
            };
            let now = self.database.database_now().await?;
            let mut transaction = self.database.immediate_transaction().await?;
            queue_in(
                &mut transaction,
                parse(row.try_get("workspace_id")?)?,
                Source::Stored(parse(row.try_get("blob_id")?)?),
                PreviewKind::Video,
                now,
            )
            .await?;
            transaction.commit().await?;
            queued += 1;
        }
        if fixed > 0 || queued > 0 {
            tracing::info!(
                fixed,
                queued,
                "chat file types fixed and video previews queued"
            );
        }
        Ok((fixed, queued))
    }
}

/// Queues the backfill once per start, unless one waits already.
pub async fn queue_backfill(database: &Database) -> Result<(), String> {
    let waiting: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM jobs WHERE kind = $1 AND state IN ('queued', 'running')",
    )
    .bind(BACKFILL_JOB)
    .fetch_one(database.pool())
    .await
    .map_err(|error| error.to_string())?;
    if waiting > 0 {
        return Ok(());
    }
    let now = database
        .database_now()
        .await
        .map_err(|error| error.to_string())?;
    JobStore::new(database.clone())
        .enqueue(&Job::new(backfill_kind(), json!({}), now).with_priority(JobPriority::Low))
        .await
        .map(drop)
        .map_err(|error| error.to_string())
}

fn payload_preview_id(payload: &Value) -> Option<Id> {
    payload.get("preview_id")?.as_str()?.parse().ok()
}

pub fn register_jobs(
    worker: Worker,
    service: PreviewService,
) -> Result<Worker, JobKindRegistrationError> {
    let backfill = service.clone();
    worker
        .with_handler(job_kind(), move |context| {
            let service = service.clone();
            async move {
                let Some(preview_id) = payload_preview_id(&context.job.payload) else {
                    return Err(JobError::Permanent(
                        "invalid file.preview payload".to_owned(),
                    ));
                };
                let cancel = context.cancellation_token();
                match service.run(preview_id, &cancel).await {
                    Ok(()) => Ok(()),
                    Err(_) if cancel.is_cancelled() => {
                        Err(JobError::Retryable("file preview stopped".to_owned()))
                    }
                    Err(error) => {
                        tracing::warn!(%error, %preview_id, "file preview failed");
                        if context.claim.attempt >= context.job.max_attempts {
                            service
                                .fail(preview_id, &error.to_string())
                                .await
                                .map_err(|error| JobError::Retryable(error.to_string()))?;
                            Err(JobError::Permanent(error.to_string()))
                        } else {
                            Err(JobError::Retryable(error.to_string()))
                        }
                    }
                }
            }
        })?
        .with_handler(backfill_kind(), move |_| {
            let service = backfill.clone();
            async move {
                service
                    .backfill()
                    .await
                    .map(drop)
                    .map_err(|error| JobError::Retryable(format!("file preview backfill: {error}")))
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn videos_and_office_files_get_previews_and_other_files_do_not() {
        assert_eq!(
            preview_kind("video/quicktime", "a.MOV"),
            Some(PreviewKind::Video)
        );
        assert_eq!(
            preview_kind(
                "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
                "a.xlsx"
            ),
            Some(PreviewKind::Document)
        );
        assert_eq!(
            preview_kind("text/csv", "a.csv"),
            Some(PreviewKind::Document)
        );
        // Drive keeps the browser's type, which may be empty.
        assert_eq!(
            preview_kind("application/octet-stream", "Plan.DOCX"),
            Some(PreviewKind::Document)
        );
        assert_eq!(
            preview_kind("application/octet-stream", "clip.mov"),
            Some(PreviewKind::Video)
        );
        assert_eq!(
            preview_kind("application/vnd.ms-excel", "export.csv"),
            Some(PreviewKind::Document)
        );
        assert_eq!(preview_kind("image/png", "a.png"), None);
        assert_eq!(preview_kind("application/pdf", "a.pdf"), None);
        assert_eq!(preview_kind("application/zip", "a.xlsx"), None);
        assert_eq!(preview_kind("application/octet-stream", "a.exe"), None);
    }

    #[test]
    fn converters_read_the_source_with_the_right_extension() {
        assert_eq!(
            input_extension(PreviewKind::Document, "text/csv", "Liste"),
            "csv"
        );
        assert_eq!(
            input_extension(PreviewKind::Document, "application/octet-stream", "a.XLS"),
            "xls"
        );
        assert_eq!(
            input_extension(
                PreviewKind::Document,
                "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
                "upload"
            ),
            "xlsx"
        );
        assert_eq!(
            input_extension(PreviewKind::Video, "video/quicktime", "IMG_1"),
            "mov"
        );
    }
}
