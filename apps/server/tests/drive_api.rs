//! Orbit Drive: permission checks on every route and before signing, names and moves, trash and restore of
//! trees, uploads (single PUT and multipart with resume) and the deletion of objects, against the in-memory S3
//! server.

use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use orbit_platform::{
    AuthenticatedUser, Id, ObjectStorage, ObjectStorageState, PasswordService, S3Bucket,
    TestDatabase, TimestampMillis, start_fake_s3,
};
use orbit_server::drive::{DriveService, MIN_PART_BYTES};
use orbit_server::drive_routes::{DriveState, drive_router};
use orbit_server::repositories::drive::TRASH_RETENTION_MILLIS;
use orbit_server::repositories::identity::{IdentityRepository, SetupRequest};
use serde_json::{Value, json};
use tower::ServiceExt;

const DAY_MILLIS: i64 = 24 * 60 * 60 * 1000;

struct Fixture {
    database: TestDatabase,
    identity: Arc<IdentityRepository>,
    bucket: S3Bucket,
    drive: DriveService,
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
    let bucket = S3Bucket::new(start_fake_s3().await).unwrap();
    let storage = ObjectStorage::default();
    storage.configure(ObjectStorageState {
        bucket: Some(bucket.clone()),
        ..Default::default()
    });
    let drive = DriveService::new((*database).clone(), storage);
    let app = drive_router(DriveState::new(
        Arc::clone(&identity),
        drive.clone(),
        orbit_server::auth_routes::CookieMode::secure(),
    ));
    Fixture {
        database,
        identity,
        bucket,
        drive,
        app,
        workspace_id: setup.workspace_id,
        owner: format!("__Host-orbit_session={}", setup.session.token),
    }
}

impl Fixture {
    fn path(&self, path: &str) -> String {
        format!("/api/v1/workspaces/{}/drive{path}", self.workspace_id)
    }

    async fn call(
        &self,
        method: &str,
        path: &str,
        cookie: Option<&str>,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut request = Request::builder().method(method).uri(path);
        if let Some(cookie) = cookie {
            request = request.header(header::COOKIE, cookie);
        }
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

    /// A call by the owner that must answer `expected`.
    async fn owner(
        &self,
        method: &str,
        path: &str,
        body: Option<Value>,
        expected: StatusCode,
    ) -> Value {
        let (status, value) = self
            .call(method, &self.path(path), Some(&self.owner), body)
            .await;
        assert_eq!(status, expected, "{method} {path}: {value}");
        value
    }

    async fn folder(&self, parent: Option<&str>, name: &str) -> String {
        self.owner(
            "POST",
            "/folders",
            Some(json!({"parent_id": parent, "name": name})),
            StatusCode::CREATED,
        )
        .await["id"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    /// Uploads a small file with one presigned PUT, as the browser does.
    async fn file(&self, parent: Option<&str>, name: &str, bytes: &[u8]) -> String {
        let started = self
            .owner(
                "POST",
                "/uploads",
                Some(json!({"parent_id": parent, "name": name, "size": bytes.len(), "mime": "text/plain"})),
                StatusCode::CREATED,
            )
            .await;
        put(started["url"].as_str().unwrap(), bytes.to_vec()).await;
        let upload = started["upload"]["id"].as_str().unwrap();
        self.owner(
            "POST",
            &format!("/uploads/{upload}/complete"),
            None,
            StatusCode::CREATED,
        )
        .await["id"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    async fn names(&self, parent: Option<&str>) -> Vec<String> {
        let query = parent.map_or(String::new(), |parent| format!("?parent_id={parent}"));
        let page = self
            .owner("GET", &format!("/nodes{query}"), None, StatusCode::OK)
            .await;
        page["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|node| node["name"].as_str().unwrap().to_owned())
            .collect()
    }

    /// A user with a session; a member of the workspace with `role`, or of nothing.
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

    async fn object_count(&self) -> usize {
        self.bucket.list("drive/").await.unwrap().len()
    }
}

async fn put(url: &str, body: Vec<u8>) -> String {
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

fn has_url(value: &Value) -> bool {
    value.to_string().contains("X-Amz-Signature")
}

#[tokio::test]
async fn every_route_refuses_callers_outside_the_workspace_before_signing() {
    let fixture = fixture().await;
    let folder = fixture.folder(None, "Docs").await;
    let file = fixture.file(Some(&folder), "a.txt", b"secret").await;
    let started = fixture
        .owner(
            "POST",
            "/uploads",
            Some(json!({"name": "big.bin", "size": 3 * MIN_PART_BYTES})),
            StatusCode::CREATED,
        )
        .await;
    let upload = started["upload"]["id"].as_str().unwrap().to_owned();
    let outsider = fixture.user("outsider@example.com", None).await;
    let routes = [
        ("GET", "/nodes".to_owned(), None),
        ("POST", "/folders".to_owned(), Some(json!({"name": "x"}))),
        ("GET", format!("/nodes/{file}"), None),
        ("DELETE", format!("/nodes/{file}"), None),
        (
            "POST",
            format!("/nodes/{file}/rename"),
            Some(json!({"name": "b.txt"})),
        ),
        (
            "POST",
            format!("/nodes/{file}/move"),
            Some(json!({"parent_id": null})),
        ),
        ("POST", format!("/nodes/{file}/trash"), None),
        ("POST", format!("/nodes/{file}/restore"), None),
        ("GET", format!("/nodes/{folder}/files"), None),
        ("GET", format!("/nodes/{file}/download"), None),
        (
            "POST",
            "/downloads".to_owned(),
            Some(json!({"node_ids": [file]})),
        ),
        ("GET", "/trash".to_owned(), None),
        ("POST", "/trash/empty".to_owned(), None),
        ("GET", "/search?q=a".to_owned(), None),
        ("GET", "/uploads".to_owned(), None),
        (
            "POST",
            "/uploads".to_owned(),
            Some(json!({"name": "x", "size": 1})),
        ),
        ("GET", format!("/uploads/{upload}"), None),
        ("DELETE", format!("/uploads/{upload}"), None),
        (
            "POST",
            format!("/uploads/{upload}/parts"),
            Some(json!({"part_numbers": [1]})),
        ),
        ("POST", format!("/uploads/{upload}/complete"), None),
    ];
    for (method, path, body) in &routes {
        let (status, value) = fixture
            .call(method, &fixture.path(path), None, body.clone())
            .await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "anonymous {method} {path}"
        );
        assert!(!has_url(&value));
        let (status, value) = fixture
            .call(method, &fixture.path(path), Some(&outsider), body.clone())
            .await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "outsider {method} {path}: {value}"
        );
        assert_eq!(value["code"], "drive_not_found");
        assert!(!has_url(&value));
    }
    // A node is only found through its own workspace.
    let other = Id::new_v7();
    let (status, _) = fixture
        .call(
            "GET",
            &format!("/api/v1/workspaces/{other}/drive/nodes/{file}/download"),
            Some(&fixture.owner),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // A member works in the drive but cannot touch another member's upload.
    let member = fixture.user("member@example.com", Some("member")).await;
    let (status, value) = fixture
        .call(
            "GET",
            &fixture.path(&format!("/nodes/{file}/download")),
            Some(&member),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(has_url(&value));
    let (status, value) = fixture
        .call(
            "POST",
            &fixture.path(&format!("/uploads/{upload}/parts")),
            Some(&member),
            Some(json!({"part_numbers": [1]})),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(!has_url(&value));
    // Nothing changed for the outsider's calls.
    assert_eq!(fixture.names(None).await, ["Docs"]);
    assert_eq!(fixture.names(Some(&folder)).await, ["a.txt"]);
}

#[tokio::test]
async fn names_are_unique_per_folder_in_any_case_and_folders_cannot_move_into_themselves() {
    let fixture = fixture().await;
    let docs = fixture.folder(None, "Docs").await;
    let problem = fixture
        .owner(
            "POST",
            "/folders",
            Some(json!({"name": " docs "})),
            StatusCode::CONFLICT,
        )
        .await;
    assert_eq!(problem["code"], "drive_name_taken");
    let renamed = fixture
        .owner(
            "POST",
            "/folders",
            Some(json!({"name": "DOCS", "on_conflict": "rename"})),
            StatusCode::CREATED,
        )
        .await;
    assert_eq!(renamed["name"], "DOCS (2)");
    for name in ["a/b", "..", ""] {
        let problem = fixture
            .owner(
                "POST",
                "/folders",
                Some(json!({"name": name})),
                StatusCode::UNPROCESSABLE_ENTITY,
            )
            .await;
        assert_eq!(problem["code"], "drive_invalid_name");
    }

    // The same name in another folder is fine; renaming onto a sibling is not.
    let inner = fixture.folder(Some(&docs), "Docs").await;
    let deeper = fixture.folder(Some(&inner), "Deeper").await;
    let file = fixture.file(Some(&docs), "notes.txt", b"n").await;
    let problem = fixture
        .owner(
            "POST",
            &format!("/nodes/{file}/rename"),
            Some(json!({"name": "DOCS"})),
            StatusCode::CONFLICT,
        )
        .await;
    assert_eq!(problem["code"], "drive_name_taken");
    let same = fixture
        .owner(
            "POST",
            &format!("/nodes/{file}/rename"),
            Some(json!({"name": "Notes.txt"})),
            StatusCode::OK,
        )
        .await;
    assert_eq!(same["name"], "Notes.txt");

    for target in [&docs, &inner, &deeper] {
        let problem = fixture
            .owner(
                "POST",
                &format!("/nodes/{docs}/move"),
                Some(json!({"parent_id": target})),
                StatusCode::UNPROCESSABLE_ENTITY,
            )
            .await;
        assert_eq!(problem["code"], "drive_move_cycle");
    }
    let problem = fixture
        .owner(
            "POST",
            &format!("/nodes/{deeper}/move"),
            Some(json!({"parent_id": file})),
            StatusCode::UNPROCESSABLE_ENTITY,
        )
        .await;
    assert_eq!(problem["code"], "drive_not_a_folder");
    // Inner "Docs" cannot go to the top level, where "Docs" exists.
    let problem = fixture
        .owner(
            "POST",
            &format!("/nodes/{inner}/move"),
            Some(json!({"parent_id": null})),
            StatusCode::CONFLICT,
        )
        .await;
    assert_eq!(problem["code"], "drive_name_taken");
    fixture
        .owner(
            "POST",
            &format!("/nodes/{deeper}/move"),
            Some(json!({"parent_id": null})),
            StatusCode::OK,
        )
        .await;
    assert_eq!(fixture.names(None).await, ["Deeper", "Docs", "DOCS (2)"]);
    let detail = fixture
        .owner("GET", &format!("/nodes/{inner}"), None, StatusCode::OK)
        .await;
    assert_eq!(detail["path"], json!([{"id": docs, "name": "Docs"}]));

    // A trashed name is free again.
    fixture
        .owner(
            "POST",
            &format!("/nodes/{docs}/trash"),
            None,
            StatusCode::NO_CONTENT,
        )
        .await;
    fixture.folder(None, "Docs").await;
}

#[tokio::test]
async fn trash_and_restore_move_whole_trees_and_restore_never_overwrites() {
    let fixture = fixture().await;
    let projects = fixture.folder(None, "Projects").await;
    let alpha = fixture.folder(Some(&projects), "Alpha").await;
    let file = fixture.file(Some(&alpha), "plan.txt", b"p").await;
    let loose = fixture.file(Some(&projects), "loose.txt", b"l").await;

    // `loose` goes to the trash first, then its folder.
    fixture
        .owner(
            "POST",
            &format!("/nodes/{loose}/trash"),
            None,
            StatusCode::NO_CONTENT,
        )
        .await;
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    fixture
        .owner(
            "POST",
            &format!("/nodes/{projects}/trash"),
            None,
            StatusCode::NO_CONTENT,
        )
        .await;
    assert!(fixture.names(None).await.is_empty());
    for hidden in [&alpha, &file] {
        fixture
            .owner(
                "GET",
                &format!("/nodes/{hidden}"),
                None,
                StatusCode::NOT_FOUND,
            )
            .await;
    }
    let found = fixture
        .owner("GET", "/search?q=plan", None, StatusCode::OK)
        .await;
    assert_eq!(found["items"], json!([]));
    let (status, _) = fixture
        .call(
            "GET",
            &fixture.path(&format!("/nodes/{file}/download")),
            Some(&fixture.owner),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // Nothing can be created in or moved into a trashed folder.
    fixture
        .owner(
            "POST",
            "/folders",
            Some(json!({"parent_id": alpha, "name": "x"})),
            StatusCode::NOT_FOUND,
        )
        .await;
    let trash = fixture.owner("GET", "/trash", None, StatusCode::OK).await;
    let trashed: Vec<&str> = trash["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|node| node["name"].as_str().unwrap())
        .collect();
    assert_eq!(trashed, ["Projects", "loose.txt"]);

    // A new "Projects" takes the name; the restored tree gets the next one and comes back whole.
    fixture.folder(None, "Projects").await;
    let restored = fixture
        .owner(
            "POST",
            &format!("/nodes/{projects}/restore"),
            None,
            StatusCode::OK,
        )
        .await;
    assert_eq!(restored["name"], "Projects (2)");
    assert_eq!(restored["trashed_at"], Value::Null);
    assert_eq!(fixture.names(Some(&projects)).await, ["Alpha"]);
    assert_eq!(fixture.names(Some(&alpha)).await, ["plan.txt"]);
    let found = fixture
        .owner("GET", "/search?q=PLAN", None, StatusCode::OK)
        .await;
    assert_eq!(found["items"][0]["folder_path"], "Projects (2)/Alpha");
    // `loose` was trashed on its own and stays in the trash.
    let trash = fixture.owner("GET", "/trash", None, StatusCode::OK).await;
    assert_eq!(trash["items"].as_array().unwrap().len(), 1);

    // An item whose folder is in the trash comes back at the top level.
    fixture
        .owner(
            "POST",
            &format!("/nodes/{projects}/trash"),
            None,
            StatusCode::NO_CONTENT,
        )
        .await;
    let problem = fixture
        .owner(
            "POST",
            &format!("/nodes/{alpha}/restore"),
            None,
            StatusCode::NOT_FOUND,
        )
        .await;
    assert_eq!(problem["code"], "drive_not_found");
    let back = fixture
        .owner(
            "POST",
            &format!("/nodes/{loose}/restore"),
            None,
            StatusCode::OK,
        )
        .await;
    assert_eq!(back["parent_id"], Value::Null);
    assert_eq!(fixture.names(None).await, ["Projects", "loose.txt"]);
}

#[tokio::test]
async fn only_admins_delete_forever_and_the_objects_leave_the_bucket() {
    let fixture = fixture().await;
    let folder = fixture.folder(None, "Old").await;
    fixture.file(Some(&folder), "a.txt", b"aaa").await;
    let kept = fixture.file(None, "kept.txt", b"kept").await;
    assert_eq!(fixture.object_count().await, 2);
    let member = fixture.user("member@example.com", Some("member")).await;
    let admin = fixture.user("admin@example.com", Some("admin")).await;

    let problem = fixture
        .owner(
            "DELETE",
            &format!("/nodes/{folder}"),
            None,
            StatusCode::CONFLICT,
        )
        .await;
    assert_eq!(problem["code"], "drive_not_trashed");
    fixture
        .owner(
            "POST",
            &format!("/nodes/{folder}/trash"),
            None,
            StatusCode::NO_CONTENT,
        )
        .await;
    for (method, path) in [
        ("DELETE", format!("/nodes/{folder}")),
        ("POST", "/trash/empty".to_owned()),
    ] {
        let (status, problem) = fixture
            .call(method, &fixture.path(&path), Some(&member), None)
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {path}");
        assert_eq!(problem["code"], "drive_forbidden");
    }
    let (status, _) = fixture
        .call(
            "DELETE",
            &fixture.path(&format!("/nodes/{folder}")),
            Some(&admin),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    fixture.drive.delete_objects().await.unwrap();
    assert_eq!(fixture.object_count().await, 1);
    let blobs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM drive_blobs")
        .fetch_one(fixture.database.pool())
        .await
        .unwrap();
    assert_eq!(blobs, 1);
    let audited: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM audit_events WHERE action = 'drive.purged'")
            .fetch_one(fixture.database.pool())
            .await
            .unwrap();
    assert_eq!(audited, 1);

    // Emptying the trash takes every trashed tree.
    fixture
        .owner(
            "POST",
            &format!("/nodes/{kept}/trash"),
            None,
            StatusCode::NO_CONTENT,
        )
        .await;
    let emptied = fixture
        .owner("POST", "/trash/empty", None, StatusCode::OK)
        .await;
    assert_eq!(emptied["deleted"], 1);
    fixture.drive.delete_objects().await.unwrap();
    assert_eq!(fixture.object_count().await, 0);
}

#[tokio::test]
async fn a_multipart_upload_resumes_and_completes_only_with_every_part_at_its_size() {
    let fixture = fixture().await;
    let part = usize::try_from(MIN_PART_BYTES).unwrap();
    let size = 2 * part + 3;
    let started = fixture
        .owner(
            "POST",
            "/uploads",
            Some(json!({"name": "video.mp4", "size": size, "mime": "video/mp4"})),
            StatusCode::CREATED,
        )
        .await;
    assert_eq!(started["url"], Value::Null);
    assert_eq!(started["upload"]["part_size"], MIN_PART_BYTES);
    assert_eq!(started["upload"]["part_count"], 3);
    let upload = started["upload"]["id"].as_str().unwrap().to_owned();
    for numbers in [json!([0]), json!([4]), json!([])] {
        let problem = fixture
            .owner(
                "POST",
                &format!("/uploads/{upload}/parts"),
                Some(json!({"part_numbers": numbers})),
                StatusCode::UNPROCESSABLE_ENTITY,
            )
            .await;
        assert_eq!(problem["code"], "drive_invalid_part");
    }
    let urls = fixture
        .owner(
            "POST",
            &format!("/uploads/{upload}/parts"),
            Some(json!({"part_numbers": [2, 3]})),
            StatusCode::OK,
        )
        .await;
    put(urls["items"][0]["url"].as_str().unwrap(), vec![2; part]).await;
    // A last part of the wrong size is caught before the bucket joins the parts.
    put(urls["items"][1]["url"].as_str().unwrap(), vec![3; 2]).await;

    // After a reload, the status says what the bucket has.
    let status = fixture
        .owner("GET", &format!("/uploads/{upload}"), None, StatusCode::OK)
        .await;
    assert_eq!(
        status["uploaded_parts"],
        json!([{"part_number": 2, "size": part}, {"part_number": 3, "size": 2}])
    );
    let listed = fixture.owner("GET", "/uploads", None, StatusCode::OK).await;
    assert_eq!(listed["items"][0]["id"], upload.as_str());
    let problem = fixture
        .owner(
            "POST",
            &format!("/uploads/{upload}/complete"),
            None,
            StatusCode::CONFLICT,
        )
        .await;
    assert_eq!(problem["code"], "drive_upload_incomplete");

    let urls = fixture
        .owner(
            "POST",
            &format!("/uploads/{upload}/parts"),
            Some(json!({"part_numbers": [1, 3]})),
            StatusCode::OK,
        )
        .await;
    put(urls["items"][0]["url"].as_str().unwrap(), vec![1; part]).await;
    let problem = fixture
        .owner(
            "POST",
            &format!("/uploads/{upload}/complete"),
            None,
            StatusCode::CONFLICT,
        )
        .await;
    assert_eq!(problem["code"], "drive_upload_incomplete");
    put(urls["items"][1]["url"].as_str().unwrap(), vec![3; 3]).await;
    let node = fixture
        .owner(
            "POST",
            &format!("/uploads/{upload}/complete"),
            None,
            StatusCode::CREATED,
        )
        .await;
    assert_eq!(node["size"], size);
    assert_eq!(node["mime"], "video/mp4");
    let listed = fixture.owner("GET", "/uploads", None, StatusCode::OK).await;
    assert_eq!(listed["items"], json!([]));

    let download = fixture
        .owner(
            "GET",
            &format!("/nodes/{}/download", node["id"].as_str().unwrap()),
            None,
            StatusCode::OK,
        )
        .await;
    let response = reqwest::get(download["url"].as_str().unwrap())
        .await
        .unwrap();
    assert!(
        response.headers()["content-disposition"]
            .to_str()
            .unwrap()
            .contains("filename=\"video.mp4\"")
    );
    let bytes = response.bytes().await.unwrap();
    assert_eq!(bytes.len(), size);
    assert!(bytes[..part].iter().all(|byte| *byte == 1));
    assert!(bytes[part..2 * part].iter().all(|byte| *byte == 2));
    assert_eq!(&bytes[2 * part..], &[3, 3, 3]);
}

#[tokio::test]
async fn a_single_put_completes_only_with_the_declared_size_and_replace_trashes_the_old_file() {
    let fixture = fixture().await;
    let old = fixture.file(None, "report.txt", b"old").await;
    let problem = fixture
        .owner(
            "POST",
            "/uploads",
            Some(json!({"name": "REPORT.txt", "size": 5})),
            StatusCode::CONFLICT,
        )
        .await;
    assert_eq!(problem["code"], "drive_name_taken");
    let started = fixture
        .owner(
            "POST",
            "/uploads",
            Some(json!({"name": "report.txt", "size": 5, "on_conflict": "replace"})),
            StatusCode::CREATED,
        )
        .await;
    let upload = started["upload"]["id"].as_str().unwrap();
    let url = started["url"].as_str().unwrap();
    fixture
        .owner(
            "POST",
            &format!("/uploads/{upload}/complete"),
            None,
            StatusCode::CONFLICT,
        )
        .await;
    put(url, b"four".to_vec()).await;
    fixture
        .owner(
            "POST",
            &format!("/uploads/{upload}/complete"),
            None,
            StatusCode::CONFLICT,
        )
        .await;
    put(url, b"fifth".to_vec()).await;
    let node = fixture
        .owner(
            "POST",
            &format!("/uploads/{upload}/complete"),
            None,
            StatusCode::CREATED,
        )
        .await;
    assert_eq!(node["name"], "report.txt");
    assert_ne!(node["id"], old.as_str());
    assert_eq!(fixture.names(None).await, ["report.txt"]);
    let trash = fixture.owner("GET", "/trash", None, StatusCode::OK).await;
    assert_eq!(trash["items"][0]["id"], old.as_str());
    // Completing twice does not add a second file.
    fixture
        .owner(
            "POST",
            &format!("/uploads/{upload}/complete"),
            None,
            StatusCode::NOT_FOUND,
        )
        .await;

    let problem = fixture
        .owner(
            "POST",
            "/uploads",
            Some(json!({"name": "huge.bin", "size": 5_i64 * 1024 * 1024 * 1024 * 1024})),
            StatusCode::PAYLOAD_TOO_LARGE,
        )
        .await;
    assert_eq!(problem["code"], "drive_file_too_large");
}

#[tokio::test]
async fn cancelled_and_stale_uploads_are_aborted_and_old_trash_expires() {
    let fixture = fixture().await;
    let start = |name: &'static str| {
        let fixture = &fixture;
        async move {
            let started = fixture
                .owner(
                    "POST",
                    "/uploads",
                    Some(json!({"name": name, "size": 3 * MIN_PART_BYTES})),
                    StatusCode::CREATED,
                )
                .await;
            let upload = started["upload"]["id"].as_str().unwrap().to_owned();
            let urls = fixture
                .owner(
                    "POST",
                    &format!("/uploads/{upload}/parts"),
                    Some(json!({"part_numbers": [1]})),
                    StatusCode::OK,
                )
                .await;
            put(
                urls["items"][0]["url"].as_str().unwrap(),
                vec![0; usize::try_from(MIN_PART_BYTES).unwrap()],
            )
            .await;
            let (key, upload_id): (String, String) =
                sqlx::query_as("SELECT r2_key, upload_id FROM drive_blobs WHERE id = $1")
                    .bind(&upload)
                    .fetch_one(fixture.database.pool())
                    .await
                    .unwrap();
            (upload, key, upload_id)
        }
    };
    let (cancelled, cancelled_key, cancelled_upload) = start("cancelled.bin").await;
    fixture
        .owner(
            "DELETE",
            &format!("/uploads/{cancelled}"),
            None,
            StatusCode::NO_CONTENT,
        )
        .await;
    fixture.drive.delete_objects().await.unwrap();
    assert!(
        fixture
            .bucket
            .list_parts(&cancelled_key, &cancelled_upload)
            .await
            .unwrap()
            .is_none()
    );

    let (_, stale_key, stale_upload) = start("stale.bin").await;
    let old = fixture.file(None, "old.txt", b"old").await;
    fixture
        .owner(
            "POST",
            &format!("/nodes/{old}/trash"),
            None,
            StatusCode::NO_CONTENT,
        )
        .await;
    let now = TimestampMillis::now().as_millis();
    // Five days later both are kept.
    fixture
        .drive
        .maintain(TimestampMillis::from_millis(now + 5 * DAY_MILLIS))
        .await
        .unwrap();
    assert!(
        fixture
            .bucket
            .list_parts(&stale_key, &stale_upload)
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(fixture.object_count().await, 1);
    fixture
        .drive
        .maintain(TimestampMillis::from_millis(
            now + TRASH_RETENTION_MILLIS + DAY_MILLIS,
        ))
        .await
        .unwrap();
    assert!(
        fixture
            .bucket
            .list_parts(&stale_key, &stale_upload)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(fixture.object_count().await, 0);
    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM drive_blobs")
        .fetch_one(fixture.database.pool())
        .await
        .unwrap();
    assert_eq!(rows, 0);
    let trash = fixture.owner("GET", "/trash", None, StatusCode::OK).await;
    assert_eq!(trash["items"], json!([]));
}

#[tokio::test]
async fn folder_files_and_batch_downloads_cover_only_live_files() {
    let fixture = fixture().await;
    let top = fixture.folder(None, "Top").await;
    let sub = fixture.folder(Some(&top), "Sub").await;
    let a = fixture.file(Some(&top), "a.txt", b"a").await;
    let b = fixture.file(Some(&sub), "b.txt", b"b").await;
    let gone = fixture.folder(Some(&top), "Gone").await;
    let c = fixture.file(Some(&gone), "c.txt", b"c").await;
    fixture
        .owner(
            "POST",
            &format!("/nodes/{gone}/trash"),
            None,
            StatusCode::NO_CONTENT,
        )
        .await;
    let files = fixture
        .owner("GET", &format!("/nodes/{top}/files"), None, StatusCode::OK)
        .await;
    let paths: Vec<(String, String)> = files["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| {
            (
                entry["folder_path"].as_str().unwrap().to_owned(),
                entry["node"]["name"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    assert_eq!(
        paths,
        [
            (String::new(), "a.txt".to_owned()),
            ("Sub".to_owned(), "b.txt".to_owned())
        ]
    );
    let downloads = fixture
        .owner(
            "POST",
            "/downloads",
            Some(json!({"node_ids": [a, b, c, top, "not-an-id"]})),
            StatusCode::OK,
        )
        .await;
    let ids: Vec<&str> = downloads["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["node_id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, [a.as_str(), b.as_str()]);
    let problem = fixture
        .owner(
            "POST",
            "/downloads",
            Some(json!({"node_ids": vec![a.clone(); 101]})),
            StatusCode::UNPROCESSABLE_ENTITY,
        )
        .await;
    assert_eq!(problem["code"], "drive_too_many_items");
}
