//! `orbit drive import-wellrox-cloud`: the Worker's tree, trash and objects become Drive rows (no bytes copied),
//! a dry run keeps nothing, a second run adds only what is new, and a real run refuses a database that a serving
//! Orbit owns. The bucket is the in-memory S3 server, or a real S3-compatible server with `ORBIT_TEST_S3_*`
//! (path-style), always under a fresh prefix.

use orbit_platform::{
    Database, DatabaseConfig, Id, PasswordService, S3Bucket, S3Config, TestDatabase,
    TimestampMillis, start_fake_s3,
};
use orbit_server::drive::DriveService;
use orbit_server::drive_import::{ImportReport, import};
use orbit_server::repositories::drive::OnConflict;
use orbit_server::repositories::identity::{IdentityRepository, SetupRequest};

/// The Worker's D1 schema (`migrations/0001_shared_drive.sql`): the tables the import reads.
const SCHEMA: &str = r"
PRAGMA defer_foreign_keys=TRUE;
CREATE TABLE blobs (
  id TEXT PRIMARY KEY NOT NULL,
  r2_key TEXT NOT NULL UNIQUE CHECK (length(r2_key) BETWEEN 1 AND 1024),
  size INTEGER NOT NULL CHECK (size BETWEEN 0 AND 5000000000),
  etag TEXT,
  content_type TEXT NOT NULL CHECK (length(content_type) BETWEEN 1 AND 200),
  sha256 TEXT CHECK (sha256 IS NULL OR (length(sha256) = 64 AND sha256 NOT GLOB '*[^0-9a-f]*')),
  sha256_status TEXT NOT NULL CHECK (sha256_status IN ('absent', 'declared', 'verified')),
  imported INTEGER NOT NULL DEFAULT 0 CHECK (imported IN (0, 1)),
  created_at TEXT NOT NULL,
  deleted_at TEXT
) STRICT;
CREATE TABLE nodes (
  id TEXT PRIMARY KEY NOT NULL,
  parent_id TEXT,
  kind TEXT NOT NULL CHECK (kind IN ('folder', 'file')),
  name TEXT NOT NULL CHECK (length(CAST(name AS BLOB)) BETWEEN 1 AND 255),
  name_key TEXT NOT NULL CHECK (length(CAST(name_key AS BLOB)) BETWEEN 1 AND 255),
  blob_id TEXT UNIQUE,
  version INTEGER NOT NULL DEFAULT 1 CHECK (version > 0),
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  created_by TEXT NOT NULL,
  modified_by TEXT NOT NULL,
  trash_entry_id TEXT,
  CHECK ((kind = 'folder' AND blob_id IS NULL) OR (kind = 'file' AND blob_id IS NOT NULL)),
  FOREIGN KEY (parent_id) REFERENCES nodes(id),
  FOREIGN KEY (blob_id) REFERENCES blobs(id)
) STRICT;
CREATE TABLE trash_entries (
  id TEXT PRIMARY KEY NOT NULL,
  root_node_id TEXT NOT NULL,
  original_parent_id TEXT NOT NULL,
  original_name TEXT NOT NULL,
  original_name_key TEXT NOT NULL,
  reason TEXT NOT NULL CHECK (reason IN ('manual', 'replace', 'restore-replace')),
  trashed_at TEXT NOT NULL,
  purge_after TEXT NOT NULL,
  trashed_by TEXT NOT NULL,
  state TEXT NOT NULL DEFAULT 'active' CHECK (state IN ('active', 'purging', 'purged'))
) STRICT;
CREATE TABLE folders (path TEXT PRIMARY KEY NOT NULL, parent_path TEXT, name TEXT NOT NULL, created_by TEXT NOT NULL, created_at TEXT NOT NULL) STRICT;
INSERT INTO nodes VALUES('root',NULL,'folder','Drive','drive',NULL,1,'2026-08-10T00:00:00.000Z','2026-08-10T00:00:00.000Z','system','system',NULL);
";

const T: &str =
    "'2026-09-01T10:00:00.000Z','2026-09-02T11:00:00.000Z','ana@wellrox.com','ana@wellrox.com'";

/// `(id, parent_id, name, kind, trashed_at, r2_key)` of a Drive node.
type NodeRow = (
    String,
    Option<String>,
    String,
    String,
    Option<i64>,
    Option<String>,
);

struct Fixture {
    database: TestDatabase,
    bucket: S3Bucket,
    drive: DriveService,
    workspace: Id,
    user: Id,
    /// The key prefix of this run in the bucket; the Worker's keys start with it.
    prefix: String,
    dir: tempfile::TempDir,
}

impl Fixture {
    async fn new() -> Self {
        let database = TestDatabase::new().await.unwrap();
        let identity = IdentityRepository::new((*database).clone());
        let now = TimestampMillis::now();
        identity
            .store_setup_token(
                "setup",
                TimestampMillis::from_millis(now.as_millis() + 60_000),
            )
            .await
            .unwrap();
        let setup = identity
            .complete_setup(
                SetupRequest {
                    token: "setup".into(),
                    email: "owner@example.com".into(),
                    display_name: "Owner".into(),
                    password_hash: PasswordService::default()
                        .hash("correct horse battery")
                        .unwrap(),
                    workspace_name: "Orbit".into(),
                    project_name: "General".into(),
                },
                now,
            )
            .await
            .unwrap();
        let prefix = format!("orbit-import-test-{}", Id::new_v7());
        let config = match std::env::var("ORBIT_TEST_S3_ENDPOINT") {
            Ok(endpoint) => S3Config {
                endpoint,
                region: "us-east-1".to_owned(),
                bucket: std::env::var("ORBIT_TEST_S3_BUCKET").unwrap(),
                prefix: prefix.clone(),
                access_key_id: std::env::var("ORBIT_TEST_S3_ACCESS_KEY").unwrap(),
                secret_access_key: std::env::var("ORBIT_TEST_S3_SECRET_KEY").unwrap(),
                path_style: true,
            },
            Err(_) => S3Config {
                prefix: prefix.clone(),
                ..start_fake_s3().await
            },
        };
        let bucket = S3Bucket::new(config).unwrap();
        let storage = orbit_platform::ObjectStorage::default();
        storage.configure(orbit_platform::ObjectStorageState {
            bucket: Some(bucket.clone()),
            ..Default::default()
        });
        let drive = DriveService::new((*database).clone(), storage);
        Self {
            database,
            bucket,
            drive,
            workspace: setup.workspace_id,
            user: setup.user_id,
            prefix,
            dir: tempfile::tempdir().unwrap(),
        }
    }

    /// A Worker key (from the bucket root) and, with `bytes`, its object in the bucket.
    async fn object(&self, name: &str, bytes: Option<&[u8]>) -> String {
        let key = format!("files/{name}");
        if let Some(bytes) = bytes {
            self.bucket.put_bytes(&key, bytes.to_vec()).await.unwrap();
        }
        format!("{}/{key}", self.prefix)
    }

    async fn export(&self, file: &str, rows: &str) -> std::path::PathBuf {
        let path = self.dir.path().join(file);
        std::fs::write(&path, format!("{SCHEMA}{rows}")).unwrap();
        path
    }

    async fn run(&self, export: &std::path::Path, dry_run: bool) -> ImportReport {
        import(
            &self.database,
            &self.bucket,
            self.workspace,
            export,
            dry_run,
            TimestampMillis::now(),
        )
        .await
        .unwrap()
    }

    /// `(path, kind, trashed_at, r2_key)` of every Drive node, by path.
    async fn tree(&self) -> Vec<(String, String, Option<i64>, Option<String>)> {
        let rows: Vec<NodeRow> = sqlx::query_as(
            "SELECT n.id, n.parent_id, n.name, n.kind, n.trashed_at, b.r2_key FROM drive_nodes n \
             LEFT JOIN drive_blobs b ON b.id = n.blob_id WHERE n.workspace_id = $1",
        )
        .bind(self.workspace.to_string())
        .fetch_all(self.database.pool())
        .await
        .unwrap();
        let path = |id: &str| {
            let mut parts = Vec::new();
            let mut current = Some(id.to_owned());
            while let Some(id) = current {
                let row = rows.iter().find(|row| row.0 == id).unwrap();
                parts.push(row.2.clone());
                current = row.1.clone();
            }
            parts.reverse();
            parts.join("/")
        };
        let mut tree: Vec<_> = rows
            .iter()
            .map(|row| (path(&row.0), row.3.clone(), row.4, row.5.clone()))
            .collect();
        tree.sort();
        tree
    }
}

fn blob(id: &str, key: &str, size: usize, deleted: bool) -> String {
    format!(
        "INSERT INTO blobs VALUES('{id}','{key}',{size},NULL,'text/plain',NULL,'absent',0,'2026-09-01T10:00:00.000Z',{});\n",
        if deleted {
            "'2026-10-01T00:00:00.000Z'"
        } else {
            "NULL"
        }
    )
}

fn node(
    id: &str,
    parent: &str,
    kind: &str,
    name: &str,
    blob: Option<&str>,
    trash: Option<&str>,
) -> String {
    let quoted =
        |value: Option<&str>| value.map_or("NULL".to_owned(), |value| format!("'{value}'"));
    format!(
        "INSERT INTO nodes VALUES('{id}','{parent}','{kind}','{}','{}',{},1,{T},{});\n",
        name.replace('\'', "''"),
        name.to_lowercase().replace('\'', "''"),
        quoted(blob),
        quoted(trash)
    )
}

fn trash(id: &str, root: &str, parent: &str, at: &str, state: &str) -> String {
    format!(
        "INSERT INTO trash_entries VALUES('{id}','{root}','{parent}','x','x','manual','{at}','2099-01-01T00:00:00.000Z','ana','{state}');\n"
    )
}

/// The Worker's drive used by the tests, with every case the mapping handles.
async fn worker_drive(fixture: &Fixture) -> String {
    let mut rows = String::new();
    let present = [
        ("b-logo", "logo.png", &b"png"[..]),
        ("b-q1", "q1.pdf", &b"pdf!"[..]),
        ("b-notes", "notes.txt", &b"notes"[..]),
        ("b-old", "old.txt", &b"old"[..]),
        ("b-gone", "gone.txt", &b"gone"[..]),
        ("b-ancient", "ancient.txt", &b"ancient"[..]),
        ("b-plan", "plan.txt", &b"plan"[..]),
    ];
    for (id, name, bytes) in present {
        let key = fixture.object(name, Some(bytes)).await;
        rows += &blob(id, &key, bytes.len(), false);
    }
    rows += &blob(
        "b-missing",
        &fixture.object("missing.docx", None).await,
        9,
        false,
    );
    rows += &blob(
        "b-deleted",
        &fixture.object("deleted.txt", Some(b"del")).await,
        3,
        true,
    );
    fixture
        .bucket
        .put_bytes("blobs/x/attach.bin", b"orbit".to_vec())
        .await
        .unwrap();
    rows += &blob(
        "b-orbit",
        &format!("{}/blobs/x/attach.bin", fixture.prefix),
        5,
        false,
    );
    rows += &node("marketing", "root", "folder", "Marketing", None, None);
    rows += &node(
        "logo",
        "marketing",
        "file",
        "logo.png",
        Some("b-logo"),
        None,
    );
    rows += &node("campaigns", "marketing", "folder", "Campaigns", None, None);
    rows += &node(
        "q1",
        "campaigns",
        "file",
        "Q1 Bericht.pdf",
        Some("b-q1"),
        None,
    );
    rows += &node("notes", "root", "file", "notes.txt", Some("b-notes"), None);
    rows += &node(
        "missing",
        "root",
        "file",
        "missing.docx",
        Some("b-missing"),
        None,
    );
    rows += &node(
        "deleted",
        "root",
        "file",
        "deleted.txt",
        Some("b-deleted"),
        None,
    );
    rows += &node(
        "attach",
        "root",
        "file",
        "attach.bin",
        Some("b-orbit"),
        None,
    );
    rows += &node(
        "plan",
        "root",
        "file",
        "Plan Q1/Q2.txt",
        Some("b-plan"),
        None,
    );
    // Trashed recently (a folder with a file), long ago (a file), and purged (a folder with a file).
    rows += &trash("t-old", "old", "root", "2026-10-30T09:00:00.000Z", "active");
    rows += &node("old", "root", "folder", "Old", None, Some("t-old"));
    rows += &node(
        "old-file",
        "old",
        "file",
        "old.txt",
        Some("b-old"),
        Some("t-old"),
    );
    rows += &trash(
        "t-ancient",
        "ancient",
        "marketing",
        "2026-01-01T00:00:00.000Z",
        "active",
    );
    rows += &node(
        "ancient",
        "marketing",
        "file",
        "ancient.txt",
        Some("b-ancient"),
        Some("t-ancient"),
    );
    rows += &trash(
        "t-gone",
        "gone",
        "root",
        "2026-09-15T00:00:00.000Z",
        "purged",
    );
    rows += &node("gone", "root", "folder", "Gone", None, Some("t-gone"));
    rows += &node(
        "gone-file",
        "gone",
        "file",
        "gone.txt",
        Some("b-gone"),
        Some("t-gone"),
    );
    rows += "INSERT INTO nodes VALUES('stray','nowhere','folder','Stray','stray',NULL,1,'2026-09-01T10:00:00.000Z','2026-09-01T10:00:00.000Z','a','a',NULL);\n";
    rows
}

fn millis(time: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(time)
        .unwrap()
        .timestamp_millis()
}

#[tokio::test]
async fn the_workers_tree_trash_and_objects_become_drive_rows_without_copying() {
    let fixture = Fixture::new().await;
    // A member already has a top-level folder of that name: the import goes next to it.
    fixture
        .drive
        .repository()
        .create_folder(
            fixture.workspace,
            fixture.user,
            None,
            "wellrox-cloud",
            OnConflict::Fail,
            TimestampMillis::now(),
        )
        .await
        .unwrap();
    let rows = worker_drive(&fixture).await;
    let export = fixture.export("d1.sql", &rows).await;
    let objects_before = fixture.bucket.list("").await.unwrap().len();

    let dry = fixture.run(&export, true).await;
    assert_eq!(fixture.tree().await.len(), 1, "a dry run keeps nothing");
    let report = fixture.run(&export, false).await;
    assert_eq!(
        dry,
        ImportReport {
            dry_run: true,
            ..report_without_dry_run(&report)
        }
    );
    assert_eq!(
        fixture.bucket.list("").await.unwrap().len(),
        objects_before,
        "no object was copied or deleted"
    );

    assert_eq!((report.folders, report.files), (3, 6));
    assert_eq!(report.bytes, 3 + 4 + 5 + 3 + 7 + 4);
    assert_eq!((report.trashed, report.expired_trash), (2, 1));
    assert_eq!(report.purged, 2, "the purged folder and its file");
    assert_eq!(report.orphans, 1);
    assert_eq!(
        report.missing_objects.len(),
        2,
        "{:?}",
        report.missing_objects
    );
    assert!(
        report
            .missing_objects
            .iter()
            .any(|item| item.contains("missing.docx"))
    );
    assert!(
        report
            .missing_objects
            .iter()
            .any(|item| item.contains("deleted in wellrox-cloud"))
    );
    assert_eq!(report.refused_keys.len(), 1, "{:?}", report.refused_keys);
    assert!(report.refused_keys[0].contains("blobs/x/attach.bin"));
    assert!(
        report
            .name_conflicts
            .iter()
            .any(|item| item == "/wellrox-cloud -> wellrox-cloud (2)")
    );
    assert!(
        report
            .name_conflicts
            .iter()
            .any(|item| item.contains("Plan Q1_Q2.txt"))
    );

    let key = |name: &str| Some(format!("files/{name}"));
    let old = millis("2026-10-30T09:00:00.000Z");
    let ancient = millis("2026-01-01T00:00:00.000Z");
    let w = "wellrox-cloud (2)";
    assert_eq!(
        fixture.tree().await,
        vec![
            ("wellrox-cloud".to_owned(), "folder".to_owned(), None, None),
            (w.to_owned(), "folder".to_owned(), None, None),
            (format!("{w}/Marketing"), "folder".to_owned(), None, None),
            (
                format!("{w}/Marketing/Campaigns"),
                "folder".to_owned(),
                None,
                None
            ),
            (
                format!("{w}/Marketing/Campaigns/Q1 Bericht.pdf"),
                "file".to_owned(),
                None,
                key("q1.pdf")
            ),
            (
                format!("{w}/Marketing/ancient.txt"),
                "file".to_owned(),
                Some(ancient),
                key("ancient.txt")
            ),
            (
                format!("{w}/Marketing/logo.png"),
                "file".to_owned(),
                None,
                key("logo.png")
            ),
            (format!("{w}/Old"), "folder".to_owned(), Some(old), None),
            (
                format!("{w}/Old/old.txt"),
                "file".to_owned(),
                None,
                key("old.txt")
            ),
            (
                format!("{w}/Plan Q1_Q2.txt"),
                "file".to_owned(),
                None,
                key("plan.txt")
            ),
            (
                format!("{w}/notes.txt"),
                "file".to_owned(),
                None,
                key("notes.txt")
            ),
        ]
    );
    // The Worker's ids and times are kept; the bytes read through Drive's own URL.
    let (source_ref, created, modified): (String, i64, i64) = sqlx::query_as(
        "SELECT source_ref, source_created_at, source_modified_at FROM drive_nodes WHERE name = 'Q1 Bericht.pdf'",
    )
    .fetch_one(fixture.database.pool())
    .await
    .unwrap();
    assert_eq!(source_ref, "q1");
    assert_eq!(
        (created, modified),
        (
            millis("2026-09-01T10:00:00.000Z"),
            millis("2026-09-02T11:00:00.000Z")
        )
    );
    let q1: String = sqlx::query_scalar("SELECT id FROM drive_nodes WHERE source_ref = 'q1'")
        .fetch_one(fixture.database.pool())
        .await
        .unwrap();
    let (_, bytes) = fixture
        .drive
        .file_bytes(fixture.workspace, fixture.user, q1.parse().unwrap())
        .await
        .unwrap();
    assert_eq!(bytes, b"pdf!");
}

fn report_without_dry_run(report: &ImportReport) -> ImportReport {
    ImportReport {
        dry_run: false,
        folders: report.folders,
        files: report.files,
        bytes: report.bytes,
        trashed: report.trashed,
        expired_trash: report.expired_trash,
        already_imported: report.already_imported,
        purged: report.purged,
        missing_objects: report.missing_objects.clone(),
        refused_keys: report.refused_keys.clone(),
        name_conflicts: report.name_conflicts.clone(),
        orphans: report.orphans,
        size_mismatches: report.size_mismatches.clone(),
    }
}

#[tokio::test]
async fn a_second_run_adds_only_what_is_new_and_keeps_members_changes() {
    let fixture = Fixture::new().await;
    let rows = worker_drive(&fixture).await;
    let first = fixture.export("first.sql", &rows).await;
    fixture.run(&first, false).await;
    let before = fixture.tree().await;

    // A member renames an imported file; the Worker gets a new file meanwhile, and the missing object appears.
    sqlx::query("UPDATE drive_nodes SET name = 'Notizen.txt', name_lower = 'notizen.txt' WHERE source_ref = 'notes'")
        .execute(fixture.database.pool())
        .await
        .unwrap();
    let key = fixture.object("new.txt", Some(b"new")).await;
    fixture.object("missing.docx", Some(b"now here")).await;
    let second = fixture
        .export(
            "second.sql",
            &format!(
                "{rows}{}{}",
                blob("b-new", &key, 3, false),
                node("new", "campaigns", "file", "new.txt", Some("b-new"), None)
            ),
        )
        .await;
    let again = fixture.run(&second, false).await;
    assert_eq!((again.files, again.folders), (2, 0), "{again}");
    assert_eq!(
        again.already_imported,
        u64::try_from(before.len() - 1).unwrap(),
        "every earlier node but the target folder"
    );
    assert!(
        again
            .size_mismatches
            .iter()
            .any(|item| item.contains("missing.docx"))
    );
    let after = fixture.tree().await;
    assert_eq!(after.len(), before.len() + 2);
    assert!(
        after
            .iter()
            .any(|row| row.0 == "wellrox-cloud/Marketing/Campaigns/new.txt")
    );
    assert!(
        after.iter().any(|row| row.0 == "wellrox-cloud/Notizen.txt"),
        "the member's rename stays"
    );
    assert!(
        !after.iter().any(|row| row.0 == "wellrox-cloud/notes.txt"),
        "no second copy"
    );

    // A third run of the same export changes nothing.
    let third = fixture.run(&second, false).await;
    assert_eq!((third.files, third.folders), (0, 0));
    assert_eq!(fixture.tree().await, after);
}

#[tokio::test]
async fn bad_exports_are_refused_before_anything_is_written() {
    let fixture = Fixture::new().await;
    for (name, dump) in [
        (
            "attach.sql",
            format!("{SCHEMA}ATTACH DATABASE '/tmp/x.db' AS x;"),
        ),
        (
            "no-trash.sql",
            SCHEMA.replace("CREATE TABLE trash_entries", "CREATE TABLE other_entries"),
        ),
        ("broken.sql", "CREATE TABLE nodes (".to_owned()),
    ] {
        let path = fixture.dir.path().join(name);
        std::fs::write(&path, dump).unwrap();
        let error = import(
            &fixture.database,
            &fixture.bucket,
            fixture.workspace,
            &path,
            false,
            TimestampMillis::now(),
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("export"), "{name}: {error}");
    }
    assert!(fixture.tree().await.is_empty());
}

#[tokio::test]
async fn a_real_run_refuses_a_database_that_a_serving_orbit_owns() {
    if orbit_platform::testing_postgres() {
        // The ownership lock of a SQLite file; Postgres has its own lock test in the platform crate.
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("orbit.sqlite");
    let owner = Database::open(&DatabaseConfig::new(&path)).await.unwrap();
    let export = dir.path().join("d1.sql");
    std::fs::write(&export, SCHEMA).unwrap();
    let run = |dry_run: bool| {
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_orbit"));
        command
            .args([
                "drive",
                "import-wellrox-cloud",
                "--workspace",
                &Id::new_v7().to_string(),
                "--d1-export",
            ])
            .arg(&export)
            .env_clear()
            .env("PATH", std::env::var("PATH").unwrap_or_default())
            .env("ORBIT_ENV", "development")
            .env(
                "ORBIT_APP_KEY",
                "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
            )
            .env("ORBIT_DATABASE_PATH", &path)
            .env("ORBIT_ATTACHMENTS_PATH", dir.path().join("attachments"))
            .env("ORBIT_BACKUPS_PATH", dir.path().join("backups"));
        if dry_run {
            command.arg("--dry-run");
        }
        command.output().unwrap()
    };
    // A dry run too: it makes the same transaction and rolls it back.
    for dry_run in [false, true] {
        let output = run(dry_run);
        assert!(!output.status.success());
        let message = String::from_utf8_lossy(&output.stderr);
        assert!(
            message.contains("Orbit is running on this database: stop it"),
            "{message}"
        );
    }
    let tables: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_master WHERE name = 'drive_nodes'")
            .fetch_one(owner.pool())
            .await
            .unwrap();
    assert_eq!(
        tables, 0,
        "the refused runs did not even migrate the database"
    );
    drop(owner);
    // Once Orbit is gone, the run gets past the lock (and stops at the missing Drive bucket).
    let output = run(true);
    let message = String::from_utf8_lossy(&output.stderr);
    assert!(message.contains("no S3 bucket for Drive"), "{message}");
}
