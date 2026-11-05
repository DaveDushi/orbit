//! Orbit Drive: the bytes of Drive files go between the browser and the bucket with presigned URLs, never
//! through Orbit. Orbit signs a URL only after [`DriveRepository`] checked the caller's membership, and it
//! checks the uploaded parts itself before it completes an upload.
//!
//! Drive uses the workspace's own bucket while it is active, else the installation's bucket. Objects are
//! `drive/<workspace id>/<blob id>`: outside `blobs/` (attachments) and `backups/`, so the attachment mover and
//! cleanup never see them. Changing the bucket does not move Drive objects.

use std::sync::Arc;
use std::time::Duration;

use orbit_platform::{
    Database, Id, ObjectStorage, S3_MAX_PARTS, S3Bucket, S3Part, TimestampMillis,
};

use crate::repositories::drive::{
    BlobDeletion, DriveError, DriveMediaStatus, DriveNode, DriveNodeKind, DriveRepository,
    DriveUpload, MAX_POSTER_BYTES, MAX_THUMB_BYTES, MediaFacts, NewUpload, OnConflict,
};

const MIB: i64 = 1024 * 1024;
/// Larger files are sent in parts (R2 wants every part but the last to have one size).
pub const SINGLE_PUT_MAX_BYTES: i64 = 32 * MIB;
pub const MIN_PART_BYTES: i64 = 16 * MIB;
/// R2 takes objects up to 4.995 TiB.
pub const MAX_FILE_BYTES: i64 = 4 * 1024 * 1024 * MIB;
pub const DOWNLOAD_URL_LIFETIME: Duration = Duration::from_secs(5 * 60);
pub const UPLOAD_URL_LIFETIME: Duration = Duration::from_secs(60 * 60);
/// The most URLs signed by one request.
pub const MAX_URLS: usize = 100;
/// The largest file an agent reads (`drive_read`), and the largest text file it writes (`drive_upload_text`): an
/// MCP request body has at most 64 KiB.
pub const MAX_READ_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_TEXT_UPLOAD_BYTES: usize = 48 * 1024;
/// The most blobs deleted from the bucket by one pass.
const DELETIONS_PER_PASS: i64 = 500;

/// A new upload, as the browser asks for it.
#[derive(Clone, Debug)]
pub struct StartUpload {
    pub parent_id: Option<Id>,
    pub name: String,
    pub size: i64,
    pub mime: String,
    pub on_conflict: OnConflict,
}

/// A started upload: a single-PUT upload has its URL; a multipart upload asks for part URLs.
#[derive(Clone, Debug)]
pub struct StartedUpload {
    pub upload: DriveUpload,
    pub url: Option<String>,
}

#[derive(Clone)]
pub struct DriveService {
    repository: Arc<DriveRepository>,
    storage: ObjectStorage,
}

impl DriveService {
    #[must_use]
    pub fn new(database: Database, storage: ObjectStorage) -> Self {
        Self {
            repository: Arc::new(DriveRepository::new(database)),
            storage,
        }
    }

    #[must_use]
    pub fn repository(&self) -> &DriveRepository {
        &self.repository
    }

    /// The bucket of the workspace's Drive.
    fn bucket(&self, workspace_id: Id) -> Result<S3Bucket, DriveError> {
        let state = self.storage.current();
        match state.workspace_buckets.get(&workspace_id.to_string()) {
            Some(own) if own.active => Ok(own.bucket.clone()),
            _ => state.bucket.ok_or(DriveError::NoStorage),
        }
    }

    pub async fn start_upload(
        &self,
        workspace_id: Id,
        user_id: Id,
        request: StartUpload,
        now: TimestampMillis,
    ) -> Result<StartedUpload, DriveError> {
        self.repository
            .require_member(workspace_id, user_id)
            .await?;
        if !(0..=MAX_FILE_BYTES).contains(&request.size) {
            return Err(DriveError::TooLarge);
        }
        let bucket = self.bucket(workspace_id)?;
        let blob_id = Id::new_v7();
        let upload = self
            .repository
            .begin_upload(
                workspace_id,
                user_id,
                NewUpload {
                    blob_id,
                    r2_key: format!("drive/{workspace_id}/{blob_id}"),
                    parent_id: request.parent_id,
                    name: request.name,
                    size: request.size,
                    mime: request.mime,
                    on_conflict: request.on_conflict,
                    part_size: part_size(request.size),
                },
                now,
            )
            .await?;
        if !upload.multipart() {
            let url = bucket.presign_put(&upload.r2_key, UPLOAD_URL_LIFETIME)?;
            return Ok(StartedUpload {
                upload,
                url: Some(url),
            });
        }
        match bucket
            .create_multipart_upload(&upload.r2_key, &upload.mime)
            .await
        {
            Ok(upload_id) => {
                self.repository
                    .set_multipart_id(blob_id, &upload_id)
                    .await?;
                Ok(StartedUpload {
                    upload: DriveUpload {
                        upload_id: Some(upload_id),
                        ..upload
                    },
                    url: None,
                })
            }
            Err(error) => {
                self.repository
                    .cancel_upload(workspace_id, user_id, blob_id, now)
                    .await?;
                Err(error.into())
            }
        }
    }

    /// URLs for parts of a multipart upload (numbers from 1 to the part count, at most [`MAX_URLS`]).
    pub async fn part_urls(
        &self,
        workspace_id: Id,
        user_id: Id,
        blob_id: Id,
        part_numbers: &[u32],
    ) -> Result<Vec<(u32, String)>, DriveError> {
        let upload = self
            .repository
            .upload(workspace_id, user_id, blob_id)
            .await?;
        let upload_id = upload.upload_id.as_deref().ok_or(DriveError::InvalidPart)?;
        let count = upload.part_count();
        if part_numbers.is_empty()
            || part_numbers.len() > MAX_URLS
            || part_numbers
                .iter()
                .any(|number| *number == 0 || i64::from(*number) > count)
        {
            return Err(DriveError::InvalidPart);
        }
        let bucket = self.bucket(workspace_id)?;
        part_numbers
            .iter()
            .map(|number| {
                bucket
                    .presign_upload_part(&upload.r2_key, upload_id, *number, UPLOAD_URL_LIFETIME)
                    .map(|url| (*number, url))
                    .map_err(Into::into)
            })
            .collect()
    }

    /// The upload and the parts that the bucket has (for a single PUT: part 1 once the object is there).
    pub async fn upload_status(
        &self,
        workspace_id: Id,
        user_id: Id,
        blob_id: Id,
    ) -> Result<(DriveUpload, Vec<S3Part>), DriveError> {
        let upload = self
            .repository
            .upload(workspace_id, user_id, blob_id)
            .await?;
        let bucket = self.bucket(workspace_id)?;
        let parts = match upload.upload_id.as_deref() {
            Some(upload_id) => bucket
                .list_parts(&upload.r2_key, upload_id)
                .await?
                .unwrap_or_default(),
            None => bucket
                .size(&upload.r2_key)
                .await?
                .map(|size| S3Part {
                    part_number: 1,
                    etag: String::new(),
                    size,
                })
                .into_iter()
                .collect(),
        };
        Ok((upload, parts))
    }

    /// Checks that the bucket has every byte of the upload, joins the parts and adds the file to the drive.
    pub async fn complete_upload(
        &self,
        workspace_id: Id,
        user_id: Id,
        blob_id: Id,
        now: TimestampMillis,
    ) -> Result<DriveNode, DriveError> {
        let upload = self
            .repository
            .upload(workspace_id, user_id, blob_id)
            .await?;
        let bucket = self.bucket(workspace_id)?;
        let size = upload.size;
        match upload.upload_id.as_deref() {
            Some(upload_id) => match bucket.list_parts(&upload.r2_key, upload_id).await? {
                Some(parts) => {
                    let parts = complete_parts(&upload, &parts)?;
                    bucket
                        .complete_multipart_upload(&upload.r2_key, upload_id, &parts)
                        .await?;
                }
                // Completed before, but Orbit did not record it.
                None if object_size(&bucket, &upload).await? == Some(size) => {}
                None => return Err(DriveError::UploadIncomplete),
            },
            None if object_size(&bucket, &upload).await? == Some(size) => {}
            None => return Err(DriveError::UploadIncomplete),
        }
        self.repository
            .finish_upload(workspace_id, user_id, blob_id, now)
            .await
    }

    pub async fn cancel_upload(
        &self,
        workspace_id: Id,
        user_id: Id,
        blob_id: Id,
        now: TimestampMillis,
    ) -> Result<(), DriveError> {
        self.repository
            .cancel_upload(workspace_id, user_id, blob_id, now)
            .await?;
        self.delete_objects_later();
        Ok(())
    }

    /// Download URLs of live files (at most [`MAX_URLS`]); other ids are left out. `inline` lets the browser
    /// show the file instead of saving it.
    pub async fn download_urls(
        &self,
        workspace_id: Id,
        user_id: Id,
        node_ids: &[Id],
        inline: bool,
    ) -> Result<Vec<(Id, String)>, DriveError> {
        if node_ids.len() > MAX_URLS {
            return Err(DriveError::TooManyItems);
        }
        let objects = self
            .repository
            .objects(workspace_id, user_id, node_ids)
            .await?;
        if objects.is_empty() {
            return Ok(Vec::new());
        }
        let bucket = self.bucket(workspace_id)?;
        objects
            .iter()
            .map(|object| {
                let name = (!inline).then_some(object.name.as_str());
                bucket
                    .presign_get(&object.r2_key, DOWNLOAD_URL_LIFETIME, name)
                    .map(|url| (object.node_id, url))
                    .map_err(Into::into)
            })
            .collect()
    }

    /// Records what the browser measured of an image or video and returns PUT URLs for its thumbnail and (for a
    /// video) poster; with `failed`, records that the browser could not read the file and returns none.
    pub async fn begin_media(
        &self,
        workspace_id: Id,
        user_id: Id,
        node_id: Id,
        facts: MediaFacts,
        failed: bool,
        now: TimestampMillis,
    ) -> Result<(Option<String>, Option<String>), DriveError> {
        let target = self
            .repository
            .begin_media(workspace_id, user_id, node_id, facts, failed, now)
            .await?;
        if failed {
            return Ok((None, None));
        }
        let bucket = self.bucket(workspace_id)?;
        let sign = |key: &Option<String>| {
            key.as_deref()
                .map(|key| bucket.presign_put(key, UPLOAD_URL_LIFETIME))
                .transpose()
        };
        Ok((sign(&target.thumb_key)?, sign(&target.poster_key)?))
    }

    /// Keeps the thumbnail and poster the browser sent when they are in the bucket and small enough (an object
    /// that is too large is deleted), and returns the node with their URLs.
    pub async fn finish_media(
        &self,
        workspace_id: Id,
        user_id: Id,
        node_id: Id,
        now: TimestampMillis,
    ) -> Result<DriveNode, DriveError> {
        let target = self
            .repository
            .media_target(workspace_id, user_id, node_id)
            .await?;
        let bucket = self.bucket(workspace_id)?;
        let mut kept = Vec::with_capacity(2);
        for (key, limit) in [
            (target.thumb_key.as_deref(), MAX_THUMB_BYTES),
            (target.poster_key.as_deref(), MAX_POSTER_BYTES),
        ] {
            let Some(key) = key else {
                kept.push(None);
                continue;
            };
            match bucket.size(key).await? {
                Some(size) if size > 0 && size <= limit => kept.push(Some(key)),
                Some(_) => {
                    bucket.delete(key).await?;
                    kept.push(None);
                }
                None => kept.push(None),
            }
        }
        let mut node = self
            .repository
            .finish_media(workspace_id, user_id, node_id, kept[0], kept[1], now)
            .await?;
        self.sign_media(workspace_id, std::slice::from_mut(&mut node));
        Ok(node)
    }

    /// The bytes of a live file (at most [`MAX_READ_BYTES`]), with its name and MIME type, for an agent to read.
    /// The bytes go through the server here: only small files, never user downloads.
    pub async fn file_bytes(
        &self,
        workspace_id: Id,
        user_id: Id,
        node_id: Id,
    ) -> Result<(DriveNode, Vec<u8>), DriveError> {
        let detail = self.repository.node(workspace_id, user_id, node_id).await?;
        let node = detail.node;
        if node.kind != DriveNodeKind::File {
            return Err(DriveError::NotAFolder);
        }
        if node.size.unwrap_or(0) > i64::try_from(MAX_READ_BYTES).unwrap_or(i64::MAX) {
            return Err(DriveError::TooLarge);
        }
        let object = self
            .repository
            .objects(workspace_id, user_id, &[node_id])
            .await?
            .into_iter()
            .next()
            .ok_or(DriveError::NotFound)?;
        let bytes = self
            .bucket(workspace_id)?
            .get_bytes(&object.r2_key)
            .await?
            .ok_or(DriveError::NotFound)?;
        Ok((node, bytes))
    }

    /// Writes a small text file into a folder from the server (an agent's `drive_upload_text`).
    #[allow(clippy::too_many_arguments)]
    pub async fn put_text(
        &self,
        workspace_id: Id,
        user_id: Id,
        parent_id: Option<Id>,
        name: String,
        mime: String,
        text: String,
        on_conflict: OnConflict,
        now: TimestampMillis,
    ) -> Result<DriveNode, DriveError> {
        if text.len() > MAX_TEXT_UPLOAD_BYTES {
            return Err(DriveError::TooLarge);
        }
        self.repository
            .require_member(workspace_id, user_id)
            .await?;
        let bucket = self.bucket(workspace_id)?;
        let blob_id = Id::new_v7();
        let upload = self
            .repository
            .begin_upload(
                workspace_id,
                user_id,
                NewUpload {
                    blob_id,
                    r2_key: format!("drive/{workspace_id}/{blob_id}"),
                    parent_id,
                    name,
                    size: i64::try_from(text.len()).unwrap_or(i64::MAX),
                    mime,
                    on_conflict,
                    part_size: None,
                },
                now,
            )
            .await?;
        if let Err(error) = bucket.put_bytes(&upload.r2_key, text.into_bytes()).await {
            self.repository
                .cancel_upload(workspace_id, user_id, blob_id, now)
                .await?;
            return Err(error.into());
        }
        self.repository
            .finish_upload(workspace_id, user_id, blob_id, now)
            .await
    }

    /// A 5-minute URL of an object of the workspace's Drive; `download_name` makes the browser save it.
    pub fn object_url(
        &self,
        workspace_id: Id,
        key: &str,
        download_name: Option<&str>,
    ) -> Result<String, DriveError> {
        Ok(self
            .bucket(workspace_id)?
            .presign_get(key, DOWNLOAD_URL_LIFETIME, download_name)?)
    }

    /// Adds 5-minute URLs of ready thumbnails and posters to nodes that the caller may see. Without a bucket the
    /// nodes stay without them.
    pub fn sign_media(&self, workspace_id: Id, nodes: &mut [DriveNode]) {
        if !nodes.iter().any(|node| node.thumb_key.is_some()) {
            return;
        }
        let Ok(bucket) = self.bucket(workspace_id) else {
            return;
        };
        for node in nodes {
            if node.media.map(|media| media.status) != Some(DriveMediaStatus::Ready) {
                continue;
            }
            let sign = |key: &Option<String>| {
                key.as_deref()
                    .and_then(|key| bucket.presign_get(key, DOWNLOAD_URL_LIFETIME, None).ok())
            };
            node.thumbnail_url = sign(&node.thumb_key);
            node.poster_url = sign(&node.poster_key);
        }
    }

    pub async fn purge(
        &self,
        workspace_id: Id,
        user_id: Id,
        node_id: Id,
        request_id: &str,
        now: TimestampMillis,
    ) -> Result<(), DriveError> {
        self.repository
            .purge(workspace_id, user_id, node_id, request_id, now)
            .await?;
        self.delete_objects_later();
        Ok(())
    }

    pub async fn empty_trash(
        &self,
        workspace_id: Id,
        user_id: Id,
        request_id: &str,
        now: TimestampMillis,
    ) -> Result<u64, DriveError> {
        let deleted = self
            .repository
            .empty_trash(workspace_id, user_id, request_id, now)
            .await?;
        self.delete_objects_later();
        Ok(deleted)
    }

    /// The hourly job: deletes old trash, cancels stale uploads and deletes the objects of deleted blobs.
    pub async fn maintain(&self, now: TimestampMillis) -> Result<(), DriveError> {
        let (nodes, uploads) = self.repository.expire(now).await?;
        if nodes > 0 || uploads > 0 {
            tracing::info!(nodes, uploads, "Drive trash and stale uploads expired");
        }
        self.delete_objects().await
    }

    fn delete_objects_later(&self) {
        let service = self.clone();
        tokio::spawn(async move {
            if let Err(error) = service.delete_objects().await {
                tracing::warn!(%error, "Drive objects could not be deleted; the hourly job retries");
            }
        });
    }

    /// Deletes the objects of deleted blobs from the bucket, then their rows. A blob whose bucket fails stays
    /// for the next pass.
    pub async fn delete_objects(&self) -> Result<(), DriveError> {
        for deletion in self.repository.deletions(DELETIONS_PER_PASS).await? {
            match self.delete_blob_objects(&deletion).await {
                Ok(()) => self.repository.forget_blob(deletion.blob_id).await?,
                Err(error) => tracing::warn!(
                    %error,
                    blob_id = %deletion.blob_id,
                    "Drive object delete failed; retried later"
                ),
            }
        }
        Ok(())
    }

    async fn delete_blob_objects(&self, deletion: &BlobDeletion) -> Result<(), DriveError> {
        let bucket = self.bucket(deletion.workspace_id)?;
        if let Some(upload_id) = &deletion.upload_id {
            bucket
                .abort_multipart_upload(&deletion.r2_key, upload_id)
                .await?;
        }
        bucket.delete(&deletion.r2_key).await?;
        for key in &deletion.extra_keys {
            bucket.delete(key).await?;
        }
        Ok(())
    }
}

async fn object_size(bucket: &S3Bucket, upload: &DriveUpload) -> Result<Option<i64>, DriveError> {
    Ok(bucket
        .size(&upload.r2_key)
        .await?
        .and_then(|size| i64::try_from(size).ok()))
}

/// The part size of a file: `None` for a single PUT, else at least [`MIN_PART_BYTES`] and large enough for
/// [`S3_MAX_PARTS`] parts, in whole MiB.
#[must_use]
pub fn part_size(size: i64) -> Option<i64> {
    if size <= SINGLE_PUT_MAX_BYTES {
        return None;
    }
    let needed = (size + i64::from(S3_MAX_PARTS) - 1) / i64::from(S3_MAX_PARTS);
    let needed = (needed + MIB - 1) / MIB * MIB;
    Some(needed.max(MIN_PART_BYTES))
}

/// The `(part number, ETag)` list to complete the upload with, when the bucket has parts 1 to n with the
/// planned sizes; else `UploadIncomplete`.
fn complete_parts(
    upload: &DriveUpload,
    parts: &[S3Part],
) -> Result<Vec<(u32, String)>, DriveError> {
    let count = upload.part_count();
    let part_size = upload.part_size.ok_or(DriveError::UploadIncomplete)?;
    if i64::try_from(parts.len()).ok() != Some(count) {
        return Err(DriveError::UploadIncomplete);
    }
    parts
        .iter()
        .enumerate()
        .map(|(index, part)| {
            let number = i64::try_from(index).unwrap_or(i64::MAX) + 1;
            let expected = if number == count {
                upload.size - part_size * (count - 1)
            } else {
                part_size
            };
            if i64::from(part.part_number) != number
                || i64::try_from(part.size).ok() != Some(expected)
            {
                return Err(DriveError::UploadIncomplete);
            }
            Ok((part.part_number, part.etag.clone()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn upload(size: i64) -> DriveUpload {
        DriveUpload {
            id: Id::new_v7(),
            r2_key: String::new(),
            upload_id: Some("u".to_owned()),
            parent_id: None,
            name: "video.mp4".to_owned(),
            size,
            mime: "video/mp4".to_owned(),
            on_conflict: OnConflict::Fail,
            part_size: part_size(size),
            created_at: TimestampMillis::from_millis(0),
            created_by: None,
        }
    }

    fn part(part_number: u32, size: i64) -> S3Part {
        S3Part {
            part_number,
            etag: format!("\"e{part_number}\""),
            size: u64::try_from(size).unwrap(),
        }
    }

    #[test]
    fn part_sizes_are_fixed_per_file_and_fit_in_the_part_limit() {
        assert_eq!(part_size(SINGLE_PUT_MAX_BYTES), None);
        assert_eq!(part_size(SINGLE_PUT_MAX_BYTES + 1), Some(MIN_PART_BYTES));
        let huge = part_size(MAX_FILE_BYTES).unwrap();
        assert_eq!(huge % MIB, 0);
        assert!((MAX_FILE_BYTES + huge - 1) / huge <= i64::from(S3_MAX_PARTS));
        assert!(huge <= 5 * 1024 * MIB, "S3 parts are at most 5 GiB");
    }

    #[test]
    fn an_upload_completes_only_with_every_part_at_its_planned_size() {
        let size = 2 * MIN_PART_BYTES + 5;
        let upload = upload(size);
        let all = [part(1, MIN_PART_BYTES), part(2, MIN_PART_BYTES), part(3, 5)];
        assert_eq!(
            complete_parts(&upload, &all).unwrap(),
            vec![
                (1, "\"e1\"".to_owned()),
                (2, "\"e2\"".to_owned()),
                (3, "\"e3\"".to_owned())
            ]
        );
        let missing = [part(1, MIN_PART_BYTES), part(3, 5)];
        let short = [
            part(1, MIN_PART_BYTES),
            part(2, MIN_PART_BYTES - 1),
            part(3, 6),
        ];
        let extra = [
            part(1, MIN_PART_BYTES),
            part(2, MIN_PART_BYTES),
            part(3, 5),
            part(4, 1),
        ];
        for parts in [&missing[..], &short[..], &extra[..], &[][..]] {
            assert!(matches!(
                complete_parts(&upload, parts),
                Err(DriveError::UploadIncomplete)
            ));
        }
    }
}
