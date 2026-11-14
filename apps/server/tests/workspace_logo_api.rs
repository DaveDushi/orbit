//! Workspace logos (Settings → General): who may set, replace, remove and read them, which files are refused, and
//! that a removed logo's file goes with the next attachment reconcile. Stored like profile pictures.

use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use orbit_platform::{
    AttachmentMutationCoordinator, AuthenticatedUser, Id, LocalBlobStore, PasswordService,
    TestDatabase, TimestampMillis, UploadLimits, UploadService,
};
use orbit_server::mail::{Mailer, Outbox};
use orbit_server::repositories::identity::{IdentityRepository, SetupRequest};
use orbit_server::repositories::workspaces::WorkspaceRepository;
use orbit_server::workspace_routes::{WorkspaceState, workspace_router};
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
    outbox: Outbox,
    app: axum::Router,
    workspace_id: Id,
    owner: String,
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
    let (mailer, outbox) = Mailer::recording((*database).clone(), "https://orbit.test");
    let app = workspace_router(
        WorkspaceState::new(
            Arc::clone(&identity),
            "https://orbit.test".to_owned(),
            orbit_server::auth_routes::CookieMode::secure(),
        )
        .with_mailer(mailer)
        .with_uploads(uploads.clone()),
    );
    Fixture {
        database,
        identity,
        attachments,
        uploads,
        outbox,
        app,
        workspace_id: setup.workspace_id,
        owner: format!("__Host-orbit_session={}", setup.session.token),
    }
}

impl Fixture {
    fn logo_uri(&self) -> String {
        format!("/api/v1/workspaces/{}/logo", self.workspace_id)
    }

    async fn send(&self, request: Request<Body>) -> (StatusCode, Vec<u8>) {
        let response = self.app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        (
            status,
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
        let (status, bytes) = self.send(request).await;
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    async fn upload(&self, cookie: &str, image: &[u8]) -> (StatusCode, Value) {
        self.upload_to(&self.logo_uri(), cookie, image).await
    }

    async fn upload_to(&self, uri: &str, cookie: &str, image: &[u8]) -> (StatusCode, Value) {
        let mut multipart =
            b"--logo\r\nContent-Disposition: form-data; name=\"file\"; filename=\"logo.png\"\r\n\r\n"
                .to_vec();
        multipart.extend_from_slice(image);
        multipart.extend_from_slice(b"\r\n--logo--\r\n");
        let request = Request::builder()
            .method("PUT")
            .uri(uri)
            .header(header::COOKIE, cookie)
            .header(header::CONTENT_TYPE, "multipart/form-data; boundary=logo")
            .body(Body::from(multipart))
            .unwrap();
        let (status, bytes) = self.send(request).await;
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    async fn image(&self, cookie: &str) -> (StatusCode, Vec<u8>) {
        let request = Request::builder()
            .uri(self.logo_uri())
            .header(header::COOKIE, cookie)
            .body(Body::empty())
            .unwrap();
        self.send(request).await
    }

    async fn user(&self, email: &str) -> (Id, String) {
        let id = Id::new_v7();
        let now = TimestampMillis::now();
        sqlx::query(
            "INSERT INTO users (id, email, normalized_email, display_name, password_hash, created_at, updated_at) \
             VALUES ($1, $2, $2, $2, 'unused', $3, $3)",
        )
        .bind(id.to_string())
        .bind(email)
        .bind(now.as_millis())
        .execute(self.database.pool())
        .await
        .unwrap();
        let session = self
            .identity
            .create_session(
                &AuthenticatedUser {
                    id,
                    email: email.to_owned(),
                    display_name: email.to_owned(),
                },
                now,
            )
            .await
            .unwrap();
        (id, format!("__Host-orbit_session={}", session.token))
    }

    /// A new user with `role` in the fixture's workspace, and their session cookie.
    async fn member(&self, role: &str) -> String {
        let (id, cookie) = self.user(&format!("{role}@example.com")).await;
        let now = TimestampMillis::now();
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
        cookie
    }

    async fn logo_blobs(&self) -> Vec<String> {
        sqlx::query_scalar(
            "SELECT b.storage_key FROM attachment_blobs b JOIN workspace_logos l ON l.blob_id = b.id",
        )
        .fetch_all(self.database.pool())
        .await
        .unwrap()
    }
}

#[tokio::test]
async fn only_owners_and_admins_set_or_remove_the_logo_and_every_member_sees_it() {
    let fixture = fixture().await;
    let member = fixture.member("member").await;
    let guest = fixture.member("guest").await;
    let admin = fixture.member("admin").await;

    for cookie in [&member, &guest] {
        let (status, problem) = fixture.upload(cookie, &png(1, 64)).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{problem}");
        assert_eq!(problem["code"], "workspace_action_forbidden");
        let (status, _) = fixture
            .call("DELETE", &fixture.logo_uri(), cookie, None)
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }
    assert!(fixture.logo_blobs().await.is_empty());

    let (status, workspace) = fixture.upload(&admin, &png(1, 64)).await;
    assert_eq!(status, StatusCode::OK, "{workspace}");
    let url = workspace["logo_url"].as_str().unwrap().to_owned();
    assert!(url.starts_with(&format!("{}?v=", fixture.logo_uri())), "{url}");
    let (_, listed) = fixture
        .call("GET", "/api/v1/workspaces", &fixture.owner, None)
        .await;
    assert_eq!(listed[0]["logo_url"], url.as_str());

    // members and guests may read it (the switcher shows it to everyone)
    for cookie in [&fixture.owner, &member, &guest] {
        let (status, bytes) = fixture.image(cookie).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(bytes, png(1, 64));
    }

    // a member's refused removal leaves it in place; the owner replaces it
    let (status, _) = fixture
        .call("DELETE", &fixture.logo_uri(), &member, None)
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, replaced) = fixture.upload(&fixture.owner, &png(2, 80)).await;
    assert_eq!(status, StatusCode::OK, "{replaced}");
    assert_eq!(fixture.image(&member).await.1, png(2, 80));
    assert_eq!(fixture.logo_blobs().await.len(), 1);
}

#[tokio::test]
async fn only_png_jpeg_and_webp_images_of_at_most_512_kib_are_accepted() {
    let fixture = fixture().await;
    let refused = [
        (
            b"<svg xmlns=\"http://www.w3.org/2000/svg\" onload=\"alert(1)\"/>".to_vec(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_logo",
        ),
        (
            b"GIF89a....".to_vec(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_logo",
        ),
        (
            png(1, 512 * 1024 + 1),
            StatusCode::PAYLOAD_TOO_LARGE,
            "logo_too_large",
        ),
    ];
    for (image, status, code) in refused {
        let (answer, problem) = fixture.upload(&fixture.owner, &image).await;
        assert_eq!(answer, status, "{problem}");
        assert_eq!(problem["code"], code);
    }
    assert!(fixture.logo_blobs().await.is_empty());

    let mut jpeg = vec![0xff, 0xd8, 0xff];
    jpeg.resize(100, 7);
    let mut webp = b"RIFF\0\0\0\0WEBP".to_vec();
    webp.resize(100, 7);
    for (image, mime_type) in [
        (jpeg, "image/jpeg"),
        (webp, "image/webp"),
        (png(1, 512 * 1024), "image/png"),
    ] {
        let (status, workspace) = fixture.upload(&fixture.owner, &image).await;
        assert_eq!(status, StatusCode::OK, "{workspace}");
        let response = fixture
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(fixture.logo_uri())
                    .header(header::COOKIE, &fixture.owner)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.headers()[header::CONTENT_TYPE], mime_type);
        assert_eq!(response.headers()["x-content-type-options"], "nosniff");
    }
}

#[tokio::test]
async fn a_member_of_another_workspace_cannot_read_replace_or_remove_the_logo() {
    let fixture = fixture().await;
    let (status, _) = fixture.upload(&fixture.owner, &png(1, 64)).await;
    assert_eq!(status, StatusCode::OK);

    // the owner of their own workspace, but no member of this one
    let (stranger_id, stranger) = fixture.user("stranger@example.com").await;
    let elsewhere = WorkspaceRepository::new((*fixture.database).clone())
        .create(
            stranger_id,
            "Elsewhere".to_owned(),
            "test",
            TimestampMillis::now(),
        )
        .await
        .unwrap();

    assert_eq!(fixture.image(&stranger).await.0, StatusCode::NOT_FOUND);
    let (status, _) = fixture.upload(&stranger, &png(9, 64)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = fixture
        .call("DELETE", &fixture.logo_uri(), &stranger, None)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(fixture.image(&fixture.owner).await.1, png(1, 64));

    // their own workspace's logo does not show up in this one, and the reverse
    let (status, own) = fixture
        .upload_to(
            &format!("/api/v1/workspaces/{}/logo", elsewhere.id),
            &stranger,
            &png(9, 64),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{own}");
    assert_eq!(fixture.image(&fixture.owner).await.1, png(1, 64));
    let (status, _) = fixture
        .call(
            "GET",
            &format!("/api/v1/workspaces/{}/logo", elsewhere.id),
            &fixture.owner,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn removing_the_logo_falls_back_to_none_and_reconcile_deletes_its_file() {
    let fixture = fixture().await;
    let (status, _) = fixture.upload(&fixture.owner, &png(1, 64)).await;
    assert_eq!(status, StatusCode::OK);
    let keys = fixture.logo_blobs().await;
    assert_eq!(keys.len(), 1);
    let file = fixture.attachments.path().join(&keys[0]);
    assert!(file.exists(), "{}", file.display());

    // a logo in use outlives the quarantine
    let later = TimestampMillis::now().as_millis() + 2 * DAY;
    let kept = fixture.uploads.reconcile(later).await.unwrap();
    assert_eq!(kept.deleted_blobs, 0);
    assert!(file.exists());

    let (status, workspace) = fixture
        .call("DELETE", &fixture.logo_uri(), &fixture.owner, None)
        .await;
    assert_eq!(status, StatusCode::OK, "{workspace}");
    assert!(workspace["logo_url"].is_null());
    let (_, listed) = fixture
        .call("GET", "/api/v1/workspaces", &fixture.owner, None)
        .await;
    assert!(listed[0]["logo_url"].is_null());
    assert_eq!(fixture.image(&fixture.owner).await.0, StatusCode::NOT_FOUND);
    // removing it again changes nothing
    let (status, _) = fixture
        .call("DELETE", &fixture.logo_uri(), &fixture.owner, None)
        .await;
    assert_eq!(status, StatusCode::OK);

    let removed = fixture.uploads.reconcile(later).await.unwrap();
    assert_eq!(removed.deleted_blobs, 1);
    assert!(!file.exists());
    let blobs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM attachment_blobs")
        .fetch_one(fixture.database.pool())
        .await
        .unwrap();
    assert_eq!(blobs, 0);
}

#[tokio::test]
async fn an_invitation_shows_the_logo_on_its_page_and_in_its_email() {
    let fixture = fixture().await;
    let invite = |email: &str, delivery: &str| {
        json!({"email": email, "role": "member", "delivery": delivery})
    };
    let invitations = format!("/api/v1/workspaces/{}/invitations", fixture.workspace_id);

    // without a logo: no picture on the page, a plain text email
    let (status, created) = fixture
        .call("POST", &invitations, &fixture.owner, Some(invite("a@example.com", "manual")))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let token = created["url"].as_str().unwrap().split("token=").nth(1).unwrap().to_owned();
    let preview_uri = "/api/v1/workspaces/invitations/preview";
    let (status, preview) = fixture
        .call("POST", preview_uri, "", Some(json!({"token": token})))
        .await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    assert!(preview["workspace_logo"].is_null());
    let (status, _) = fixture
        .call("POST", &invitations, &fixture.owner, Some(invite("b@example.com", "smtp")))
        .await;
    assert_eq!(status, StatusCode::CREATED);
    assert!(fixture.outbox.lock().unwrap()[0].html.is_none());

    let (status, _) = fixture.upload(&fixture.owner, &png(1, 64)).await;
    assert_eq!(status, StatusCode::OK);
    // the invitee is no member yet, so the page gets the picture itself
    let (_, preview) = fixture
        .call("POST", preview_uri, "", Some(json!({"token": token})))
        .await;
    let logo = preview["workspace_logo"].as_str().unwrap();
    assert!(logo.starts_with("data:image/png;base64,"), "{logo}");
    assert!(preview.get("workspace_id").is_none());

    let (status, _) = fixture
        .call("POST", &invitations, &fixture.owner, Some(invite("c@example.com", "smtp")))
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let outbox = fixture.outbox.lock().unwrap();
    let email = outbox.last().unwrap();
    assert!(email.html.as_deref().unwrap().contains("cid:logo"));
    let attached = email.logo.as_ref().unwrap();
    assert_eq!(attached.mime_type, "image/png");
    assert_eq!(attached.bytes, png(1, 64));
}
