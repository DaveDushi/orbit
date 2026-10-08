//! Object storage (S3) settings in Admin → Storage, against the in-memory S3 server.

use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use orbit_platform::{
    AttachmentMutationCoordinator, LocalBlobStore, ObjectStorage, PasswordService, TestDatabase,
    TieredBlobStore, TimestampMillis, start_fake_s3,
};
use orbit_server::mail::Mailer;
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
