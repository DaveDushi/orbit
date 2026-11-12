//! Orbit Drive: the folder tree of a workspace's files (one drive per workspace, every member reads and writes).
//! The one exception are the folders of chat ([`DriveRepository::chat_folder`]): a node with a
//! `chat_conversation_id` is seen only by the people who can read that conversation (a DM's files by its members).
//! Only names, the tree and object keys are here; the bytes are in S3 and move between the browser and the
//! bucket ([`crate::drive`]). Every function checks the caller's membership in its own transaction.
//!
//! Only the top node of a trashed tree has `trashed_at`: its descendants are hidden with it and come back with
//! it. A node is live when neither it nor any ancestor is trashed.

use orbit_domain::{Actor, Permission};
use orbit_platform::{Database, Id, Row, SqlBuilder, TimestampMillis};
use serde::{Deserialize, Serialize};
use sqlx::AnyConnection;
use thiserror::Error;
use utoipa::ToSchema;

use crate::audit::{self, AuditOutcome};
use crate::repositories::membership;

const DAY_MILLIS: i64 = 24 * 60 * 60 * 1000;
/// Trashed items are deleted forever after this long.
pub const TRASH_RETENTION_MILLIS: i64 = 30 * DAY_MILLIS;
/// Uploads not completed within this time are cancelled (R2 drops incomplete multipart uploads after 7 days).
pub const STALE_UPLOAD_MILLIS: i64 = 6 * DAY_MILLIS;
pub const MAX_NAME_CHARS: usize = 255;
/// The most items of one page of a list.
pub const MAX_PAGE_SIZE: i64 = 500;
/// Stops a recursive walk on a broken tree (the API never makes a cycle).
const MAX_DEPTH: i64 = 10_000;
/// Free names tried for "name (2).ext", "name (3).ext", ….
const MAX_NAME_SUFFIX: u32 = 1_000;

const NODE_COLUMNS: &str = "drive_nodes.id, drive_nodes.parent_id, drive_nodes.kind, drive_nodes.name, \
    drive_nodes.size, drive_nodes.mime, drive_nodes.created_by, drive_nodes.created_at, drive_nodes.updated_by, \
    drive_nodes.updated_at, drive_nodes.trashed_at, drive_media.width, drive_media.height, drive_media.duration_ms, \
    drive_media.status AS media_status, drive_media.thumb_key, drive_media.poster_key, drive_nodes.workspace_id, \
    pv.status AS preview_status, \
    EXISTS (SELECT 1 FROM file_preview_outputs po WHERE po.preview_id = pv.id AND po.role = 'play') AS preview_play, \
    EXISTS (SELECT 1 FROM file_preview_outputs po WHERE po.preview_id = pv.id AND po.role = 'poster') AS preview_poster, \
    EXISTS (SELECT 1 FROM file_preview_outputs po WHERE po.preview_id = pv.id AND po.role = 'pdf') AS preview_pdf";
/// The nodes with their media (images and videos the browser measured) and their preview (one per blob).
const NODE_FROM: &str = "drive_nodes LEFT JOIN drive_media ON drive_media.blob_id = drive_nodes.blob_id \
     LEFT JOIN file_previews pv ON pv.drive_blob_id = drive_nodes.blob_id";
/// The top-level folder of the chat folders.
pub const UPLOADS_FOLDER: &str = "Uploads";
/// The largest thumbnail and poster objects that are kept.
pub const MAX_THUMB_BYTES: u64 = 1024 * 1024;
pub const MAX_POSTER_BYTES: u64 = 4 * 1024 * 1024;
const UPLOAD_COLUMNS: &str = "id, r2_key, size, mime, upload_id, part_size, upload_parent_id, upload_name, \
    upload_conflict, created_by, created_at";

#[derive(
    Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, ToSchema, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum DriveNodeKind {
    Folder,
    File,
}

impl DriveNodeKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Folder => "folder",
            Self::File => "file",
        }
    }
}

/// A folder or file of the drive.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, ToSchema)]
pub struct DriveNode {
    #[schema(value_type = String)]
    pub id: Id,
    /// `null` at the top level.
    #[schema(value_type = Option<String>)]
    pub parent_id: Option<Id>,
    pub kind: DriveNodeKind,
    pub name: String,
    /// Bytes; `null` for folders.
    pub size: Option<i64>,
    /// `null` for folders.
    pub mime: Option<String>,
    #[schema(value_type = Option<String>)]
    pub created_by: Option<Id>,
    #[schema(value_type = String, format = DateTime)]
    pub created_at: TimestampMillis,
    #[schema(value_type = Option<String>)]
    pub updated_by: Option<Id>,
    #[schema(value_type = String, format = DateTime)]
    pub updated_at: TimestampMillis,
    /// Set on an item in the trash (only on the item that was trashed, not on what it contains).
    #[schema(value_type = Option<String>, format = DateTime)]
    pub trashed_at: Option<TimestampMillis>,
    /// What the browser measured of an image or video; `null` until a browser did.
    pub media: Option<DriveMedia>,
    /// A 5-minute URL of the thumbnail (images, videos), when there is one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumbnail_url: Option<String>,
    /// A 5-minute URL of a large frame of a video, for the player.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub poster_url: Option<String>,
    /// The playable copy of a video or the PDF of an office file; `null` for other files.
    #[schema(required = true)]
    pub preview: Option<crate::previews::FilePreview>,
    #[serde(skip)]
    pub thumb_key: Option<String>,
    #[serde(skip)]
    pub poster_key: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum DriveMediaStatus {
    /// A browser is making the thumbnail.
    Pending,
    Ready,
    /// The browser could not read the file (a format it cannot show): no thumbnail.
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, ToSchema)]
pub struct DriveMedia {
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub duration_ms: Option<i64>,
    pub status: DriveMediaStatus,
}

/// What a browser measured of an image or video.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MediaFacts {
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub duration_ms: Option<i64>,
}

/// Where a browser puts the thumbnail and poster of a file.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MediaTarget {
    pub blob_id: Id,
    pub thumb_key: Option<String>,
    pub poster_key: Option<String>,
}

/// One folder on the way from the top level to a node.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, ToSchema)]
pub struct DriveCrumb {
    #[schema(value_type = String)]
    pub id: Id,
    pub name: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, ToSchema)]
pub struct DriveNodeDetail {
    pub node: DriveNode,
    /// The folders above the node, from the top level down.
    pub path: Vec<DriveCrumb>,
}

/// A node found below a folder or by a search, with the path of its folder (`""` at the top level,
/// else `a/b`).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, ToSchema)]
pub struct DriveEntry {
    pub node: DriveNode,
    pub folder_path: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, ToSchema)]
pub struct DriveNodePage {
    pub items: Vec<DriveNode>,
    /// The `offset` of the next page; `null` on the last page.
    pub next_offset: Option<i64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, ToSchema)]
pub struct DriveEntryPage {
    pub items: Vec<DriveEntry>,
    pub next_offset: Option<i64>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum DriveSort {
    #[default]
    Name,
    UpdatedAt,
    Size,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SortOrder {
    #[default]
    Asc,
    Desc,
}

/// What happens when the name is already used in the folder.
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    Eq,
    PartialEq,
    Serialize,
    Deserialize,
    ToSchema,
    schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum OnConflict {
    /// Refuse with `drive_name_taken`.
    #[default]
    Fail,
    /// Use the next free name: `name (2).ext`.
    Rename,
    /// Files only: move the existing file to the trash.
    Replace,
}

impl OnConflict {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Fail => "fail",
            Self::Rename => "rename",
            Self::Replace => "replace",
        }
    }

    fn parse(value: &str) -> Self {
        match value {
            "rename" => Self::Rename,
            "replace" => Self::Replace,
            _ => Self::Fail,
        }
    }
}

/// A page of a list: `offset` items skipped, at most `limit` returned.
#[derive(Clone, Copy, Debug)]
pub struct Paging {
    pub offset: i64,
    pub limit: i64,
}

impl Paging {
    #[must_use]
    pub fn new(offset: Option<i64>, limit: Option<i64>) -> Self {
        Self {
            offset: offset.unwrap_or(0).max(0),
            limit: limit.unwrap_or(100).clamp(1, MAX_PAGE_SIZE),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct DriveSearch {
    /// Part of the name, any case.
    pub query: Option<String>,
    pub kind: Option<DriveNodeKind>,
    /// Start of the MIME type, such as `image/`.
    pub mime_prefix: Option<String>,
    pub updated_after: Option<TimestampMillis>,
    pub updated_before: Option<TimestampMillis>,
}

/// A file that a member is uploading.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, ToSchema)]
pub struct DriveUpload {
    #[schema(value_type = String)]
    pub id: Id,
    #[serde(skip)]
    pub r2_key: String,
    #[serde(skip)]
    pub upload_id: Option<String>,
    #[schema(value_type = Option<String>)]
    pub parent_id: Option<Id>,
    pub name: String,
    pub size: i64,
    pub mime: String,
    pub on_conflict: OnConflict,
    /// `null` for a single PUT; else every part but the last has this size.
    pub part_size: Option<i64>,
    #[schema(value_type = String, format = DateTime)]
    pub created_at: TimestampMillis,
    #[serde(skip)]
    pub created_by: Option<Id>,
}

impl DriveUpload {
    /// The number of parts of a multipart upload (1 for a single PUT).
    #[must_use]
    pub fn part_count(&self) -> i64 {
        match self.part_size {
            Some(part_size) if self.size > 0 => (self.size + part_size - 1) / part_size,
            _ => 1,
        }
    }

    #[must_use]
    pub fn multipart(&self) -> bool {
        self.part_size.is_some()
    }
}

/// What a new upload is for.
#[derive(Clone, Debug)]
pub struct NewUpload {
    pub blob_id: Id,
    pub r2_key: String,
    pub parent_id: Option<Id>,
    pub name: String,
    pub size: i64,
    pub mime: String,
    pub on_conflict: OnConflict,
    pub part_size: Option<i64>,
}

/// A file to download: where its bytes are and the name to save them under.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DriveObject {
    pub node_id: Id,
    pub r2_key: String,
    pub name: String,
    pub mime: String,
}

/// An object to delete from the bucket (and the upload to abort) before its blob row goes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlobDeletion {
    pub blob_id: Id,
    pub workspace_id: Id,
    pub r2_key: String,
    pub upload_id: Option<String>,
    /// Thumbnail and poster objects.
    pub extra_keys: Vec<String>,
}

#[derive(Debug, Error)]
pub enum DriveError {
    /// Not a member, or no such live node (also used for nodes of other workspaces).
    #[error("drive item not found")]
    NotFound,
    #[error("the role may not do this")]
    Forbidden,
    #[error("invalid name")]
    InvalidName,
    #[error("the name is already used in this folder")]
    NameTaken,
    #[error("the target is not a folder")]
    NotAFolder,
    #[error("a folder cannot move into itself or its own subfolder")]
    MoveCycle,
    #[error("only an item in the trash can be deleted forever")]
    NotTrashed,
    #[error("the upload is not complete")]
    UploadIncomplete,
    #[error("the file is too large")]
    TooLarge,
    #[error("invalid part number")]
    InvalidPart,
    #[error("too many items in one request")]
    TooManyItems,
    #[error("only images and videos have a thumbnail")]
    NotMedia,
    #[error("the workspace has no S3 storage for Drive")]
    NoStorage,
    #[error(transparent)]
    Storage(#[from] orbit_platform::S3Error),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

#[derive(Clone)]
pub struct DriveRepository {
    database: Database,
}

impl DriveRepository {
    #[must_use]
    pub const fn new(database: Database) -> Self {
        Self { database }
    }

    /// Checks that the caller is a member of the workspace.
    pub async fn require_member(&self, workspace_id: Id, user_id: Id) -> Result<(), DriveError> {
        let mut connection = self.database.pool().acquire().await?;
        require_actor(&mut connection, workspace_id, user_id)
            .await
            .map(drop)
    }

    /// The live items of a folder (`None`: the top level): folders first, then by `sort`.
    pub async fn children(
        &self,
        workspace_id: Id,
        user_id: Id,
        parent_id: Option<Id>,
        sort: DriveSort,
        order: SortOrder,
        paging: Paging,
    ) -> Result<DriveNodePage, DriveError> {
        let mut connection = self.database.pool().acquire().await?;
        require_actor(&mut connection, workspace_id, user_id).await?;
        live_folder(&mut connection, workspace_id, parent_id, Some(user_id)).await?;
        list_children(
            &mut connection,
            workspace_id,
            parent_id,
            sort,
            order,
            paging,
            &Visible::Member(user_id),
        )
        .await
    }

    /// A live node and the folders above it.
    pub async fn node(
        &self,
        workspace_id: Id,
        user_id: Id,
        node_id: Id,
    ) -> Result<DriveNodeDetail, DriveError> {
        let mut connection = self.database.pool().acquire().await?;
        require_actor(&mut connection, workspace_id, user_id).await?;
        let chain = live_chain(&mut connection, workspace_id, node_id, Some(user_id)).await?;
        let node = load_node(&mut connection, workspace_id, node_id)
            .await?
            .ok_or(DriveError::NotFound)?;
        Ok(DriveNodeDetail {
            node,
            path: chain[..chain.len() - 1]
                .iter()
                .map(|link| DriveCrumb {
                    id: link.id,
                    name: link.name.clone(),
                })
                .collect(),
        })
    }

    pub async fn create_folder(
        &self,
        workspace_id: Id,
        user_id: Id,
        parent_id: Option<Id>,
        name: &str,
        on_conflict: OnConflict,
        now: TimestampMillis,
    ) -> Result<DriveNode, DriveError> {
        let name = valid_name(name)?;
        let mut transaction = self.database.immediate_transaction().await?;
        require_actor(&mut transaction, workspace_id, user_id).await?;
        live_folder(&mut transaction, workspace_id, parent_id, Some(user_id)).await?;
        let name = match on_conflict {
            OnConflict::Fail | OnConflict::Replace => {
                if sibling(&mut transaction, workspace_id, parent_id, &name, None)
                    .await?
                    .is_some()
                {
                    return Err(DriveError::NameTaken);
                }
                name
            }
            OnConflict::Rename => {
                free_name(&mut transaction, workspace_id, parent_id, &name).await?
            }
        };
        let id = Id::new_v7();
        insert_node(
            &mut transaction,
            &NewNode {
                id,
                workspace_id,
                parent_id,
                kind: DriveNodeKind::Folder,
                name: &name,
                blob: None,
                user_id,
                chat_root: None,
            },
            now,
        )
        .await?;
        let node = load_node(&mut transaction, workspace_id, id)
            .await?
            .ok_or(DriveError::NotFound)?;
        transaction.commit().await?;
        Ok(node)
    }

    pub async fn rename(
        &self,
        workspace_id: Id,
        user_id: Id,
        node_id: Id,
        name: &str,
        now: TimestampMillis,
    ) -> Result<DriveNode, DriveError> {
        let name = valid_name(name)?;
        let mut transaction = self.database.immediate_transaction().await?;
        require_actor(&mut transaction, workspace_id, user_id).await?;
        let chain = live_chain(&mut transaction, workspace_id, node_id, Some(user_id)).await?;
        let parent_id = chain.len().checked_sub(2).map(|index| chain[index].id);
        if sibling(
            &mut transaction,
            workspace_id,
            parent_id,
            &name,
            Some(node_id),
        )
        .await?
        .is_some()
        {
            return Err(DriveError::NameTaken);
        }
        sqlx::query(
            "UPDATE drive_nodes SET name = $1, name_lower = $2, updated_by = $3, updated_at = $4 WHERE id = $5",
        )
        .bind(&name)
        .bind(name.to_lowercase())
        .bind(user_id.to_string())
        .bind(now.as_millis())
        .bind(node_id.to_string())
        .execute(&mut *transaction)
        .await?;
        let node = load_node(&mut transaction, workspace_id, node_id)
            .await?
            .ok_or(DriveError::NotFound)?;
        transaction.commit().await?;
        Ok(node)
    }

    /// Moves a live node into a live folder (`None`: the top level).
    pub async fn move_node(
        &self,
        workspace_id: Id,
        user_id: Id,
        node_id: Id,
        parent_id: Option<Id>,
        now: TimestampMillis,
    ) -> Result<DriveNode, DriveError> {
        let mut transaction = self.database.immediate_transaction().await?;
        require_actor(&mut transaction, workspace_id, user_id).await?;
        let chain = live_chain(&mut transaction, workspace_id, node_id, Some(user_id)).await?;
        let node = &chain[chain.len() - 1];
        if let Some(parent_id) = parent_id {
            let target =
                live_chain(&mut transaction, workspace_id, parent_id, Some(user_id)).await?;
            if target.iter().any(|link| link.id == node_id) {
                return Err(DriveError::MoveCycle);
            }
            if target[target.len() - 1].kind != DriveNodeKind::Folder {
                return Err(DriveError::NotAFolder);
            }
        }
        if sibling(
            &mut transaction,
            workspace_id,
            parent_id,
            &node.name,
            Some(node_id),
        )
        .await?
        .is_some()
        {
            return Err(DriveError::NameTaken);
        }
        sqlx::query(
            "UPDATE drive_nodes SET parent_id = $1, updated_by = $2, updated_at = $3 WHERE id = $4",
        )
        .bind(parent_id.map(|id| id.to_string()))
        .bind(user_id.to_string())
        .bind(now.as_millis())
        .bind(node_id.to_string())
        .execute(&mut *transaction)
        .await?;
        rescope(&mut transaction, node_id).await?;
        let node = load_node(&mut transaction, workspace_id, node_id)
            .await?
            .ok_or(DriveError::NotFound)?;
        transaction.commit().await?;
        Ok(node)
    }

    /// Moves a live node, with everything in it, to the trash.
    pub async fn trash(
        &self,
        workspace_id: Id,
        user_id: Id,
        node_id: Id,
        now: TimestampMillis,
    ) -> Result<(), DriveError> {
        let mut transaction = self.database.immediate_transaction().await?;
        require_actor(&mut transaction, workspace_id, user_id).await?;
        live_chain(&mut transaction, workspace_id, node_id, Some(user_id)).await?;
        trash_node(&mut transaction, node_id, user_id, now).await?;
        transaction.commit().await?;
        Ok(())
    }

    /// Brings a trashed item back, with everything in it, into its folder; to the top level when that folder
    /// is gone or in the trash. A used name gets the next free one.
    pub async fn restore(
        &self,
        workspace_id: Id,
        user_id: Id,
        node_id: Id,
        now: TimestampMillis,
    ) -> Result<DriveNode, DriveError> {
        let mut transaction = self.database.immediate_transaction().await?;
        require_actor(&mut transaction, workspace_id, user_id).await?;
        let chain = chain(&mut transaction, workspace_id, node_id, Some(user_id)).await?;
        let Some((node, above)) = chain.split_last() else {
            return Err(DriveError::NotFound);
        };
        if !node.trashed {
            return Err(DriveError::NotFound);
        }
        let parent_id = if above.iter().any(|link| link.trashed) {
            None
        } else {
            above.last().map(|link| link.id)
        };
        let name = free_name(&mut transaction, workspace_id, parent_id, &node.name).await?;
        sqlx::query(
            "UPDATE drive_nodes SET trashed_at = NULL, trashed_by = NULL, parent_id = $1, name = $2, \
             name_lower = $3, updated_by = $4, updated_at = $5 WHERE id = $6",
        )
        .bind(parent_id.map(|id| id.to_string()))
        .bind(&name)
        .bind(name.to_lowercase())
        .bind(user_id.to_string())
        .bind(now.as_millis())
        .bind(node_id.to_string())
        .execute(&mut *transaction)
        .await?;
        rescope(&mut transaction, node_id).await?;
        let node = load_node(&mut transaction, workspace_id, node_id)
            .await?
            .ok_or(DriveError::NotFound)?;
        transaction.commit().await?;
        Ok(node)
    }

    /// The items that were trashed, newest first.
    pub async fn trashed(
        &self,
        workspace_id: Id,
        user_id: Id,
        paging: Paging,
    ) -> Result<DriveNodePage, DriveError> {
        let mut connection = self.database.pool().acquire().await?;
        require_actor(&mut connection, workspace_id, user_id).await?;
        let mut query = SqlBuilder::new(format!(
            "SELECT {NODE_COLUMNS} FROM {NODE_FROM} WHERE drive_nodes.workspace_id = "
        ));
        query.push_bind(workspace_id.to_string());
        push_visible(&mut query, "drive_nodes", user_id);
        query.push(
            " AND drive_nodes.trashed_at IS NOT NULL ORDER BY drive_nodes.trashed_at DESC, drive_nodes.id DESC",
        );
        let rows = page_rows(&mut query, paging, &mut connection).await?;
        node_page(&rows, paging)
    }

    /// Deletes a trashed item and everything in it forever. Its objects are deleted from the bucket afterwards
    /// ([`Self::deletions`]).
    pub async fn purge(
        &self,
        workspace_id: Id,
        user_id: Id,
        node_id: Id,
        request_id: &str,
        now: TimestampMillis,
    ) -> Result<(), DriveError> {
        let mut transaction = self.database.immediate_transaction().await?;
        let actor = require_actor(&mut transaction, workspace_id, user_id).await?;
        let trashed = sqlx::query(
            "SELECT name, trashed_at FROM drive_nodes WHERE id = $1 AND workspace_id = $2",
        )
        .bind(node_id.to_string())
        .bind(workspace_id.to_string())
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(DriveError::NotFound)?;
        if !actor.can(Permission::DrivePurge) {
            return Err(DriveError::Forbidden);
        }
        if trashed.try_get::<Option<i64>, _>("trashed_at")?.is_none() {
            return Err(DriveError::NotTrashed);
        }
        let deleted = purge_trees(
            &mut transaction,
            |query| {
                query.push("SELECT id, blob_id FROM drive_nodes WHERE id = ");
                query.push_bind(node_id.to_string());
            },
            now,
        )
        .await?;
        audit::record(
            &mut transaction,
            workspace_id,
            Some(user_id),
            "drive.purged",
            AuditOutcome::Success,
            "drive_node",
            Some(node_id),
            request_id,
            serde_json::json!({"name": trashed.try_get::<String, _>("name")?, "nodes": deleted}),
            now,
        )
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    /// Deletes everything in the trash forever. Returns the number of deleted nodes.
    pub async fn empty_trash(
        &self,
        workspace_id: Id,
        user_id: Id,
        request_id: &str,
        now: TimestampMillis,
    ) -> Result<u64, DriveError> {
        let mut transaction = self.database.immediate_transaction().await?;
        let actor = require_actor(&mut transaction, workspace_id, user_id).await?;
        if !actor.can(Permission::DrivePurge) {
            return Err(DriveError::Forbidden);
        }
        let deleted = purge_trees(
            &mut transaction,
            |query| {
                query.push(
                    "SELECT id, blob_id FROM drive_nodes WHERE trashed_at IS NOT NULL AND workspace_id = ",
                );
                query.push_bind(workspace_id.to_string());
            },
            now,
        )
        .await?;
        if deleted > 0 {
            audit::record(
                &mut transaction,
                workspace_id,
                Some(user_id),
                "drive.trash_emptied",
                AuditOutcome::Success,
                "workspace",
                Some(workspace_id),
                request_id,
                serde_json::json!({"nodes": deleted}),
                now,
            )
            .await?;
        }
        transaction.commit().await?;
        Ok(deleted)
    }

    /// Live nodes whose name contains the query (any case), sorted by name.
    pub async fn search(
        &self,
        workspace_id: Id,
        user_id: Id,
        search: &DriveSearch,
        paging: Paging,
    ) -> Result<DriveEntryPage, DriveError> {
        let mut connection = self.database.pool().acquire().await?;
        require_actor(&mut connection, workspace_id, user_id).await?;
        let mut query = SqlBuilder::new(
            "WITH RECURSIVE live(id, folder_path, depth) AS (SELECT id, CAST('' AS TEXT), 0 FROM drive_nodes \
             WHERE parent_id IS NULL AND trashed_at IS NULL AND workspace_id = ",
        );
        query.push_bind(workspace_id.to_string());
        push_live_walk(&mut query);
        query.push(format!(
            " SELECT {NODE_COLUMNS}, live.folder_path FROM {NODE_FROM} JOIN live ON live.id = drive_nodes.id WHERE 1 = 1"
        ));
        push_visible(&mut query, "drive_nodes", user_id);
        if let Some(text) = search
            .query
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
        {
            query.push(" AND drive_nodes.name_lower LIKE ");
            query.push_bind(format!("%{}%", like_escape(&text.to_lowercase())));
            query.push(" ESCAPE '\\'");
        }
        if let Some(kind) = search.kind {
            query.push(" AND drive_nodes.kind = ");
            query.push_bind(kind.as_str());
        }
        if let Some(prefix) = search
            .mime_prefix
            .as_deref()
            .filter(|prefix| !prefix.is_empty())
        {
            query.push(" AND drive_nodes.mime LIKE ");
            query.push_bind(format!("{}%", like_escape(&prefix.to_lowercase())));
            query.push(" ESCAPE '\\'");
        }
        if let Some(after) = search.updated_after {
            query.push(" AND drive_nodes.updated_at >= ");
            query.push_bind(after.as_millis());
        }
        if let Some(before) = search.updated_before {
            query.push(" AND drive_nodes.updated_at < ");
            query.push_bind(before.as_millis());
        }
        query.push(" ORDER BY drive_nodes.name_lower, drive_nodes.id");
        let rows = page_rows(&mut query, paging, &mut connection).await?;
        entry_page(&rows, paging)
    }

    /// The live files below a folder, at any depth, by path (for a ZIP download). `folder_path` is relative to
    /// the folder.
    pub async fn files_below(
        &self,
        workspace_id: Id,
        user_id: Id,
        folder_id: Id,
        paging: Paging,
    ) -> Result<DriveEntryPage, DriveError> {
        let mut connection = self.database.pool().acquire().await?;
        require_actor(&mut connection, workspace_id, user_id).await?;
        live_folder(
            &mut connection,
            workspace_id,
            Some(folder_id),
            Some(user_id),
        )
        .await?;
        let mut query = SqlBuilder::new(
            "WITH RECURSIVE live(id, folder_path, depth) AS (SELECT id, CAST('' AS TEXT), 0 FROM drive_nodes \
             WHERE trashed_at IS NULL AND parent_id = ",
        );
        query.push_bind(folder_id.to_string());
        push_live_walk(&mut query);
        query.push(format!(
            " SELECT {NODE_COLUMNS}, live.folder_path FROM {NODE_FROM} JOIN live ON live.id = drive_nodes.id \
             WHERE drive_nodes.kind = 'file'"
        ));
        push_visible(&mut query, "drive_nodes", user_id);
        query.push(" ORDER BY live.folder_path, drive_nodes.name_lower, drive_nodes.id");
        let rows = page_rows(&mut query, paging, &mut connection).await?;
        entry_page(&rows, paging)
    }

    /// Where the bytes of live files are, for download URLs. Ids that are not live files are left out.
    pub async fn objects(
        &self,
        workspace_id: Id,
        user_id: Id,
        node_ids: &[Id],
    ) -> Result<Vec<DriveObject>, DriveError> {
        let mut connection = self.database.pool().acquire().await?;
        require_actor(&mut connection, workspace_id, user_id).await?;
        let mut objects = Vec::with_capacity(node_ids.len());
        for node_id in node_ids {
            match live_chain(&mut connection, workspace_id, *node_id, Some(user_id)).await {
                Ok(chain) if chain[chain.len() - 1].kind == DriveNodeKind::File => {}
                Ok(_) | Err(DriveError::NotFound) => continue,
                Err(error) => return Err(error),
            }
            let row = sqlx::query(
                "SELECT drive_nodes.name, drive_blobs.r2_key, drive_blobs.mime FROM drive_nodes \
                 JOIN drive_blobs ON drive_blobs.id = drive_nodes.blob_id \
                 WHERE drive_nodes.id = $1 AND drive_blobs.status = 'ready'",
            )
            .bind(node_id.to_string())
            .fetch_optional(&mut *connection)
            .await?;
            if let Some(row) = row {
                objects.push(DriveObject {
                    node_id: *node_id,
                    r2_key: row.try_get("r2_key")?,
                    name: row.try_get("name")?,
                    mime: row.try_get("mime")?,
                });
            }
        }
        Ok(objects)
    }

    /// Records what a browser measured of a live image or video and where its thumbnail (and, for a video, its
    /// poster) go. They stay `pending` until [`Self::finish_media`]; with `failed` the file gets no thumbnail.
    pub async fn begin_media(
        &self,
        workspace_id: Id,
        user_id: Id,
        node_id: Id,
        facts: MediaFacts,
        failed: bool,
        now: TimestampMillis,
    ) -> Result<MediaTarget, DriveError> {
        let mut transaction = self.database.immediate_transaction().await?;
        require_actor(&mut transaction, workspace_id, user_id).await?;
        let chain = live_chain(&mut transaction, workspace_id, node_id, Some(user_id)).await?;
        if chain[chain.len() - 1].kind != DriveNodeKind::File {
            return Err(DriveError::NotMedia);
        }
        let row = sqlx::query(
            "SELECT drive_blobs.id, drive_blobs.mime FROM drive_nodes JOIN drive_blobs ON drive_blobs.id = drive_nodes.blob_id \
             WHERE drive_nodes.id = $1",
        )
        .bind(node_id.to_string())
        .fetch_one(&mut *transaction)
        .await?;
        let blob_id = parse_id(row.try_get("id")?)?;
        let mime: String = row.try_get("mime")?;
        let video = mime.starts_with("video/");
        if !video && !mime.starts_with("image/") {
            return Err(DriveError::NotMedia);
        }
        let key = |kind: &str| (!failed).then(|| format!("drive/{workspace_id}/{blob_id}.{kind}"));
        let target = MediaTarget {
            blob_id,
            thumb_key: key("thumb"),
            poster_key: if video { key("poster") } else { None },
        };
        let positive = |value: Option<i64>| value.filter(|value| *value > 0);
        sqlx::query(
            "INSERT INTO drive_media (blob_id, width, height, duration_ms, thumb_key, poster_key, status, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8) ON CONFLICT (blob_id) DO UPDATE SET width = excluded.width, \
             height = excluded.height, duration_ms = excluded.duration_ms, thumb_key = excluded.thumb_key, \
             poster_key = excluded.poster_key, status = excluded.status, updated_at = excluded.updated_at",
        )
        .bind(blob_id.to_string())
        .bind(positive(facts.width))
        .bind(positive(facts.height))
        .bind(facts.duration_ms.filter(|value| *value >= 0))
        .bind(target.thumb_key.clone())
        .bind(target.poster_key.clone())
        .bind(if failed { "failed" } else { "pending" })
        .bind(now.as_millis())
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(target)
    }

    /// The pending thumbnail and poster of a live file, for the member who checks them.
    pub async fn media_target(
        &self,
        workspace_id: Id,
        user_id: Id,
        node_id: Id,
    ) -> Result<MediaTarget, DriveError> {
        let mut connection = self.database.pool().acquire().await?;
        require_actor(&mut connection, workspace_id, user_id).await?;
        live_chain(&mut connection, workspace_id, node_id, Some(user_id)).await?;
        let row = sqlx::query(
            "SELECT drive_media.blob_id, drive_media.thumb_key, drive_media.poster_key FROM drive_nodes \
             JOIN drive_media ON drive_media.blob_id = drive_nodes.blob_id WHERE drive_nodes.id = $1",
        )
        .bind(node_id.to_string())
        .fetch_optional(&mut *connection)
        .await?
        .ok_or(DriveError::NotMedia)?;
        Ok(MediaTarget {
            blob_id: parse_id(row.try_get("blob_id")?)?,
            thumb_key: row.try_get("thumb_key")?,
            poster_key: row.try_get("poster_key")?,
        })
    }

    /// Keeps the thumbnail and poster that the bucket has (the caller checked them): `ready` with a thumbnail,
    /// else `failed`. Returns the node.
    pub async fn finish_media(
        &self,
        workspace_id: Id,
        user_id: Id,
        node_id: Id,
        thumb_key: Option<&str>,
        poster_key: Option<&str>,
        now: TimestampMillis,
    ) -> Result<DriveNode, DriveError> {
        let mut transaction = self.database.immediate_transaction().await?;
        require_actor(&mut transaction, workspace_id, user_id).await?;
        live_chain(&mut transaction, workspace_id, node_id, Some(user_id)).await?;
        sqlx::query(
            "UPDATE drive_media SET thumb_key = $1, poster_key = $2, status = $3, updated_at = $4 \
             WHERE blob_id = (SELECT blob_id FROM drive_nodes WHERE id = $5)",
        )
        .bind(thumb_key)
        .bind(poster_key)
        .bind(if thumb_key.is_some() {
            "ready"
        } else {
            "failed"
        })
        .bind(now.as_millis())
        .bind(node_id.to_string())
        .execute(&mut *transaction)
        .await?;
        let node = load_node(&mut transaction, workspace_id, node_id)
            .await?
            .ok_or(DriveError::NotFound)?;
        transaction.commit().await?;
        Ok(node)
    }

    /// Checks the target of a new upload and records it. With `OnConflict::Fail` a used name is refused now;
    /// the name is checked again when the upload completes.
    pub async fn begin_upload(
        &self,
        workspace_id: Id,
        user_id: Id,
        upload: NewUpload,
        now: TimestampMillis,
    ) -> Result<DriveUpload, DriveError> {
        let name = valid_name(&upload.name)?;
        let mut transaction = self.database.immediate_transaction().await?;
        require_actor(&mut transaction, workspace_id, user_id).await?;
        live_folder(
            &mut transaction,
            workspace_id,
            upload.parent_id,
            Some(user_id),
        )
        .await?;
        let existing = sibling(
            &mut transaction,
            workspace_id,
            upload.parent_id,
            &name,
            None,
        )
        .await?
        .map(|(_, kind)| kind);
        match (upload.on_conflict, existing) {
            (_, None)
            | (OnConflict::Rename, _)
            | (OnConflict::Replace, Some(DriveNodeKind::File)) => {}
            (OnConflict::Fail | OnConflict::Replace, Some(_)) => return Err(DriveError::NameTaken),
        }
        sqlx::query(
            "INSERT INTO drive_blobs (id, workspace_id, r2_key, size, mime, status, part_size, upload_parent_id, \
             upload_name, upload_conflict, created_by, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, 'uploading', $6, $7, $8, $9, $10, $11, $11)",
        )
        .bind(upload.blob_id.to_string())
        .bind(workspace_id.to_string())
        .bind(&upload.r2_key)
        .bind(upload.size)
        .bind(&upload.mime)
        .bind(upload.part_size)
        .bind(upload.parent_id.map(|id| id.to_string()))
        .bind(&name)
        .bind(upload.on_conflict.as_str())
        .bind(user_id.to_string())
        .bind(now.as_millis())
        .execute(&mut *transaction)
        .await?;
        let upload = load_upload(&mut transaction, workspace_id, upload.blob_id)
            .await?
            .ok_or(DriveError::NotFound)?;
        transaction.commit().await?;
        Ok(upload)
    }

    /// Records the S3 multipart upload of an upload that [`Self::begin_upload`] recorded.
    pub async fn set_multipart_id(&self, blob_id: Id, upload_id: &str) -> Result<(), DriveError> {
        let mut transaction = self.database.immediate_transaction().await?;
        sqlx::query("UPDATE drive_blobs SET upload_id = $1 WHERE id = $2 AND status = 'uploading'")
            .bind(upload_id)
            .bind(blob_id.to_string())
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok(())
    }

    /// An upload in progress, for the member who started it.
    pub async fn upload(
        &self,
        workspace_id: Id,
        user_id: Id,
        blob_id: Id,
    ) -> Result<DriveUpload, DriveError> {
        let mut connection = self.database.pool().acquire().await?;
        own_upload(&mut connection, workspace_id, user_id, blob_id).await
    }

    /// The caller's uploads in progress, oldest first.
    pub async fn uploads(
        &self,
        workspace_id: Id,
        user_id: Id,
    ) -> Result<Vec<DriveUpload>, DriveError> {
        let mut connection = self.database.pool().acquire().await?;
        require_actor(&mut connection, workspace_id, user_id).await?;
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT {UPLOAD_COLUMNS} FROM drive_blobs WHERE workspace_id = $1 AND created_by = $2 \
             AND status = 'uploading' ORDER BY created_at, id LIMIT 500"
        )))
        .bind(workspace_id.to_string())
        .bind(user_id.to_string())
        .fetch_all(&mut *connection)
        .await?
        .iter()
        .map(upload_from_row)
        .collect::<Result<_, _>>()
        .map_err(Into::into)
    }

    /// Makes a completed upload a file of the drive. The bytes are in the bucket (the caller checked). When
    /// its folder went away meanwhile, the file goes to the top level; a name used meanwhile gets the next
    /// free one (with `OnConflict::Replace`, an existing file goes to the trash instead).
    pub async fn finish_upload(
        &self,
        workspace_id: Id,
        user_id: Id,
        blob_id: Id,
        now: TimestampMillis,
    ) -> Result<DriveNode, DriveError> {
        let mut transaction = self.database.immediate_transaction().await?;
        let upload = own_upload(&mut transaction, workspace_id, user_id, blob_id).await?;
        let parent_id = match live_folder(
            &mut transaction,
            workspace_id,
            upload.parent_id,
            Some(user_id),
        )
        .await
        {
            Ok(()) => upload.parent_id,
            // A file for a chat folder never lands where more people would see it.
            Err(DriveError::NotFound | DriveError::NotAFolder)
                if scope_of(&mut transaction, upload.parent_id)
                    .await?
                    .is_none() =>
            {
                None
            }
            Err(error) => return Err(error),
        };
        let existing = sibling(
            &mut transaction,
            workspace_id,
            parent_id,
            &upload.name,
            None,
        )
        .await?;
        let name = match (upload.on_conflict, existing) {
            (_, None) => upload.name.clone(),
            (OnConflict::Replace, Some((existing_id, DriveNodeKind::File))) => {
                trash_node(&mut transaction, existing_id, user_id, now).await?;
                upload.name.clone()
            }
            _ => free_name(&mut transaction, workspace_id, parent_id, &upload.name).await?,
        };
        let id = Id::new_v7();
        insert_node(
            &mut transaction,
            &NewNode {
                id,
                workspace_id,
                parent_id,
                kind: DriveNodeKind::File,
                name: &name,
                blob: Some((blob_id, upload.size, &upload.mime)),
                user_id,
                chat_root: None,
            },
            now,
        )
        .await?;
        sqlx::query(
            "UPDATE drive_blobs SET status = 'ready', upload_id = NULL, upload_parent_id = NULL, upload_name = NULL, \
             upload_conflict = NULL, updated_at = $1 WHERE id = $2",
        )
        .bind(now.as_millis())
        .bind(blob_id.to_string())
        .execute(&mut *transaction)
        .await?;
        if let Some(kind) = crate::previews::preview_kind(&upload.mime, &name) {
            crate::previews::queue_in(
                &mut transaction,
                workspace_id,
                crate::previews::Source::Drive(blob_id),
                kind,
                now,
            )
            .await?;
        }
        let node = load_node(&mut transaction, workspace_id, id)
            .await?
            .ok_or(DriveError::NotFound)?;
        transaction.commit().await?;
        Ok(node)
    }

    /// Stops an upload; its parts and object are deleted afterwards ([`Self::deletions`]).
    pub async fn cancel_upload(
        &self,
        workspace_id: Id,
        user_id: Id,
        blob_id: Id,
        now: TimestampMillis,
    ) -> Result<(), DriveError> {
        let mut transaction = self.database.immediate_transaction().await?;
        own_upload(&mut transaction, workspace_id, user_id, blob_id).await?;
        sqlx::query("UPDATE drive_blobs SET status = 'deleted', updated_at = $1 WHERE id = $2")
            .bind(now.as_millis())
            .bind(blob_id.to_string())
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok(())
    }

    /// Deletes trash older than [`TRASH_RETENTION_MILLIS`] and cancels uploads older than
    /// [`STALE_UPLOAD_MILLIS`], in every workspace. Returns the number of deleted nodes and cancelled uploads.
    pub async fn expire(&self, now: TimestampMillis) -> Result<(u64, u64), DriveError> {
        let mut transaction = self.database.immediate_transaction().await?;
        let nodes = purge_trees(
            &mut transaction,
            |query| {
                query.push(
                    "SELECT id, blob_id FROM drive_nodes WHERE trashed_at IS NOT NULL AND trashed_at < ",
                );
                query.push_bind(now.as_millis() - TRASH_RETENTION_MILLIS);
            },
            now,
        )
        .await?;
        let uploads = sqlx::query(
            "UPDATE drive_blobs SET status = 'deleted', updated_at = $1 WHERE status = 'uploading' AND created_at < $2",
        )
        .bind(now.as_millis())
        .bind(now.as_millis() - STALE_UPLOAD_MILLIS)
        .execute(&mut *transaction)
        .await?
        .rows_affected();
        transaction.commit().await?;
        Ok((nodes, uploads))
    }

    /// Blobs whose objects wait to be deleted from the bucket (with their thumbnail, poster and preview
    /// objects), oldest first.
    pub async fn deletions(&self, limit: i64) -> Result<Vec<BlobDeletion>, DriveError> {
        let rows = sqlx::query(
            "SELECT drive_blobs.id, drive_blobs.workspace_id, drive_blobs.r2_key, drive_blobs.upload_id, \
             drive_media.thumb_key, drive_media.poster_key FROM drive_blobs \
             LEFT JOIN drive_media ON drive_media.blob_id = drive_blobs.id \
             WHERE drive_blobs.status = 'deleted' ORDER BY drive_blobs.updated_at, drive_blobs.id LIMIT $1",
        )
        .bind(limit)
        .fetch_all(self.database.pool())
        .await?;
        let mut deletions = Vec::with_capacity(rows.len());
        for row in &rows {
            let blob_id: String = row.try_get("id")?;
            let previews: Vec<String> = sqlx::query_scalar(
                "SELECT o.object_key FROM file_previews p JOIN file_preview_outputs o ON o.preview_id = p.id \
                 WHERE p.drive_blob_id = $1 AND o.object_key IS NOT NULL",
            )
            .bind(&blob_id)
            .fetch_all(self.database.pool())
            .await?;
            deletions.push(BlobDeletion {
                blob_id: parse_id(blob_id)?,
                workspace_id: parse_id(row.try_get("workspace_id")?)?,
                r2_key: row.try_get("r2_key")?,
                upload_id: row.try_get("upload_id")?,
                extra_keys: [
                    row.try_get::<Option<String>, _>("thumb_key")?,
                    row.try_get::<Option<String>, _>("poster_key")?,
                ]
                .into_iter()
                .flatten()
                .chain(previews)
                .collect(),
            });
        }
        Ok(deletions)
    }

    /// The Drive folder of a chat conversation, made when it is first needed: `Uploads/#sales` for a channel,
    /// `Uploads/Anna, Ben` for a direct message. Its files are seen by the people who read the conversation
    /// ([`push_visible`]). Only a member of the conversation who can write in it gets the folder.
    pub async fn chat_folder(
        &self,
        workspace_id: Id,
        user_id: Id,
        conversation_id: Id,
        now: TimestampMillis,
    ) -> Result<DriveNode, DriveError> {
        let mut transaction = self.database.immediate_transaction().await?;
        require_actor(&mut transaction, workspace_id, user_id).await?;
        let conversation = sqlx::query(
            "SELECT c.kind, c.name FROM chat_conversations c \
             JOIN chat_members m ON m.conversation_id = c.id AND m.user_id = $2 \
             WHERE c.id = $1 AND c.workspace_id = $3 AND c.archived_at IS NULL",
        )
        .bind(conversation_id.to_string())
        .bind(user_id.to_string())
        .bind(workspace_id.to_string())
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(DriveError::NotFound)?;
        let folders: Vec<String> = sqlx::query_scalar(
            "SELECT id FROM drive_nodes WHERE workspace_id = $1 AND chat_root = 1 AND chat_conversation_id = $2 \
             AND trashed_at IS NULL ORDER BY created_at, id",
        )
        .bind(workspace_id.to_string())
        .bind(conversation_id.to_string())
        .fetch_all(&mut *transaction)
        .await?;
        for folder in folders {
            let folder = parse_id(folder)?;
            if live_chain(&mut transaction, workspace_id, folder, Some(user_id))
                .await
                .is_ok()
            {
                let node = load_node(&mut transaction, workspace_id, folder)
                    .await?
                    .ok_or(DriveError::NotFound)?;
                transaction.commit().await?;
                return Ok(node);
            }
        }
        let uploads = match sibling(&mut transaction, workspace_id, None, UPLOADS_FOLDER, None)
            .await?
        {
            Some((id, DriveNodeKind::Folder))
                if scope_of(&mut transaction, Some(id)).await?.is_none() =>
            {
                id
            }
            _ => {
                let name = free_name(&mut transaction, workspace_id, None, UPLOADS_FOLDER).await?;
                let id = Id::new_v7();
                insert_node(
                    &mut transaction,
                    &NewNode {
                        id,
                        workspace_id,
                        parent_id: None,
                        kind: DriveNodeKind::Folder,
                        name: &name,
                        blob: None,
                        user_id,
                        chat_root: None,
                    },
                    now,
                )
                .await?;
                id
            }
        };
        let kind: String = conversation.try_get("kind")?;
        let name = if kind == "dm" {
            let names: Vec<String> = sqlx::query_scalar(
                "SELECT u.display_name FROM chat_members m JOIN users u ON u.id = m.user_id \
                 WHERE m.conversation_id = $1 ORDER BY lower(u.display_name), u.id",
            )
            .bind(conversation_id.to_string())
            .fetch_all(&mut *transaction)
            .await?;
            names.join(", ")
        } else {
            format!("#{}", conversation.try_get::<String, _>("name")?)
        };
        let name: String = name
            .chars()
            .map(|c| {
                if c == '/' || c == '\\' || c.is_control() {
                    '-'
                } else {
                    c
                }
            })
            .take(200)
            .collect();
        let name = valid_name(&name).unwrap_or_else(|_| "Direct message".to_owned());
        let name = free_name(&mut transaction, workspace_id, Some(uploads), &name).await?;
        let id = Id::new_v7();
        insert_node(
            &mut transaction,
            &NewNode {
                id,
                workspace_id,
                parent_id: Some(uploads),
                kind: DriveNodeKind::Folder,
                name: &name,
                blob: None,
                user_id,
                chat_root: Some(conversation_id),
            },
            now,
        )
        .await?;
        let node = load_node(&mut transaction, workspace_id, id)
            .await?
            .ok_or(DriveError::NotFound)?;
        transaction.commit().await?;
        Ok(node)
    }

    /// Queues the preview of a live file that has none (a file from before previews), and returns the file.
    pub async fn request_preview(
        &self,
        workspace_id: Id,
        user_id: Id,
        node_id: Id,
        now: TimestampMillis,
    ) -> Result<DriveNode, DriveError> {
        let mut transaction = self.database.immediate_transaction().await?;
        require_actor(&mut transaction, workspace_id, user_id).await?;
        live_chain(&mut transaction, workspace_id, node_id, Some(user_id)).await?;
        let node = load_node(&mut transaction, workspace_id, node_id)
            .await?
            .ok_or(DriveError::NotFound)?;
        let blob_id: Option<String> =
            sqlx::query_scalar("SELECT blob_id FROM drive_nodes WHERE id = $1")
                .bind(node_id.to_string())
                .fetch_one(&mut *transaction)
                .await?;
        let kind = node
            .mime
            .as_deref()
            .and_then(|mime| crate::previews::preview_kind(mime, &node.name));
        if let (Some(kind), Some(blob_id)) = (kind, blob_id) {
            crate::previews::queue_in(
                &mut transaction,
                workspace_id,
                crate::previews::Source::Drive(parse_id(blob_id)?),
                kind,
                now,
            )
            .await?;
        }
        let node = load_node(&mut transaction, workspace_id, node_id)
            .await?
            .ok_or(DriveError::NotFound)?;
        transaction.commit().await?;
        Ok(node)
    }

    /// The preview output `role` of a live file the caller sees.
    pub async fn preview_output(
        &self,
        workspace_id: Id,
        user_id: Id,
        node_id: Id,
        role: &str,
    ) -> Result<crate::previews::PreviewOutput, DriveError> {
        let mut connection = self.database.pool().acquire().await?;
        require_actor(&mut connection, workspace_id, user_id).await?;
        live_chain(&mut connection, workspace_id, node_id, Some(user_id)).await?;
        let blob_id: Option<String> =
            sqlx::query_scalar("SELECT blob_id FROM drive_nodes WHERE id = $1")
                .bind(node_id.to_string())
                .fetch_one(&mut *connection)
                .await?;
        let blob_id = parse_id(blob_id.ok_or(DriveError::NotFound)?)?;
        crate::previews::output(
            &mut connection,
            crate::previews::Source::Drive(blob_id),
            role,
        )
        .await?
        .ok_or(DriveError::NotFound)
    }

    /// Drops a deleted blob whose objects are gone from the bucket.
    pub async fn forget_blob(&self, blob_id: Id) -> Result<(), DriveError> {
        let mut transaction = self.database.immediate_transaction().await?;
        sqlx::query("DELETE FROM drive_blobs WHERE id = $1 AND status = 'deleted'")
            .bind(blob_id.to_string())
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok(())
    }
}

/// The live items of a folder (`None`: the top level), folders first, then by `sort`. The caller checked that
/// the folder is live.
pub(crate) async fn list_children(
    connection: &mut AnyConnection,
    workspace_id: Id,
    parent_id: Option<Id>,
    sort: DriveSort,
    order: SortOrder,
    paging: Paging,
    visible: &Visible,
) -> Result<DriveNodePage, DriveError> {
    let column = match sort {
        DriveSort::Name => "drive_nodes.name_lower",
        DriveSort::UpdatedAt => "drive_nodes.updated_at",
        DriveSort::Size => "drive_nodes.size",
    };
    let direction = match order {
        SortOrder::Asc => "ASC",
        SortOrder::Desc => "DESC",
    };
    let mut query = SqlBuilder::new(format!("SELECT {NODE_COLUMNS} FROM {NODE_FROM} WHERE "));
    query.push("drive_nodes.workspace_id = ");
    query.push_bind(workspace_id.to_string());
    push_parent(&mut query, "drive_nodes.parent_id", parent_id);
    match visible {
        Visible::Member(user_id) => push_visible(&mut query, "drive_nodes", *user_id),
        Visible::Scope(None) => {
            query.push(" AND drive_nodes.chat_conversation_id IS NULL");
        }
        Visible::Scope(Some(scope)) => {
            query.push(" AND (drive_nodes.chat_conversation_id IS NULL OR drive_nodes.chat_conversation_id = ");
            query.push_bind(scope.clone());
            query.push(")");
        }
    }
    query.push(format!(
            " AND drive_nodes.trashed_at IS NULL ORDER BY CASE WHEN drive_nodes.kind = 'folder' THEN 0 ELSE 1 END, \
             {column} {direction}, drive_nodes.id {direction}"
        ));
    let rows = page_rows(&mut query, paging, connection).await?;
    node_page(&rows, paging)
}

/// The caller as a member of the workspace; `NotFound` for everybody else.
pub(crate) async fn require_actor(
    connection: &mut AnyConnection,
    workspace_id: Id,
    user_id: Id,
) -> Result<Actor, DriveError> {
    membership::actor(&mut *connection, workspace_id, user_id)
        .await?
        .ok_or(DriveError::NotFound)
}

/// An upload in progress that the caller started.
async fn own_upload(
    connection: &mut AnyConnection,
    workspace_id: Id,
    user_id: Id,
    blob_id: Id,
) -> Result<DriveUpload, DriveError> {
    let actor = require_actor(connection, workspace_id, user_id).await?;
    let upload = load_upload(connection, workspace_id, blob_id)
        .await?
        .ok_or(DriveError::NotFound)?;
    if actor.can_continue_drive_upload(upload.created_by) {
        Ok(upload)
    } else {
        Err(DriveError::NotFound)
    }
}

async fn load_upload(
    connection: &mut AnyConnection,
    workspace_id: Id,
    blob_id: Id,
) -> Result<Option<DriveUpload>, sqlx::Error> {
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "SELECT {UPLOAD_COLUMNS} FROM drive_blobs WHERE id = $1 AND workspace_id = $2 AND status = 'uploading'"
    )))
    .bind(blob_id.to_string())
    .bind(workspace_id.to_string())
    .fetch_optional(&mut *connection)
    .await?
    .as_ref()
    .map(upload_from_row)
    .transpose()
}

fn upload_from_row(row: &sqlx::any::AnyRow) -> Result<DriveUpload, sqlx::Error> {
    Ok(DriveUpload {
        id: parse_id(row.try_get("id")?)?,
        r2_key: row.try_get("r2_key")?,
        upload_id: row.try_get("upload_id")?,
        parent_id: optional_id(row.try_get("upload_parent_id")?)?,
        name: row
            .try_get::<Option<String>, _>("upload_name")?
            .unwrap_or_default(),
        size: row.try_get("size")?,
        mime: row.try_get("mime")?,
        on_conflict: OnConflict::parse(
            &row.try_get::<Option<String>, _>("upload_conflict")?
                .unwrap_or_default(),
        ),
        part_size: row.try_get("part_size")?,
        created_at: TimestampMillis::from_millis(row.try_get("created_at")?),
        created_by: optional_id(row.try_get("created_by")?)?,
    })
}

/// One node on the way from the top level to a node.
#[derive(Debug)]
pub(crate) struct Link {
    pub(crate) id: Id,
    pub(crate) name: String,
    pub(crate) kind: DriveNodeKind,
    trashed: bool,
    /// The chat conversation whose readers see the node ([`push_visible`]).
    pub(crate) scope: Option<String>,
}

/// The node and the folders above it, from the top level down; empty when the node is not in the workspace, or
/// when `viewer` (a member; `None`: no person, as for a public share link) cannot see it.
async fn chain(
    connection: &mut AnyConnection,
    workspace_id: Id,
    node_id: Id,
    viewer: Option<Id>,
) -> Result<Vec<Link>, DriveError> {
    let rows = sqlx::query(
        "WITH RECURSIVE chain(id, parent_id, name, kind, trashed_at, scope, depth) AS ( \
             SELECT id, parent_id, name, kind, trashed_at, chat_conversation_id, 0 FROM drive_nodes \
             WHERE id = $1 AND workspace_id = $2 \
             UNION ALL \
             SELECT drive_nodes.id, drive_nodes.parent_id, drive_nodes.name, drive_nodes.kind, \
             drive_nodes.trashed_at, drive_nodes.chat_conversation_id, chain.depth + 1 FROM drive_nodes \
             JOIN chain ON drive_nodes.id = chain.parent_id WHERE chain.depth < $3) \
         SELECT id, name, kind, trashed_at, scope FROM chain ORDER BY depth DESC",
    )
    .bind(node_id.to_string())
    .bind(workspace_id.to_string())
    .bind(MAX_DEPTH)
    .fetch_all(&mut *connection)
    .await?;
    let links = rows
        .iter()
        .map(|row| {
            Ok(Link {
                id: parse_id(row.try_get("id")?)?,
                name: row.try_get("name")?,
                kind: parse_kind(&row.try_get::<String, _>("kind")?)?,
                trashed: row.try_get::<Option<i64>, _>("trashed_at")?.is_some(),
                scope: row.try_get("scope")?,
            })
        })
        .collect::<Result<Vec<_>, DriveError>>()?;
    if let Some(viewer) = viewer {
        let mut scopes: Vec<&str> = links
            .iter()
            .filter_map(|link| link.scope.as_deref())
            .collect();
        scopes.dedup();
        for scope in scopes {
            if !conversation_readable(connection, scope, viewer).await? {
                return Ok(Vec::new());
            }
        }
    }
    Ok(links)
}

/// Whether `user_id` (a member of the conversation's workspace, not a guest) reads the conversation.
async fn conversation_readable(
    connection: &mut AnyConnection,
    conversation_id: &str,
    user_id: Id,
) -> Result<bool, DriveError> {
    let readable: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM chat_conversations c WHERE c.id = $1 AND (c.kind = 'public' \
         OR EXISTS (SELECT 1 FROM chat_members m WHERE m.conversation_id = c.id AND m.user_id = $2))",
    )
    .bind(conversation_id)
    .bind(user_id.to_string())
    .fetch_one(&mut *connection)
    .await?;
    Ok(readable > 0)
}

/// Who a listing is for.
pub(crate) enum Visible {
    /// A member: nodes of chat conversations they do not read are left out.
    Member(Id),
    /// A public link: only nodes of the shared node's own scope, never a chat folder nested below it.
    Scope(Option<String>),
}

/// Keeps the nodes (`alias`) that `user_id` can see: those outside chat folders, and those of conversations the
/// user reads.
fn push_visible(query: &mut SqlBuilder, alias: &str, user_id: Id) {
    query.push(format!(
        " AND ({alias}.chat_conversation_id IS NULL OR EXISTS (SELECT 1 FROM chat_conversations vc \
         WHERE vc.id = {alias}.chat_conversation_id AND (vc.kind = 'public' OR EXISTS (SELECT 1 FROM chat_members vm \
         WHERE vm.conversation_id = vc.id AND vm.user_id = "
    ));
    query.push_bind(user_id.to_string());
    query.push("))))");
}

/// As [`chain`], for a live node only.
pub(crate) async fn live_chain(
    connection: &mut AnyConnection,
    workspace_id: Id,
    node_id: Id,
    viewer: Option<Id>,
) -> Result<Vec<Link>, DriveError> {
    let chain = chain(connection, workspace_id, node_id, viewer).await?;
    if chain.is_empty() || chain.iter().any(|link| link.trashed) {
        return Err(DriveError::NotFound);
    }
    Ok(chain)
}

/// `None` (the top level) or a live folder that `viewer` sees.
async fn live_folder(
    connection: &mut AnyConnection,
    workspace_id: Id,
    folder_id: Option<Id>,
    viewer: Option<Id>,
) -> Result<(), DriveError> {
    let Some(folder_id) = folder_id else {
        return Ok(());
    };
    let chain = live_chain(connection, workspace_id, folder_id, viewer).await?;
    if chain[chain.len() - 1].kind == DriveNodeKind::Folder {
        Ok(())
    } else {
        Err(DriveError::NotAFolder)
    }
}

/// The id and kind of the live node with this name in the folder, other than `except`.
pub(crate) async fn sibling(
    connection: &mut AnyConnection,
    workspace_id: Id,
    parent_id: Option<Id>,
    name: &str,
    except: Option<Id>,
) -> Result<Option<(Id, DriveNodeKind)>, DriveError> {
    let mut query = SqlBuilder::new("SELECT id, kind FROM drive_nodes WHERE workspace_id = ");
    query.push_bind(workspace_id.to_string());
    push_parent(&mut query, "parent_id", parent_id);
    query.push(" AND trashed_at IS NULL AND name_lower = ");
    query.push_bind(name.to_lowercase());
    if let Some(except) = except {
        query.push(" AND id <> ");
        query.push_bind(except.to_string());
    }
    let row = query.build().fetch_optional(&mut *connection).await?;
    Ok(row
        .map(|row| {
            Ok::<_, sqlx::Error>((
                parse_id(row.try_get("id")?)?,
                parse_kind(&row.try_get::<String, _>("kind")?)?,
            ))
        })
        .transpose()?)
}

/// `name`, or the first free `name (n)` (before the extension) in the folder.
pub(crate) async fn free_name(
    connection: &mut AnyConnection,
    workspace_id: Id,
    parent_id: Option<Id>,
    name: &str,
) -> Result<String, DriveError> {
    if sibling(connection, workspace_id, parent_id, name, None)
        .await?
        .is_none()
    {
        return Ok(name.to_owned());
    }
    let (stem, extension) = match name.rfind('.') {
        Some(dot) if dot > 0 => name.split_at(dot),
        _ => (name, ""),
    };
    for number in 2..=MAX_NAME_SUFFIX {
        let suffix = format!(" ({number}){extension}");
        let keep = MAX_NAME_CHARS.saturating_sub(suffix.chars().count());
        let candidate: String = stem.chars().take(keep).collect::<String>() + &suffix;
        if sibling(connection, workspace_id, parent_id, &candidate, None)
            .await?
            .is_none()
        {
            return Ok(candidate);
        }
    }
    Err(DriveError::NameTaken)
}

struct NewNode<'a> {
    id: Id,
    workspace_id: Id,
    parent_id: Option<Id>,
    kind: DriveNodeKind,
    name: &'a str,
    /// Files: the blob, its size and MIME type.
    blob: Option<(Id, i64, &'a str)>,
    user_id: Id,
    /// The folder of this chat conversation; every other node takes its parent's scope.
    chat_root: Option<Id>,
}

async fn insert_node(
    connection: &mut AnyConnection,
    node: &NewNode<'_>,
    now: TimestampMillis,
) -> Result<(), DriveError> {
    sqlx::query(
        "INSERT INTO drive_nodes (id, workspace_id, parent_id, kind, name, name_lower, blob_id, size, mime, \
         created_by, created_at, updated_by, updated_at, chat_conversation_id, chat_root) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $10, $11, \
         COALESCE($12, (SELECT p.chat_conversation_id FROM drive_nodes p WHERE p.id = $3)), $13)",
    )
    .bind(node.id.to_string())
    .bind(node.workspace_id.to_string())
    .bind(node.parent_id.map(|id| id.to_string()))
    .bind(node.kind.as_str())
    .bind(node.name)
    .bind(node.name.to_lowercase())
    .bind(node.blob.map(|(blob_id, _, _)| blob_id.to_string()))
    .bind(node.blob.map(|(_, size, _)| size))
    .bind(node.blob.map(|(_, _, mime)| mime.to_owned()))
    .bind(node.user_id.to_string())
    .bind(now.as_millis())
    .bind(node.chat_root.map(|id| id.to_string()))
    .bind(i64::from(node.chat_root.is_some()))
    .execute(&mut *connection)
    .await?;
    Ok(())
}

/// The scope of a node (any state), or `None` for the top level.
pub(crate) async fn scope_of(
    connection: &mut AnyConnection,
    node_id: Option<Id>,
) -> Result<Option<String>, DriveError> {
    let Some(node_id) = node_id else {
        return Ok(None);
    };
    Ok(sqlx::query_scalar::<_, Option<String>>(
        "SELECT chat_conversation_id FROM drive_nodes WHERE id = $1",
    )
    .bind(node_id.to_string())
    .fetch_optional(&mut *connection)
    .await?
    .flatten())
}

/// Gives a moved or restored node, and everything below it, the scope of its new place: a chat folder keeps its
/// own conversation, every other node takes its parent's.
async fn rescope(connection: &mut AnyConnection, node_id: Id) -> Result<(), DriveError> {
    let rows = sqlx::query(
        "WITH RECURSIVE tree(id, scope, depth) AS ( \
             SELECT n.id, CASE WHEN n.chat_root = 1 THEN n.chat_conversation_id \
             ELSE (SELECT p.chat_conversation_id FROM drive_nodes p WHERE p.id = n.parent_id) END, 0 \
             FROM drive_nodes n WHERE n.id = $1 \
             UNION ALL \
             SELECT c.id, CASE WHEN c.chat_root = 1 THEN c.chat_conversation_id ELSE tree.scope END, tree.depth + 1 \
             FROM drive_nodes c JOIN tree ON c.parent_id = tree.id WHERE tree.depth < $2) \
         SELECT tree.id, tree.scope FROM tree JOIN drive_nodes n ON n.id = tree.id \
         WHERE n.chat_conversation_id IS DISTINCT FROM tree.scope",
    )
    .bind(node_id.to_string())
    .bind(MAX_DEPTH)
    .fetch_all(&mut *connection)
    .await?;
    for row in rows {
        sqlx::query("UPDATE drive_nodes SET chat_conversation_id = $1 WHERE id = $2")
            .bind(row.try_get::<Option<String>, _>("scope")?)
            .bind(row.try_get::<String, _>("id")?)
            .execute(&mut *connection)
            .await?;
    }
    Ok(())
}

async fn trash_node(
    connection: &mut AnyConnection,
    node_id: Id,
    user_id: Id,
    now: TimestampMillis,
) -> Result<(), DriveError> {
    sqlx::query("UPDATE drive_nodes SET trashed_at = $1, trashed_by = $2 WHERE id = $3")
        .bind(now.as_millis())
        .bind(user_id.to_string())
        .bind(node_id.to_string())
        .execute(&mut *connection)
        .await?;
    Ok(())
}

/// Deletes the trees under the nodes that `seed` selects (it pushes `SELECT id, blob_id …`) and marks their
/// blobs for deletion from the bucket. Returns the number of deleted nodes.
async fn purge_trees(
    connection: &mut AnyConnection,
    seed: impl FnOnce(&mut SqlBuilder),
    now: TimestampMillis,
) -> Result<u64, DriveError> {
    let mut query = SqlBuilder::new(
        "WITH RECURSIVE tree(id, blob_id, depth) AS (SELECT seed.id, seed.blob_id, 0 FROM (",
    );
    seed(&mut query);
    query.push(
        ") seed UNION ALL SELECT drive_nodes.id, drive_nodes.blob_id, tree.depth + 1 FROM drive_nodes \
         JOIN tree ON drive_nodes.parent_id = tree.id WHERE tree.depth < ",
    );
    query.push(MAX_DEPTH);
    // A trashed item inside another one is reached twice.
    query.push(") SELECT DISTINCT id, blob_id FROM tree");
    let rows = query.build().fetch_all(&mut *connection).await?;
    let mut node_ids = Vec::with_capacity(rows.len());
    let mut blob_ids = Vec::new();
    for row in &rows {
        node_ids.push(row.try_get::<String, _>("id")?);
        if let Some(blob_id) = row.try_get::<Option<String>, _>("blob_id")? {
            blob_ids.push(blob_id);
        }
    }
    for chunk in node_ids.chunks(500) {
        let mut delete = SqlBuilder::new("DELETE FROM drive_nodes WHERE id IN (");
        let mut list = delete.separated(", ");
        for id in chunk {
            list.push_bind(id.clone());
        }
        delete.push(")");
        delete.build().execute(&mut *connection).await?;
    }
    for chunk in blob_ids.chunks(500) {
        let mut update =
            SqlBuilder::new("UPDATE drive_blobs SET status = 'deleted', updated_at = ");
        update.push_bind(now.as_millis());
        update.push(" WHERE id IN (");
        let mut list = update.separated(", ");
        for id in chunk {
            list.push_bind(id.clone());
        }
        update.push(")");
        update.build().execute(&mut *connection).await?;
    }
    Ok(node_ids.len() as u64)
}

/// The rest of a `WITH RECURSIVE live(id, folder_path, depth)` walk down the live tree, after its seed.
fn push_live_walk(query: &mut SqlBuilder) {
    query.push(
        " UNION ALL SELECT drive_nodes.id, \
         CASE WHEN live.folder_path = '' THEN parent.name ELSE live.folder_path || '/' || parent.name END, \
         live.depth + 1 FROM drive_nodes JOIN live ON drive_nodes.parent_id = live.id \
         JOIN drive_nodes parent ON parent.id = live.id \
         WHERE drive_nodes.trashed_at IS NULL AND live.depth < ",
    );
    query.push(MAX_DEPTH);
    query.push(")");
}

fn push_parent(query: &mut SqlBuilder, column: &str, parent_id: Option<Id>) {
    match parent_id {
        None => {
            query.push(format!(" AND {column} IS NULL"));
        }
        Some(parent_id) => {
            query.push(format!(" AND {column} = "));
            query.push_bind(parent_id.to_string());
        }
    }
}

/// Runs the query for one page (one row more, to know whether another page follows).
async fn page_rows(
    query: &mut SqlBuilder,
    paging: Paging,
    connection: &mut AnyConnection,
) -> Result<Vec<sqlx::any::AnyRow>, DriveError> {
    query.push(" LIMIT ");
    query.push_bind(paging.limit + 1);
    query.push(" OFFSET ");
    query.push_bind(paging.offset);
    Ok(query.build().fetch_all(&mut *connection).await?)
}

fn next_offset(rows: usize, paging: Paging) -> Option<i64> {
    (rows as i64 > paging.limit).then_some(paging.offset + paging.limit)
}

fn node_page(rows: &[sqlx::any::AnyRow], paging: Paging) -> Result<DriveNodePage, DriveError> {
    Ok(DriveNodePage {
        items: rows
            .iter()
            .take(paging.limit as usize)
            .map(node_from_row)
            .collect::<Result<_, _>>()?,
        next_offset: next_offset(rows.len(), paging),
    })
}

fn entry_page(rows: &[sqlx::any::AnyRow], paging: Paging) -> Result<DriveEntryPage, DriveError> {
    Ok(DriveEntryPage {
        items: rows
            .iter()
            .take(paging.limit as usize)
            .map(|row| {
                Ok(DriveEntry {
                    node: node_from_row(row)?,
                    folder_path: row.try_get("folder_path")?,
                })
            })
            .collect::<Result<_, sqlx::Error>>()?,
        next_offset: next_offset(rows.len(), paging),
    })
}

pub(crate) async fn load_node(
    connection: &mut AnyConnection,
    workspace_id: Id,
    node_id: Id,
) -> Result<Option<DriveNode>, sqlx::Error> {
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "SELECT {NODE_COLUMNS} FROM {NODE_FROM} WHERE drive_nodes.id = $1 AND drive_nodes.workspace_id = $2"
    )))
    .bind(node_id.to_string())
    .bind(workspace_id.to_string())
    .fetch_optional(&mut *connection)
    .await?
    .as_ref()
    .map(node_from_row)
    .transpose()
}

fn node_from_row(row: &sqlx::any::AnyRow) -> Result<DriveNode, sqlx::Error> {
    Ok(DriveNode {
        id: parse_id(row.try_get("id")?)?,
        parent_id: optional_id(row.try_get("parent_id")?)?,
        kind: parse_kind(&row.try_get::<String, _>("kind")?)?,
        name: row.try_get("name")?,
        size: row.try_get("size")?,
        mime: row.try_get("mime")?,
        created_by: optional_id(row.try_get("created_by")?)?,
        created_at: TimestampMillis::from_millis(row.try_get("created_at")?),
        updated_by: optional_id(row.try_get("updated_by")?)?,
        updated_at: TimestampMillis::from_millis(row.try_get("updated_at")?),
        trashed_at: row
            .try_get::<Option<i64>, _>("trashed_at")?
            .map(TimestampMillis::from_millis),
        media: row
            .try_get::<Option<String>, _>("media_status")?
            .map(|status| {
                Ok::<_, sqlx::Error>(DriveMedia {
                    width: row.try_get("width")?,
                    height: row.try_get("height")?,
                    duration_ms: row.try_get("duration_ms")?,
                    status: match status.as_str() {
                        "ready" => DriveMediaStatus::Ready,
                        "failed" => DriveMediaStatus::Failed,
                        _ => DriveMediaStatus::Pending,
                    },
                })
            })
            .transpose()?,
        thumbnail_url: None,
        poster_url: None,
        preview: match (
            row.try_get::<Option<String>, _>("mime")?,
            row.try_get::<String, _>("workspace_id"),
        ) {
            (Some(mime), Ok(workspace_id)) => crate::previews::preview_from_row(
                row,
                &mime,
                &row.try_get::<String, _>("name")?,
                &format!(
                    "/api/v1/workspaces/{workspace_id}/drive/nodes/{}/preview",
                    row.try_get::<String, _>("id")?
                ),
            )?,
            _ => None,
        },
        thumb_key: row.try_get("thumb_key")?,
        poster_key: row.try_get("poster_key")?,
    })
}

pub(crate) fn parse_id(value: String) -> Result<Id, sqlx::Error> {
    value
        .parse()
        .map_err(|error| sqlx::Error::Decode(Box::new(error)))
}

pub(crate) fn optional_id(value: Option<String>) -> Result<Option<Id>, sqlx::Error> {
    value.map(parse_id).transpose()
}

fn parse_kind(value: &str) -> Result<DriveNodeKind, sqlx::Error> {
    match value {
        "folder" => Ok(DriveNodeKind::Folder),
        "file" => Ok(DriveNodeKind::File),
        other => Err(sqlx::Error::Decode(
            format!("unknown drive node kind {other}").into(),
        )),
    }
}

/// The name without surrounding spaces, when it can be a file name: 1 to 255 characters, no `/`, `\` or
/// control characters, not `.` or `..`.
pub fn valid_name(name: &str) -> Result<String, DriveError> {
    let name = name.trim();
    let length = name.chars().count();
    if length == 0
        || length > MAX_NAME_CHARS
        || name == "."
        || name == ".."
        || name
            .chars()
            .any(|c| c == '/' || c == '\\' || c.is_control())
    {
        return Err(DriveError::InvalidName);
    }
    Ok(name.to_owned())
}

/// Escapes `%`, `_` and `\` for `LIKE … ESCAPE '\'`.
fn like_escape(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_trimmed_and_cannot_be_paths_or_control_text() {
        assert_eq!(valid_name("  Q1 Bericht.pdf ").unwrap(), "Q1 Bericht.pdf");
        for bad in [
            "",
            "   ",
            ".",
            "..",
            "a/b",
            "a\\b",
            "tab\there",
            &"x".repeat(256),
        ] {
            assert!(
                matches!(valid_name(bad), Err(DriveError::InvalidName)),
                "{bad:?}"
            );
        }
        assert!(valid_name(&"ä".repeat(255)).is_ok());
    }

    #[test]
    fn part_counts_round_up_and_a_single_put_is_one_part() {
        let upload = |size, part_size| DriveUpload {
            id: Id::new_v7(),
            r2_key: String::new(),
            upload_id: None,
            parent_id: None,
            name: "f".to_owned(),
            size,
            mime: "x/y".to_owned(),
            on_conflict: OnConflict::Fail,
            part_size,
            created_at: TimestampMillis::from_millis(0),
            created_by: None,
        };
        assert_eq!(upload(100, None).part_count(), 1);
        assert_eq!(upload(32, Some(16)).part_count(), 2);
        assert_eq!(upload(33, Some(16)).part_count(), 3);
        assert_eq!(upload(0, Some(16)).part_count(), 1);
    }
}
