//! The team agent's `move_to_trash` (`/agent-tools/trash`): Docs pages and tasks go to the Trash as the driver, with
//! the web's soft delete (sub-pages and sub-issues along), only once per "yes" of the driver to the whole list. Items
//! the driver cannot see are refused before anyone is asked; guests and runs the caller does not drive get nothing,
//! and agent sessions cannot reach the direct delete routes. Runs on SQLite, and on Postgres with
//! `ORBIT_TEST_POSTGRES_URL`.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use orbit_platform::{
    AttachmentMutationCoordinator, AuthenticatedUser, Id, LocalBlobStore, PasswordService,
    TestDatabase, TimestampMillis, UploadLimits, UploadService,
};
use orbit_server::agent::{AgentConfig, AgentState};
use orbit_server::auth_routes::CookieMode;
use orbit_server::chat_routes::{ChatState, chat_router};
use orbit_server::page_routes::{PageState, page_router};
use orbit_server::repositories::identity::{IdentityRepository, SetupRequest};
use orbit_server::task_routes::{TaskState, task_router};
use serde_json::{Value, json};
use tower::ServiceExt;

const SECRET: &str = "test-agent-secret-0123456789abcdef0123456789";

struct Fixture {
    database: TestDatabase,
    identity: Arc<IdentityRepository>,
    app: axum::Router,
    workspace_id: Id,
    owner: Member,
    nonce: AtomicU64,
    _root: tempfile::TempDir,
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
        );
        let app = chat_router(chat)
            .merge(task_router(TaskState::new(
                Arc::clone(&identity),
                CookieMode::secure(),
            )))
            .merge(page_router(PageState::new(
                Arc::clone(&identity),
                CookieMode::secure(),
            )))
            .merge(orbit_server::agent::router(agent))
            .layer(axum::middleware::from_fn_with_state(
                (*database).clone(),
                orbit_server::agent::scope::layer,
            ));
        Self {
            database,
            identity,
            app,
            workspace_id: setup.workspace_id,
            owner: Member {
                id: setup.user_id,
                cookie: format!("__Host-orbit_session={}", setup.session.token),
            },
            nonce: AtomicU64::new(0),
            _root: root,
        }
    }

    fn path(&self, rest: &str) -> String {
        format!("/api/v1/workspaces/{}{rest}", self.workspace_id)
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

    /// A write as a person (cookie); its JSON answer.
    async fn as_member(&self, member: &Member, method: &str, rest: &str, body: Value) -> Value {
        let (status, value) = self
            .request(
                method,
                &self.path(rest),
                ("cookie", &member.cookie),
                Some(body),
            )
            .await;
        assert!(status.is_success(), "{method} {rest}: {status} {value}");
        value
    }

    async fn send(&self, member: &Member, conversation: &str, body: &str, root: Option<&str>) {
        let nonce = format!("n{}", self.nonce.fetch_add(1, Ordering::Relaxed));
        self.as_member(
            member,
            "POST",
            &format!("/chat/conversations/{conversation}/messages"),
            json!({ "body": body, "thread_root_id": root, "nonce": nonce }),
        )
        .await;
    }

    /// A thread rooted at a mention of the agent by `driver`, now running.
    async fn run(&self, driver: &Member) -> Run {
        let (status, value) = self.internal("GET", "/inputs", None).await;
        assert_eq!(status, StatusCode::OK, "{value}");
        let bot = value["bot_user_id"].as_str().unwrap().to_owned();
        let (_, conversations) = self
            .request(
                "GET",
                &self.path("/chat/conversations"),
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
        let nonce = format!("n{}", self.nonce.fetch_add(1, Ordering::Relaxed));
        let root = self
            .as_member(
                driver,
                "POST",
                &format!("/chat/conversations/{conversation}/messages"),
                json!({ "body": format!("<@{bot}> clean up"), "nonce": nonce }),
            )
            .await["result"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
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

    async fn trash_call(&self, run: &Run, arguments: Value) -> (StatusCode, Value) {
        self.request(
            "POST",
            &self.path("/agent-tools/trash"),
            ("authorization", &run.bearer),
            Some(json!({
                "tool": "move_to_trash",
                "arguments": arguments,
                "conversation_id": run.conversation,
                "root_message_id": run.root,
            })),
        )
        .await
    }

    async fn trash(&self, run: &Run, arguments: Value) -> Value {
        let (status, value) = self.trash_call(run, arguments).await;
        assert_eq!(status, StatusCode::OK, "{value}");
        value
    }

    async fn page(&self, member: &Member, body: Value) -> String {
        self.as_member(member, "POST", "/pages", body).await["id"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    /// A task in `project` (the first project when `None`), with an optional parent.
    async fn task(
        &self,
        member: &Member,
        project: Option<&str>,
        title: &str,
        parent: Option<&str>,
    ) -> String {
        let project = match project {
            Some(project) => project.to_owned(),
            None => self
                .database
                .scalar::<String>(&format!(
                    "SELECT id FROM projects WHERE workspace_id = '{}' ORDER BY created_at LIMIT 1",
                    self.workspace_id
                ))
                .await
                .unwrap(),
        };
        let status: String = self
            .database
            .scalar(&format!(
                "SELECT id FROM task_statuses WHERE project_id = '{project}' ORDER BY position LIMIT 1"
            ))
            .await
            .unwrap();
        self.as_member(
            member,
            "POST",
            "/tasks",
            json!({ "project_id": project, "status_id": status, "title": title, "parent_task_id": parent }),
        )
        .await["id"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    async fn trashed(&self, table: &str, id: &str) -> bool {
        self.database
            .scalar::<i64>(&format!(
                "SELECT COUNT(*) FROM {table} WHERE id = '{id}' AND deleted_at IS NOT NULL"
            ))
            .await
            .unwrap()
            == 1
    }

    async fn approvals(&self) -> i64 {
        self.database
            .scalar("SELECT COUNT(*) FROM agent_drive_approvals WHERE tool = 'move_to_trash'")
            .await
            .unwrap()
    }
}

#[tokio::test]
async fn one_yes_moves_the_whole_list_with_sub_pages_and_sub_issues_to_the_trash() {
    let fixture = Fixture::new().await;
    let bea = fixture.add_member("bea").await;
    let plan = fixture.page(&bea, json!({ "title": "Old plan" })).await;
    let child = fixture
        .page(&bea, json!({ "title": "Notes", "parent_id": plan }))
        .await;
    let grandchild = fixture
        .page(&bea, json!({ "title": "Draft", "parent_id": child }))
        .await;
    let keep = fixture.page(&bea, json!({ "title": "Keep me" })).await;
    let task = fixture.task(&bea, None, "Old launch", None).await;
    let sub = fixture.task(&bea, None, "Old step", Some(&task)).await;
    let run = fixture.run(&bea).await;
    let arguments = json!({ "page_ids": [plan], "task_ids": [task] });

    // One question with the whole list, the titles and what goes along.
    let asked = fixture.trash(&run, arguments.clone()).await;
    assert_eq!(asked["status"], "approval_required", "{asked}");
    let action = asked["action"].as_str().unwrap();
    assert!(action.contains("these 2 items"), "{action}");
    assert!(
        action.contains("Docs page \"Old plan\"")
            && action.contains("also moves 2 sub-pages to the Trash"),
        "{action}"
    );
    assert!(
        action.contains("Task \"Old launch\"")
            && action.contains("also moves 1 sub-issue to the Trash"),
        "{action}"
    );
    assert!(!fixture.trashed("pages", &plan).await);

    // A "yes" of someone else does not approve; the driver's yes covers only these arguments.
    fixture
        .send(&fixture.owner, &run.conversation, "yes", Some(&run.root))
        .await;
    assert_eq!(
        fixture.trash(&run, arguments.clone()).await["status"],
        "approval_required"
    );
    fixture
        .send(&bea, &run.conversation, "yes", Some(&run.root))
        .await;
    let swapped = fixture.trash(&run, json!({ "page_ids": [keep] })).await;
    assert_eq!(swapped["status"], "approval_required", "{swapped}");
    let done = fixture.trash(&run, arguments.clone()).await;
    assert_eq!(done["status"], "ok", "{done}");
    assert_eq!(done["result"]["trashed"][0]["sub_pages"], 2, "{done}");
    assert_eq!(done["result"]["trashed"][1]["sub_issues"], 1, "{done}");
    for (table, id) in [
        ("pages", &plan),
        ("pages", &child),
        ("pages", &grandchild),
        ("tasks", &task),
        ("tasks", &sub),
    ] {
        assert!(fixture.trashed(table, id).await, "{table} {id}");
    }
    assert!(!fixture.trashed("pages", &keep).await);

    // The web's trash: the page is listed there and restores with its sub-pages.
    let (_, listed) = fixture
        .request(
            "GET",
            &fixture.path("/pages/trash"),
            ("cookie", &bea.cookie),
            None,
        )
        .await;
    assert!(listed.to_string().contains(&plan), "{listed}");
    let version: i64 = fixture
        .database
        .scalar(&format!("SELECT version FROM pages WHERE id = '{plan}'"))
        .await
        .unwrap();
    let (status, value) = fixture
        .request(
            "POST",
            &fixture.path(&format!("/pages/{plan}/restore")),
            ("cookie", &bea.cookie),
            Some(json!({ "expected_version": version })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{value}");
    assert!(!fixture.trashed("pages", &grandchild).await);
    let version: i64 = fixture
        .database
        .scalar(&format!("SELECT version FROM tasks WHERE id = '{task}'"))
        .await
        .unwrap();
    let (status, value) = fixture
        .request(
            "POST",
            &fixture.path(&format!("/tasks/{task}/restore")),
            ("cookie", &bea.cookie),
            Some(json!({ "expected_version": version })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{value}");

    // The yes is used up; who approved is in the audit log, once per item.
    assert_eq!(
        fixture.trash(&run, arguments).await["status"],
        "approval_required"
    );
    let audited: i64 = fixture
        .database
        .scalar(&format!(
            "SELECT COUNT(*) FROM audit_events WHERE action = 'agent_trash.approved_action' \
             AND outcome = 'success' AND actor_id = '{}'",
            bea.id
        ))
        .await
        .unwrap();
    assert_eq!(audited, 2);
}

#[tokio::test]
async fn items_the_driver_cannot_see_are_refused_before_anyone_is_asked() {
    let fixture = Fixture::new().await;
    let bea = fixture.add_member("bea").await;
    let owner = fixture.owner.clone();
    // The owner's private page and a task in a private project without bea.
    let private_page = fixture
        .page(&owner, json!({ "title": "Salaries", "private": true }))
        .await;
    let project = fixture
        .as_member(
            &owner,
            "POST",
            "/projects",
            json!({ "name": "Secret", "key": "SEC", "color": "#336699", "private": true }),
        )
        .await["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let secret_task = fixture
        .task(&owner, Some(&project), "Secret plan", None)
        .await;
    let visible = fixture.task(&bea, None, "Visible", None).await;
    let run = fixture.run(&bea).await;

    for arguments in [
        json!({ "page_ids": [private_page] }),
        json!({ "task_ids": [secret_task] }),
        // One hidden item stops the whole list.
        json!({ "task_ids": [visible, secret_task] }),
        json!({ "task_ids": [Id::new_v7().to_string()] }),
    ] {
        let answer = fixture.trash(&run, arguments.clone()).await;
        assert_eq!(answer["status"], "failed", "{arguments}: {answer}");
        let detail = answer["detail"].as_str().unwrap();
        assert!(
            !detail.contains("Salaries") && !detail.contains("Secret plan"),
            "{detail}"
        );
    }
    assert_eq!(fixture.approvals().await, 0);
    for (table, id) in [
        ("pages", &private_page),
        ("tasks", &secret_task),
        ("tasks", &visible),
    ] {
        assert!(!fixture.trashed(table, id).await);
    }

    // Bad input is refused: nothing, an id twice, another tool.
    for arguments in [
        json!({}),
        json!({ "task_ids": [visible, visible] }),
        json!({ "task_ids": ["x"] }),
    ] {
        assert_eq!(fixture.trash(&run, arguments).await["status"], "refused");
    }
    let (_, other) = fixture
        .request(
            "POST",
            &fixture.path("/agent-tools/trash"),
            ("authorization", &run.bearer),
            Some(json!({ "tool": "purge", "arguments": { "task_ids": [visible] }, "conversation_id": run.conversation, "root_message_id": run.root })),
        )
        .await;
    assert_eq!(other["status"], "refused", "{other}");

    // Agent sessions cannot skip the approval through the direct delete routes, or delete forever.
    for (method, rest) in [
        ("DELETE", format!("/tasks/{visible}?expected_version=0")),
        ("DELETE", format!("/tasks/{visible}/permanent")),
        ("POST", "/tasks/trash/empty".to_owned()),
        (
            "DELETE",
            format!("/pages/{private_page}?expected_version=0"),
        ),
        ("POST", "/pages/trash/empty".to_owned()),
        ("DELETE", format!("/projects/{project}")),
    ] {
        let (status, value) = fixture
            .request(
                method,
                &fixture.path(&rest),
                ("authorization", &run.bearer),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {rest}: {value}");
    }
    assert!(!fixture.trashed("tasks", &visible).await);
}

#[tokio::test]
async fn guests_people_and_runs_the_caller_does_not_drive_get_no_delete() {
    let fixture = Fixture::new().await;
    let bea = fixture.add_member("bea").await;
    let task = fixture.task(&bea, None, "Keep", None).await;
    let run = fixture.run(&bea).await;
    let arguments = json!({ "task_ids": [task] });

    // A person's own session is no agent session.
    let (status, _) = fixture
        .request(
            "POST",
            &fixture.path("/agent-tools/trash"),
            ("cookie", &bea.cookie),
            Some(json!({ "tool": "move_to_trash", "arguments": arguments, "conversation_id": run.conversation, "root_message_id": run.root })),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // A thread that the caller does not drive now (another thread, or no run): refused.
    let (status, _) = fixture
        .request(
            "POST",
            &fixture.path("/agent-tools/trash"),
            ("authorization", &run.bearer),
            Some(json!({ "tool": "move_to_trash", "arguments": arguments, "conversation_id": run.conversation, "root_message_id": Id::new_v7().to_string() })),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // A driver who became a guest: no membership actor, so nothing (not even a question).
    sqlx::query("UPDATE memberships SET role = 'guest' WHERE user_id = $1")
        .bind(bea.id.to_string())
        .execute(fixture.database.pool())
        .await
        .unwrap();
    let (status, value) = fixture.trash_call(&run, arguments).await;
    assert!(
        matches!(status, StatusCode::NOT_FOUND | StatusCode::FORBIDDEN),
        "{status} {value}"
    );
    assert_eq!(fixture.approvals().await, 0);
    assert!(!fixture.trashed("tasks", &task).await);
}
