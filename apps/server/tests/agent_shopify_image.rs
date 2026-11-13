//! A Drive image to Shopify (`/agent-tools/shopify-image`): one call reads the image as the driver, uploads it to a
//! fake Shopify (staged upload, `fileCreate`) and sets it on an article, only after the driver's yes. Files the driver
//! cannot see and files that are not images are refused before anything leaves Orbit. Runs on SQLite, and on Postgres
//! with `ORBIT_TEST_POSTGRES_URL`.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use axum::body::{Body, to_bytes};
use axum::extract::State;
use axum::http::{HeaderMap, Request, StatusCode, header};
use axum::routing::post;
use orbit_platform::{
    AttachmentMutationCoordinator, AuthenticatedUser, Id, LocalBlobStore, ObjectStorage,
    ObjectStorageState, PasswordService, S3Bucket, TestDatabase, TimestampMillis, UploadLimits,
    UploadService, start_fake_s3,
};
use orbit_server::agent::{AgentConfig, AgentState};
use orbit_server::auth_routes::CookieMode;
use orbit_server::chat_routes::{ChatState, chat_router};
use orbit_server::drive::{DriveService, StartUpload};
use orbit_server::repositories::drive::OnConflict;
use orbit_server::repositories::identity::{IdentityRepository, SetupRequest};
use orbit_server::repositories::workspaces::WorkspaceRepository;
use serde_json::{Value, json};
use tower::ServiceExt;

const SECRET: &str = "test-agent-secret-0123456789abcdef0123456789";
const TOKEN: &str = "shpat_very-secret-store-token";
const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR fake image bytes";
const CDN: &str = "https://cdn.shopify.com/s/files/1/hero.png?v=1";

/// What the fake Shopify received, and its one article.
#[derive(Default)]
struct Shop {
    graphql: Vec<Value>,
    tokens: Vec<Option<String>>,
    uploads: Vec<(Option<String>, Vec<u8>)>,
    article_body: String,
    article_image: Value,
}

type Seen = Arc<Mutex<Shop>>;

/// A Shopify Admin API under `/admin/api/2025-10` and a staged upload target on the same host.
async fn fake_shopify() -> (String, Seen) {
    async fn graphql(
        State((seen, base)): State<(Seen, String)>,
        headers: HeaderMap,
        axum::Json(request): axum::Json<Value>,
    ) -> axum::Json<Value> {
        let mut shop = seen.lock().unwrap();
        shop.tokens.push(
            headers
                .get("x-shopify-access-token")
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned),
        );
        shop.graphql.push(request.clone());
        let query = request["query"].as_str().unwrap_or_default();
        let variables = &request["variables"];
        let data = if query.contains("stagedUploadsCreate") {
            json!({ "stagedUploadsCreate": { "stagedTargets": [{
                "url": format!("{base}/staged-upload"),
                "resourceUrl": format!("{base}/staged/tmp/1/hero.png"),
                "parameters": [{ "name": "key", "value": "tmp/1/hero.png" }, { "name": "policy", "value": "signed" }],
            }], "userErrors": [] } })
        } else if query.contains("fileCreate") {
            json!({ "fileCreate": { "files": [{ "id": "gid://shopify/MediaImage/1", "fileStatus": "UPLOADED" }], "userErrors": [] } })
        } else if query.contains("node(") {
            json!({ "node": { "fileStatus": "READY", "image": { "url": CDN } } })
        } else if query.contains("articleUpdate") {
            let article = &variables["article"];
            if let Some(body) = article["body"].as_str() {
                shop.article_body = body.to_owned();
            }
            if !article["image"].is_null() {
                shop.article_image = article["image"].clone();
            }
            json!({ "articleUpdate": { "article": { "id": variables["id"] }, "userErrors": [] } })
        } else if query.contains("article(") {
            json!({ "article": { "body": shop.article_body } })
        } else {
            json!({})
        };
        axum::Json(json!({ "data": data }))
    }
    async fn staged(
        State((seen, _)): State<(Seen, String)>,
        headers: HeaderMap,
        body: axum::body::Bytes,
    ) -> StatusCode {
        seen.lock().unwrap().uploads.push((
            headers
                .get("x-shopify-access-token")
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned),
            body.to_vec(),
        ));
        StatusCode::CREATED
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let seen: Seen = Arc::new(Mutex::new(Shop {
        article_body: "<p>Move more.</p>".to_owned(),
        ..Shop::default()
    }));
    let router = axum::Router::new()
        .route("/admin/api/2025-10/graphql.json", post(graphql))
        .route("/staged-upload", post(staged))
        .with_state((Arc::clone(&seen), base.clone()));
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    (base, seen)
}

struct Fixture {
    _root: tempfile::TempDir,
    database: TestDatabase,
    identity: Arc<IdentityRepository>,
    drive: DriveService,
    app: axum::Router,
    workspace_id: Id,
    owner: (Id, String),
    nonce: AtomicU64,
}

/// The thread of a run and the driver's agent session.
struct Run {
    conversation: String,
    root: String,
    bearer: String,
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
        let chat = ChatState::new(Arc::clone(&identity), uploads, CookieMode::secure());
        let agent = AgentState::new(
            Some(AgentConfig {
                url: "http://127.0.0.1:9".to_owned(),
                secret: SECRET.to_owned(),
            }),
            chat.clone(),
            Arc::clone(&identity),
            CookieMode::secure(),
            Some([3; 32]),
        )
        .with_drive(drive.clone(), "https://orbit.test")
        .with_integration_test_hosts(&["127.0.0.1"]);
        let app = chat_router(chat)
            .merge(orbit_server::agent::router(agent))
            .layer(axum::middleware::from_fn_with_state(
                (*database).clone(),
                orbit_server::agent::scope::layer,
            ));
        Self {
            _root: root,
            database,
            identity,
            drive,
            app,
            workspace_id: setup.workspace_id,
            owner: (
                setup.user_id,
                format!("__Host-orbit_session={}", setup.session.token),
            ),
            nonce: AtomicU64::new(0),
        }
    }

    async fn request(
        &self,
        method: &str,
        uri: &str,
        auth: (&str, &str),
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let builder = Request::builder()
            .method(method)
            .uri(uri)
            .header(auth.0, auth.1);
        let request = match body {
            Some(value) => builder
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(value.to_string())),
            None => builder.body(Body::empty()),
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

    async fn internal(&self, method: &str, path: &str, body: Option<Value>) -> (StatusCode, Value) {
        self.request(
            method,
            &format!("/api/internal/agent{path}"),
            ("authorization", &format!("Bearer {SECRET}")),
            body,
        )
        .await
    }

    /// An HTTP API integration named `name` at `url`, with the store token.
    async fn integration(&self, name: &str, url: &str) {
        let (status, created) = self
            .request(
                "POST",
                &format!(
                    "/api/v1/workspaces/{}/agent-integrations",
                    self.workspace_id
                ),
                ("cookie", &self.owner.1),
                Some(json!({
                    "kind": "http",
                    "name": name,
                    "url": url,
                    "auth_header": "X-Shopify-Access-Token",
                    "secret": TOKEN,
                    "allowed_methods": ["GET", "POST", "PUT"],
                })),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
    }

    async fn send(&self, conversation: &str, body: &str, root: Option<&str>) -> String {
        self.send_as(&self.owner.clone(), conversation, body, root)
            .await
    }

    async fn send_as(
        &self,
        member: &(Id, String),
        conversation: &str,
        body: &str,
        root: Option<&str>,
    ) -> String {
        let nonce = format!("n{}", self.nonce.fetch_add(1, Ordering::Relaxed));
        let (status, value) = self
            .request(
                "POST",
                &format!(
                    "/api/v1/workspaces/{}/chat/conversations/{conversation}/messages",
                    self.workspace_id
                ),
                ("cookie", &member.1),
                Some(json!({ "body": body, "thread_root_id": root, "nonce": nonce })),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{value}");
        value["result"]["id"].as_str().unwrap().to_owned()
    }

    /// A member of the workspace (role member) and their session cookie.
    async fn add_member(&self, name: &str) -> (Id, String) {
        let id = Id::new_v7();
        let now = TimestampMillis::now().as_millis();
        let email = format!("{name}@example.com");
        sqlx::query(
            "INSERT INTO users (id, email, normalized_email, display_name, password_hash, created_at, updated_at) \
             VALUES ($1, $2, $2, $3, 'unused', $4, $4)",
        )
        .bind(id.to_string())
        .bind(&email)
        .bind(name)
        .bind(now)
        .execute(self.database.pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO memberships (id, workspace_id, user_id, role, version, created_at, updated_at) \
             VALUES ($1, $2, $3, 'member', 0, $4, $4)",
        )
        .bind(Id::new_v7().to_string())
        .bind(self.workspace_id.to_string())
        .bind(id.to_string())
        .bind(now)
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
                TimestampMillis::now(),
            )
            .await
            .unwrap();
        (id, format!("__Host-orbit_session={}", session.token))
    }

    /// A thread rooted at a mention of the agent by the owner, now running.
    async fn run(&self) -> Run {
        self.run_as(&self.owner.clone()).await
    }

    /// A thread rooted at a mention of the agent by `driver`, now running.
    async fn run_as(&self, driver: &(Id, String)) -> Run {
        let (_, value) = self.internal("GET", "/inputs", None).await;
        let bot = value["bot_user_id"].as_str().unwrap().to_owned();
        let (_, conversations) = self
            .request(
                "GET",
                &format!(
                    "/api/v1/workspaces/{}/chat/conversations",
                    self.workspace_id
                ),
                ("cookie", &self.owner.1),
                None,
            )
            .await;
        let conversation = conversations
            .as_array()
            .unwrap()
            .iter()
            .find(|conversation| conversation["is_default"] == true)
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let root = self
            .send_as(
                driver,
                &conversation,
                &format!("<@{bot}> put the picture on the blog"),
                None,
            )
            .await;
        let (status, value) = self
            .internal(
                "PUT",
                &format!("/threads/{root}"),
                Some(json!({
                    "workspace_id": self.workspace_id.to_string(),
                    "conversation_id": conversation,
                    "root_message_id": root,
                    "status": "running",
                    "driver_user_id": driver.0.to_string(),
                    "waiting_for_user_id": null,
                })),
            )
            .await;
        assert_eq!(status, StatusCode::NO_CONTENT, "{value}");
        let (status, session) = self
            .internal(
                "POST",
                "/sessions",
                Some(json!({ "user_id": driver.0.to_string(), "workspace_id": self.workspace_id.to_string() })),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{session}");
        Run {
            conversation,
            root,
            bearer: format!("Bearer {}", session["token"].as_str().unwrap()),
        }
    }

    async fn call(&self, run: &Run, arguments: Value) -> Value {
        let (status, value) = self
            .request(
                "POST",
                &format!(
                    "/api/v1/workspaces/{}/agent-tools/shopify-image",
                    self.workspace_id
                ),
                ("authorization", &run.bearer),
                Some(json!({
                    "arguments": arguments,
                    "conversation_id": run.conversation,
                    "root_message_id": run.root,
                })),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{value}");
        value
    }

    /// A Drive file of `workspace` with these bytes.
    async fn file(&self, workspace: Id, name: &str, bytes: &[u8]) -> String {
        self.file_in(workspace, None, name, bytes).await
    }

    /// A Drive file in the folder `parent` (or the top level) of `workspace`, made by the owner.
    async fn file_in(&self, workspace: Id, parent: Option<Id>, name: &str, bytes: &[u8]) -> String {
        let started = self
            .drive
            .start_upload(
                workspace,
                self.owner.0,
                StartUpload {
                    parent_id: parent,
                    name: name.to_owned(),
                    size: i64::try_from(bytes.len()).unwrap(),
                    mime: "application/octet-stream".to_owned(),
                    on_conflict: OnConflict::Fail,
                },
                TimestampMillis::now(),
            )
            .await
            .unwrap();
        let put = reqwest::Client::new()
            .put(started.url.as_deref().unwrap())
            .body(bytes.to_vec())
            .send()
            .await
            .unwrap();
        assert!(put.status().is_success());
        self.drive
            .complete_upload(
                workspace,
                self.owner.0,
                started.upload.id,
                TimestampMillis::now(),
            )
            .await
            .unwrap()
            .id
            .to_string()
    }
}

#[tokio::test]
async fn one_call_uploads_the_image_and_sets_it_on_the_article_after_the_drivers_yes() {
    let fixture = Fixture::new().await;
    let (shop, seen) = fake_shopify().await;
    fixture
        .integration("Shopify", &format!("{shop}/admin/api/2025-10"))
        .await;
    let image = fixture
        .file(fixture.workspace_id, "Hero shot.png", PNG)
        .await;
    let run = fixture.run().await;
    let arguments = json!({
        "integration": "shopify",
        "node_id": image,
        "alt": "A ball in a living room",
        "article_id": "635382366358",
        "place": "image",
    });

    // First the driver's yes: nothing leaves Orbit before it.
    let asked = fixture.call(&run, arguments.clone()).await;
    assert_eq!(asked["status"], "approval_required", "{asked}");
    let action = asked["action"].as_str().unwrap();
    assert!(action.contains("\"Hero shot.png\""), "{action}");
    assert!(
        action.contains("image of article gid://shopify/Article/635382366358"),
        "{action}"
    );
    assert!(seen.lock().unwrap().graphql.is_empty());
    fixture
        .send(&run.conversation, "yes", Some(&run.root))
        .await;

    let done = fixture.call(&run, arguments.clone()).await;
    assert_eq!(done["status"], "ok", "{done}");
    assert_eq!(done["result"]["url"], CDN);
    assert_eq!(
        done["result"]["shopify_file_id"],
        "gid://shopify/MediaImage/1"
    );
    assert_eq!(done["result"]["article"]["placed"], "image");
    // Neither the token nor the signed upload target reaches the agent.
    let text = done.to_string();
    assert!(!text.contains(TOKEN) && !text.contains("staged"), "{text}");

    let resource = format!("{shop}/staged/tmp/1/hero.png");
    let calls = {
        let shop = seen.lock().unwrap();
        let queries: Vec<&str> = shop
            .graphql
            .iter()
            .map(|request| request["query"].as_str().unwrap())
            .collect();
        assert!(queries[0].contains("stagedUploadsCreate"));
        let input = &shop.graphql[0]["variables"]["input"][0];
        assert_eq!(input["resource"], "IMAGE");
        assert_eq!(input["filename"], "Hero-shot.png");
        assert_eq!(input["mimeType"], "image/png");
        assert_eq!(input["httpMethod"], "POST");
        assert_eq!(input["fileSize"], PNG.len().to_string());
        // The file goes to the signed target as multipart, after its fields, without the store token.
        let (upload_token, body) = &shop.uploads[0];
        assert_eq!(*upload_token, None);
        let body = String::from_utf8_lossy(body);
        assert!(
            body.contains("name=\"key\"\r\n\r\ntmp/1/hero.png\r\n"),
            "{body}"
        );
        assert!(body.find("name=\"policy\"") < body.find("name=\"file\""));
        assert!(body.contains("filename=\"Hero-shot.png\"\r\nContent-Type: image/png\r\n\r\n"));
        assert!(body.contains(&*String::from_utf8_lossy(PNG)));
        assert!(queries[1].contains("fileCreate"));
        let created = &shop.graphql[1]["variables"]["files"][0];
        assert_eq!(created["originalSource"], resource);
        assert_eq!(created["contentType"], "IMAGE");
        assert_eq!(created["alt"], "A ball in a living room");
        assert!(queries[2].contains("node("));
        assert!(queries[3].contains("articleUpdate"));
        assert_eq!(
            shop.graphql[3]["variables"]["id"],
            "gid://shopify/Article/635382366358"
        );
        assert_eq!(
            shop.article_image,
            json!({ "url": CDN, "altText": "A ball in a living room" })
        );
        assert_eq!(
            shop.article_body, "<p>Move more.</p>",
            "the body is unchanged"
        );
        assert!(
            shop.tokens
                .iter()
                .all(|token| token.as_deref() == Some(TOKEN))
        );
        shop.graphql.len()
    };

    // One yes, one upload: the same call asks again.
    let again = fixture.call(&run, arguments).await;
    assert_eq!(again["status"], "approval_required", "{again}");
    assert_eq!(seen.lock().unwrap().graphql.len(), calls);

    let audit: String = fixture
        .database
        .scalar(
            "SELECT metadata_json FROM audit_events WHERE action = 'agent_integration.shopify_image'",
        )
        .await
        .unwrap();
    let audit: Value = serde_json::from_str(&audit).unwrap();
    assert_eq!(audit["node_id"], image);
    assert_eq!(audit["shopify_file_id"], "gid://shopify/MediaImage/1");
    assert!(!audit.to_string().contains(TOKEN));
}

#[tokio::test]
async fn place_body_adds_the_image_at_the_end_of_the_article_body() {
    let fixture = Fixture::new().await;
    let (shop, seen) = fake_shopify().await;
    fixture
        .integration("Shopify", &format!("{shop}/admin/api/2025-10/"))
        .await;
    let image = fixture
        .file(
            fixture.workspace_id,
            "ball.jpg",
            b"\xFF\xD8\xFF\xE0 jpeg bytes",
        )
        .await;
    let run = fixture.run().await;
    let arguments = json!({
        "integration": "shopify",
        "node_id": image,
        "alt": "Ball \"blue\" <b>",
        "article_id": "gid://shopify/Article/7",
        "place": "body",
    });
    let asked = fixture.call(&run, arguments.clone()).await;
    assert_eq!(asked["status"], "approval_required", "{asked}");
    assert!(
        asked["action"]
            .as_str()
            .unwrap()
            .contains("end of the body of article gid://shopify/Article/7")
    );
    fixture
        .send(&run.conversation, "Approve", Some(&run.root))
        .await;
    let done = fixture.call(&run, arguments).await;
    assert_eq!(done["status"], "ok", "{done}");
    assert_eq!(done["result"]["article"]["placed"], "body");
    let shop = seen.lock().unwrap();
    assert_eq!(
        shop.graphql[0]["variables"]["input"][0]["mimeType"],
        "image/jpeg"
    );
    assert_eq!(
        shop.article_body,
        format!(
            "<p>Move more.</p>\n<p><img src=\"{}\" alt=\"Ball &quot;blue&quot; &lt;b&gt;\"></p>",
            CDN.replace('&', "&amp;")
        )
    );
    assert_eq!(
        shop.article_image,
        Value::Null,
        "the article's image is unchanged"
    );
}

#[tokio::test]
async fn files_the_driver_cannot_read_and_non_images_are_refused_before_anything_leaves() {
    let fixture = Fixture::new().await;
    let (shop, seen) = fake_shopify().await;
    fixture
        .integration("Shopify", &format!("{shop}/admin/api/2025-10"))
        .await;
    fixture
        .integration("Other API", &format!("{shop}/api/v1"))
        .await;
    let text = fixture
        .file(fixture.workspace_id, "fake.png", b"<svg onload=alert(1)>")
        .await;
    let image = fixture.file(fixture.workspace_id, "hero.png", PNG).await;
    // A workspace the driver is not in: Bea drives, and the owner made the file there.
    let bea = fixture.add_member("bea").await;
    let other = WorkspaceRepository::new((*fixture.database).clone())
        .create(
            fixture.owner.0,
            "Other".into(),
            "test",
            TimestampMillis::now(),
        )
        .await
        .unwrap();
    let foreign = fixture.file(other.id, "secret.png", PNG).await;
    let run = fixture.run_as(&bea).await;
    let base = json!({ "integration": "shopify" });
    let with = |extra: Value| {
        let mut arguments = base.clone();
        arguments
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        arguments
    };

    for (arguments, status, detail) in [
        (
            with(json!({ "node_id": foreign })),
            "failed",
            "not available",
        ),
        (
            with(json!({ "node_id": Id::new_v7().to_string() })),
            "failed",
            "not available",
        ),
        (
            with(json!({ "node_id": text })),
            "refused",
            "JPEG, PNG, GIF and WebP",
        ),
        (
            with(json!({ "node_id": image, "article_id": "7" })),
            "refused",
            "together",
        ),
        (
            with(
                json!({ "node_id": image, "article_id": "gid://shopify/Product/7", "place": "image" }),
            ),
            "refused",
            "article_id",
        ),
        (
            json!({ "integration": "other_api", "node_id": image }),
            "refused",
            "not a Shopify Admin API",
        ),
        (
            json!({ "integration": "nope", "node_id": image }),
            "refused",
            "no integration",
        ),
        (
            with(json!({ "node_id": image, "extra": 1 })),
            "refused",
            "Give integration",
        ),
    ] {
        let answer = fixture.call(&run, arguments.clone()).await;
        assert_eq!(answer["status"], status, "{arguments}: {answer}");
        assert!(
            answer["detail"].as_str().unwrap().contains(detail),
            "{arguments}: {answer}"
        );
    }
    {
        let shop = seen.lock().unwrap();
        assert!(shop.graphql.is_empty() && shop.uploads.is_empty());
    }
    let approvals: i64 = fixture
        .database
        .scalar("SELECT COUNT(*) FROM agent_integration_approvals")
        .await
        .unwrap();
    assert_eq!(approvals, 0, "nobody is asked about a refused call");
}

#[tokio::test]
async fn only_the_drivers_yes_lets_the_image_go() {
    let fixture = Fixture::new().await;
    let (shop, seen) = fake_shopify().await;
    fixture
        .integration("Shopify", &format!("{shop}/admin/api/2025-10"))
        .await;
    let image = fixture.file(fixture.workspace_id, "hero.png", PNG).await;
    let bea = fixture.add_member("bea").await;
    let run = fixture.run().await;
    let arguments = json!({ "integration": "shopify", "node_id": image });
    let asked = fixture.call(&run, arguments.clone()).await;
    assert_eq!(asked["status"], "approval_required", "{asked}");

    // Bea is in the thread, but the owner drives: her yes is no approval.
    fixture
        .send_as(&bea, &run.conversation, "yes", Some(&run.root))
        .await;
    let again = fixture.call(&run, arguments.clone()).await;
    assert_eq!(again["status"], "approval_required", "{again}");
    assert!(seen.lock().unwrap().graphql.is_empty());

    // A person's own session (not the agent's) never reaches the tool route.
    let (status, value) = fixture
        .request(
            "POST",
            &format!(
                "/api/v1/workspaces/{}/agent-tools/shopify-image",
                fixture.workspace_id
            ),
            ("cookie", &fixture.owner.1),
            Some(json!({
                "arguments": arguments,
                "conversation_id": run.conversation,
                "root_message_id": run.root,
            })),
        )
        .await;
    assert!(status.is_client_error(), "{status} {value}");
    assert!(seen.lock().unwrap().graphql.is_empty());

    fixture
        .send(&run.conversation, "yes", Some(&run.root))
        .await;
    let done = fixture.call(&run, arguments).await;
    assert_eq!(done["status"], "ok", "{done}");
    assert_eq!(done["result"]["url"], CDN);
    assert!(done["result"].get("article").is_none(), "{done}");
}

#[tokio::test]
async fn private_dm_files_big_files_and_guests_are_refused() {
    let fixture = Fixture::new().await;
    let (shop, seen) = fake_shopify().await;
    fixture
        .integration("Shopify", &format!("{shop}/admin/api/2025-10"))
        .await;
    let bea = fixture.add_member("bea").await;
    let cid = fixture.add_member("cid").await;
    // The Drive folder of the owner's direct message with Cid: Bea does not see it.
    let (status, dm) = fixture
        .request(
            "POST",
            &format!("/api/v1/workspaces/{}/chat/dms", fixture.workspace_id),
            ("cookie", &fixture.owner.1),
            Some(json!({ "user_ids": [cid.0.to_string()] })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{dm}");
    let dm: Id = dm["result"]["id"].as_str().unwrap().parse().unwrap();
    let folder = fixture
        .drive
        .repository()
        .chat_folder(
            fixture.workspace_id,
            fixture.owner.0,
            dm,
            TimestampMillis::now(),
        )
        .await
        .unwrap();
    let private = fixture
        .file_in(fixture.workspace_id, Some(folder.id), "private.png", PNG)
        .await;
    let mut big = PNG.to_vec();
    big.resize(16 * 1024 * 1024 + 1, 0);
    let big = fixture.file(fixture.workspace_id, "big.png", &big).await;
    let image = fixture.file(fixture.workspace_id, "hero.png", PNG).await;

    let run = fixture.run_as(&bea).await;
    let answer = fixture
        .call(&run, json!({ "integration": "shopify", "node_id": private }))
        .await;
    assert_eq!(answer["status"], "failed", "{answer}");
    assert!(
        answer["detail"].as_str().unwrap().contains("not available"),
        "{answer}"
    );
    let answer = fixture
        .call(&run, json!({ "integration": "shopify", "node_id": big }))
        .await;
    assert_eq!(answer["status"], "refused", "{answer}");
    assert!(
        answer["detail"].as_str().unwrap().contains("16 MiB"),
        "{answer}"
    );

    // Bea becomes a guest while her run goes on: her agent session no longer reaches the tool.
    sqlx::query("UPDATE memberships SET role = 'guest' WHERE user_id = $1")
        .bind(bea.0.to_string())
        .execute(fixture.database.pool())
        .await
        .unwrap();
    let (status, value) = fixture
        .request(
            "POST",
            &format!(
                "/api/v1/workspaces/{}/agent-tools/shopify-image",
                fixture.workspace_id
            ),
            ("authorization", &run.bearer),
            Some(json!({
                "arguments": { "integration": "shopify", "node_id": image },
                "conversation_id": run.conversation,
                "root_message_id": run.root,
            })),
        )
        .await;
    assert!(status.is_client_error(), "{status} {value}");

    let shop = seen.lock().unwrap();
    assert!(shop.graphql.is_empty() && shop.uploads.is_empty());
}
