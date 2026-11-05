//! Public links to one Drive file or folder (`/s/<token>`). Only the token's SHA-256 is stored. A link works
//! while it is not revoked, not expired, and its file or folder is live; a folder link reaches only what is
//! below that folder. Members make and see links; the creator, or a member with `drive.shares.manage`, turns
//! one off.

use orbit_domain::Actor;
use orbit_platform::{Database, Id, Row, TimestampMillis};
use serde::Serialize;
use sqlx::AnyConnection;
use utoipa::ToSchema;

use crate::audit::{self, AuditOutcome};
use crate::repositories::drive::{
    DriveCrumb, DriveError, DriveNode, DriveNodeKind, DriveNodePage, DriveObject, DriveSort,
    Paging, SortOrder, list_children, live_chain, load_node, optional_id, parse_id, require_actor,
};

/// A public link as members see it (the token is shown once, when the link is made).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, ToSchema)]
pub struct DriveShare {
    #[schema(value_type = String)]
    pub id: Id,
    #[schema(value_type = String)]
    pub node_id: Id,
    #[schema(value_type = Option<String>)]
    pub created_by: Option<Id>,
    #[schema(value_type = String, format = DateTime)]
    pub created_at: TimestampMillis,
    #[schema(value_type = Option<String>, format = DateTime)]
    pub expires_at: Option<TimestampMillis>,
    pub has_password: bool,
    pub download_count: i64,
    #[schema(value_type = Option<String>, format = DateTime)]
    pub revoked_at: Option<TimestampMillis>,
    /// The caller may turn the link off.
    pub can_revoke: bool,
}

/// A link that works now, found by its token.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShareAccess {
    pub share_id: Id,
    pub workspace_id: Id,
    pub node_id: Id,
    /// Argon2 hash; `None` when the link has no password.
    pub password_hash: Option<String>,
    pub expires_at: Option<TimestampMillis>,
}

const SHARE_COLUMNS: &str =
    "id, node_id, created_by, created_at, expires_at, password_hash, download_count, revoked_at";

#[derive(Clone)]
pub struct DriveShareRepository {
    database: Database,
}

impl DriveShareRepository {
    #[must_use]
    pub const fn new(database: Database) -> Self {
        Self { database }
    }

    /// Makes a link to a live file or folder.
    #[allow(clippy::too_many_arguments)]
    pub async fn create(
        &self,
        workspace_id: Id,
        user_id: Id,
        node_id: Id,
        token_hash: &str,
        expires_at: Option<TimestampMillis>,
        password_hash: Option<String>,
        request_id: &str,
        now: TimestampMillis,
    ) -> Result<DriveShare, DriveError> {
        let mut transaction = self.database.immediate_transaction().await?;
        let actor = require_actor(&mut transaction, workspace_id, user_id).await?;
        live_chain(&mut transaction, workspace_id, node_id).await?;
        let id = Id::new_v7();
        sqlx::query(
            "INSERT INTO drive_shares (id, workspace_id, node_id, token_hash, created_by, created_at, expires_at, \
             password_hash) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(id.to_string())
        .bind(workspace_id.to_string())
        .bind(node_id.to_string())
        .bind(token_hash)
        .bind(user_id.to_string())
        .bind(now.as_millis())
        .bind(expires_at.map(TimestampMillis::as_millis))
        .bind(password_hash)
        .execute(&mut *transaction)
        .await?;
        audit::record(
            &mut transaction,
            workspace_id,
            Some(user_id),
            "drive.share_created",
            AuditOutcome::Success,
            "drive_node",
            Some(node_id),
            request_id,
            serde_json::json!({"share_id": id.to_string(), "expires_at": expires_at.map(TimestampMillis::as_millis)}),
            now,
        )
        .await?;
        let share = load_share(&mut transaction, workspace_id, id, actor)
            .await?
            .ok_or(DriveError::NotFound)?;
        transaction.commit().await?;
        Ok(share)
    }

    /// The links of a live file or folder, newest first, also revoked and expired ones.
    pub async fn list(
        &self,
        workspace_id: Id,
        user_id: Id,
        node_id: Id,
    ) -> Result<Vec<DriveShare>, DriveError> {
        let mut connection = self.database.pool().acquire().await?;
        let actor = require_actor(&mut connection, workspace_id, user_id).await?;
        live_chain(&mut connection, workspace_id, node_id).await?;
        let rows = sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT {SHARE_COLUMNS} FROM drive_shares WHERE workspace_id = $1 AND node_id = $2 \
             ORDER BY created_at DESC, id DESC LIMIT 200"
        )))
        .bind(workspace_id.to_string())
        .bind(node_id.to_string())
        .fetch_all(&mut *connection)
        .await?;
        rows.iter()
            .map(|row| share_from_row(row, actor).map_err(Into::into))
            .collect()
    }

    /// Turns a link off at once. Turning off a link that is already off changes nothing.
    pub async fn revoke(
        &self,
        workspace_id: Id,
        user_id: Id,
        share_id: Id,
        request_id: &str,
        now: TimestampMillis,
    ) -> Result<DriveShare, DriveError> {
        let mut transaction = self.database.immediate_transaction().await?;
        let actor = require_actor(&mut transaction, workspace_id, user_id).await?;
        let share = load_share(&mut transaction, workspace_id, share_id, actor)
            .await?
            .ok_or(DriveError::NotFound)?;
        if !share.can_revoke {
            return Err(DriveError::Forbidden);
        }
        if share.revoked_at.is_none() {
            sqlx::query("UPDATE drive_shares SET revoked_at = $1 WHERE id = $2")
                .bind(now.as_millis())
                .bind(share_id.to_string())
                .execute(&mut *transaction)
                .await?;
            audit::record(
                &mut transaction,
                workspace_id,
                Some(user_id),
                "drive.share_revoked",
                AuditOutcome::Success,
                "drive_node",
                Some(share.node_id),
                request_id,
                serde_json::json!({"share_id": share_id.to_string()}),
                now,
            )
            .await?;
        }
        let share = load_share(&mut transaction, workspace_id, share_id, actor)
            .await?
            .ok_or(DriveError::NotFound)?;
        transaction.commit().await?;
        Ok(share)
    }

    /// The link with this token hash when it works now: not revoked, not expired, its node live.
    pub async fn resolve(
        &self,
        token_hash: &str,
        now: TimestampMillis,
    ) -> Result<Option<ShareAccess>, DriveError> {
        let mut connection = self.database.pool().acquire().await?;
        let Some(row) = sqlx::query(
            "SELECT id, workspace_id, node_id, password_hash, expires_at FROM drive_shares \
             WHERE token_hash = $1 AND revoked_at IS NULL AND (expires_at IS NULL OR expires_at > $2)",
        )
        .bind(token_hash)
        .bind(now.as_millis())
        .fetch_optional(&mut *connection)
        .await?
        else {
            return Ok(None);
        };
        let access = ShareAccess {
            share_id: parse_id(row.try_get("id")?)?,
            workspace_id: parse_id(row.try_get("workspace_id")?)?,
            node_id: parse_id(row.try_get("node_id")?)?,
            password_hash: row.try_get("password_hash")?,
            expires_at: row
                .try_get::<Option<i64>, _>("expires_at")?
                .map(TimestampMillis::from_millis),
        };
        match live_chain(&mut connection, access.workspace_id, access.node_id).await {
            Ok(_) => Ok(Some(access)),
            Err(DriveError::NotFound) => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// The shared file or folder itself.
    pub async fn shared_node(&self, access: &ShareAccess) -> Result<DriveNode, DriveError> {
        let mut connection = self.database.pool().acquire().await?;
        load_node(&mut connection, access.workspace_id, access.node_id)
            .await?
            .ok_or(DriveError::NotFound)
    }

    /// A folder of a folder link (`None`: the shared folder) and the folders from the shared one down to it,
    /// with its items by name.
    pub async fn folder(
        &self,
        access: &ShareAccess,
        folder_id: Option<Id>,
        paging: Paging,
    ) -> Result<(Vec<DriveCrumb>, DriveNodePage), DriveError> {
        let mut connection = self.database.pool().acquire().await?;
        let folder_id = folder_id.unwrap_or(access.node_id);
        let path = below_share(&mut connection, access, folder_id, DriveNodeKind::Folder).await?;
        let page = list_children(
            &mut connection,
            access.workspace_id,
            Some(folder_id),
            DriveSort::Name,
            SortOrder::Asc,
            paging,
        )
        .await?;
        Ok((path, page))
    }

    /// Where the bytes of a live file of the link are: the shared file, or a file below the shared folder.
    pub async fn object(
        &self,
        access: &ShareAccess,
        node_id: Id,
    ) -> Result<DriveObject, DriveError> {
        let mut connection = self.database.pool().acquire().await?;
        below_share(&mut connection, access, node_id, DriveNodeKind::File).await?;
        let row = sqlx::query(
            "SELECT drive_nodes.name, drive_blobs.r2_key, drive_blobs.mime FROM drive_nodes \
             JOIN drive_blobs ON drive_blobs.id = drive_nodes.blob_id \
             WHERE drive_nodes.id = $1 AND drive_blobs.status = 'ready'",
        )
        .bind(node_id.to_string())
        .fetch_optional(&mut *connection)
        .await?
        .ok_or(DriveError::NotFound)?;
        Ok(DriveObject {
            node_id,
            r2_key: row.try_get("r2_key")?,
            name: row.try_get("name")?,
            mime: row.try_get("mime")?,
        })
    }

    /// Counts a download of the link.
    pub async fn count_download(&self, share_id: Id) -> Result<(), DriveError> {
        let mut transaction = self.database.immediate_transaction().await?;
        sqlx::query("UPDATE drive_shares SET download_count = download_count + 1 WHERE id = $1")
            .bind(share_id.to_string())
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok(())
    }
}

/// The folders from the shared node down to `node_id` (both included) when `node_id` is a live node of `kind`
/// at or below the shared node; `NotFound` otherwise, so nothing outside the link can be named.
async fn below_share(
    connection: &mut AnyConnection,
    access: &ShareAccess,
    node_id: Id,
    kind: DriveNodeKind,
) -> Result<Vec<DriveCrumb>, DriveError> {
    let chain = live_chain(connection, access.workspace_id, node_id).await?;
    let start = chain
        .iter()
        .position(|link| link.id == access.node_id)
        .ok_or(DriveError::NotFound)?;
    if chain[chain.len() - 1].kind != kind {
        return Err(DriveError::NotFound);
    }
    Ok(chain[start..]
        .iter()
        .map(|link| DriveCrumb {
            id: link.id,
            name: link.name.clone(),
        })
        .collect())
}

async fn load_share(
    connection: &mut AnyConnection,
    workspace_id: Id,
    share_id: Id,
    actor: Actor,
) -> Result<Option<DriveShare>, sqlx::Error> {
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "SELECT {SHARE_COLUMNS} FROM drive_shares WHERE id = $1 AND workspace_id = $2"
    )))
    .bind(share_id.to_string())
    .bind(workspace_id.to_string())
    .fetch_optional(&mut *connection)
    .await?
    .as_ref()
    .map(|row| share_from_row(row, actor))
    .transpose()
}

fn share_from_row(row: &sqlx::any::AnyRow, actor: Actor) -> Result<DriveShare, sqlx::Error> {
    let created_by = optional_id(row.try_get("created_by")?)?;
    Ok(DriveShare {
        id: parse_id(row.try_get("id")?)?,
        node_id: parse_id(row.try_get("node_id")?)?,
        created_by,
        created_at: TimestampMillis::from_millis(row.try_get("created_at")?),
        expires_at: row
            .try_get::<Option<i64>, _>("expires_at")?
            .map(TimestampMillis::from_millis),
        has_password: row.try_get::<Option<String>, _>("password_hash")?.is_some(),
        download_count: row.try_get("download_count")?,
        revoked_at: row
            .try_get::<Option<i64>, _>("revoked_at")?
            .map(TimestampMillis::from_millis),
        can_revoke: actor.can_revoke_drive_share(created_by),
    })
}
