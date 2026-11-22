//! Project icons (project settings): who may set, replace, remove and read them, which files are refused, and that a
//! replaced or removed icon's file goes with the next attachment reconcile. Stored like workspace logos.

use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::http::{HeaderMap, Request, StatusCode, header};
use orbit_platform::{
    AttachmentMutationCoordinator, AuthenticatedUser, Id, LocalBlobStore, PasswordService,
    TestDatabase, TimestampMillis, UploadLimits, UploadService,
};
use orbit_server::repositories::identity::{IdentityRepository, SetupRequest};
use orbit_server::repositories::workspaces::{StorageFileUseKind, WorkspaceRepository};
use orbit_server::task_routes::{TaskState, task_router};
use serde_json::{Value, json};
use tower::ServiceExt;

const DAY: i64 = 24 * 60 * 60 * 1000;

fn png(fill: u8, len: usize) -> Vec<u8> {
    let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
    bytes.resize(len, fill);
    bytes
}

struct Fixture {
    database: TestDatabase,
    identity: Arc<IdentityRepository>,
    attachments: tempfile::TempDir,
    uploads: UploadService,
    app: axum::Router,
    workspace_id: Id,
    owner_id: Id,
    owner: String,
    project: Value,
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
    let uploads = UploadService::new(
        (*database).clone(),
        Arc::new(LocalBlobStore::new(attachments.path())),
        AttachmentMutationCoordinator::default(),
        UploadLimits::default(),
    );
    let app = task_router(
        TaskState::new(
            Arc::clone(&identity),
            orbit_server::auth_routes::CookieMode::secure(),
        )
        .with_uploads(uploads.clone()),
    );
    let mut fixture = Fixture {
        database,
        identity,
        attachments,
        uploads,
        app,
        workspace_id: setup.workspace_id,
        owner_id: setup.user_id,
        owner: format!("__Host-orbit_session={}", setup.session.token),
        project: Value::Null,
    };
    fixture.project = fixture.listed(&fixture.owner).await[0].clone();
    assert!(fixture.project["icon_url"].is_null());
    fixture
}

impl Fixture {
    fn project_uri(&self) -> String {
        format!(
            "/api/v1/workspaces/{}/projects/{}",
            self.workspace_id,
            self.project["id"].as_str().unwrap()
        )
    }

    fn icon_uri(&self) -> String {
        format!("{}/icon", self.project_uri())
    }

    async fn send(&self, request: Request<Body>) -> (StatusCode, HeaderMap, Vec<u8>) {
        let response = self.app.clone().oneshot(request).await.unwrap();
        (
            response.status(),
            response.headers().clone(),
            to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap()
                .to_vec(),
        )
    }

    async fn call(
        &self,
        method: &str,
        uri: &str,
        cookie: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let request = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::COOKIE, cookie);
        let request = match body {
            Some(body) => request
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string())),
            None => request.body(Body::empty()),
        }
        .unwrap();
        let (status, _, bytes) = self.send(request).await;
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    async fn upload_to(&self, uri: &str, cookie: &str, image: &[u8]) -> (StatusCode, Value) {
        let mut multipart =
            b"--icon\r\nContent-Disposition: form-data; name=\"file\"; filename=\"icon.png\"\r\n\r\n"
                .to_vec();
        multipart.extend_from_slice(image);
        multipart.extend_from_slice(b"\r\n--icon--\r\n");
        let request = Request::builder()
            .method("PUT")
            .uri(uri)
            .header(header::COOKIE, cookie)
            .header(header::CONTENT_TYPE, "multipart/form-data; boundary=icon")
            .body(Body::from(multipart))
            .unwrap();
        let (status, _, bytes) = self.send(request).await;
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    async fn upload(&self, cookie: &str, image: &[u8]) -> (StatusCode, Value) {
        self.upload_to(&self.icon_uri(), cookie, image).await
    }

    async fn remove(&self, cookie: &str) -> (StatusCode, Value) {
        self.call("DELETE", &self.icon_uri(), cookie, None).await
    }

    async fn image(&self, cookie: &str) -> (StatusCode, HeaderMap, Vec<u8>) {
        let request = Request::builder()
            .uri(self.icon_uri())
            .header(header::COOKIE, cookie)
            .body(Body::empty())
            .unwrap();
        self.send(request).await
    }

    async fn listed(&self, cookie: &str) -> Vec<Value> {
        let uri = format!("/api/v1/workspaces/{}/projects", self.workspace_id);
        let (status, page) = self.call("GET", &uri, cookie, None).await;
        assert_eq!(status, StatusCode::OK, "{page}");
        page["items"].as_array().unwrap().clone()
    }

    /// A new user, with `role` in the fixture's workspace unless `None`, and their session cookie.
    async fn user(&self, name: &str, role: Option<&str>) -> (Id, String) {
        let id = Id::new_v7();
        let now = TimestampMillis::now();
        let email = format!("{name}@example.com");
        sqlx::query(
            "INSERT INTO users (id, email, normalized_email, display_name, password_hash, created_at, updated_at) \
             VALUES ($1, $2, $2, $2, 'unused', $3, $3)",
        )
        .bind(id.to_string())
        .bind(&email)
        .bind(now.as_millis())
        .execute(self.database.pool())
        .await
        .unwrap();
        if let Some(role) = role {
            sqlx::query(
                "INSERT INTO memberships (id, workspace_id, user_id, role, version, created_at, updated_at) \
                 VALUES ($1, $2, $3, $4, 0, $5, $5)",
            )
            .bind(Id::new_v7().to_string())
            .bind(self.workspace_id.to_string())
            .bind(id.to_string())
            .bind(role)
            .bind(now.as_millis())
            .execute(self.database.pool())
            .await
            .unwrap();
        }
        let session = self
            .identity
            .create_session(
                &AuthenticatedUser {
                    id,
                    email: email.clone(),
                    display_name: email,
                },
                now,
            )
            .await
            .unwrap();
        (id, format!("__Host-orbit_session={}", session.token))
    }

    /// The storage keys of the blobs the icons use.
    async fn icon_keys(&self) -> Vec<String> {
        sqlx::query_scalar(
            "SELECT b.storage_key FROM attachment_blobs b JOIN project_icons i ON i.blob_id = b.id",
        )
        .fetch_all(self.database.pool())
        .await
        .unwrap()
    }

    async fn blobs(&self) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM attachment_blobs")
            .fetch_one(self.database.pool())
            .await
            .unwrap()
    }

    /// Nobody in `cookies` can read, replace or remove the icon, which stays as it is.
    async fn refuses(&self, cookies: &[(&str, &str, StatusCode)]) {
        for (who, cookie, refusal) in cookies {
            let blobs = self.blobs().await;
            assert_eq!(self.image(cookie).await.0, *refusal, "{who} reads");
            let (status, problem) = self.upload(cookie, &png(9, 99)).await;
            assert_eq!(status, *refusal, "{who} uploads: {problem}");
            let (status, problem) = self.remove(cookie).await;
            assert_eq!(status, *refusal, "{who} removes: {problem}");
            // a refused upload stores nothing
            assert_eq!(self.blobs().await, blobs, "{who}");
        }
        assert_eq!(self.image(&self.owner).await.2, png(1, 64));
    }
}

#[tokio::test]
async fn members_set_the_icon_and_guests_outsiders_and_suspended_people_cannot_reach_it() {
    let fixture = fixture().await;
    let (_, member) = fixture.user("member", Some("member")).await;
    let (_, guest) = fixture.user("guest", Some("guest")).await;
    let (stranger_id, stranger) = fixture.user("stranger", None).await;
    let elsewhere = WorkspaceRepository::new((*fixture.database).clone())
        .create(
            stranger_id,
            "Elsewhere".to_owned(),
            "test",
            TimestampMillis::now(),
        )
        .await
        .unwrap();
    let (suspended_id, suspended) = fixture.user("suspended", Some("admin")).await;
    sqlx::query("UPDATE users SET suspended_at = $1 WHERE id = $2")
        .bind(TimestampMillis::now().as_millis())
        .bind(suspended_id.to_string())
        .execute(fixture.database.pool())
        .await
        .unwrap();

    // the right is the one of the project settings: every member who sees the project
    let (status, project) = fixture.upload(&member, &png(1, 64)).await;
    assert_eq!(status, StatusCode::OK, "{project}");
    let url = project["icon_url"].as_str().unwrap();
    assert!(
        url.starts_with(&format!("{}?v=", fixture.icon_uri())),
        "{url}"
    );
    assert_eq!(project["color"], fixture.project["color"]);
    assert_eq!(fixture.listed(&fixture.owner).await[0]["icon_url"], url);
    for cookie in [&fixture.owner, &member] {
        let (status, _, bytes) = fixture.image(cookie).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(bytes, png(1, 64));
    }

    fixture
        .refuses(&[
            ("a guest", &guest, StatusCode::NOT_FOUND),
            ("a stranger", &stranger, StatusCode::NOT_FOUND),
            ("suspended", &suspended, StatusCode::UNAUTHORIZED),
            ("signed out", "", StatusCode::UNAUTHORIZED),
        ])
        .await;

    // the project's id under another workspace that the caller owns
    let smuggled = fixture
        .icon_uri()
        .replace(&fixture.workspace_id.to_string(), &elsewhere.id.to_string());
    let (status, _) = fixture.call("GET", &smuggled, &stranger, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = fixture.upload_to(&smuggled, &stranger, &png(9, 64)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = fixture.call("DELETE", &smuggled, &stranger, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(fixture.image(&member).await.2, png(1, 64));
}

#[tokio::test]
async fn a_private_projects_icon_is_only_for_the_people_who_see_the_project() {
    let fixture = fixture().await;
    let (_, outsider) = fixture.user("outsider", Some("member")).await;
    let (insider_id, insider) = fixture.user("insider", Some("member")).await;
    let (_, admin) = fixture.user("admin", Some("admin")).await;
    let (status, _) = fixture.upload(&fixture.owner, &png(1, 64)).await;
    assert_eq!(status, StatusCode::OK);
    let project = &fixture.project;
    let (status, problem) = fixture
        .call(
            "PATCH",
            &fixture.project_uri(),
            &fixture.owner,
            Some(json!({
                "name": project["name"], "key": project["key"], "color": project["color"],
                "private": true, "expected_version": project["version"],
            })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{problem}");
    let (status, problem) = fixture
        .call(
            "POST",
            &format!("{}/members", fixture.project_uri()),
            &fixture.owner,
            Some(json!({"user_id": insider_id.to_string()})),
        )
        .await;
    assert!(status.is_success(), "{problem}");

    fixture
        .refuses(&[("not on the list", &outsider, StatusCode::NOT_FOUND)])
        .await;
    for cookie in [&insider, &admin] {
        assert_eq!(fixture.image(cookie).await.0, StatusCode::OK);
    }
    let (status, project) = fixture.upload(&insider, &png(2, 64)).await;
    assert_eq!(status, StatusCode::OK, "{project}");
    assert_eq!(project["private"], true);
    let (status, _) = fixture.remove(&admin).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn only_png_jpeg_and_webp_images_of_at_most_512_kib_are_accepted_and_served_safely() {
    let fixture = fixture().await;
    let refused = [
        (
            b"<svg xmlns=\"http://www.w3.org/2000/svg\" onload=\"alert(1)\"/>".to_vec(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_icon",
        ),
        (
            b"<html><script>alert(1)</script>".to_vec(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_icon",
        ),
        (
            b"GIF89a....".to_vec(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_icon",
        ),
        (Vec::new(), StatusCode::UNPROCESSABLE_ENTITY, "invalid_icon"),
        (
            png(1, 512 * 1024 + 1),
            StatusCode::PAYLOAD_TOO_LARGE,
            "icon_too_large",
        ),
        (
            png(1, 2 * 1024 * 1024),
            StatusCode::PAYLOAD_TOO_LARGE,
            "icon_too_large",
        ),
    ];
    for (image, status, code) in refused {
        let (answer, problem) = fixture.upload(&fixture.owner, &image).await;
        assert_eq!(answer, status, "{problem}");
        assert_eq!(problem["code"], code);
    }
    assert_eq!(fixture.blobs().await, 0);

    let mut jpeg = vec![0xff, 0xd8, 0xff];
    jpeg.resize(100, 7);
    let mut webp = b"RIFF\0\0\0\0WEBP".to_vec();
    webp.resize(100, 7);
    for (image, mime_type) in [
        (jpeg, "image/jpeg"),
        (webp, "image/webp"),
        (png(1, 512 * 1024), "image/png"),
    ] {
        let (status, project) = fixture.upload(&fixture.owner, &image).await;
        assert_eq!(status, StatusCode::OK, "{project}");
        let (_, headers, bytes) = fixture.image(&fixture.owner).await;
        assert_eq!(bytes, image);
        assert_eq!(headers[header::CONTENT_TYPE], mime_type);
        assert_eq!(headers["x-content-type-options"], "nosniff");
        assert!(
            headers[header::CACHE_CONTROL]
                .to_str()
                .unwrap()
                .starts_with("private")
        );
    }
}

#[tokio::test]
async fn an_icon_in_use_stays_and_a_replaced_or_removed_one_frees_its_file() {
    let fixture = fixture().await;
    let file = |key: &str| fixture.attachments.path().join(key);
    let (status, _) = fixture.upload(&fixture.owner, &png(1, 64)).await;
    assert_eq!(status, StatusCode::OK);
    let first = fixture.icon_keys().await.remove(0);

    // in use: it outlives the quarantine, the storage page names it, and an admin cannot delete it there
    let later = TimestampMillis::now().as_millis() + 2 * DAY;
    assert_eq!(
        fixture
            .uploads
            .reconcile(later)
            .await
            .unwrap()
            .deleted_blobs,
        0
    );
    assert!(file(&first).exists());
    let workspaces = WorkspaceRepository::new((*fixture.database).clone());
    let storage = workspaces
        .storage(fixture.workspace_id, fixture.owner_id, 10, 0)
        .await
        .unwrap();
    let [stored] = storage.largest_files.as_slice() else {
        panic!("{:?}", storage.largest_files);
    };
    assert_eq!(stored.byte_size, 64);
    assert_eq!(stored.uses[0].kind, StorageFileUseKind::ProjectIcon);
    assert_eq!(stored.uses[0].title.as_deref(), Some("General"));
    assert!(
        !fixture
            .uploads
            .delete_blob(fixture.workspace_id, stored.id)
            .await
            .unwrap()
    );

    let (status, _) = fixture.upload(&fixture.owner, &png(2, 80)).await;
    assert_eq!(status, StatusCode::OK);
    let second = fixture.icon_keys().await.remove(0);
    assert_eq!(
        fixture
            .uploads
            .reconcile(later)
            .await
            .unwrap()
            .deleted_blobs,
        1
    );
    assert!(!file(&first).exists() && file(&second).exists());
    assert_eq!(fixture.image(&fixture.owner).await.2, png(2, 80));

    let (status, project) = fixture.remove(&fixture.owner).await;
    assert_eq!(status, StatusCode::OK);
    assert!(project["icon_url"].is_null());
    assert!(fixture.listed(&fixture.owner).await[0]["icon_url"].is_null());
    assert_eq!(fixture.image(&fixture.owner).await.0, StatusCode::NOT_FOUND);
    // removing it again changes nothing
    assert_eq!(fixture.remove(&fixture.owner).await.0, StatusCode::OK);
    assert_eq!(
        fixture
            .uploads
            .reconcile(later)
            .await
            .unwrap()
            .deleted_blobs,
        1
    );
    assert!(!file(&second).exists());
    assert_eq!(fixture.blobs().await, 0);
}

#[tokio::test]
async fn deleting_a_project_or_its_workspace_forever_frees_the_icons_file() {
    let fixture = fixture().await;
    let workspaces = WorkspaceRepository::with_blob_store(
        (*fixture.database).clone(),
        Arc::new(LocalBlobStore::new(fixture.attachments.path())),
    );
    let (status, project) = fixture
        .call(
            "POST",
            &format!("/api/v1/workspaces/{}/projects", fixture.workspace_id),
            &fixture.owner,
            Some(json!({"name": "Short lived", "key": "SHORT", "color": "#6366f1"})),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{project}");
    let uri = format!(
        "/api/v1/workspaces/{}/projects/{}",
        fixture.workspace_id,
        project["id"].as_str().unwrap()
    );
    let (status, _) = fixture
        .upload_to(&format!("{uri}/icon"), &fixture.owner, &png(3, 64))
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = fixture.upload(&fixture.owner, &png(4, 64)).await;
    assert_eq!(status, StatusCode::OK);

    // in the trash the icon still shows, but it cannot change
    let (status, problem) = fixture
        .call(
            "DELETE",
            &format!("{uri}?expected_version=0"),
            &fixture.owner,
            None,
        )
        .await;
    assert!(status.is_success(), "{problem}");
    let (status, _) = fixture
        .call("GET", &format!("{uri}/icon"), &fixture.owner, None)
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = fixture
        .upload_to(&format!("{uri}/icon"), &fixture.owner, &png(5, 64))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let far = TimestampMillis::from_millis(TimestampMillis::now().as_millis() + 60 * DAY);
    workspaces.purge_retention(far).await.unwrap();
    let keys = fixture.icon_keys().await;
    assert_eq!(keys.len(), 1);
    fixture.uploads.reconcile(far.as_millis()).await.unwrap();
    assert_eq!(fixture.blobs().await, 1);

    sqlx::query("UPDATE workspaces SET deleted_at = $1")
        .bind(TimestampMillis::now().as_millis())
        .execute(fixture.database.pool())
        .await
        .unwrap();
    let summary = workspaces.purge_retention(far).await.unwrap();
    assert_eq!(summary.workspaces_purged, 1);
    assert_eq!(summary.attachment_blobs_purged, 1);
    assert_eq!(fixture.blobs().await, 0);
    // the retention service deletes the queued file next
    let queued: Vec<String> = sqlx::query_scalar("SELECT path FROM attachment_file_deletions")
        .fetch_all(fixture.database.pool())
        .await
        .unwrap();
    assert_eq!(queued, keys);
}
