//! Attachments and backups in S3. The tests run against a small in-memory S3 server; set `ORBIT_TEST_S3_ENDPOINT`,
//! `ORBIT_TEST_S3_BUCKET`, `ORBIT_TEST_S3_ACCESS_KEY` and `ORBIT_TEST_S3_SECRET_KEY` to run them against a real
//! S3-compatible server (path-style URLs) instead.

use std::sync::Arc;

use chrono::Utc;
use orbit_platform::{
    AttachmentMutationCoordinator, BackupService, BlobStore, Database, DatabaseConfig, Id,
    LocalBlobStore, MigrationRunner, NewAttachmentReference, ObjectStorage, ObjectStorageState,
    S3_MIN_PART_BYTES, S3Bucket, S3Config, S3Error, TestDatabase, TieredBlobStore, UploadLimits,
    UploadService, WorkspaceBucket, start_fake_s3,
};
use tokio::io::AsyncReadExt;

const DAY_MILLIS: i64 = 24 * 60 * 60 * 1000;

/// A bucket under a fresh prefix, so runs against a real server do not see each other's objects.
async fn bucket() -> S3Bucket {
    let prefix = format!("orbit-test-{}", Id::new_v7());
    let config = match std::env::var("ORBIT_TEST_S3_ENDPOINT") {
        Ok(endpoint) => S3Config {
            endpoint,
            region: "us-east-1".to_owned(),
            bucket: std::env::var("ORBIT_TEST_S3_BUCKET").unwrap(),
            prefix,
            access_key_id: std::env::var("ORBIT_TEST_S3_ACCESS_KEY").unwrap(),
            secret_access_key: std::env::var("ORBIT_TEST_S3_SECRET_KEY").unwrap(),
            path_style: true,
        },
        Err(_) => S3Config {
            prefix,
            ..start_fake_s3().await
        },
    };
    S3Bucket::new(config).unwrap()
}

struct Fixture {
    _root: tempfile::TempDir,
    database: TestDatabase,
    local: LocalBlobStore,
    bucket: S3Bucket,
    storage: ObjectStorage,
    store: TieredBlobStore,
    service: UploadService,
}

impl Fixture {
    async fn new(attachments_in_s3: bool) -> Self {
        let root = tempfile::tempdir().unwrap();
        let database = TestDatabase::new().await.unwrap();
        let local = LocalBlobStore::new(root.path().join("attachments"));
        let bucket = bucket().await;
        let storage = ObjectStorage::default();
        storage.configure(ObjectStorageState {
            bucket: Some(bucket.clone()),
            attachments: attachments_in_s3,
            backups: false,
            ..Default::default()
        });
        let mutations = AttachmentMutationCoordinator::default();
        let store = TieredBlobStore::new(local.clone(), storage.clone(), mutations.clone());
        let service = UploadService::new(
            (*database).clone(),
            Arc::new(store.clone()),
            mutations,
            UploadLimits::default(),
        );
        Self {
            _root: root,
            database,
            local,
            bucket,
            storage,
            store,
            service,
        }
    }

    async fn upload(&self, contents: &'static [u8]) -> String {
        self.upload_to(Id::new_v7(), contents).await
    }

    async fn upload_to(&self, workspace_id: Id, contents: &'static [u8]) -> String {
        let staged = self
            .service
            .stage(workspace_id, Id::new_v7(), "file.txt", contents)
            .await
            .unwrap();
        self.service
            .finalize(&staged, NewAttachmentReference::new(Id::new_v7(), None))
            .await
            .unwrap()
            .storage_key
    }

    async fn read(&self, storage_key: &str) -> Vec<u8> {
        let mut bytes = Vec::new();
        self.store
            .open(storage_key)
            .await
            .unwrap()
            .read_to_end(&mut bytes)
            .await
            .unwrap();
        bytes
    }

    async fn in_bucket(&self, storage_key: &str) -> bool {
        self.bucket
            .last_modified(storage_key)
            .await
            .unwrap()
            .is_some()
    }
}

#[tokio::test]
async fn s3_attachments_are_stored_read_and_kept_after_release_until_the_delete_delay() {
    let fixture = Fixture::new(true).await;
    let key = fixture.upload(b"in the bucket").await;
    assert!(fixture.in_bucket(&key).await);
    assert!(fixture.local.blobs().await.unwrap().is_empty());
    assert!(fixture.local.temporary_files().await.unwrap().is_empty());
    assert_eq!(fixture.read(&key).await, b"in the bucket");

    fixture
        .database
        .execute("DELETE FROM attachment_references")
        .await
        .unwrap();
    let now = Utc::now().timestamp_millis();
    let released = fixture
        .service
        .reconcile(now + 2 * DAY_MILLIS)
        .await
        .unwrap();
    assert_eq!(released.deleted_blobs, 1);
    assert_eq!(released.deleted_untracked_files, 0);
    assert!(
        fixture.in_bucket(&key).await,
        "a restored backup may still need the file"
    );

    let later = fixture
        .service
        .reconcile(now + 39 * DAY_MILLIS)
        .await
        .unwrap();
    assert_eq!(later.deleted_untracked_files, 0);
    let expired = fixture
        .service
        .reconcile(now + 41 * DAY_MILLIS)
        .await
        .unwrap();
    assert_eq!(expired.deleted_untracked_files, 1);
    assert!(!fixture.in_bucket(&key).await);
}

#[tokio::test]
async fn the_mover_moves_blobs_to_s3_and_back_while_reads_keep_working() {
    let fixture = Fixture::new(false).await;
    let key = fixture.upload(b"first on disk").await;
    assert_eq!(fixture.local.blobs().await.unwrap().len(), 1);
    assert!(!fixture.in_bucket(&key).await);

    fixture.storage.configure(ObjectStorageState {
        bucket: Some(fixture.bucket.clone()),
        attachments: true,
        backups: false,
        ..Default::default()
    });
    assert_eq!(fixture.read(&key).await, b"first on disk");
    assert_eq!(fixture.store.move_blobs().await.unwrap(), 1);
    assert!(fixture.local.blobs().await.unwrap().is_empty());
    assert!(fixture.in_bucket(&key).await);
    assert_eq!(fixture.read(&key).await, b"first on disk");
    assert_eq!(fixture.store.status().remaining, 0);

    fixture.storage.configure(ObjectStorageState {
        bucket: Some(fixture.bucket.clone()),
        attachments: false,
        backups: false,
        ..Default::default()
    });
    assert_eq!(fixture.store.move_blobs().await.unwrap(), 1);
    assert!(!fixture.in_bucket(&key).await);
    assert_eq!(fixture.local.blobs().await.unwrap().len(), 1);
    assert_eq!(fixture.read(&key).await, b"first on disk");
    assert!(!fixture.bucket.any("blobs/").await.unwrap());
}

fn with_workspace_bucket(
    fixture: &Fixture,
    workspace_id: Id,
    bucket: &S3Bucket,
    active: bool,
) -> ObjectStorageState {
    ObjectStorageState {
        bucket: Some(fixture.bucket.clone()),
        attachments: true,
        backups: false,
        workspace_buckets: [(
            workspace_id.to_string(),
            WorkspaceBucket {
                bucket: bucket.clone(),
                active,
            },
        )]
        .into(),
    }
}

async fn holds(bucket: &S3Bucket, storage_key: &str) -> bool {
    bucket.last_modified(storage_key).await.unwrap().is_some()
}

#[tokio::test]
async fn a_workspace_bucket_takes_the_workspace_files_and_gives_them_back_when_removed() {
    let fixture = Fixture::new(true).await;
    let workspace = Id::new_v7();
    let other = fixture.upload(b"another workspace").await;
    let before = fixture.upload_to(workspace, b"before the bucket").await;
    let own = bucket().await;

    fixture
        .storage
        .configure(with_workspace_bucket(&fixture, workspace, &own, true));
    let after = fixture.upload_to(workspace, b"after the bucket").await;
    assert!(holds(&own, &after).await);
    assert!(!fixture.in_bucket(&after).await);
    assert_eq!(fixture.read(&before).await, b"before the bucket");

    assert_eq!(fixture.store.move_blobs().await.unwrap(), 1);
    assert!(holds(&own, &before).await);
    assert!(!fixture.in_bucket(&before).await);
    assert!(fixture.in_bucket(&other).await, "other workspaces stay");
    assert!(!holds(&own, &other).await);
    assert_eq!(
        fixture
            .store
            .workspace_status(&workspace.to_string())
            .unwrap()
            .remaining,
        0
    );
    assert_eq!(fixture.read(&before).await, b"before the bucket");
    assert_eq!(fixture.read(&other).await, b"another workspace");

    fixture
        .storage
        .configure(with_workspace_bucket(&fixture, workspace, &own, false));
    assert_eq!(fixture.read(&after).await, b"after the bucket");
    let removed = fixture.upload_to(workspace, b"after removal").await;
    assert!(fixture.in_bucket(&removed).await);
    assert!(!holds(&own, &removed).await);

    fixture.store.move_blobs().await.unwrap();
    assert!(!own.any("blobs/").await.unwrap());
    assert!(fixture.in_bucket(&before).await);
    assert!(fixture.in_bucket(&after).await);
    assert!(fixture.local.blobs().await.unwrap().is_empty());
    assert_eq!(fixture.read(&after).await, b"after the bucket");
}

#[tokio::test]
async fn existing_files_on_this_server_move_to_a_workspace_bucket_and_back() {
    let fixture = Fixture::new(false).await;
    let workspace = Id::new_v7();
    let id = workspace.to_string();
    let first = fixture.upload_to(workspace, b"first").await;
    let second = fixture.upload_to(workspace, b"second").await;
    let other = fixture.upload(b"another workspace").await;
    let own = bucket().await;
    let state = |active| ObjectStorageState {
        workspace_buckets: [(
            id.clone(),
            WorkspaceBucket {
                bucket: own.clone(),
                active,
            },
        )]
        .into(),
        ..Default::default()
    };

    fixture.storage.configure(state(true));
    assert_eq!(fixture.store.workspace_status(&id), None, "not counted yet");
    assert_eq!(fixture.store.move_blobs().await.unwrap(), 2);
    assert_eq!(fixture.store.workspace_status(&id).unwrap().remaining, 0);
    let local: Vec<_> = fixture
        .local
        .blobs()
        .await
        .unwrap()
        .into_iter()
        .map(|object| object.storage_key)
        .collect();
    assert_eq!(local, std::slice::from_ref(&other));
    assert!(holds(&own, &first).await && holds(&own, &second).await);
    assert_eq!(fixture.read(&second).await, b"second");

    fixture.storage.configure(state(false));
    assert_eq!(
        fixture.store.workspace_status(&id),
        None,
        "not counted since the removal"
    );
    assert_eq!(fixture.store.move_blobs().await.unwrap(), 2);
    assert_eq!(fixture.store.workspace_status(&id).unwrap().remaining, 0);
    assert!(!own.any("blobs/").await.unwrap());
    assert_eq!(fixture.local.blobs().await.unwrap().len(), 3);
    assert_eq!(fixture.read(&first).await, b"first");
}

#[tokio::test]
async fn a_workspace_bucket_that_fails_does_not_stop_the_cleanup() {
    let fixture = Fixture::new(false).await;
    let workspace = Id::new_v7();
    let unreachable = S3Bucket::new(S3Config {
        endpoint: "http://127.0.0.1:9".to_owned(),
        ..fixture.bucket.config().clone()
    })
    .unwrap();
    let key = fixture.upload(b"unused").await;
    fixture.storage.configure(ObjectStorageState {
        bucket: Some(fixture.bucket.clone()),
        workspace_buckets: [(
            workspace.to_string(),
            WorkspaceBucket {
                bucket: unreachable,
                active: true,
            },
        )]
        .into(),
        ..Default::default()
    });
    fixture
        .database
        .execute("DELETE FROM attachment_references")
        .await
        .unwrap();

    let result = fixture
        .service
        .reconcile(Utc::now().timestamp_millis() + 2 * DAY_MILLIS)
        .await
        .unwrap();
    assert_eq!(result.deleted_blobs, 1);
    assert!(
        fixture
            .local
            .blobs()
            .await
            .unwrap()
            .iter()
            .all(|object| object.storage_key != key)
    );

    fixture.storage.configure(ObjectStorageState {
        bucket: Some(fixture.bucket.clone()),
        ..Default::default()
    });
    let waiting = fixture.upload_to(workspace, b"waits for the bucket").await;
    fixture.storage.configure(ObjectStorageState {
        bucket: Some(fixture.bucket.clone()),
        workspace_buckets: [(
            workspace.to_string(),
            WorkspaceBucket {
                bucket: S3Bucket::new(S3Config {
                    endpoint: "http://127.0.0.1:9".to_owned(),
                    ..fixture.bucket.config().clone()
                })
                .unwrap(),
                active: true,
            },
        )]
        .into(),
        ..Default::default()
    });
    fixture.store.move_blobs().await.unwrap();
    let status = fixture
        .store
        .workspace_status(&workspace.to_string())
        .unwrap();
    assert!(status.error.is_some());
    assert_eq!(status.remaining, 1);
    assert_eq!(fixture.read(&waiting).await, b"waits for the bucket");
}

#[test]
fn a_workspace_bucket_must_be_public_https() {
    let config = |endpoint: &str| S3Config {
        endpoint: endpoint.to_owned(),
        region: String::new(),
        bucket: "files".to_owned(),
        prefix: String::new(),
        access_key_id: "key".to_owned(),
        secret_access_key: "secret".to_owned(),
        path_style: true,
    };
    for endpoint in [
        "http://s3.example.com",
        "https://127.0.0.1",
        "https://10.0.0.2:9000",
        "https://169.254.169.254",
        "https://[::1]",
    ] {
        assert!(
            S3Bucket::new_public(config(endpoint)).is_err(),
            "{endpoint}"
        );
    }
    assert!(S3Bucket::new_public(config("https://s3.example.com")).is_ok());
    assert!(S3Bucket::new(config("http://127.0.0.1:9000")).is_ok());
}

#[tokio::test]
async fn a_duplicate_upload_to_s3_refreshes_the_existing_object() {
    let fixture = Fixture::new(true).await;
    let first = fixture.upload(b"same").await;
    let before = fixture.bucket.last_modified(&first).await.unwrap().unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    let staged = fixture
        .service
        .stage(
            first.split('/').nth(1).unwrap().parse().unwrap(),
            Id::new_v7(),
            "again.txt",
            &b"same"[..],
        )
        .await
        .unwrap();
    let second = fixture
        .service
        .finalize(&staged, NewAttachmentReference::new(Id::new_v7(), None))
        .await
        .unwrap();
    assert_eq!(second.storage_key, first);
    let after = fixture.bucket.last_modified(&first).await.unwrap().unwrap();
    assert!(after > before, "deduplication restarts the quarantine");
}

#[tokio::test]
async fn backups_go_to_s3_and_restore_from_it() {
    if orbit_platform::testing_postgres() {
        return; // file snapshots are SQLite-only
    }
    let root = tempfile::tempdir().unwrap();
    let database = Database::open(&DatabaseConfig::new(root.path().join("orbit.sqlite")))
        .await
        .unwrap();
    MigrationRunner::embedded(env!("CARGO_PKG_VERSION"))
        .run(&database)
        .await
        .unwrap();
    let attachments = root.path().join("attachments");
    std::fs::create_dir_all(&attachments).unwrap();
    std::fs::write(attachments.join("record.txt"), b"attachment contents").unwrap();
    let storage = ObjectStorage::default();
    storage.configure(ObjectStorageState {
        bucket: Some(bucket().await),
        attachments: false,
        backups: true,
        ..Default::default()
    });
    let backups_root = root.path().join("backups");
    let service = BackupService::new(&backups_root, &attachments).with_object_storage(storage);

    let snapshot = service.create(&database).await.unwrap();
    assert!(snapshot.in_object_storage);
    assert!(!backups_root.join("snapshots").join(&snapshot.id).exists());
    let listed = service.list().await.unwrap();
    assert_eq!(listed.len(), 1);
    assert!(listed[0].in_object_storage);
    assert_eq!(listed[0].manifest, snapshot.manifest);

    let verified = service.verify(&snapshot.id).await.unwrap();
    assert!(verified.in_object_storage);
    drop(database);
    let target = root.path().join("restored");
    service
        .restore_to(
            &snapshot.id,
            &target.join("orbit.sqlite"),
            &target.join("attachments"),
        )
        .await
        .unwrap();
    assert_eq!(
        std::fs::read(target.join("attachments/record.txt")).unwrap(),
        b"attachment contents"
    );
    assert!(target.join("orbit.sqlite").exists());

    service.delete(&snapshot.id).await.unwrap();
    assert!(service.list().await.unwrap().is_empty());
    assert!(!backups_root.join("downloads").join(&snapshot.id).exists());
    assert!(matches!(
        service.delete(&snapshot.id).await,
        Err(orbit_platform::BackupError::NotFound { .. })
    ));
}

#[test]
fn buckets_overlap_on_one_server_and_bucket_when_one_prefix_contains_the_other() {
    let config = |endpoint: &str, bucket: &str, prefix: &str| S3Config {
        endpoint: endpoint.to_owned(),
        region: String::new(),
        bucket: bucket.to_owned(),
        prefix: prefix.to_owned(),
        access_key_id: "key".to_owned(),
        secret_access_key: "secret".to_owned(),
        path_style: true,
    };
    let at = |prefix: &str| config("https://s3.example.com", "files", prefix);
    for (left, right) in [
        ("", ""),
        ("x", "x"),
        ("", "x/"),
        ("x/", "x/y/"),
        ("/x", "x"),
    ] {
        assert!(at(left).overlaps(&at(right)), "{left:?} {right:?}");
        assert!(at(right).overlaps(&at(left)), "{right:?} {left:?}");
    }
    for (left, right) in [("a/", "b/"), ("x", "xy"), ("x/y", "x/z")] {
        assert!(!at(left).overlaps(&at(right)), "{left:?} {right:?}");
    }
    // The same server written another way.
    for endpoint in [
        "https://S3.Example.com/",
        "https://s3.example.com:443",
        "http://s3.example.com:443",
    ] {
        assert!(
            config(endpoint, "files", "").overlaps(&at("")),
            "{endpoint}"
        );
    }
    assert!(
        config("https://s3.eu-central-1.amazonaws.com", "files", "").overlaps(&config(
            "https://s3.amazonaws.com",
            "files",
            ""
        )),
        "bucket names are global on AWS"
    );
    for other in [
        config("https://s3.example.com", "other", ""),
        config("https://s3.example.com:9000", "files", ""),
        config("https://minio.example.com", "files", ""),
    ] {
        assert!(!other.overlaps(&at("")), "{other:?}");
    }
}

/// A workspace bucket and the installation's bucket that are the same place (as two saves once allowed): the
/// mover must not delete what it believes it moved.
async fn same_place(fixture: &Fixture, workspace: Id, attachments: bool, active: bool) {
    fixture.storage.configure(ObjectStorageState {
        bucket: Some(fixture.bucket.clone()),
        attachments,
        workspace_buckets: [(
            workspace.to_string(),
            WorkspaceBucket {
                bucket: S3Bucket::new(fixture.bucket.config().clone()).unwrap(),
                active,
            },
        )]
        .into(),
        ..Default::default()
    });
}

#[tokio::test]
async fn the_mover_keeps_files_when_a_workspace_bucket_is_the_installations_bucket() {
    let fixture = Fixture::new(true).await;
    let workspace = Id::new_v7();
    let keys = [
        fixture.upload_to(workspace, b"first").await,
        fixture.upload_to(workspace, b"second").await,
    ];
    assert!(fixture.in_bucket(&keys[0]).await && fixture.in_bucket(&keys[1]).await);

    same_place(&fixture, workspace, true, true).await;
    fixture.store.move_blobs().await.unwrap();
    for key in &keys {
        assert!(fixture.in_bucket(key).await, "{key} was deleted");
    }
    assert_eq!(fixture.read(&keys[0]).await, b"first");
    assert_eq!(
        fixture
            .store
            .workspace_status(&workspace.to_string())
            .unwrap()
            .remaining,
        0
    );
}

#[tokio::test]
async fn the_installation_pass_keeps_files_when_a_workspace_bucket_is_the_installations_bucket() {
    for (attachments, active) in [(false, true), (true, false), (false, false)] {
        let fixture = Fixture::new(true).await;
        let workspace = Id::new_v7();
        let own = fixture.upload_to(workspace, b"own").await;
        let other = fixture.upload(b"another workspace").await;

        same_place(&fixture, workspace, attachments, active).await;
        fixture.store.move_blobs().await.unwrap();
        fixture.store.move_blobs().await.unwrap();
        for (key, contents) in [(&own, &b"own"[..]), (&other, &b"another workspace"[..])] {
            assert_eq!(
                fixture.read(key).await,
                contents,
                "attachments {attachments}, active {active}"
            );
        }
    }
}

const PRESIGN_LIFETIME: std::time::Duration = std::time::Duration::from_secs(300);

/// A part of the smallest size S3 accepts before the last part.
fn full_part(fill: u8) -> Vec<u8> {
    vec![fill; usize::try_from(S3_MIN_PART_BYTES).unwrap()]
}

/// Sends the bytes to a presigned upload URL the way a browser does (no credentials) and returns the ETag.
async fn put_presigned(url: String, body: Vec<u8>) -> String {
    let response = reqwest::Client::new()
        .put(url)
        .body(body)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    response.headers()["etag"].to_str().unwrap().to_owned()
}

#[tokio::test]
async fn presigned_urls_upload_and_download_without_credentials() {
    let bucket = bucket().await;
    let upload = bucket.presign_put("drive/file", PRESIGN_LIFETIME).unwrap();
    put_presigned(upload, b"from the browser".to_vec()).await;
    assert_eq!(
        bucket.get_bytes("drive/file").await.unwrap().as_deref(),
        Some(b"from the browser".as_slice())
    );

    let download = bucket
        .presign_get("drive/file", PRESIGN_LIFETIME, Some("Q1 Bericht ä.txt"))
        .unwrap();
    let response = reqwest::get(download)
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    assert_eq!(
        response.headers()["content-disposition"].to_str().unwrap(),
        "attachment; filename=\"Q1 Bericht _.txt\"; filename*=UTF-8''Q1%20Bericht%20%C3%A4.txt"
    );
    assert_eq!(
        response.bytes().await.unwrap(),
        b"from the browser".as_slice()
    );

    let mut tampered = bucket
        .presign_get("drive/file", PRESIGN_LIFETIME, None)
        .unwrap();
    tampered.truncate(tampered.find("X-Amz-Signature=").unwrap());
    assert!(!reqwest::get(tampered).await.unwrap().status().is_success());
}

#[tokio::test]
async fn a_multipart_upload_resumes_from_its_listed_parts_and_completes() {
    let bucket = bucket().await;
    let key = "drive/video";
    let upload_id = bucket
        .create_multipart_upload(key, "video/mp4")
        .await
        .unwrap();
    // Part 2 first: the browser sends parts in parallel, in any order.
    let second = put_presigned(
        bucket
            .presign_upload_part(key, &upload_id, 2, PRESIGN_LIFETIME)
            .unwrap(),
        full_part(2),
    )
    .await;

    // After a reload, the listed parts say what is left to send.
    let listed = bucket.list_parts(key, &upload_id).await.unwrap().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(
        (
            listed[0].part_number,
            listed[0].etag.as_str(),
            listed[0].size
        ),
        (2, second.as_str(), S3_MIN_PART_BYTES)
    );
    let first = bucket
        .upload_part(key, &upload_id, 1, full_part(1))
        .await
        .unwrap();
    let third = put_presigned(
        bucket
            .presign_upload_part(key, &upload_id, 3, PRESIGN_LIFETIME)
            .unwrap(),
        b"end".to_vec(),
    )
    .await;
    assert!(bucket.get_bytes(key).await.unwrap().is_none());

    bucket
        .complete_multipart_upload(key, &upload_id, &[(3, third), (1, first), (2, second)])
        .await
        .unwrap();
    let mut expected = full_part(1);
    expected.extend(full_part(2));
    expected.extend(b"end");
    assert!(bucket.get_bytes(key).await.unwrap() == Some(expected));
    assert!(bucket.list_parts(key, &upload_id).await.unwrap().is_none());
    // Once completed, the upload is gone; aborting it is not an error.
    bucket
        .abort_multipart_upload(key, &upload_id)
        .await
        .unwrap();
}

#[tokio::test]
async fn completing_with_a_wrong_or_too_small_part_fails_and_abort_drops_the_parts() {
    let bucket = bucket().await;
    let key = "drive/broken";
    let upload_id = bucket
        .create_multipart_upload(key, "application/octet-stream")
        .await
        .unwrap();
    let small = bucket
        .upload_part(key, &upload_id, 1, b"too small".to_vec())
        .await
        .unwrap();
    let last = bucket
        .upload_part(key, &upload_id, 2, b"last".to_vec())
        .await
        .unwrap();
    let refused =
        |result: Result<(), S3Error>| matches!(result, Err(S3Error::Status { status: 400, .. }));
    assert!(refused(
        bucket
            .complete_multipart_upload(key, &upload_id, &[(1, small.clone()), (2, last.clone())])
            .await
    ));
    assert!(refused(
        bucket
            .complete_multipart_upload(key, &upload_id, &[(1, last.clone())])
            .await
    ));
    assert!(refused(
        bucket
            .complete_multipart_upload(key, &upload_id, &[(3, last)])
            .await
    ));

    bucket
        .abort_multipart_upload(key, &upload_id)
        .await
        .unwrap();
    assert!(bucket.list_parts(key, &upload_id).await.unwrap().is_none());
    assert!(bucket.get_bytes(key).await.unwrap().is_none());
    bucket
        .abort_multipart_upload(key, &upload_id)
        .await
        .unwrap();
}

#[tokio::test]
async fn copy_object_copies_within_the_bucket_and_reports_a_missing_source() {
    let bucket = bucket().await;
    bucket
        .put_bytes("drive/source name", b"original".to_vec())
        .await
        .unwrap();
    assert!(
        bucket
            .copy_object("drive/source name", "drive/copy")
            .await
            .unwrap()
    );
    assert_eq!(
        bucket.get_bytes("drive/copy").await.unwrap().as_deref(),
        Some(b"original".as_slice())
    );
    assert!(
        bucket
            .get_bytes("drive/source name")
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(bucket.size("drive/copy").await.unwrap(), Some(8));
    assert_eq!(bucket.size("drive/missing").await.unwrap(), None);
    assert!(
        !bucket
            .copy_object("drive/missing", "drive/other")
            .await
            .unwrap()
    );
    assert!(bucket.get_bytes("drive/other").await.unwrap().is_none());
}
