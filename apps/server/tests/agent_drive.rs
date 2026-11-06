//! The team agent's Drive tools (`/agent-tools/drive`): they act as the driver, the normal Drive routes stay closed to
//! agent sessions, and moving to the trash or making a public link runs only once per "yes" of the driver, for exactly
//! the approved arguments. Runs on SQLite, and on Postgres with `ORBIT_TEST_POSTGRES_URL`.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use orbit_platform::{
    AttachmentMutationCoordinator, AuthenticatedUser, Id, LocalBlobStore, ObjectStorage,
    ObjectStorageState, PasswordService, S3Bucket, TestDatabase, TimestampMillis, UploadLimits,
    UploadService, start_fake_s3,
};
use orbit_server::agent::{AgentConfig, AgentState};
use orbit_server::auth_routes::CookieMode;
use orbit_server::chat_routes::{ChatState, chat_router};
use orbit_server::drive::DriveService;
use orbit_server::drive_routes::{DriveState, drive_router};
use orbit_server::repositories::drive::OnConflict;
use orbit_server::repositories::identity::{IdentityRepository, SetupRequest};
use orbit_server::repositories::workspaces::WorkspaceRepository;
use serde_json::{Value, json};
use tower::ServiceExt;

const SECRET: &str = "test-agent-secret-0123456789abcdef0123456789";

struct Fixture {
    _root: tempfile::TempDir,
    database: TestDatabase,
    identity: Arc<IdentityRepository>,
    drive: DriveService,
    app: axum::Router,
    workspace_id: Id,
    owner: Member,
    nonce: AtomicU64,
}

#[derive(Clone)]
struct Member {
    id: Id,
    cookie: String,
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
        .with_drive(drive.clone(), "https://orbit.test");
        let app = chat_router(chat)
            .merge(orbit_server::agent::router(agent))
            .merge(drive_router(DriveState::new(
                Arc::clone(&identity),
                drive.clone(),
                CookieMode::secure(),
            )))
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
            owner: Member {
                id: setup.user_id,
                cookie: format!("__Host-orbit_session={}", setup.session.token),
            },
            nonce: AtomicU64::new(0),
        }
    }

    async fn add_member(&self, name: &str) -> Member {
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
        Member {
            id,
            cookie: format!("__Host-orbit_session={}", session.token),
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

    async fn send(
        &self,
        member: &Member,
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
                ("cookie", &member.cookie),
                Some(json!({ "body": body, "thread_root_id": root, "nonce": nonce })),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{value}");
        value["result"]["id"].as_str().unwrap().to_owned()
    }

    /// A thread rooted at a mention of the agent by `driver`, now running.
    async fn run(&self, driver: &Member) -> Run {
        let (status, value) = self.internal("GET", "/inputs", None).await;
        assert_eq!(status, StatusCode::OK, "{value}");
        let bot = value["bot_user_id"].as_str().unwrap().to_owned();
        let (_, conversations) = self
            .request(
                "GET",
                &format!(
                    "/api/v1/workspaces/{}/chat/conversations",
                    self.workspace_id
                ),
                ("cookie", &self.owner.cookie),
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
            .send(
                driver,
                &conversation,
                &format!("<@{bot}> tidy the drive"),
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
                    "driver_user_id": driver.id.to_string(),
                    "waiting_for_user_id": null,
                })),
            )
            .await;
        assert_eq!(status, StatusCode::NO_CONTENT, "{value}");
        let (status, session) = self
            .internal(
                "POST",
                "/sessions",
                Some(json!({ "user_id": driver.id.to_string(), "workspace_id": self.workspace_id.to_string() })),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{session}");
        Run {
            conversation,
            root,
            bearer: format!("Bearer {}", session["token"].as_str().unwrap()),
        }
    }

    async fn drive(&self, run: &Run, tool: &str, arguments: Value) -> Value {
        let (status, value) = self
            .request(
                "POST",
                &format!("/api/v1/workspaces/{}/agent-tools/drive", self.workspace_id),
                ("authorization", &run.bearer),
                Some(json!({
                    "tool": tool,
                    "arguments": arguments,
                    "conversation_id": run.conversation,
                    "root_message_id": run.root,
                })),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{value}");
        value
    }

    async fn file(&self, name: &str) -> String {
        self.drive
            .put_text(
                self.workspace_id,
                self.owner.id,
                None,
                name.to_owned(),
                "text/plain".to_owned(),
                "Quarterly numbers".to_owned(),
                OnConflict::Fail,
                TimestampMillis::now(),
            )
            .await
            .unwrap()
            .id
            .to_string()
    }

    async fn trashed(&self, node: &str) -> bool {
        self.database
            .scalar::<i64>(&format!(
                "SELECT COUNT(*) FROM drive_nodes WHERE id = '{node}' AND trashed_at IS NOT NULL"
            ))
            .await
            .unwrap()
            == 1
    }
}

#[tokio::test]
async fn trash_runs_once_per_yes_of_the_driver_for_exactly_the_approved_arguments() {
    let fixture = Fixture::new().await;
    let bea = fixture.add_member("bea").await;
    let report = fixture.file("report.txt").await;
    let other = fixture.file("other.txt").await;
    let run = fixture.run(&bea).await;

    // Reading needs no approval.
    let listed = fixture
        .drive(&run, "drive_search", json!({ "query": "report" }))
        .await;
    assert_eq!(listed["status"], "ok", "{listed}");
    assert_eq!(listed["result"]["items"][0]["id"], report.as_str());

    let asked = fixture
        .drive(&run, "drive_trash", json!({ "node_id": report }))
        .await;
    assert_eq!(asked["status"], "approval_required", "{asked}");
    assert!(
        asked["action"].as_str().unwrap().contains("\"report.txt\""),
        "{asked}"
    );
    let approval = asked["approval_id"].clone();
    assert!(!fixture.trashed(&report).await);

    // No answer, a "no", and a "yes" of someone else do not approve.
    let again = fixture
        .drive(&run, "drive_trash", json!({ "node_id": report }))
        .await;
    assert_eq!(
        again["approval_id"], approval,
        "the open approval is asked again"
    );
    fixture
        .send(&bea, &run.conversation, "no, keep it", Some(&run.root))
        .await;
    fixture
        .send(&fixture.owner, &run.conversation, "yes", Some(&run.root))
        .await;
    let refused = fixture
        .drive(&run, "drive_trash", json!({ "node_id": report }))
        .await;
    assert_eq!(refused["status"], "approval_required", "{refused}");
    assert!(!fixture.trashed(&report).await);

    // The driver's yes allows only the approved arguments.
    fixture
        .send(&bea, &run.conversation, "Yes", Some(&run.root))
        .await;
    let swapped = fixture
        .drive(&run, "drive_trash", json!({ "node_id": other }))
        .await;
    assert_eq!(swapped["status"], "approval_required", "{swapped}");
    assert!(!fixture.trashed(&other).await);
    let done = fixture
        .drive(&run, "drive_trash", json!({ "node_id": report }))
        .await;
    assert_eq!(done["status"], "ok", "{done}");
    assert!(fixture.trashed(&report).await);

    // Who approved it is in the audit log.
    let audit: String = fixture
        .database
        .scalar(&format!(
            "SELECT metadata_json FROM audit_events WHERE action = 'agent_drive.approved_action' AND resource_id = '{report}'"
        ))
        .await
        .unwrap();
    let audit: Value = serde_json::from_str(&audit).unwrap();
    assert_eq!(audit["approved_by"], bea.id.to_string());
    assert_eq!(audit["approval_id"], approval);
}

#[tokio::test]
async fn a_public_link_needs_its_own_yes_each_time() {
    let fixture = Fixture::new().await;
    let report = fixture.file("report.txt").await;
    let owner = fixture.owner.clone();
    let run = fixture.run(&owner).await;
    let arguments = json!({ "node_id": report, "expires_in_days": 7 });

    let asked = fixture.drive(&run, "drive_share", arguments.clone()).await;
    assert_eq!(asked["status"], "approval_required", "{asked}");
    assert!(
        asked["action"].as_str().unwrap().contains("for 7 days"),
        "{asked}"
    );
    fixture
        .send(&owner, &run.conversation, "yes", Some(&run.root))
        .await;
    let shared = fixture.drive(&run, "drive_share", arguments.clone()).await;
    assert_eq!(shared["status"], "ok", "{shared}");
    assert!(
        shared["result"]["url"]
            .as_str()
            .unwrap()
            .starts_with("https://orbit.test/s/")
    );

    // The yes is used up: a second link needs a second approval.
    let second = fixture.drive(&run, "drive_share", arguments).await;
    assert_eq!(second["status"], "approval_required", "{second}");
    assert_ne!(second["approval_id"], asked["approval_id"]);
    let links: i64 = fixture
        .database
        .scalar("SELECT COUNT(*) FROM drive_shares")
        .await
        .unwrap();
    assert_eq!(links, 1);
}

#[tokio::test]
async fn the_agent_cannot_skip_the_approval_or_reach_another_workspace() {
    let fixture = Fixture::new().await;
    let bea = fixture.add_member("bea").await;
    let report = fixture.file("report.txt").await;
    let run = fixture.run(&bea).await;

    // The normal Drive routes are closed to agent sessions.
    for (method, path) in [
        ("POST", format!("/drive/nodes/{report}/trash")),
        ("DELETE", format!("/drive/nodes/{report}")),
        ("GET", "/drive/nodes".to_owned()),
    ] {
        let (status, value) = fixture
            .request(
                method,
                &format!("/api/v1/workspaces/{}{path}", fixture.workspace_id),
                ("authorization", &run.bearer),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {path}: {value}");
    }
    // A text file that would replace (and trash) another is refused.
    let replace = fixture
        .drive(
            &run,
            "drive_upload_text",
            json!({ "name": "report.txt", "text": "x", "on_conflict": "replace" }),
        )
        .await;
    assert_eq!(replace["status"], "refused", "{replace}");
    assert!(!fixture.trashed(&report).await);
    // Tools outside the list do not exist.
    let purge = fixture
        .drive(&run, "drive_purge", json!({ "node_id": report }))
        .await;
    assert_eq!(purge["status"], "refused", "{purge}");

    // An item of a workspace the driver is not in: refused before anyone is asked.
    let other = WorkspaceRepository::new((*fixture.database).clone())
        .create(
            fixture.owner.id,
            "Other".into(),
            "test",
            TimestampMillis::now(),
        )
        .await
        .unwrap();
    let foreign = fixture
        .drive
        .put_text(
            other.id,
            fixture.owner.id,
            None,
            "secret.txt".to_owned(),
            "text/plain".to_owned(),
            "secret".to_owned(),
            OnConflict::Fail,
            TimestampMillis::now(),
        )
        .await
        .unwrap()
        .id
        .to_string();
    for (tool, arguments) in [
        ("drive_read", json!({ "node_id": foreign, "limit": 100 })),
        ("drive_trash", json!({ "node_id": foreign })),
        ("drive_share", json!({ "node_id": foreign })),
    ] {
        let answer = fixture.drive(&run, tool, arguments).await;
        assert_eq!(answer["status"], "failed", "{tool}: {answer}");
    }
    let approvals: i64 = fixture
        .database
        .scalar("SELECT COUNT(*) FROM agent_drive_approvals")
        .await
        .unwrap();
    assert_eq!(approvals, 0);

    // Without a running thread of the caller, nothing runs.
    let (status, _) = fixture
        .request(
            "POST",
            &format!(
                "/api/v1/workspaces/{}/agent-tools/drive",
                fixture.workspace_id
            ),
            ("authorization", &run.bearer),
            Some(json!({
                "tool": "drive_list",
                "conversation_id": run.conversation,
                "root_message_id": Id::new_v7().to_string(),
            })),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}
