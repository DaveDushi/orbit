//! `orbit drive import-wellrox-cloud`: the files of the old wellrox-cloud Worker become Drive rows of one
//! workspace, in the folder `/wellrox-cloud/`. No bytes are copied: the objects stay in the same bucket and their
//! keys go into `drive_blobs.r2_key` as they are.
//!
//! The input is a `wrangler d1 export` SQL dump of the Worker's D1 database (tables `nodes`, `blobs`,
//! `trash_entries`; the `folders` and `file_metadata` compatibility tables of the older portal are not read). It
//! is loaded into an in-memory SQLite database. The tree starts at the node `root`. An item in the Worker's trash
//! comes into Orbit's trash with its original date; items whose trash entry was purged are left out. Each object
//! is checked with a HEAD request: a missing object is reported and its file left out. A key in Orbit's own areas of
//! the bucket (`blobs/`, `backups/`, `drive/`) is refused, because Orbit's cleanup could delete it. The Worker's own
//! keys `blobs/<22-character id>` are the exception: Orbit's attachment keys always have a folder below `blobs/`
//! (`blobs/{workspace_id}/…`), and its cleanup only takes those.
//!
//! Each imported node keeps its Worker id in `source_ref`, so a second run adds only what is new. The whole import
//! is one transaction; a dry run makes the same transaction and rolls it back, so its counts and name conflicts
//! are exact.

use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::path::Path;

use orbit_platform::{Database, Id, Row, S3Bucket, TimestampMillis};
use sqlx::{AnyConnection, ConnectOptions, Connection};
use thiserror::Error;

use crate::repositories::drive::{DriveNodeKind, TRASH_RETENTION_MILLIS, free_name, valid_name};

/// The Worker's top folder.
const SOURCE_ROOT: &str = "root";
/// The folder the import fills, at the top of the workspace's drive.
pub const TARGET_FOLDER: &str = "wellrox-cloud";
/// Prefixes of the bucket that belong to Orbit: a legacy object there could be deleted by Orbit's cleanup.
const ORBIT_PREFIXES: [&str; 4] = ["blobs/", "backups/", "drive/", ".orbit-probe/"];

/// A key outside Orbit's areas, or a Worker key `blobs/<id>`: 22 URL-safe characters and no folder.
fn importable_key(key: &str) -> bool {
    if let Some(id) = key.strip_prefix("blobs/") {
        return id.len() == 22
            && id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_');
    }
    !ORBIT_PREFIXES.iter().any(|orbit| key.starts_with(orbit))
}

#[derive(Debug, Error)]
pub enum ImportError {
    #[error("cannot read the export: {0}")]
    Read(#[from] std::io::Error),
    #[error("the export is not a wellrox-cloud D1 dump: {0}")]
    Export(String),
    #[error("the export database failed: {0}")]
    Source(#[from] sqlx::Error),
    #[error("{0}")]
    Refused(String),
    #[error("object storage failed: {0}")]
    Storage(#[from] orbit_platform::S3Error),
    #[error("the Orbit database failed: {0}")]
    Database(String),
}

/// What an import did (or, in a dry run, would do).
#[derive(Debug, Default, Eq, PartialEq)]
pub struct ImportReport {
    pub dry_run: bool,
    pub folders: u64,
    pub files: u64,
    pub bytes: u64,
    /// Items that come into Orbit's trash (top items of trashed trees).
    pub trashed: u64,
    /// Of those, the ones trashed more than 30 days ago: Orbit's hourly job deletes them, with their objects.
    pub expired_trash: u64,
    /// Nodes that a previous run imported.
    pub already_imported: u64,
    /// Nodes left out because their trash entry was purged.
    pub purged: u64,
    /// Files whose object is not in the bucket (or whose blob was deleted), by key.
    pub missing_objects: Vec<String>,
    /// Keys in Orbit's own areas of the bucket, left out.
    pub refused_keys: Vec<String>,
    /// Items that got another name: `parent/name -> new name`.
    pub name_conflicts: Vec<String>,
    /// Nodes not below `root`, or below a node that was left out.
    pub orphans: u64,
    /// Files whose object size differs from the export (the object's size is kept).
    pub size_mismatches: Vec<String>,
}

impl fmt::Display for ImportReport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            formatter,
            "{} wellrox-cloud import into /{TARGET_FOLDER}/",
            if self.dry_run {
                "Dry run of the"
            } else {
                "Done:"
            }
        )?;
        writeln!(formatter, "folders: {}", self.folders)?;
        writeln!(formatter, "files: {} ({} bytes)", self.files, self.bytes)?;
        writeln!(
            formatter,
            "in the trash: {} (older than 30 days, deleted by Orbit within an hour: {})",
            self.trashed, self.expired_trash
        )?;
        writeln!(formatter, "already imported: {}", self.already_imported)?;
        writeln!(
            formatter,
            "left out, purged in wellrox-cloud: {}",
            self.purged
        )?;
        writeln!(
            formatter,
            "left out, not below the Drive root or below a left-out item: {}",
            self.orphans
        )?;
        for (title, items) in [
            ("missing objects", &self.missing_objects),
            (
                "refused keys (in Orbit's own areas of the bucket)",
                &self.refused_keys,
            ),
            ("name conflicts (renamed)", &self.name_conflicts),
            (
                "size differences (the object's size is kept)",
                &self.size_mismatches,
            ),
        ] {
            writeln!(formatter, "{title}: {}", items.len())?;
            for item in items {
                writeln!(formatter, "  {item}")?;
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
struct SourceNode {
    id: String,
    parent_id: Option<String>,
    kind: DriveNodeKind,
    name: String,
    blob_id: Option<String>,
    created_at: i64,
    updated_at: i64,
    trash_entry_id: Option<String>,
}

#[derive(Clone, Debug)]
struct SourceBlob {
    r2_key: String,
    size: i64,
    content_type: String,
    sha256: Option<String>,
    created_at: i64,
    deleted: bool,
}

#[derive(Clone, Debug)]
struct SourceTrash {
    root_node_id: String,
    trashed_at: i64,
    active: bool,
}

struct Export {
    nodes: Vec<SourceNode>,
    blobs: HashMap<String, SourceBlob>,
    trash: HashMap<String, SourceTrash>,
}

/// One node to write, parents before children.
#[derive(Debug)]
struct Planned {
    source_ref: String,
    /// `None`: the target folder.
    parent_ref: Option<String>,
    kind: DriveNodeKind,
    name: String,
    created_at: i64,
    updated_at: i64,
    trashed_at: Option<i64>,
    blob: Option<PlannedBlob>,
}

#[derive(Debug)]
struct PlannedBlob {
    key: String,
    size: i64,
    mime: String,
    sha256: Option<String>,
    created_at: i64,
}

/// Imports the export into the workspace's drive; a dry run reports the same without keeping anything.
pub async fn import(
    database: &Database,
    bucket: &S3Bucket,
    workspace_id: Id,
    export_path: &Path,
    dry_run: bool,
    now: TimestampMillis,
) -> Result<ImportReport, ImportError> {
    let export = read_export(export_path).await?;
    let mut report = ImportReport {
        dry_run,
        ..ImportReport::default()
    };
    let plan = plan(&export, bucket, now, &mut report).await?;
    let mut transaction = database
        .immediate_transaction()
        .await
        .map_err(|error| ImportError::Database(error.to_string()))?;
    write(&mut transaction, workspace_id, &plan, now, &mut report)
        .await
        .map_err(|error| ImportError::Database(error.to_string()))?;
    if dry_run {
        transaction
            .rollback()
            .await
            .map_err(|error| ImportError::Database(error.to_string()))?;
    } else {
        transaction
            .commit()
            .await
            .map_err(|error| ImportError::Database(error.to_string()))?;
    }
    Ok(report)
}

/// The dump in an in-memory SQLite database, read into memory.
async fn read_export(path: &Path) -> Result<Export, ImportError> {
    let dump = tokio::fs::read_to_string(path).await?;
    // The dump runs in a database of its own: it may not reach files or code.
    if let Some(word) = sql_words(&dump).find(|word| {
        word.eq_ignore_ascii_case("attach")
            || word.eq_ignore_ascii_case("vacuum")
            || word.eq_ignore_ascii_case("load_extension")
    }) {
        return Err(ImportError::Export(format!(
            "the dump uses {word}, which an export never does"
        )));
    }
    // Foreign keys are not checked: a node whose parent is gone is reported as an orphan, not a failure. The dump
    // holds the Worker's secrets (API key verifiers): sqlx must not log it as a (slow) statement.
    let options =
        <sqlx::sqlite::SqliteConnectOptions as std::str::FromStr>::from_str("sqlite::memory:")?
            .foreign_keys(false)
            .disable_statement_logging();
    let mut source = sqlx::SqliteConnection::connect_with(&options).await?;
    sqlx::raw_sql(sqlx::AssertSqlSafe(dump))
        .execute(&mut source)
        .await
        .map_err(|error| ImportError::Export(format!("the dump does not run: {error}")))?;
    for table in ["nodes", "blobs", "trash_entries"] {
        let found: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?",
        )
        .bind(table)
        .fetch_one(&mut source)
        .await?;
        if found == 0 {
            return Err(ImportError::Export(format!("table {table} is missing")));
        }
    }
    let time = |value: &str| {
        chrono::DateTime::parse_from_rfc3339(value)
            .map(|time| time.timestamp_millis())
            .map_err(|_| ImportError::Export(format!("invalid time {value}")))
    };
    let mut nodes = Vec::new();
    for row in sqlx::query(
        "SELECT id, parent_id, kind, name, blob_id, created_at, updated_at, trash_entry_id FROM nodes ORDER BY id",
    )
    .fetch_all(&mut source)
    .await?
    {
        use sqlx::Row as _;
        let kind: String = row.try_get("kind")?;
        nodes.push(SourceNode {
            id: row.try_get("id")?,
            parent_id: row.try_get("parent_id")?,
            kind: match kind.as_str() {
                "folder" => DriveNodeKind::Folder,
                "file" => DriveNodeKind::File,
                other => return Err(ImportError::Export(format!("unknown node kind {other}"))),
            },
            name: row.try_get("name")?,
            blob_id: row.try_get("blob_id")?,
            created_at: time(&row.try_get::<String, _>("created_at")?)?,
            updated_at: time(&row.try_get::<String, _>("updated_at")?)?,
            trash_entry_id: row.try_get("trash_entry_id")?,
        });
    }
    let mut blobs = HashMap::new();
    for row in sqlx::query(
        "SELECT id, r2_key, size, content_type, sha256, created_at, deleted_at FROM blobs",
    )
    .fetch_all(&mut source)
    .await?
    {
        use sqlx::Row as _;
        blobs.insert(
            row.try_get::<String, _>("id")?,
            SourceBlob {
                r2_key: row.try_get("r2_key")?,
                size: row.try_get("size")?,
                content_type: row.try_get("content_type")?,
                sha256: row.try_get("sha256")?,
                created_at: time(&row.try_get::<String, _>("created_at")?)?,
                deleted: row.try_get::<Option<String>, _>("deleted_at")?.is_some(),
            },
        );
    }
    let mut trash = HashMap::new();
    for row in sqlx::query("SELECT id, root_node_id, trashed_at, state FROM trash_entries")
        .fetch_all(&mut source)
        .await?
    {
        use sqlx::Row as _;
        trash.insert(
            row.try_get::<String, _>("id")?,
            SourceTrash {
                root_node_id: row.try_get("root_node_id")?,
                trashed_at: time(&row.try_get::<String, _>("trashed_at")?)?,
                active: row.try_get::<String, _>("state")? == "active",
            },
        );
    }
    if !nodes.iter().any(|node| node.id == SOURCE_ROOT) {
        return Err(ImportError::Export("the node `root` is missing".to_owned()));
    }
    Ok(Export {
        nodes,
        blobs,
        trash,
    })
}

/// The words of SQL outside string literals, quoted names and comments (`'…'`, `"…"`, `` `…` ``, `[…]`, `--`,
/// `/* */`), so data such as a file named `attach.txt` is never mistaken for a statement.
fn sql_words(sql: &str) -> impl Iterator<Item = &str> {
    let bytes = sql.as_bytes();
    let mut words = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        let closing = match byte {
            b'\'' => Some(b'\''),
            b'"' => Some(b'"'),
            b'`' => Some(b'`'),
            b'[' => Some(b']'),
            _ => None,
        };
        if let Some(closing) = closing {
            index += 1;
            while index < bytes.len() {
                if bytes[index] == closing {
                    // A doubled quote is a quote inside the literal.
                    if closing != b']' && bytes.get(index + 1) == Some(&closing) {
                        index += 2;
                        continue;
                    }
                    break;
                }
                index += 1;
            }
            index += 1;
        } else if sql[index..].starts_with("--") {
            index = sql[index..]
                .find('\n')
                .map_or(bytes.len(), |end| index + end);
        } else if sql[index..].starts_with("/*") {
            index = sql[index + 2..]
                .find("*/")
                .map_or(bytes.len(), |end| index + end + 4);
        } else if byte.is_ascii_alphanumeric() || byte == b'_' {
            let start = index;
            while index < bytes.len()
                && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_')
            {
                index += 1;
            }
            words.push(&sql[start..index]);
        } else {
            index += 1;
        }
    }
    words.into_iter()
}

/// The nodes to write, parents first, after the trash and the bucket were checked.
async fn plan(
    export: &Export,
    bucket: &S3Bucket,
    now: TimestampMillis,
    report: &mut ImportReport,
) -> Result<Vec<Planned>, ImportError> {
    let prefix = bucket.config().prefix.trim_matches('/').to_owned();
    let mut children: HashMap<&str, Vec<&SourceNode>> = HashMap::new();
    for node in &export.nodes {
        if let Some(parent) = node.parent_id.as_deref() {
            children.entry(parent).or_default().push(node);
        }
    }
    let mut plan = Vec::new();
    let mut reached = HashSet::from([SOURCE_ROOT]);
    let mut queue = VecDeque::from([SOURCE_ROOT]);
    while let Some(parent) = queue.pop_front() {
        for node in children.get(parent).map(Vec::as_slice).unwrap_or_default() {
            if !reached.insert(node.id.as_str()) {
                continue;
            }
            let mut trashed_at = None;
            if let Some(entry_id) = &node.trash_entry_id {
                match export.trash.get(entry_id) {
                    Some(entry) if entry.active => {
                        if entry.root_node_id == node.id {
                            trashed_at = Some(entry.trashed_at);
                        }
                    }
                    // Purged (or being purged) in wellrox-cloud: its objects are gone or going.
                    _ => {
                        report.purged += 1 + count_below(&children, &node.id);
                        mark_below(&children, &node.id, &mut reached);
                        continue;
                    }
                }
            }
            let blob = match node.kind {
                DriveNodeKind::Folder => None,
                DriveNodeKind::File => {
                    match check_blob(export, bucket, &prefix, node, report).await? {
                        Some(blob) => Some(blob),
                        None => continue,
                    }
                }
            };
            let name = match valid_name(&node.name) {
                Ok(name) => name,
                Err(_) => {
                    let fixed = sanitized_name(&node.name);
                    report
                        .name_conflicts
                        .push(format!("{} -> {fixed} (not a valid name)", node.name));
                    fixed
                }
            };
            if let Some(trashed_at) = trashed_at {
                report.trashed += 1;
                if trashed_at < now.as_millis() - TRASH_RETENTION_MILLIS {
                    report.expired_trash += 1;
                }
            }
            plan.push(Planned {
                source_ref: node.id.clone(),
                parent_ref: (parent != SOURCE_ROOT).then(|| parent.to_owned()),
                kind: node.kind,
                name,
                created_at: node.created_at,
                updated_at: node.updated_at,
                trashed_at,
                blob,
            });
            if node.kind == DriveNodeKind::Folder {
                queue.push_back(node.id.as_str());
            }
        }
    }
    report.orphans = export
        .nodes
        .iter()
        .filter(|node| !reached.contains(node.id.as_str()))
        .count() as u64;
    Ok(plan)
}

/// The object of a file when it can be imported; `None` (and a note in the report) otherwise.
async fn check_blob(
    export: &Export,
    bucket: &S3Bucket,
    prefix: &str,
    node: &SourceNode,
    report: &mut ImportReport,
) -> Result<Option<PlannedBlob>, ImportError> {
    let Some(blob) = node.blob_id.as_ref().and_then(|id| export.blobs.get(id)) else {
        report
            .missing_objects
            .push(format!("(no blob) for {}", node.name));
        return Ok(None);
    };
    if blob.deleted {
        report
            .missing_objects
            .push(format!("{} (deleted in wellrox-cloud)", blob.r2_key));
        return Ok(None);
    }
    // Drive keys are relative to the bucket's prefix; the Worker's are from the bucket root.
    let key = if prefix.is_empty() {
        blob.r2_key.clone()
    } else if let Some(key) = blob.r2_key.strip_prefix(&format!("{prefix}/")) {
        key.to_owned()
    } else {
        report.refused_keys.push(format!(
            "{} (outside the bucket prefix {prefix}/)",
            blob.r2_key
        ));
        return Ok(None);
    };
    if !importable_key(&key) {
        report.refused_keys.push(blob.r2_key.clone());
        return Ok(None);
    }
    let Some(size) = bucket.size(&key).await? else {
        report.missing_objects.push(blob.r2_key.clone());
        return Ok(None);
    };
    let size = i64::try_from(size).unwrap_or(i64::MAX);
    if size != blob.size {
        report.size_mismatches.push(format!(
            "{}: export {} bytes, bucket {size} bytes",
            blob.r2_key, blob.size
        ));
    }
    Ok(Some(PlannedBlob {
        key,
        size,
        mime: Some(blob.content_type.trim().to_ascii_lowercase())
            .filter(|mime| mime.contains('/') && mime.len() <= 255)
            .unwrap_or_else(|| "application/octet-stream".to_owned()),
        sha256: blob.sha256.clone(),
        created_at: blob.created_at,
    }))
}

fn count_below(children: &HashMap<&str, Vec<&SourceNode>>, id: &str) -> u64 {
    children
        .get(id)
        .map(|list| {
            list.iter()
                .map(|child| 1 + count_below(children, &child.id))
                .sum()
        })
        .unwrap_or(0)
}

fn mark_below<'a>(
    children: &HashMap<&str, Vec<&'a SourceNode>>,
    id: &str,
    reached: &mut HashSet<&'a str>,
) {
    for child in children.get(id).map(Vec::as_slice).unwrap_or_default() {
        if reached.insert(child.id.as_str()) {
            mark_below(children, &child.id, reached);
        }
    }
}

/// A Drive name from a name Drive does not take: `/`, `\` and control characters become `_`, at most 255
/// characters, never empty, `.` or `..`.
fn sanitized_name(name: &str) -> String {
    let cleaned: String = name
        .trim()
        .chars()
        .map(|c| {
            if c == '/' || c == '\\' || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .take(255)
        .collect();
    match cleaned.as_str() {
        "" | "." | ".." => "_".to_owned(),
        _ => cleaned,
    }
}

async fn write(
    connection: &mut AnyConnection,
    workspace_id: Id,
    plan: &[Planned],
    now: TimestampMillis,
    report: &mut ImportReport,
) -> Result<(), crate::repositories::drive::DriveError> {
    let workspace = workspace_id.to_string();
    let exists: Option<String> =
        sqlx::query_scalar("SELECT id FROM workspaces WHERE id = $1 AND deleted_at IS NULL")
            .bind(&workspace)
            .fetch_optional(&mut *connection)
            .await?;
    if exists.is_none() {
        return Err(crate::repositories::drive::DriveError::NotFound);
    }
    // Nodes of earlier runs, by their Worker id.
    let mut imported: HashMap<String, Id> = HashMap::new();
    for row in sqlx::query(
        "SELECT id, source_ref FROM drive_nodes WHERE workspace_id = $1 AND source_kind = 'wellrox_cloud' \
         AND source_ref IS NOT NULL",
    )
    .bind(&workspace)
    .fetch_all(&mut *connection)
    .await?
    {
        imported.insert(
            row.try_get("source_ref")?,
            crate::repositories::drive::parse_id(row.try_get("id")?)?,
        );
    }
    let target = match imported.get(SOURCE_ROOT) {
        Some(id) => *id,
        None => {
            let name = free_name(connection, workspace_id, None, TARGET_FOLDER).await?;
            if name != TARGET_FOLDER {
                report
                    .name_conflicts
                    .push(format!("/{TARGET_FOLDER} -> {name}"));
            }
            let id = Id::new_v7();
            insert(
                connection,
                workspace_id,
                id,
                None,
                DriveNodeKind::Folder,
                &name,
                None,
                SOURCE_ROOT,
                now.as_millis(),
                now.as_millis(),
                None,
            )
            .await?;
            id
        }
    };
    for node in plan {
        if imported.contains_key(&node.source_ref) {
            report.already_imported += 1;
            continue;
        }
        let parent = match &node.parent_ref {
            None => target,
            Some(parent) => match imported.get(parent) {
                Some(id) => *id,
                // The parent was left out on this run (it is checked before its children).
                None => {
                    report.orphans += 1;
                    continue;
                }
            },
        };
        // A trashed item does not take a name among the live items of its folder.
        let name = if node.trashed_at.is_some() {
            node.name.clone()
        } else {
            free_name(connection, workspace_id, Some(parent), &node.name).await?
        };
        if name != node.name {
            report
                .name_conflicts
                .push(format!("{} -> {name}", node.name));
        }
        let blob_id = match &node.blob {
            None => None,
            Some(blob) => {
                let taken: Option<String> =
                    sqlx::query_scalar("SELECT id FROM drive_blobs WHERE r2_key = $1")
                        .bind(&blob.key)
                        .fetch_optional(&mut *connection)
                        .await?;
                if taken.is_some() {
                    // Another Drive file (or a deletion in progress) uses this object.
                    report
                        .refused_keys
                        .push(format!("{} (already used by Drive)", blob.key));
                    continue;
                }
                let id = Id::new_v7();
                sqlx::query(
                    "INSERT INTO drive_blobs (id, workspace_id, r2_key, size, mime, content_hash, status, created_at, \
                     updated_at) VALUES ($1, $2, $3, $4, $5, $6, 'ready', $7, $8)",
                )
                .bind(id.to_string())
                .bind(&workspace)
                .bind(&blob.key)
                .bind(blob.size)
                .bind(&blob.mime)
                .bind(blob.sha256.clone())
                .bind(blob.created_at)
                .bind(now.as_millis())
                .execute(&mut *connection)
                .await?;
                report.files += 1;
                report.bytes += u64::try_from(blob.size).unwrap_or_default();
                Some((id, blob))
            }
        };
        if node.kind == DriveNodeKind::Folder {
            report.folders += 1;
        }
        let id = Id::new_v7();
        insert(
            connection,
            workspace_id,
            id,
            Some(parent),
            node.kind,
            &name,
            blob_id,
            &node.source_ref,
            node.created_at,
            node.updated_at,
            node.trashed_at,
        )
        .await?;
        imported.insert(node.source_ref.clone(), id);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn insert(
    connection: &mut AnyConnection,
    workspace_id: Id,
    id: Id,
    parent_id: Option<Id>,
    kind: DriveNodeKind,
    name: &str,
    blob: Option<(Id, &PlannedBlob)>,
    source_ref: &str,
    created_at: i64,
    updated_at: i64,
    trashed_at: Option<i64>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO drive_nodes (id, workspace_id, parent_id, kind, name, name_lower, blob_id, size, mime, \
         created_at, updated_at, trashed_at, source_kind, source_ref, source_created_at, source_modified_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, 'wellrox_cloud', $13, $10, $11)",
    )
    .bind(id.to_string())
    .bind(workspace_id.to_string())
    .bind(parent_id.map(|id| id.to_string()))
    .bind(match kind {
        DriveNodeKind::Folder => "folder",
        DriveNodeKind::File => "file",
    })
    .bind(name)
    .bind(name.to_lowercase())
    .bind(blob.map(|(id, _)| id.to_string()))
    .bind(blob.map(|(_, blob)| blob.size))
    .bind(blob.map(|(_, blob)| blob.mime.clone()))
    .bind(created_at)
    .bind(updated_at)
    .bind(trashed_at)
    .bind(source_ref)
    .execute(&mut *connection)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_sql_outside_quotes_counts_as_statements() {
        let words: Vec<&str> = sql_words(
            "INSERT INTO nodes VALUES('attach','it''s attach.bin',\"vacuum\"); -- attach\n/* load_extension */ [attach]",
        )
        .collect();
        assert_eq!(words, ["INSERT", "INTO", "nodes", "VALUES"]);
        let words: Vec<&str> =
            sql_words("CREATE TABLE x(a);\nATTACH DATABASE 'f.db' AS f;").collect();
        assert!(words.contains(&"ATTACH"));
    }

    #[test]
    fn names_drive_refuses_are_made_valid() {
        assert_eq!(sanitized_name("a/b\\c\td"), "a_b_c_d");
        assert_eq!(sanitized_name(".."), "_");
        assert_eq!(sanitized_name("   "), "_");
        assert_eq!(sanitized_name(&"x".repeat(300)).chars().count(), 255);
    }

    #[test]
    fn worker_keys_under_blobs_are_importable_and_orbits_own_keys_are_not() {
        assert!(importable_key("blobs/AbCdEfGhIjKlMnOpQr_-09"));
        assert!(importable_key("Rachel Uploads/report.pdf"));
        for key in [
            "blobs/",
            "blobs/a/b",
            "blobs/01a13079-d065-729b-917b-e30d8d9aa51c/ab-cd",
            "blobs/AbCdEfGhIjKlMnOpQr_-09/x",
            "blobs/AbCdEfGhIjKlMnOpQr_-0",
            "blobs/AbCdEfGhIjKlMnOpQr_-099",
            "blobs/AbCdEfGhIjKlMnOpQr..09",
            "blobs/../AbCdEfGhIjKlMnOpQ",
            "backups/snapshot.zst",
            "drive/x",
            ".orbit-probe/x",
        ] {
            assert!(!importable_key(key), "{key}");
        }
    }

    #[derive(Clone, Default)]
    struct Captured(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for Captured {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn the_dump_is_never_logged_as_a_statement() {
        // sqlx logs from the connection's worker thread, so only a global subscriber sees it.
        let captured = Captured::default();
        let writer = captured.clone();
        tracing::subscriber::set_global_default(
            tracing_subscriber::fmt()
                .with_max_level(tracing::Level::TRACE)
                .with_writer(move || writer.clone())
                .finish(),
        )
        .unwrap();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("d1.sql");
        tokio::fs::write(
            &path,
            "CREATE TABLE api_keys (verifier TEXT); INSERT INTO api_keys VALUES ('secret-verifier');\n\
             CREATE TABLE nodes (id, parent_id, kind, name, blob_id, created_at, updated_at, trash_entry_id);\n\
             INSERT INTO nodes VALUES ('root', NULL, 'folder', 'root', NULL, '2026-09-01T10:00:00Z', \
             '2026-09-01T10:00:00Z', NULL);\n\
             CREATE TABLE blobs (id, r2_key, size, content_type, sha256, created_at, deleted_at);\n\
             CREATE TABLE trash_entries (id, root_node_id, trashed_at, state);",
        )
        .await
        .unwrap();
        read_export(&path).await.unwrap();
        // The same statement on a connection that logs: the capture sees statements.
        let mut logging = sqlx::SqliteConnection::connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::raw_sql("SELECT 'logged-statement'")
            .execute(&mut logging)
            .await
            .unwrap();

        let log = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
        assert!(log.contains("logged-statement"), "{log}");
        assert!(!log.contains("secret-verifier"), "{log}");
    }
}
