//! Chat media: file types from the bytes, previews (one per stored file version, queued once, failed and skipped
//! conversions), files larger than the chat limit in the conversation's Drive folder (resumable multipart upload),
//! and the privacy of a direct message's Drive folder. Against the in-memory S3 server.

use std::path::PathBuf;
use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use orbit_platform::{
    AttachmentMutationCoordinator, AuthenticatedUser, Id, LocalBlobStore, ObjectStorage,
    ObjectStorageState, PasswordService, S3Bucket, TestDatabase, TimestampMillis, UploadLimits,
    UploadService, start_fake_s3,
};
use orbit_server::auth_routes::CookieMode;
use orbit_server::chat_routes::{ChatState, chat_router};
use orbit_server::drive::{DriveService, MIN_PART_BYTES};
use orbit_server::drive_routes::{DriveState, drive_router};
use orbit_server::previews::{PreviewService, PreviewSettings};
use orbit_server::repositories::identity::{IdentityRepository, SetupRequest};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;

struct Fixture {
    _root: tempfile::TempDir,
    database: TestDatabase,
    identity: Arc<IdentityRepository>,
    uploads: UploadService,
    drive: DriveService,
    app: axum::Router,
    workspace_id: String,
    owner: Member,
    nonce: std::sync::atomic::AtomicU64,
}

#[derive(Clone)]
struct Member {
    id: Id,
    cookie: String,
}

/// The start of an iPhone video: an `ftyp` box of brand `qt  `.
fn quicktime_bytes() -> Vec<u8> {
    let mut bytes = vec![0, 0, 0, 0x14];
    bytes.extend_from_slice(b"ftypqt  \0\0\0\0qt  ");
    bytes.extend_from_slice(&[0; 64]);
    bytes
}

/// The start of an `.xlsx` (a ZIP with `[Content_Types].xml`).
fn xlsx_bytes() -> Vec<u8> {
    let mut bytes = b"PK\x03\x04".to_vec();
    bytes.extend_from_slice(&[0; 26]);
    bytes.extend_from_slice(b"[Content_Types].xml xl/workbook.xml");
    bytes
}

impl Fixture {
    async fn new() -> Self {
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
        let root = tempfile::tempdir().unwrap();
        let uploads = UploadService::new(
            (*database).clone(),
            Arc::new(LocalBlobStore::new(root.path().join("attachments"))),
            AttachmentMutationCoordinator::default(),
            UploadLimits::default(),
        );
        let storage = ObjectStorage::default();
        storage.configure(ObjectStorageState {
            bucket: Some(S3Bucket::new(start_fake_s3().await).unwrap()),
            ..Default::default()
        });
        let drive = DriveService::new((*database).clone(), storage);
        let app = chat_router(
            ChatState::new(Arc::clone(&identity), uploads.clone(), CookieMode::secure())
                .with_drive(drive.clone()),
        )
        .merge(drive_router(DriveState::new(
            Arc::clone(&identity),
            drive.clone(),
            CookieMode::secure(),
        )));
        Self {
            _root: root,
            database,
            identity,
            uploads,
            drive,
            app,
            workspace_id: setup.workspace_id.to_string(),
            owner: Member {
                id: setup.user_id,
                cookie: format!("__Host-orbit_session={}", setup.session.token),
            },
            nonce: std::sync::atomic::AtomicU64::new(0),
        }
    }

    async fn member(&self, name: &str) -> Member {
        let id = Id::new_v7();
        let now = TimestampMillis::now();
        let email = format!("{name}@example.com");
        sqlx::query(
            "INSERT INTO users (id, email, normalized_email, display_name, password_hash, created_at, updated_at) \
             VALUES ($1, $2, $2, $3, 'unused', $4, $4)",
        )
        .bind(id.to_string())
        .bind(&email)
        .bind(name)
        .bind(now.as_millis())
        .execute(self.database.pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO memberships (id, workspace_id, user_id, role, version, created_at, updated_at) \
             VALUES ($1, $2, $3, 'member', 0, $4, $4)",
        )
        .bind(Id::new_v7().to_string())
        .bind(&self.workspace_id)
        .bind(id.to_string())
        .bind(now.as_millis())
        .execute(self.database.pool())
        .await
        .unwrap();
        let session = self
            .identity
            .create_session(
                &AuthenticatedUser {
                    id,
                    email,
                    display_name: name.to_owned(),
                },
                now,
            )
            .await
            .unwrap();
        Member {
            id,
            cookie: format!("__Host-orbit_session={}", session.token),
        }
    }

    async fn request(
        &self,
        member: &Member,
        method: &str,
        path: &str,
        body: Option<Value>,
        range: Option<&str>,
    ) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
        let mut builder = Request::builder()
            .method(method)
            .uri(path)
            .header(header::COOKIE, &member.cookie);
        if let Some(range) = range {
            builder = builder.header(header::RANGE, range);
        }
        let request = match body {
            Some(value) => builder
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(value.to_string())),
            None => builder.body(Body::empty()),
        }
        .unwrap();
        let response = self.app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, headers, bytes.to_vec())
    }

    /// A call under `/api/v1/workspaces/{id}/{area}` that must answer `expected`.
    async fn call(
        &self,
        member: &Member,
        method: &str,
        path: &str,
        body: Option<Value>,
        expected: StatusCode,
    ) -> Value {
        let path = format!("/api/v1/workspaces/{}{path}", self.workspace_id);
        let (status, _, bytes) = self.request(member, method, &path, body, None).await;
        let value: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        assert_eq!(status, expected, "{method} {path}: {value}");
        value
    }

    async fn upload(&self, member: &Member, name: &str, bytes: &[u8]) -> Value {
        let boundary = "orbit-test-boundary";
        let mut body = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{name}\"\r\n\
             Content-Type: application/octet-stream\r\n\r\n"
        )
        .into_bytes();
        body.extend_from_slice(bytes);
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        let request = Request::builder()
            .method("POST")
            .uri(format!(
                "/api/v1/workspaces/{}/chat/files",
                self.workspace_id
            ))
            .header(header::COOKIE, &member.cookie)
            .header(
                header::CONTENT_TYPE,
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(Body::from(body))
            .unwrap();
        let response = self.app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let value: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
                .unwrap_or(Value::Null);
        assert_eq!(status, StatusCode::CREATED, "{value}");
        value
    }

    async fn send(&self, member: &Member, conversation: &str, file_ids: &[&str]) -> Value {
        let nonce = self
            .nonce
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.call(
            member,
            "POST",
            &format!("/chat/conversations/{conversation}/messages"),
            Some(json!({ "body": "", "file_ids": file_ids, "nonce": format!("n{nonce}") })),
            StatusCode::OK,
        )
        .await["result"]
            .clone()
    }

    async fn general(&self) -> String {
        let conversations = self
            .call(
                &self.owner,
                "GET",
                "/chat/conversations",
                None,
                StatusCode::OK,
            )
            .await;
        conversations
            .as_array()
            .unwrap()
            .iter()
            .find(|conversation| conversation["is_default"] == true)
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    async fn scalar(&self, sql: &'static str) -> i64 {
        sqlx::query_scalar(sql)
            .fetch_one(self.database.pool())
            .await
            .unwrap()
    }

    async fn preview_id(&self, kind: &str) -> Id {
        let id: String = sqlx::query_scalar(
            "SELECT id FROM file_previews WHERE kind = $1 ORDER BY created_at DESC, id DESC LIMIT 1",
        )
        .bind(kind)
        .fetch_one(self.database.pool())
        .await
        .unwrap();
        id.parse().unwrap()
    }

    fn previews(&self, settings: PreviewSettings) -> PreviewService {
        PreviewService::new(
            (*self.database).clone(),
            self.uploads.clone(),
            self.drive.clone(),
            settings,
        )
    }
}

async fn put(url: &str, body: Vec<u8>) {
    reqwest::Client::new()
        .put(url)
        .body(body)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
}

#[tokio::test]
async fn chat_files_are_typed_by_their_bytes_and_each_version_gets_one_preview() {
    let fixture = Fixture::new().await;
    let owner = fixture.owner.clone();
    // The client called it an octet stream; its bytes say it is a QuickTime video.
    let video = fixture
        .upload(&owner, "IMG_0001.MOV", &quicktime_bytes())
        .await;
    assert_eq!(video["mime_type"], "video/quicktime");
    assert_eq!(video["preview"]["kind"], "video");
    assert_eq!(video["preview"]["status"], "pending");
    assert_eq!(video["preview"]["play_url"], Value::Null);
    let sheet = fixture.upload(&owner, "Budget.xlsx", &xlsx_bytes()).await;
    assert_eq!(
        sheet["mime_type"],
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
    );
    assert_eq!(sheet["preview"]["kind"], "document");
    let text = fixture.upload(&owner, "notes.txt", b"just text").await;
    assert_eq!(text["preview"], Value::Null);
    assert_eq!(
        fixture
            .scalar("SELECT COUNT(*) FROM jobs WHERE kind = 'file.preview'")
            .await,
        2
    );

    // The same bytes again (a forward, a second upload) share the preview of their stored version.
    let again = fixture.upload(&owner, "copy.mov", &quicktime_bytes()).await;
    assert_eq!(again["preview"]["status"], "pending");
    assert_eq!(
        fixture.scalar("SELECT COUNT(*) FROM file_previews").await,
        2
    );
    assert_eq!(
        fixture
            .scalar("SELECT COUNT(*) FROM jobs WHERE kind = 'file.preview'")
            .await,
        2
    );
    // A new version (other bytes) gets its own.
    let mut changed = quicktime_bytes();
    changed.push(1);
    fixture.upload(&owner, "IMG_0001.MOV", &changed).await;
    assert_eq!(
        fixture.scalar("SELECT COUNT(*) FROM file_previews").await,
        3
    );

    // A preview is asked for again until it is made; it is never queued twice.
    let id = video["id"].as_str().unwrap();
    let preview = fixture
        .call(
            &owner,
            "POST",
            &format!("/chat/files/{id}/preview"),
            None,
            StatusCode::OK,
        )
        .await;
    assert_eq!(preview["status"], "pending");
    assert_eq!(
        fixture.scalar("SELECT COUNT(*) FROM file_previews").await,
        3
    );
    let text_id = text["id"].as_str().unwrap();
    fixture
        .call(
            &owner,
            "GET",
            &format!("/chat/files/{text_id}/preview"),
            None,
            StatusCode::NOT_FOUND,
        )
        .await;
}

#[tokio::test]
async fn a_failed_conversion_keeps_the_original_and_a_missing_converter_skips_the_preview() {
    let fixture = Fixture::new().await;
    let owner = fixture.owner.clone();
    assert_eq!(
        orbit_server::previews::job_kind()
            .default_limits()
            .concurrency,
        Some(1),
        "one conversion at a time on all servers"
    );
    let video = fixture.upload(&owner, "clip.mov", &quicktime_bytes()).await;
    let general = fixture.general().await;
    fixture
        .send(&owner, &general, &[video["id"].as_str().unwrap()])
        .await;
    let failing = fixture.previews(PreviewSettings {
        ffprobe: PathBuf::from("/bin/false"),
        ffmpeg: PathBuf::from("/bin/false"),
        ..PreviewSettings::default()
    });
    failing
        .run(fixture.preview_id("video").await, &CancellationToken::new())
        .await
        .unwrap();
    let id = video["id"].as_str().unwrap();
    let preview = fixture
        .call(
            &owner,
            "GET",
            &format!("/chat/files/{id}/preview"),
            None,
            StatusCode::OK,
        )
        .await;
    assert_eq!(preview["status"], "failed");
    assert_eq!(preview["play_url"], Value::Null);
    // The original stays: the client falls back to it.
    let url = video["url"].as_str().unwrap();
    let (status, headers, bytes) = fixture
        .request(&owner, "GET", url, None, Some("bytes=4-11"))
        .await;
    assert_eq!(status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(bytes, b"ftypqt  ");
    assert_eq!(
        headers[header::CONTENT_RANGE],
        format!("bytes 4-11/{}", quicktime_bytes().len())
    );

    let sheet = fixture.upload(&owner, "Budget.xlsx", &xlsx_bytes()).await;
    fixture
        .previews(PreviewSettings {
            soffice: PathBuf::from("/no/such/soffice"),
            ..PreviewSettings::default()
        })
        .run(
            fixture.preview_id("document").await,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    let id = sheet["id"].as_str().unwrap();
    let preview = fixture
        .call(
            &owner,
            "GET",
            &format!("/chat/files/{id}/preview"),
            None,
            StatusCode::OK,
        )
        .await;
    assert_eq!(preview["status"], "skipped");
    assert_eq!(preview["pdf_url"], Value::Null);
    let detail: String =
        sqlx::query_scalar("SELECT detail FROM file_previews WHERE kind = 'document'")
            .fetch_one(fixture.database.pool())
            .await
            .unwrap();
    assert!(detail.contains("not installed"), "{detail}");
}

/// Makes a real (tiny) video with ffmpeg; `None` when ffmpeg is not installed.
fn tiny_video(directory: &std::path::Path) -> Option<Vec<u8>> {
    let path = directory.join("tiny.mov");
    let made = std::process::Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc=size=160x120:rate=10:duration=2",
            "-c:v",
            "mpeg4",
            "-y",
        ])
        .arg(&path)
        .status()
        .ok()?;
    made.success().then(|| std::fs::read(&path).ok()).flatten()
}

#[tokio::test]
async fn a_video_gets_a_seekable_playable_copy_and_a_poster() {
    let fixture = Fixture::new().await;
    let Some(bytes) = tiny_video(fixture._root.path()) else {
        eprintln!("ffmpeg is not installed: skipped");
        return;
    };
    let owner = fixture.owner.clone();
    let video = fixture.upload(&owner, "tiny.mov", &bytes).await;
    assert_eq!(video["mime_type"], "video/quicktime");
    fixture
        .previews(PreviewSettings::default())
        .run(fixture.preview_id("video").await, &CancellationToken::new())
        .await
        .unwrap();
    let id = video["id"].as_str().unwrap();
    let preview = fixture
        .call(
            &owner,
            "GET",
            &format!("/chat/files/{id}/preview"),
            None,
            StatusCode::OK,
        )
        .await;
    assert_eq!(preview["status"], "ready", "{preview}");
    let play = preview["play_url"].as_str().unwrap();
    let (status, headers, body) = fixture
        .request(&owner, "GET", play, None, Some("bytes=0-99"))
        .await;
    assert_eq!(status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(body.len(), 100);
    assert_eq!(&body[4..8], b"ftyp");
    assert!(
        headers[header::CONTENT_RANGE]
            .to_str()
            .unwrap()
            .starts_with("bytes 0-99/")
    );
    let (status, _, poster) = fixture
        .request(
            &owner,
            "GET",
            preview["poster_url"].as_str().unwrap(),
            None,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(poster.starts_with(b"\xff\xd8\xff"), "a JPEG");
    // The outputs are blobs that the source's preview holds: they are references, not orphans.
    assert_eq!(
        fixture
            .scalar("SELECT COUNT(*) FROM file_preview_outputs WHERE blob_id IS NOT NULL")
            .await,
        2
    );
}

#[tokio::test]
async fn a_big_file_goes_to_the_conversations_drive_folder_and_a_dm_folder_stays_private() {
    let fixture = Fixture::new().await;
    let owner = fixture.owner.clone();
    let ada = fixture.member("Ada").await;
    let outsider = fixture.member("Zed").await;
    let limit = fixture
        .call(&owner, "GET", "/chat/upload-limit", None, StatusCode::OK)
        .await;
    assert_eq!(
        limit["max_file_bytes"],
        UploadLimits::default().max_file_bytes()
    );
    assert_eq!(limit["drive"], true);

    let dm = fixture
        .call(
            &owner,
            "POST",
            "/chat/dms",
            Some(json!({ "user_ids": [ada.id.to_string()] })),
            StatusCode::OK,
        )
        .await["result"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let folder = fixture
        .call(
            &owner,
            "POST",
            &format!("/chat/conversations/{dm}/drive-folder"),
            None,
            StatusCode::OK,
        )
        .await;
    assert_eq!(folder["name"], "Ada, Owner");
    let folder_id = folder["id"].as_str().unwrap().to_owned();
    let uploads_id = folder["parent_id"].as_str().unwrap().to_owned();
    // The same folder the next time, also for the other member.
    let again = fixture
        .call(
            &ada,
            "POST",
            &format!("/chat/conversations/{dm}/drive-folder"),
            None,
            StatusCode::OK,
        )
        .await;
    assert_eq!(again["id"], folder_id.as_str());
    // Someone outside the conversation gets no folder.
    fixture
        .call(
            &outsider,
            "POST",
            &format!("/chat/conversations/{dm}/drive-folder"),
            None,
            StatusCode::NOT_FOUND,
        )
        .await;

    // A multipart upload whose connection drops after the first part, then resumes.
    let part = usize::try_from(MIN_PART_BYTES).unwrap();
    let size = 2 * part + 3;
    let started = fixture
        .call(
            &owner,
            "POST",
            "/drive/uploads",
            Some(json!({"parent_id": folder_id, "name": "screen recording.mov", "size": size, "mime": "video/quicktime"})),
            StatusCode::CREATED,
        )
        .await;
    let upload = started["upload"]["id"].as_str().unwrap().to_owned();
    let urls = fixture
        .call(
            &owner,
            "POST",
            &format!("/drive/uploads/{upload}/parts"),
            Some(json!({"part_numbers": [1]})),
            StatusCode::OK,
        )
        .await;
    put(urls["items"][0]["url"].as_str().unwrap(), vec![1; part]).await;
    let status = fixture
        .call(
            &owner,
            "GET",
            &format!("/drive/uploads/{upload}"),
            None,
            StatusCode::OK,
        )
        .await;
    assert_eq!(
        status["uploaded_parts"],
        json!([{"part_number": 1, "size": part}])
    );
    let urls = fixture
        .call(
            &owner,
            "POST",
            &format!("/drive/uploads/{upload}/parts"),
            Some(json!({"part_numbers": [2, 3]})),
            StatusCode::OK,
        )
        .await;
    put(urls["items"][0]["url"].as_str().unwrap(), vec![2; part]).await;
    put(urls["items"][1]["url"].as_str().unwrap(), vec![3; 3]).await;
    let node = fixture
        .call(
            &owner,
            "POST",
            &format!("/drive/uploads/{upload}/complete"),
            None,
            StatusCode::CREATED,
        )
        .await;
    let node_id = node["id"].as_str().unwrap().to_owned();
    assert_eq!(node["preview"]["kind"], "video");
    assert_eq!(node["preview"]["status"], "pending");

    // Only the uploader makes it a chat file, and only for its own conversation.
    fixture
        .call(
            &ada,
            "POST",
            "/chat/files/drive",
            Some(json!({"node_id": node_id})),
            StatusCode::NOT_FOUND,
        )
        .await;
    let file = fixture
        .call(
            &owner,
            "POST",
            "/chat/files/drive",
            Some(json!({"node_id": node_id})),
            StatusCode::CREATED,
        )
        .await;
    assert_eq!(file["drive_node_id"], node_id.as_str());
    let general = fixture.general().await;
    let refused = fixture
        .call(
            &owner,
            "POST",
            &format!("/chat/conversations/{general}/messages"),
            Some(json!({ "body": "", "file_ids": [file["id"]], "nonce": "elsewhere" })),
            StatusCode::UNPROCESSABLE_ENTITY,
        )
        .await;
    assert_eq!(refused["code"], "validation_failed", "{refused}");
    let message = fixture
        .send(&owner, &dm, &[file["id"].as_str().unwrap()])
        .await;
    assert_eq!(message["attachments"][0]["drive_node_id"], node_id.as_str());
    assert_eq!(message["attachments"][0]["size_bytes"], size);
    // Its URL sends the reader to the bucket.
    let (status, headers, _) = fixture
        .request(
            &ada,
            "GET",
            message["attachments"][0]["url"].as_str().unwrap(),
            None,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::TEMPORARY_REDIRECT);
    assert!(
        headers[header::LOCATION]
            .to_str()
            .unwrap()
            .contains("X-Amz-Signature")
    );

    // The direct message's folder and file are the members' only.
    for (member, expected) in [(&ada, StatusCode::OK), (&outsider, StatusCode::NOT_FOUND)] {
        fixture
            .call(
                member,
                "GET",
                &format!("/drive/nodes/{folder_id}"),
                None,
                expected,
            )
            .await;
        fixture
            .call(
                member,
                "GET",
                &format!("/drive/nodes/{node_id}"),
                None,
                expected,
            )
            .await;
        fixture
            .call(
                member,
                "GET",
                &format!("/drive/nodes/{node_id}/download"),
                None,
                expected,
            )
            .await;
    }
    let listed = fixture
        .call(
            &outsider,
            "GET",
            &format!("/drive/nodes?parent_id={uploads_id}"),
            None,
            StatusCode::OK,
        )
        .await;
    assert_eq!(listed["items"], json!([]));
    let found = fixture
        .call(
            &outsider,
            "GET",
            "/drive/search?q=screen",
            None,
            StatusCode::OK,
        )
        .await;
    assert_eq!(found["items"], json!([]));
    let found = fixture
        .call(&ada, "GET", "/drive/search?q=screen", None, StatusCode::OK)
        .await;
    assert_eq!(found["items"].as_array().unwrap().len(), 1);
    fixture
        .call(
            &outsider,
            "GET",
            &format!("/chat/files/{}", file["id"].as_str().unwrap()),
            None,
            StatusCode::NOT_FOUND,
        )
        .await;

    // A channel's folder is seen by every member; moving the Uploads folder keeps the DM folder private.
    let channel_folder = fixture
        .call(
            &owner,
            "POST",
            &format!("/chat/conversations/{general}/drive-folder"),
            None,
            StatusCode::OK,
        )
        .await;
    assert_eq!(channel_folder["name"], "#general");
    assert_eq!(channel_folder["parent_id"], uploads_id.as_str());
    let archive = fixture
        .call(
            &owner,
            "POST",
            "/drive/folders",
            Some(json!({"name": "Archive"})),
            StatusCode::CREATED,
        )
        .await;
    fixture
        .call(
            &owner,
            "POST",
            &format!("/drive/nodes/{uploads_id}/move"),
            Some(json!({"parent_id": archive["id"]})),
            StatusCode::OK,
        )
        .await;
    let listed = fixture
        .call(
            &outsider,
            "GET",
            &format!("/drive/nodes?parent_id={uploads_id}"),
            None,
            StatusCode::OK,
        )
        .await;
    let names: Vec<&str> = listed["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|node| node["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["#general"]);
    fixture
        .call(
            &outsider,
            "GET",
            &format!("/drive/nodes/{node_id}"),
            None,
            StatusCode::NOT_FOUND,
        )
        .await;
    // A member moves the file out of the direct message's folder: it is no longer private.
    fixture
        .call(
            &owner,
            "POST",
            &format!("/drive/nodes/{node_id}/move"),
            Some(json!({"parent_id": archive["id"]})),
            StatusCode::OK,
        )
        .await;
    fixture
        .call(
            &outsider,
            "GET",
            &format!("/drive/nodes/{node_id}"),
            None,
            StatusCode::OK,
        )
        .await;
}
