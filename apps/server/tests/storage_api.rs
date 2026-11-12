//! Object storage (S3) settings in Admin → Storage, against the in-memory S3 server.

use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use orbit_platform::{
    AttachmentMutationCoordinator, LocalBlobStore, ObjectStorage, PasswordService, TestDatabase,
    TieredBlobStore, TimestampMillis, start_fake_s3,
};
use orbit_server::mail::Mailer;
use orbit_server::object_storage::saved_storage;
use orbit_server::repositories::identity::{IdentityRepository, SetupRequest};
use orbit_server::workspace_routes::{WorkspaceState, workspace_router};
use serde_json::{Value, json};
use tower::ServiceExt;

struct Fixture {
    database: TestDatabase,
    _attachments: tempfile::TempDir,
    storage: ObjectStorage,
    app: axum::Router,
    cookie: String,
    workspace_id: orbit_platform::Id,
}

async fn fixture() -> Fixture {
    let database = TestDatabase::new().await.unwrap();
    let identity = Arc::new(IdentityRepository::new((*database).clone()));
    let now = TimestampMillis::now();
    identity
        .store_setup_token(
            "operator-secret",
            TimestampMillis::from_millis(now.as_millis() + 60_000),
        )
        .await
        .unwrap();
    let setup = identity
        .complete_setup(
            SetupRequest {
                token: "operator-secret".to_owned(),
                email: "root@example.com".to_owned(),
                display_name: "Root".to_owned(),
                password_hash: PasswordService::default()
                    .hash("correct horse battery")
                    .unwrap(),
                workspace_name: "Orbit".to_owned(),
                project_name: "General".to_owned(),
            },
            now,
        )
        .await
        .unwrap();
    let attachments = tempfile::tempdir().unwrap();
    let storage = ObjectStorage::default();
    let store = TieredBlobStore::new(
        LocalBlobStore::new(attachments.path()),
        storage.clone(),
        AttachmentMutationCoordinator::default(),
    );
    let (mailer, _) = Mailer::recording((*database).clone(), "https://orbit.test");
    let app = workspace_router(
        WorkspaceState::new(
            Arc::clone(&identity),
            "https://orbit.test".to_owned(),
            orbit_server::auth_routes::CookieMode::secure(),
        )
        .with_mailer(mailer)
        .with_object_storage(storage.clone(), store),
    );
    Fixture {
        database,
        _attachments: attachments,
        storage,
        app,
        cookie: format!("__Host-orbit_session={}", setup.session.token),
        workspace_id: setup.workspace_id,
    }
}

impl Fixture {
    async fn call(&self, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        let request = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::COOKIE, &self.cookie);
        let request = match body {
            Some(body) => request
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string())),
            None => request.body(Body::empty()),
        }
        .unwrap();
        let response = self.app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }
}

async fn s3_body() -> Value {
    let config = start_fake_s3().await;
    json!({
        "endpoint": config.endpoint, "region": "", "bucket": config.bucket, "prefix": "/orbit/",
        "access_key_id": "key", "secret_access_key": "plain-secret-value", "path_style": true
    })
}

#[tokio::test]
async fn the_root_user_saves_a_checked_bucket_with_an_encrypted_secret() {
    let fixture = fixture().await;
    let mut body = s3_body().await;

    body["access_key_id"] = json!("wrong");
    let (status, problem) = fixture
        .call("PUT", "/api/v1/admin/settings/s3", Some(body.clone()))
        .await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(problem["code"], "storage_unreachable");

    body["access_key_id"] = json!("key");
    let (status, view) = fixture
        .call("PUT", "/api/v1/admin/settings/s3", Some(body.clone()))
        .await;
    assert_eq!(status, StatusCode::OK, "{view}");
    let storage = &view["storage"];
    assert_eq!(storage["s3"]["prefix"], "orbit");
    assert_eq!(storage["s3"]["region"], "us-east-1");
    assert_eq!(storage["attachments_in_s3"], false);
    assert!(storage["s3"].get("secret_access_key").is_none());
    let sealed: Vec<u8> = fixture
        .database
        .scalar("SELECT s3_secret_access_key FROM instance_settings")
        .await
        .unwrap();
    assert!(
        !sealed
            .windows(b"plain-secret-value".len())
            .any(|window| window == b"plain-secret-value")
    );
    assert!(fixture.storage.bucket().is_some());

    // An absent secret keeps the saved one, which still works.
    body.as_object_mut().unwrap().remove("secret_access_key");
    body["region"] = json!("eu-central-1");
    let (status, view) = fixture
        .call("PUT", "/api/v1/admin/settings/s3", Some(body))
        .await;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert_eq!(view["storage"]["s3"]["region"], "eu-central-1");
}

#[tokio::test]
async fn storage_options_need_a_bucket_and_the_bucket_stays_while_attachments_use_it() {
    let fixture = fixture().await;
    let options =
        json!({"attachments_in_s3": true, "backups_in_s3": true, "backup_schedule": "daily"});
    let (status, problem) = fixture
        .call(
            "PUT",
            "/api/v1/admin/settings/storage",
            Some(options.clone()),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(problem["code"], "storage_not_configured");

    let (status, _) = fixture
        .call("PUT", "/api/v1/admin/settings/s3", Some(s3_body().await))
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, view) = fixture
        .call("PUT", "/api/v1/admin/settings/storage", Some(options))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(view["storage"]["attachments_in_s3"], true);
    assert_eq!(view["storage"]["backup_schedule"], "daily");
    let running = fixture.storage.current();
    assert!(running.attachments && running.backups);

    let (status, problem) = fixture
        .call("DELETE", "/api/v1/admin/settings/s3", None)
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(problem["code"], "storage_in_use");

    let (status, _) = fixture
        .call(
            "PUT",
            "/api/v1/admin/settings/storage",
            Some(json!({"attachments_in_s3": false, "backups_in_s3": false, "backup_schedule": "off"})),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, view) = fixture
        .call("DELETE", "/api/v1/admin/settings/s3", None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(view["storage"]["s3"].is_null());
    assert!(fixture.storage.bucket().is_none());
}

#[tokio::test]
async fn settings_another_server_saved_apply_on_reload() {
    let fixture = fixture().await;
    let (status, _) = fixture
        .call("PUT", "/api/v1/admin/settings/s3", Some(s3_body().await))
        .await;
    assert_eq!(status, StatusCode::OK);
    let options =
        json!({"attachments_in_s3": true, "backups_in_s3": false, "backup_schedule": "off"});
    let (status, _) = fixture
        .call("PUT", "/api/v1/admin/settings/storage", Some(options))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(fixture.storage.current().attachments);

    // Another server of the database turns on backups in S3.
    let root: String = fixture
        .database
        .scalar("SELECT id FROM users")
        .await
        .unwrap();
    orbit_server::repositories::instance_settings::InstanceSettingsRepository::new(
        (*fixture.database).clone(),
    )
    .save_storage_options(
        root.parse().unwrap(),
        true,
        true,
        orbit_server::repositories::instance_settings::BackupSchedule::Off,
        None,
        "other-server",
        TimestampMillis::now(),
    )
    .await
    .unwrap();
    assert!(
        !fixture.storage.current().backups,
        "not applied before a reload"
    );

    orbit_server::object_storage::reload_saved(&fixture.database)
        .await
        .unwrap();
    assert!(fixture.storage.current().backups);
}

#[tokio::test]
async fn a_command_without_the_server_finds_the_backups_in_the_saved_bucket() {
    let fixture = fixture().await;
    let (status, _) = fixture
        .call("PUT", "/api/v1/admin/settings/s3", Some(s3_body().await))
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = fixture
        .call(
            "PUT",
            "/api/v1/admin/settings/storage",
            Some(json!({"attachments_in_s3": false, "backups_in_s3": true, "backup_schedule": "off"})),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (mailer, _) = Mailer::recording((*fixture.database).clone(), "https://orbit.test");
    let app_key = *mailer.app_key().unwrap();
    let backups = tempfile::tempdir().unwrap();
    let attachments = tempfile::tempdir().unwrap();
    let service = |storage| {
        orbit_platform::BackupService::new(backups.path(), attachments.path())
            .with_object_storage(storage)
    };

    let storage = saved_storage(&fixture.database, Some(&app_key))
        .await
        .unwrap();
    let created = service(storage).create(&fixture.database).await.unwrap();
    assert!(created.in_object_storage);

    let listed = service(
        saved_storage(&fixture.database, Some(&app_key))
            .await
            .unwrap(),
    )
    .list()
    .await
    .unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, created.id);
    assert!(listed[0].in_object_storage);

    let error = saved_storage(&fixture.database, Some(&[9; 32]))
        .await
        .unwrap_err();
    assert!(error.contains("cannot be decrypted"), "{error}");
}

/// A workspace bucket in the fake S3 server (a workspace may not save one there through the API) under `prefix`.
async fn save_workspace_bucket(
    fixture: &Fixture,
    workspace_id: orbit_platform::Id,
    body: &Value,
    prefix: &str,
) {
    sqlx::query(
        "INSERT INTO workspace_storage (workspace_id, endpoint, region, bucket, prefix, access_key_id, \
         secret_access_key, path_style, active, updated_at) VALUES ($1, $2, 'us-east-1', $3, $4, 'key', $5, 1, 1, 0)",
    )
    .bind(workspace_id.to_string())
    .bind(body["endpoint"].as_str().unwrap())
    .bind(body["bucket"].as_str().unwrap())
    .bind(prefix)
    .bind(b"sealed".to_vec())
    .execute(fixture.database.pool())
    .await
    .unwrap();
}

async fn other_workspace(fixture: &Fixture) -> orbit_platform::Id {
    let (status, workspace) = fixture
        .call("POST", "/api/v1/workspaces", Some(json!({"name": "Other"})))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{workspace}");
    workspace["id"].as_str().unwrap().parse().unwrap()
}

fn with_prefix(body: &Value, prefix: &str) -> Value {
    let mut body = body.clone();
    body["prefix"] = json!(prefix);
    body
}

#[tokio::test]
async fn the_installation_bucket_cannot_overlap_a_workspace_bucket() {
    for (workspace, installation) in [
        ("orbit", "/orbit/"),
        ("", "x/"),
        ("x", ""),
        ("x/", "x/y/"),
        ("x/y", "x"),
    ] {
        let fixture = fixture().await;
        let body = s3_body().await;
        save_workspace_bucket(&fixture, fixture.workspace_id, &body, workspace).await;
        // A removed bucket counts until its files moved back.
        sqlx::query("UPDATE workspace_storage SET active = 0")
            .execute(fixture.database.pool())
            .await
            .unwrap();
        let (status, problem) = fixture
            .call(
                "PUT",
                "/api/v1/admin/settings/s3",
                Some(with_prefix(&body, installation)),
            )
            .await;
        assert_eq!(
            status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{workspace:?} {installation:?} {problem}"
        );
        assert_eq!(problem["code"], "invalid_storage_settings");
        assert!(
            problem["detail"]
                .as_str()
                .unwrap()
                .starts_with("bucket: a workspace keeps its files"),
            "{problem}"
        );
        assert!(fixture.storage.bucket().is_none());
    }

    // Prefixes side by side are separate places.
    let fixture = fixture().await;
    let body = s3_body().await;
    save_workspace_bucket(&fixture, fixture.workspace_id, &body, "a").await;
    let (status, view) = fixture
        .call(
            "PUT",
            "/api/v1/admin/settings/s3",
            Some(with_prefix(&body, "b/")),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{view}");
}

#[tokio::test]
async fn a_workspace_bucket_cannot_overlap_the_installation_or_another_workspace() {
    let uri =
        |fixture: &Fixture| format!("/api/v1/workspaces/{}/storage/bucket", fixture.workspace_id);
    for (installation, workspace) in [
        ("orbit", "orbit"),
        ("", "x/"),
        ("x", ""),
        ("x/", "x/y/"),
        ("x/y", "x"),
    ] {
        let fixture = fixture().await;
        let body = s3_body().await;
        let (status, view) = fixture
            .call(
                "PUT",
                "/api/v1/admin/settings/s3",
                Some(with_prefix(&body, installation)),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{view}");
        let (status, problem) = fixture
            .call("PUT", &uri(&fixture), Some(with_prefix(&body, workspace)))
            .await;
        assert_eq!(
            status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{installation:?} {workspace:?} {problem}"
        );
        assert!(
            problem["detail"]
                .as_str()
                .unwrap()
                .starts_with("bucket: the installation keeps its files"),
            "{problem}"
        );

        // Another workspace's bucket, also a removed one.
        let fixture = self::fixture().await;
        let other = other_workspace(&fixture).await;
        save_workspace_bucket(&fixture, other, &body, installation).await;
        sqlx::query("UPDATE workspace_storage SET active = 0")
            .execute(fixture.database.pool())
            .await
            .unwrap();
        let (status, problem) = fixture
            .call("PUT", &uri(&fixture), Some(with_prefix(&body, workspace)))
            .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
        assert!(
            problem["detail"]
                .as_str()
                .unwrap()
                .starts_with("bucket: another workspace keeps its files"),
            "{problem}"
        );
    }

    // Prefixes side by side pass the check; the local fake server then fails the public-internet rule.
    let fixture = fixture().await;
    let body = s3_body().await;
    let (status, _) = fixture
        .call(
            "PUT",
            "/api/v1/admin/settings/s3",
            Some(with_prefix(&body, "a/")),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let other = other_workspace(&fixture).await;
    save_workspace_bucket(&fixture, other, &body, "c").await;
    let (status, problem) = fixture
        .call("PUT", &uri(&fixture), Some(with_prefix(&body, "b/")))
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
    assert!(
        problem["detail"].as_str().unwrap().starts_with("endpoint:"),
        "{problem}"
    );
}
