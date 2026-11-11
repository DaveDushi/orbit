//! Drive files in Docs pages (`driveFile` blocks): the reader's Drive access is checked on every read (available,
//! locked without a name, unavailable after the trash or a delete), writes need Drive access to the file and the right
//! to edit the page, and exports show only what the exporting person may see.

use std::io::{Cursor, Read};
use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use orbit_platform::{
    AttachmentMutationCoordinator, AuthenticatedUser, Id, LocalBlobStore, ObjectStorage,
    ObjectStorageState, PasswordService, S3Bucket, TestDatabase, TimestampMillis, UploadLimits,
    UploadService, start_fake_s3,
};
use orbit_server::auth_routes::CookieMode;
use orbit_server::drive::DriveService;
use orbit_server::drive_routes::{DriveState, drive_router};
use orbit_server::export_routes::{ExportState, export_router};
use orbit_server::page_drive_routes::page_drive_router;
use orbit_server::page_routes::{PageState, page_router};
use orbit_server::repositories::identity::{IdentityRepository, SetupRequest};
use orbit_server::repositories::workspaces::WorkspaceRepository;
use serde_json::{Value, json};
use tower::ServiceExt;

const ORIGIN: &str = "https://orbit.example";

struct Fixture {
    _root: tempfile::TempDir,
    database: TestDatabase,
    identity: Arc<IdentityRepository>,
    app: axum::Router,
    workspace_id: Id,
    owner_id: Id,
    owner: String,
}

async fn fixture() -> Fixture {
    let root = tempfile::tempdir().unwrap();
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
                email: "owner@example.com".to_owned(),
                display_name: "Owner".to_owned(),
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
    let storage = ObjectStorage::default();
    storage.configure(ObjectStorageState {
        bucket: Some(S3Bucket::new(start_fake_s3().await).unwrap()),
        ..Default::default()
    });
    let drive = DriveState::new(
        Arc::clone(&identity),
        DriveService::new((*database).clone(), storage),
        CookieMode::secure(),
    );
    let uploads = UploadService::new(
        (*database).clone(),
        Arc::new(LocalBlobStore::new(root.path().join("attachments"))),
        AttachmentMutationCoordinator::default(),
        UploadLimits::default(),
    );
    let app = page_router(PageState::new(Arc::clone(&identity), CookieMode::secure()))
        .merge(page_drive_router(drive.clone()))
        .merge(drive_router(drive))
        .merge(export_router(ExportState::new(
            Arc::clone(&identity),
            uploads,
            CookieMode::secure(),
            format!("{ORIGIN}/"),
        )));
    Fixture {
        _root: root,
        database,
        identity,
        app,
        workspace_id: setup.workspace_id,
        owner_id: setup.user_id,
        owner: format!("__Host-orbit_session={}", setup.session.token),
    }
}

impl Fixture {
    async fn call(
        &self,
        cookie: &str,
        method: &str,
        uri: &str,
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
        let response = self.app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    async fn expect(
        &self,
        cookie: &str,
        method: &str,
        uri: &str,
        body: Option<Value>,
        expected: StatusCode,
    ) -> Value {
        let (status, value) = self.call(cookie, method, uri, body).await;
        assert_eq!(status, expected, "{method} {uri}: {value}");
        value
    }

    fn pages(&self) -> String {
        format!("/api/v1/workspaces/{}/pages", self.workspace_id)
    }

    fn files_of(&self, page: &str) -> String {
        format!("{}/{page}/drive-files", self.pages())
    }

    /// Uploads a file into the Drive of `workspace` as the owner.
    async fn drive_file(&self, workspace: Id, name: &str, mime: &str) -> String {
        let base = format!("/api/v1/workspaces/{workspace}/drive");
        let bytes = b"bytes".to_vec();
        let started = self
            .expect(
                &self.owner,
                "POST",
                &format!("{base}/uploads"),
                Some(json!({"name": name, "size": bytes.len(), "mime": mime})),
                StatusCode::CREATED,
            )
            .await;
        reqwest::Client::new()
            .put(started["url"].as_str().unwrap())
            .body(bytes)
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
        let upload = started["upload"]["id"].as_str().unwrap();
        self.expect(
            &self.owner,
            "POST",
            &format!("{base}/uploads/{upload}/complete"),
            None,
            StatusCode::CREATED,
        )
        .await["id"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    async fn drive(&self, method: &str, workspace: Id, path: &str, expected: StatusCode) {
        self.expect(
            &self.owner,
            method,
            &format!("/api/v1/workspaces/{workspace}/drive{path}"),
            None,
            expected,
        )
        .await;
    }

    async fn page(&self, cookie: &str, body: Value) -> String {
        self.expect(
            cookie,
            "POST",
            &self.pages(),
            Some(body),
            StatusCode::CREATED,
        )
        .await["id"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    async fn set_content(&self, page: &str, content: Value) {
        let uri = format!("{}/{page}", self.pages());
        let current = self
            .expect(&self.owner, "GET", &uri, None, StatusCode::OK)
            .await;
        self.expect(
            &self.owner,
            "PATCH",
            &uri,
            Some(json!({"expected_version": current["version"], "content": content})),
            StatusCode::OK,
        )
        .await;
    }

    async fn insert(&self, cookie: &str, page: &str, node: &str, expected: StatusCode) -> Value {
        self.expect(
            cookie,
            "POST",
            &self.files_of(page),
            Some(json!({"node_id": node})),
            expected,
        )
        .await
    }

    async fn files(&self, cookie: &str, page: &str) -> Vec<Value> {
        self.expect(cookie, "GET", &self.files_of(page), None, StatusCode::OK)
            .await["items"]
            .as_array()
            .unwrap()
            .clone()
    }

    /// A user with a session; with a role, a member of the workspace.
    async fn user(&self, email: &str, role: Option<&str>) -> String {
        let id = Id::new_v7();
        let now = TimestampMillis::now();
        sqlx::query(
            "INSERT INTO users (id, email, normalized_email, display_name, password_hash, created_at, updated_at) \
             VALUES ($1, $2, $2, 'User', 'unused', $3, $3)",
        )
        .bind(id.to_string())
        .bind(email)
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
                    email: email.to_owned(),
                    display_name: "User".to_owned(),
                },
                now,
            )
            .await
            .unwrap();
        format!("__Host-orbit_session={}", session.token)
    }

    /// A second workspace of the owner (the member is not in it).
    async fn other_workspace(&self) -> Id {
        WorkspaceRepository::new((*self.database).clone())
            .create(
                self.owner_id,
                "Other".to_owned(),
                "test",
                TimestampMillis::now(),
            )
            .await
            .unwrap()
            .id
    }

    /// The text of every file of a Markdown export of `page`.
    async fn export(&self, cookie: &str, page: &str) -> String {
        let request = Request::builder()
            .uri(format!("{}/{page}/export?format=markdown", self.pages()))
            .header(header::COOKIE, cookie)
            .body(Body::empty())
            .unwrap();
        let response = self.app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes.to_vec())).unwrap();
        let mut text = String::new();
        for index in 0..archive.len() {
            archive
                .by_index(index)
                .unwrap()
                .read_to_string(&mut text)
                .unwrap();
        }
        text
    }
}

fn block(workspace: impl ToString, node: &str) -> Value {
    json!({"id": Id::new_v7().to_string(), "type": "driveFile",
        "props": {"workspaceId": workspace.to_string(), "nodeId": node}})
}

fn statuses(items: &[Value]) -> Vec<&str> {
    items
        .iter()
        .map(|item| item["status"].as_str().unwrap())
        .collect()
}

#[tokio::test]
async fn every_read_checks_the_readers_drive_access() {
    let fixture = fixture().await;
    let member = fixture.user("member@example.com", Some("member")).await;
    let guest = fixture.user("guest@example.com", Some("guest")).await;
    let outsider = fixture.user("outsider@example.com", None).await;
    let workspace = fixture.workspace_id;
    let image = fixture
        .drive_file(workspace, "photo.png", "image/png")
        .await;
    let pdf = fixture
        .drive_file(workspace, "report.pdf", "application/pdf")
        .await;
    let other = fixture.other_workspace().await;
    let secret = fixture
        .drive_file(other, "secret-plan.pdf", "application/pdf")
        .await;
    let page = fixture
        .page(&fixture.owner, json!({"title": "Notes"}))
        .await;
    fixture
        .set_content(
            &page,
            json!([
                block(workspace, &image),
                block(workspace, &pdf),
                block(other, &secret)
            ]),
        )
        .await;

    // The owner is in both workspaces: every file, the image with an inline URL.
    let items = fixture.files(&fixture.owner, &page).await;
    assert_eq!(statuses(&items), ["available", "available", "available"]);
    assert_eq!(items[0]["file"]["name"], "photo.png");
    assert!(
        items[0]["url"]
            .as_str()
            .unwrap()
            .contains("X-Amz-Signature")
    );
    assert_eq!(items[1]["url"], Value::Null);
    assert_eq!(
        items
            .iter()
            .map(|item| item["position"].clone())
            .collect::<Vec<_>>(),
        [json!(0), json!(1), json!(2)]
    );

    // The member has no Drive access to the other workspace: locked, with nothing about the file.
    let items = fixture.files(&member, &page).await;
    assert_eq!(statuses(&items), ["available", "available", "locked"]);
    assert_eq!(items[2]["file"], Value::Null);
    assert_eq!(items[2]["url"], Value::Null);
    assert!(!Value::Array(items).to_string().contains("secret-plan"));

    // Blocks the editor has not saved yet resolve the same way, in request order.
    let resolve = format!("{}/resolve", fixture.files_of(&page));
    let references = json!({"references": [
        {"workspace_id": other.to_string(), "node_id": secret},
        {"workspace_id": workspace.to_string(), "node_id": image},
        {"workspace_id": workspace.to_string(), "node_id": Id::new_v7().to_string()},
        {"workspace_id": "x", "node_id": "y"},
    ]});
    let resolved = fixture
        .expect(
            &member,
            "POST",
            &resolve,
            Some(references.clone()),
            StatusCode::OK,
        )
        .await;
    assert_eq!(
        statuses(resolved["items"].as_array().unwrap()),
        ["locked", "available", "unavailable", "unavailable"]
    );
    assert!(!resolved.to_string().contains("secret-plan"));
    let too_many = json!({"references": vec![json!({"workspace_id": "x", "node_id": "y"}); 101]});
    fixture
        .expect(
            &member,
            "POST",
            &resolve,
            Some(too_many),
            StatusCode::UNPROCESSABLE_ENTITY,
        )
        .await;

    // Guests and people outside the workspace do not see the page at all.
    for cookie in [&guest, &outsider] {
        fixture
            .expect(
                cookie,
                "GET",
                &fixture.files_of(&page),
                None,
                StatusCode::NOT_FOUND,
            )
            .await;
        fixture
            .expect(
                cookie,
                "POST",
                &resolve,
                Some(references.clone()),
                StatusCode::NOT_FOUND,
            )
            .await;
    }

    // The trash, a restore and a delete forever show at once: the block always shows Drive as it is now.
    fixture
        .drive(
            "POST",
            workspace,
            &format!("/nodes/{pdf}/trash"),
            StatusCode::NO_CONTENT,
        )
        .await;
    assert_eq!(
        statuses(&fixture.files(&member, &page).await),
        ["available", "unavailable", "locked"]
    );
    fixture
        .drive(
            "POST",
            workspace,
            &format!("/nodes/{pdf}/restore"),
            StatusCode::OK,
        )
        .await;
    assert_eq!(
        statuses(&fixture.files(&member, &page).await),
        ["available", "available", "locked"]
    );
    fixture
        .drive(
            "POST",
            workspace,
            &format!("/nodes/{pdf}/trash"),
            StatusCode::NO_CONTENT,
        )
        .await;
    fixture
        .drive(
            "DELETE",
            workspace,
            &format!("/nodes/{pdf}"),
            StatusCode::NO_CONTENT,
        )
        .await;
    assert_eq!(
        statuses(&fixture.files(&fixture.owner, &page).await),
        ["available", "unavailable", "available"]
    );

    // A renamed file shows its new name: a reference, not a copy.
    fixture
        .expect(
            &fixture.owner,
            "POST",
            &format!("/api/v1/workspaces/{workspace}/drive/nodes/{image}/rename"),
            Some(json!({"name": "renamed.png"})),
            StatusCode::OK,
        )
        .await;
    assert_eq!(
        fixture.files(&member, &page).await[0]["file"]["name"],
        "renamed.png"
    );
}

#[tokio::test]
async fn writes_need_drive_access_to_the_file_and_the_right_to_edit_the_page() {
    let fixture = fixture().await;
    let member = fixture.user("member@example.com", Some("member")).await;
    let guest = fixture.user("guest@example.com", Some("guest")).await;
    let workspace = fixture.workspace_id;
    let pdf = fixture
        .drive_file(workspace, "report.pdf", "application/pdf")
        .await;
    let video = fixture.drive_file(workspace, "clip.mp4", "video/mp4").await;
    let folder = fixture
        .expect(
            &fixture.owner,
            "POST",
            &format!("/api/v1/workspaces/{workspace}/drive/folders"),
            Some(json!({"name": "Folder"})),
            StatusCode::CREATED,
        )
        .await["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let other = fixture.other_workspace().await;
    let foreign = fixture
        .drive_file(other, "foreign.pdf", "application/pdf")
        .await;
    let page = fixture.page(&member, json!({"title": "Team page"})).await;
    let private = fixture
        .page(&fixture.owner, json!({"title": "Mine", "private": true}))
        .await;

    // Only live files of the page's own Drive.
    for node in [
        folder.as_str(),
        foreign.as_str(),
        &Id::new_v7().to_string(),
        "nope",
    ] {
        let problem = fixture
            .insert(&member, &page, node, StatusCode::NOT_FOUND)
            .await;
        assert_eq!(problem["code"], "drive_not_found", "{node}");
    }
    // Pages the person cannot see or edit, and guests.
    fixture
        .insert(&member, &private, &pdf, StatusCode::NOT_FOUND)
        .await;
    fixture
        .insert(&guest, &page, &pdf, StatusCode::NOT_FOUND)
        .await;

    let first = fixture
        .insert(&member, &page, &pdf, StatusCode::CREATED)
        .await;
    assert_eq!(first["status"], "available");
    assert_eq!(first["file"]["name"], "report.pdf");
    // At a position among the top-level blocks.
    let second = fixture
        .expect(
            &member,
            "POST",
            &fixture.files_of(&page),
            Some(json!({"node_id": video, "position": 0})),
            StatusCode::CREATED,
        )
        .await;
    assert_eq!(second["position"], 0);
    assert!(second["url"].as_str().unwrap().contains("X-Amz-Signature"));
    let items = fixture.files(&member, &page).await;
    assert_eq!(items[0]["node_id"], video.as_str());
    assert_eq!(items[1]["node_id"], pdf.as_str());

    // Replace keeps the block and its place.
    let block = first["block_id"].as_str().unwrap();
    let uri = format!("{}/{block}", fixture.files_of(&page));
    fixture
        .expect(
            &member,
            "PUT",
            &uri,
            Some(json!({"node_id": foreign})),
            StatusCode::NOT_FOUND,
        )
        .await;
    let replaced = fixture
        .expect(
            &member,
            "PUT",
            &uri,
            Some(json!({"node_id": video})),
            StatusCode::OK,
        )
        .await;
    assert_eq!(replaced["block_id"], block);
    assert_eq!(replaced["position"], 1);
    assert_eq!(replaced["node_id"], video.as_str());

    // Remove takes the block out of the page; the file stays in Drive.
    fixture
        .expect(&guest, "DELETE", &uri, None, StatusCode::NOT_FOUND)
        .await;
    fixture
        .expect(&member, "DELETE", &uri, None, StatusCode::NO_CONTENT)
        .await;
    let problem = fixture
        .expect(&member, "DELETE", &uri, None, StatusCode::NOT_FOUND)
        .await;
    assert_eq!(problem["code"], "drive_file_block_not_found");
    assert_eq!(fixture.files(&member, &page).await.len(), 1);
    fixture
        .drive("GET", workspace, &format!("/nodes/{video}"), StatusCode::OK)
        .await;

    // A locked page refuses every change.
    fixture
        .expect(
            &fixture.owner,
            "POST",
            &format!("{}/{page}/lock", fixture.pages()),
            Some(json!({"locked": true})),
            StatusCode::OK,
        )
        .await;
    fixture
        .insert(&member, &page, &pdf, StatusCode::LOCKED)
        .await;
}

#[tokio::test]
async fn pages_using_a_file_are_only_those_the_person_can_see() {
    let fixture = fixture().await;
    let member = fixture.user("member@example.com", Some("member")).await;
    let workspace = fixture.workspace_id;
    let pdf = fixture
        .drive_file(workspace, "report.pdf", "application/pdf")
        .await;
    let shared = fixture
        .page(&fixture.owner, json!({"title": "Shared"}))
        .await;
    let private = fixture
        .page(&fixture.owner, json!({"title": "Private", "private": true}))
        .await;
    let mention = fixture
        .page(&fixture.owner, json!({"title": "Text only"}))
        .await;
    fixture
        .set_content(
            &shared,
            json!([block(workspace, &pdf), block(workspace, &pdf)]),
        )
        .await;
    fixture
        .set_content(&private, json!([block(workspace, &pdf)]))
        .await;
    // The id in text is no use of the file.
    fixture
        .set_content(
            &mention,
            json!([{"type": "paragraph", "content": [{"type": "text", "text": pdf, "styles": {}}]}]),
        )
        .await;
    let uses = |cookie: String| {
        let fixture = &fixture;
        let pdf = pdf.clone();
        async move {
            fixture
                .expect(
                    &cookie,
                    "GET",
                    &format!("{}/drive-file-uses/{pdf}", fixture.pages()),
                    None,
                    StatusCode::OK,
                )
                .await["items"]
                .clone()
        }
    };
    let mine = uses(fixture.owner.clone()).await;
    assert_eq!(mine.as_array().unwrap().len(), 2);
    assert_eq!(mine[0]["title"], "Private");
    assert_eq!(mine[1]["title"], "Shared");
    assert_eq!(mine[1]["blocks"], 2);
    let theirs = uses(member).await;
    assert_eq!(
        theirs,
        json!([{"page_id": shared, "title": "Shared", "blocks": 2}])
    );
}

#[tokio::test]
async fn an_export_shows_only_the_files_the_exporting_person_may_see() {
    let fixture = fixture().await;
    let member = fixture.user("member@example.com", Some("member")).await;
    let workspace = fixture.workspace_id;
    let visible = fixture
        .drive_file(workspace, "visible.pdf", "application/pdf")
        .await;
    let gone = fixture
        .drive_file(workspace, "old.pdf", "application/pdf")
        .await;
    let other = fixture.other_workspace().await;
    let secret = fixture
        .drive_file(other, "secret-plan.pdf", "application/pdf")
        .await;
    let page = fixture
        .page(&fixture.owner, json!({"title": "Notes"}))
        .await;
    fixture
        .set_content(
            &page,
            json!([
                block(workspace, &visible),
                block(other, &secret),
                block(workspace, &gone)
            ]),
        )
        .await;
    fixture
        .drive(
            "POST",
            workspace,
            &format!("/nodes/{gone}/trash"),
            StatusCode::NO_CONTENT,
        )
        .await;

    let theirs = fixture.export(&member, &page).await;
    assert!(
        theirs.contains(&format!("[visible.pdf]({ORIGIN}/drive)")),
        "{theirs}"
    );
    assert!(theirs.contains("*Locked Drive file*"), "{theirs}");
    assert!(theirs.contains("*File not available*"), "{theirs}");
    assert!(!theirs.contains("secret-plan"), "{theirs}");
    assert!(!theirs.contains("old.pdf"), "{theirs}");

    let mine = fixture.export(&fixture.owner, &page).await;
    assert!(
        mine.contains(&format!("[secret-plan.pdf]({ORIGIN}/drive)")),
        "{mine}"
    );
}
