//! Drive files in Docs pages. A `driveFile` block holds a reference (the Drive's workspace id and the node id),
//! never a copy, so a page always shows the file's current version. Every read resolves the references again for
//! the reader: a reader who has no Drive access to the file's workspace gets [`DriveRefState::Locked`] (no name,
//! no URL), and a file in the trash, deleted or not a file gets [`DriveRefState::Unavailable`].
//!
//! Drive has no rights per file: Drive access is a (non-guest) membership of the workspace
//! ([`super::drive::require_actor`]).

use std::collections::HashMap;

use orbit_platform::{Database, Id};
use serde_json::{Value, json};
use sqlx::AnyConnection;

use super::drive::{DriveNode, DriveNodeKind, live_chain, load_node};
use super::membership;
use super::pages::{PageError, PageRecord, PageRepository, revision, visible};

/// The block type of a Drive file reference (`packages/editor/src/schema.ts`).
pub const DRIVE_FILE_BLOCK: &str = "driveFile";
/// A page write that loses a race with co-editing is tried again this often.
const WRITE_ATTEMPTS: usize = 3;

/// One `driveFile` block of a page.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DriveRef {
    pub block_id: String,
    pub workspace_id: String,
    pub node_id: String,
    /// The block's index among the page's top-level blocks; `None` for a nested block.
    pub position: Option<usize>,
}

/// What one reader may see of a reference.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DriveRefState {
    /// A live file the reader has Drive access to.
    Available(Box<DriveNode>),
    /// The reader has no Drive access to the file's workspace: nothing about the file is shown.
    Locked,
    /// In the trash, deleted forever, a folder, or an invalid reference.
    Unavailable,
}

/// The `driveFile` blocks of `blocks`, nested blocks included, in document order.
#[must_use]
pub fn drive_refs(blocks: &[Value]) -> Vec<DriveRef> {
    fn visit(blocks: &[Value], top: bool, out: &mut Vec<DriveRef>) {
        for (index, block) in blocks.iter().enumerate() {
            if block["type"] == DRIVE_FILE_BLOCK {
                let prop = |name: &str| block["props"][name].as_str().unwrap_or("").to_owned();
                out.push(DriveRef {
                    block_id: block["id"].as_str().unwrap_or("").to_owned(),
                    workspace_id: prop("workspaceId"),
                    node_id: prop("nodeId"),
                    position: top.then_some(index),
                });
            }
            if let Some(children) = block["children"].as_array() {
                visit(children, false, out);
            }
        }
    }
    let mut out = Vec::new();
    visit(blocks, true, &mut out);
    out
}

/// A new `driveFile` block.
#[must_use]
pub fn drive_file_block(workspace_id: Id, node_id: Id) -> Value {
    json!({
        "id": Id::new_v7().to_string(),
        "type": DRIVE_FILE_BLOCK,
        "props": {"workspaceId": workspace_id.to_string(), "nodeId": node_id.to_string()},
        "children": [],
    })
}

/// What `user_id` may see of the file `node_id` of the Drive of `workspace_id`. Checked on every call.
pub async fn resolve(
    connection: &mut AnyConnection,
    user_id: Id,
    workspace_id: &str,
    node_id: &str,
) -> Result<DriveRefState, sqlx::Error> {
    let (Ok(workspace_id), Ok(node_id)) = (workspace_id.parse::<Id>(), node_id.parse::<Id>())
    else {
        return Ok(DriveRefState::Unavailable);
    };
    // Membership first: a reader without Drive access learns nothing, not even whether the file exists.
    if membership::actor(&mut *connection, workspace_id, user_id)
        .await?
        .is_none()
    {
        return Ok(DriveRefState::Locked);
    }
    let chain = match live_chain(connection, workspace_id, node_id).await {
        Ok(chain) => chain,
        Err(super::drive::DriveError::Database(error)) => return Err(error),
        Err(_) => return Ok(DriveRefState::Unavailable),
    };
    if chain.last().map(|link| link.kind) != Some(DriveNodeKind::File) {
        return Ok(DriveRefState::Unavailable);
    }
    Ok(load_node(connection, workspace_id, node_id)
        .await?
        .map_or(DriveRefState::Unavailable, |node| {
            DriveRefState::Available(Box::new(node))
        }))
}

/// [`resolve`] for every distinct reference of `refs`, by `(workspace_id, node_id)`.
pub async fn resolve_all<'a>(
    connection: &mut AnyConnection,
    user_id: Id,
    refs: impl IntoIterator<Item = &'a DriveRef>,
) -> Result<HashMap<(String, String), DriveRefState>, sqlx::Error> {
    let mut states = HashMap::new();
    for reference in refs {
        let key = (reference.workspace_id.clone(), reference.node_id.clone());
        if states.contains_key(&key) {
            continue;
        }
        let state = resolve(connection, user_id, &key.0, &key.1).await?;
        states.insert(key, state);
    }
    Ok(states)
}

/// A page that shows a Drive file.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PageUse {
    pub page_id: Id,
    pub title: String,
    /// How many `driveFile` blocks of the page show the file.
    pub blocks: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum DriveRefError {
    /// No live Drive file with this id that the caller can see.
    #[error("Drive file was not found")]
    FileNotFound,
    /// No `driveFile` block with this id on the page.
    #[error("Drive file block was not found")]
    BlockNotFound,
    #[error(transparent)]
    Page(#[from] PageError),
}

impl From<sqlx::Error> for DriveRefError {
    fn from(error: sqlx::Error) -> Self {
        Self::Page(PageError::Unavailable(error))
    }
}

/// Reads and changes the Drive file references of pages, as one member.
#[derive(Clone)]
pub struct DriveRefRepository {
    database: Database,
    pages: PageRepository,
}

impl DriveRefRepository {
    #[must_use]
    pub fn new(database: Database) -> Self {
        Self {
            pages: PageRepository::new(database.clone()),
            database,
        }
    }

    /// The page (when `actor_id` can see it) and what the actor may see of each of its references.
    pub async fn page_files(
        &self,
        workspace_id: Id,
        page_id: Id,
        actor_id: Id,
    ) -> Result<Vec<(DriveRef, DriveRefState)>, DriveRefError> {
        let page = self.pages.get_page(workspace_id, page_id, actor_id).await?;
        let refs = drive_refs(&page.content);
        let mut connection = self.database.pool().acquire().await?;
        let states = resolve_all(&mut connection, actor_id, &refs).await?;
        Ok(refs
            .into_iter()
            .map(|reference| {
                let state =
                    states[&(reference.workspace_id.clone(), reference.node_id.clone())].clone();
                (reference, state)
            })
            .collect())
    }

    /// What `actor_id` may see of `references` (`(workspace id, node id)`, as blocks store them) when they can see
    /// the page: for blocks of a live document that are not stored yet.
    pub async fn resolve_for_page(
        &self,
        workspace_id: Id,
        page_id: Id,
        actor_id: Id,
        references: &[(String, String)],
    ) -> Result<Vec<DriveRefState>, DriveRefError> {
        self.pages
            .get_visible(workspace_id, page_id, actor_id)
            .await?;
        let mut connection = self.database.pool().acquire().await?;
        let mut states: HashMap<&(String, String), DriveRefState> = HashMap::new();
        let mut out = Vec::with_capacity(references.len());
        for reference in references {
            if !states.contains_key(reference) {
                let state = resolve(&mut connection, actor_id, &reference.0, &reference.1).await?;
                states.insert(reference, state);
            }
            out.push(states[reference].clone());
        }
        Ok(out)
    }

    /// The live pages of the workspace that `actor_id` can see and that show the file `node_id`.
    pub async fn pages_using(
        &self,
        workspace_id: Id,
        actor_id: Id,
        node_id: Id,
    ) -> Result<Vec<PageUse>, DriveRefError> {
        let pool = self.database.pool();
        membership::actor(pool, workspace_id, actor_id)
            .await?
            .ok_or(PageError::NotFound)?;
        if let Some(hub) = crate::collab::CollabHub::existing(&self.database) {
            hub.flush_workspace(workspace_id).await;
        }
        // The id is a UUID, so a text match finds every candidate; the JSON check below drops ids in text.
        let rows: Vec<(String, String, String)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT id, title, content_json FROM pages WHERE workspace_id = $1 AND deleted_at IS NULL \
             AND {visible} AND content_json LIKE $3 ORDER BY lower(title), id",
            visible = visible(2)
        )))
        .bind(workspace_id.to_string())
        .bind(actor_id.to_string())
        .bind(format!("%{node_id}%"))
        .fetch_all(pool)
        .await?;
        let (workspace, node) = (workspace_id.to_string(), node_id.to_string());
        let mut uses = Vec::new();
        for (id, title, content) in rows {
            let blocks: Vec<Value> =
                serde_json::from_str(&content).map_err(|_| PageError::Corrupt)?;
            let count = drive_refs(&blocks)
                .iter()
                .filter(|reference| {
                    reference.workspace_id == workspace && reference.node_id == node
                })
                .count();
            if count > 0 {
                uses.push(PageUse {
                    page_id: id.parse().map_err(|_| PageError::Corrupt)?,
                    title,
                    blocks: count,
                });
            }
        }
        Ok(uses)
    }

    /// Adds a reference to the file `node_id` of the page's Drive at `position` (an index among the top-level
    /// blocks; `None`: at the end). The actor needs Drive access to the file and may edit the page.
    pub async fn insert(
        &self,
        workspace_id: Id,
        page_id: Id,
        actor_id: Id,
        node_id: Id,
        position: Option<i64>,
        request_id: &str,
    ) -> Result<(DriveRef, DriveNode), DriveRefError> {
        let node = self.live_file(workspace_id, actor_id, node_id).await?;
        let block = drive_file_block(workspace_id, node_id);
        let block_id = block["id"].as_str().unwrap_or("").to_owned();
        let page = self
            .edit(workspace_id, page_id, actor_id, request_id, |content| {
                let index = position.map_or(content.len(), |position| {
                    usize::try_from(position.max(0))
                        .unwrap_or(usize::MAX)
                        .min(content.len())
                });
                content.insert(index, block.clone());
                Ok(())
            })
            .await?;
        let reference = drive_refs(&page.content)
            .into_iter()
            .find(|reference| reference.block_id == block_id)
            .ok_or(PageError::Corrupt)?;
        Ok((reference, node))
    }

    /// Points the reference `block_id` at the file `node_id` of the page's Drive.
    pub async fn replace(
        &self,
        workspace_id: Id,
        page_id: Id,
        actor_id: Id,
        block_id: &str,
        node_id: Id,
        request_id: &str,
    ) -> Result<(DriveRef, DriveNode), DriveRefError> {
        let node = self.live_file(workspace_id, actor_id, node_id).await?;
        let page = self
            .edit(workspace_id, page_id, actor_id, request_id, |content| {
                let block = find_block(content, block_id).ok_or(DriveRefError::BlockNotFound)?;
                block["props"] = json!({
                    "workspaceId": workspace_id.to_string(),
                    "nodeId": node_id.to_string(),
                });
                Ok(())
            })
            .await?;
        let reference = drive_refs(&page.content)
            .into_iter()
            .find(|reference| reference.block_id == block_id)
            .ok_or(PageError::Corrupt)?;
        Ok((reference, node))
    }

    /// Removes the reference `block_id` from the page (the file stays in Drive).
    pub async fn remove(
        &self,
        workspace_id: Id,
        page_id: Id,
        actor_id: Id,
        block_id: &str,
        request_id: &str,
    ) -> Result<(), DriveRefError> {
        self.edit(workspace_id, page_id, actor_id, request_id, |content| {
            if remove_block(content, block_id) {
                Ok(())
            } else {
                Err(DriveRefError::BlockNotFound)
            }
        })
        .await
        .map(drop)
    }

    async fn live_file(
        &self,
        workspace_id: Id,
        actor_id: Id,
        node_id: Id,
    ) -> Result<DriveNode, DriveRefError> {
        let mut connection = self.database.pool().acquire().await?;
        match resolve(
            &mut connection,
            actor_id,
            &workspace_id.to_string(),
            &node_id.to_string(),
        )
        .await?
        {
            DriveRefState::Available(node) => Ok(*node),
            DriveRefState::Locked | DriveRefState::Unavailable => Err(DriveRefError::FileNotFound),
        }
    }

    /// Changes the page's current content with `change` and writes it at the revision it read (so a co-edit in
    /// between is never overwritten); a lost race reads the page again.
    async fn edit(
        &self,
        workspace_id: Id,
        page_id: Id,
        actor_id: Id,
        request_id: &str,
        change: impl Fn(&mut Vec<Value>) -> Result<(), DriveRefError>,
    ) -> Result<PageRecord, DriveRefError> {
        let mut attempt = 0;
        loop {
            attempt += 1;
            let page = self.pages.get_page(workspace_id, page_id, actor_id).await?;
            let mut content = page.content.clone();
            change(&mut content)?;
            let written = self
                .pages
                .update_page_at_revision(
                    workspace_id,
                    page_id,
                    actor_id,
                    revision(page.version, page.updated_at),
                    super::pages::PageChanges {
                        content: Some(content),
                        ..super::pages::PageChanges::default()
                    },
                    request_id,
                    orbit_platform::TimestampMillis::now(),
                )
                .await;
            match written {
                Err(PageError::VersionConflict { .. }) if attempt < WRITE_ATTEMPTS => {}
                other => return other.map_err(Into::into),
            }
        }
    }
}

fn find_block<'a>(blocks: &'a mut [Value], block_id: &str) -> Option<&'a mut Value> {
    for block in blocks {
        if block["type"] == DRIVE_FILE_BLOCK && block["id"] == block_id {
            return Some(block);
        }
        if let Some(children) = block.get_mut("children").and_then(Value::as_array_mut)
            && let Some(found) = find_block(children, block_id)
        {
            return Some(found);
        }
    }
    None
}

fn remove_block(blocks: &mut Vec<Value>, block_id: &str) -> bool {
    if let Some(index) = blocks
        .iter()
        .position(|block| block["type"] == DRIVE_FILE_BLOCK && block["id"] == block_id)
    {
        blocks.remove(index);
        return true;
    }
    blocks.iter_mut().any(|block| {
        block
            .get_mut("children")
            .and_then(Value::as_array_mut)
            .is_some_and(|children| remove_block(children, block_id))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn references_are_read_at_any_depth_with_their_top_level_position() {
        let blocks = vec![
            json!({"id": "a", "type": "paragraph", "children": [
                {"id": "b", "type": "driveFile", "props": {"workspaceId": "w", "nodeId": "n1"}}
            ]}),
            json!({"id": "c", "type": "driveFile", "props": {"workspaceId": "w", "nodeId": "n2"}}),
            json!({"id": "d", "type": "driveFile", "props": {}}),
        ];
        assert_eq!(
            drive_refs(&blocks),
            vec![
                DriveRef {
                    block_id: "b".into(),
                    workspace_id: "w".into(),
                    node_id: "n1".into(),
                    position: None
                },
                DriveRef {
                    block_id: "c".into(),
                    workspace_id: "w".into(),
                    node_id: "n2".into(),
                    position: Some(1)
                },
                DriveRef {
                    block_id: "d".into(),
                    workspace_id: String::new(),
                    node_id: String::new(),
                    position: Some(2)
                },
            ]
        );
    }

    #[test]
    fn nested_references_are_found_and_removed() {
        let mut blocks = vec![json!({"id": "a", "type": "paragraph", "children": [
            {"id": "b", "type": "driveFile", "props": {"workspaceId": "w", "nodeId": "n1"}}
        ]})];
        assert!(find_block(&mut blocks, "b").is_some());
        assert!(find_block(&mut blocks, "a").is_none());
        assert!(remove_block(&mut blocks, "b"));
        assert!(!remove_block(&mut blocks, "b"));
        assert_eq!(blocks[0]["children"], json!([]));
    }
}
