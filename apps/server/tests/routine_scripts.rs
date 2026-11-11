//! Saved scripts, script approval, test runs and run history (step 12): a routine's script writes only
//! after its creator's yes to that exact code, a changed script runs without writes, a test run posts only
//! in the creator's DM and Orbit refuses every write of its agent sessions, and each run is kept with the
//! sidecar's report. Runs on SQLite, and on Postgres with `ORBIT_TEST_POSTGRES_URL`.
#![allow(dead_code)]
use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use orbit_platform::{
    AttachmentMutationCoordinator, AuthenticatedUser, Id, LocalBlobStore, PasswordService, Row,
    TestDatabase, TimestampMillis, UploadLimits, UploadService,
};
use orbit_server::agent::{AgentConfig, AgentState};
use orbit_server::auth_routes::CookieMode;
use orbit_server::chat_routes::{ChatState, chat_router};
use orbit_server::repositories::identity::{IdentityRepository, SetupRequest};
use orbit_server::scheduled::run_due;
use serde_json::{Value, json};
use tower::ServiceExt;

const SECRET: &str = "test-agent-secret-0123456789abcdef0123456789";
const HOUR: i64 = 60 * 60 * 1000;

struct Fixture {
    _root: tempfile::TempDir,
    database: TestDatabase,
    identity: Arc<IdentityRepository>,
    agent: AgentState,
    app: axum::Router,
    workspace_id: String,
    owner: Member,
}

#[derive(Clone)]
struct Member {
    id: Id,
    cookie: String,
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
            .merge(orbit_server::agent::router(agent.clone()))
            .layer(axum::middleware::from_fn_with_state(
                (*database).clone(),
                orbit_server::agent::scope::layer,
            ));
        let fixture = Self {
            _root: root,
            database,
            identity,
            agent,
            app,
            workspace_id: setup.workspace_id.to_string(),
            owner: Member {
                id: setup.user_id,
                cookie: format!("__Host-orbit_session={}", setup.session.token),
            },
        };
        // Makes the bot and puts it in the workspace.
        let (status, value) = fixture
            .request(
                "GET",
                "/api/internal/agent/inputs",
                ("authorization", &format!("Bearer {SECRET}")),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{value}");
        fixture
    }

    async fn add_person(&self, name: &str, role: &str) -> Member {
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
             VALUES ($1, $2, $3, $4, 0, $5, $5)",
        )
        .bind(Id::new_v7().to_string())
        .bind(&self.workspace_id)
        .bind(id.to_string())
        .bind(role)
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

    /// `path` below the workspace, as `member`.
    async fn api(
        &self,
        member: &Member,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let uri = format!("/api/v1/workspaces/{}{path}", self.workspace_id);
        self.request(method, &uri, ("cookie", &member.cookie), body)
            .await
    }

    async fn ok(&self, member: &Member, method: &str, path: &str, body: Option<Value>) -> Value {
        let (status, value) = self.api(member, method, path, body).await;
        assert!(status.is_success(), "{method} {path}: {status} {value}");
        value
    }

    async fn general(&self) -> String {
        self.ok(&self.owner, "GET", "/chat/conversations", None)
            .await
            .as_array()
            .unwrap()
            .iter()
            .find(|conversation| conversation["is_default"] == true)
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    async fn private_channel(&self, creator: &Member, name: &str, members: &[&Member]) -> String {
        let ids: Vec<String> = members.iter().map(|member| member.id.to_string()).collect();
        self.ok(
            creator,
            "POST",
            "/chat/conversations",
            Some(json!({ "name": name, "kind": "private", "member_ids": ids })),
        )
        .await["result"]["id"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    async fn routine(&self, member: &Member, body: Value) -> (StatusCode, Value) {
        self.api(member, "POST", "/routines", Some(body)).await
    }

    async fn connect_chatgpt(&self, member: &Member) {
        let now = TimestampMillis::now().as_millis();
        sqlx::query(
            "INSERT INTO agent_credentials (user_id, sealed, account_id, created_at, updated_at) \
             VALUES ($1, $2, NULL, $3, $3)",
        )
        .bind(member.id.to_string())
        .bind(vec![1_u8, 2, 3])
        .bind(now)
        .execute(self.database.pool())
        .await
        .unwrap();
    }

    async fn set_due(&self, routine: &str, at: i64) {
        sqlx::query("UPDATE agent_routines SET next_run_at = $1 WHERE id = $2")
            .bind(at)
            .bind(routine)
            .execute(self.database.pool())
            .await
            .unwrap();
    }

    async fn routine_row(
        &self,
        routine: &str,
    ) -> (
        String,
        Option<String>,
        Option<i64>,
        Option<String>,
        Option<String>,
    ) {
        let row = sqlx::query(
            "SELECT status, paused_reason, next_run_at, last_run_status, pending_run_id FROM agent_routines WHERE id = $1",
        )
        .bind(routine)
        .fetch_one(self.database.pool())
        .await
        .unwrap();
        (
            row.get("status"),
            row.get("paused_reason"),
            row.get("next_run_at"),
            row.get("last_run_status"),
            row.get("pending_run_id"),
        )
    }

    async fn count(&self, sql: &'static str, value: &str) -> i64 {
        sqlx::query_scalar(sql)
            .bind(value)
            .fetch_one(self.database.pool())
            .await
            .unwrap()
    }

    async fn inputs_of(&self, routine: &str) -> i64 {
        self.count(
            "SELECT COUNT(*) FROM agent_inputs WHERE routine_id = $1",
            routine,
        )
        .await
    }
}

fn now() -> i64 {
    TimestampMillis::now().as_millis()
}

fn at(ms: i64) -> TimestampMillis {
    TimestampMillis::from_millis(ms)
}

impl Fixture {
    /// An agent session of the owner, driving a running thread in #general (no thread root).
    async fn agent_run(&self) -> (String, String) {
        self.connect_chatgpt(&self.owner).await;
        let general = self.general().await;
        // Orbit makes agent sessions only for the driver of a running thread.
        let (status, value) = self
            .request(
                "PUT",
                &format!("/api/internal/agent/threads/{general}"),
                ("authorization", &format!("Bearer {SECRET}")),
                Some(json!({
                    "workspace_id": self.workspace_id, "conversation_id": general, "root_message_id": null,
                    "status": "running", "driver_user_id": self.owner.id.to_string(), "waiting_for_user_id": null,
                })),
            )
            .await;
        assert_eq!(status, StatusCode::NO_CONTENT, "{value}");
        let (status, value) = self
            .request(
                "POST",
                "/api/internal/agent/sessions",
                ("authorization", &format!("Bearer {SECRET}")),
                Some(json!({ "user_id": self.owner.id.to_string(), "workspace_id": self.workspace_id })),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{value}");
        (
            format!("Bearer {}", value["token"].as_str().unwrap()),
            general,
        )
    }

    /// The agent's call of a scheduling tool in the run's thread.
    async fn agent_tool(&self, bearer: &str, thread: &str, tool: &str, arguments: Value) -> Value {
        let (status, value) = self
            .request(
                "POST",
                &format!("/api/v1/workspaces/{}/agent-tools/scheduled", self.workspace_id),
                ("authorization", bearer),
                Some(json!({ "tool": tool, "arguments": arguments, "conversation_id": thread, "root_message_id": null })),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{tool}: {value}");
        value
    }

    async fn say(&self, member: &Member, conversation: &str, body: &str) {
        self.ok(
            member,
            "POST",
            &format!("/chat/conversations/{conversation}/messages"),
            Some(json!({ "body": body, "nonce": Id::new_v7().to_string() })),
        )
        .await;
    }
}

fn chrono_like(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .unwrap()
        .to_rfc3339()
}

impl Fixture {
    /// The sidecar's view of a run's thread: running, driven by `driver`. Then an agent session for it,
    /// with `thread_key` when given. Returns the bearer and whether Orbit made it a dry run.
    async fn thread_session(
        &self,
        driver: &Member,
        (thread_key, conversation, root): (&str, &str, Option<&str>),
        with_key: bool,
    ) -> (String, bool) {
        let (status, value) = self
            .request(
                "PUT",
                &format!("/api/internal/agent/threads/{thread_key}"),
                ("authorization", &format!("Bearer {SECRET}")),
                Some(json!({
                    "workspace_id": self.workspace_id, "conversation_id": conversation, "root_message_id": root,
                    "status": "running", "driver_user_id": driver.id.to_string(), "waiting_for_user_id": null,
                })),
            )
            .await;
        assert_eq!(status, StatusCode::NO_CONTENT, "{value}");
        let mut body =
            json!({ "user_id": driver.id.to_string(), "workspace_id": self.workspace_id });
        if with_key {
            body["thread_key"] = json!(thread_key);
        }
        let (status, value) = self
            .request(
                "POST",
                "/api/internal/agent/sessions",
                ("authorization", &format!("Bearer {SECRET}")),
                Some(body),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{value}");
        (
            format!("Bearer {}", value["token"].as_str().unwrap()),
            value["dry_run"].as_bool().unwrap(),
        )
    }

    /// A tool call of the agent in a thread with a root message.
    async fn agent_tool_in(
        &self,
        bearer: &str,
        (conversation, root): (&str, &str),
        tool: &str,
        arguments: Value,
    ) -> Value {
        let (status, value) = self
            .request(
                "POST",
                &format!("/api/v1/workspaces/{}/agent-tools/scheduled", self.workspace_id),
                ("authorization", bearer),
                Some(json!({ "tool": tool, "arguments": arguments, "conversation_id": conversation, "root_message_id": root })),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{tool}: {value}");
        value
    }

    /// The newest scheduled (not test) run of a routine: id, writes, thread key, conversation, root.
    async fn newest_run(
        &self,
        routine: &str,
        test: bool,
    ) -> (String, String, String, String, String) {
        let row = sqlx::query(
            "SELECT id, writes, thread_key, conversation_id, message_id FROM agent_routine_runs \
             WHERE routine_id = $1 AND test = $2 ORDER BY started_at DESC, id DESC LIMIT 1",
        )
        .bind(routine)
        .bind(i64::from(test))
        .fetch_one(self.database.pool())
        .await
        .unwrap();
        (
            row.get("id"),
            row.get("writes"),
            row.get("thread_key"),
            row.get("conversation_id"),
            row.get("message_id"),
        )
    }

    async fn post_message(&self, bearer: &str, conversation: &str) -> (StatusCode, Value) {
        self.request(
            "POST",
            &format!(
                "/api/v1/workspaces/{}/chat/conversations/{conversation}/messages",
                self.workspace_id
            ),
            ("authorization", bearer),
            Some(json!({ "body": "hello", "nonce": Id::new_v7().to_string() })),
        )
        .await
    }

    async fn finish(&self, run: &str, report: Value) -> StatusCode {
        self.request(
            "PUT",
            &format!("/api/internal/agent/routine-runs/{run}"),
            ("authorization", &format!("Bearer {SECRET}")),
            Some(report),
        )
        .await
        .0
    }
}

#[tokio::test]
async fn a_script_routine_writes_only_after_a_yes_to_its_exact_code() {
    let fixture = Fixture::new().await;
    let (bearer, general) = fixture.agent_run().await;
    let saved = fixture
        .agent_tool(
            &bearer,
            &general,
            "save_script",
            json!({ "name": "Digest", "code": "text(1)", "description": "Counts" }),
        )
        .await;
    assert_eq!(saved["status"], "ok", "{saved}");
    let script = saved["result"]["id"].as_str().unwrap().to_owned();
    let first_hash = saved["result"]["code_hash"].as_str().unwrap().to_owned();

    // The yes to the routine is a yes to the script's code, which the question shows.
    let args = json!({ "name": "Daily", "prompt": "Post the digest.", "schedule": "0 9 * * 1-5", "conversation_id": general, "script_id": script });
    let asked = fixture
        .agent_tool(&bearer, &general, "schedule_routine", args.clone())
        .await;
    assert_eq!(asked["status"], "approval_required", "{asked}");
    let action = asked["action"].as_str().unwrap();
    assert!(
        action.contains("\"Digest\"") && action.contains("text(1)"),
        "{action}"
    );
    fixture.say(&fixture.owner, &general, "yes").await;
    let made = fixture
        .agent_tool(&bearer, &general, "schedule_routine", args)
        .await;
    assert_eq!(made["status"], "ok", "{made}");
    assert_eq!(made["result"]["script_approved"], true);
    let routine = made["result"]["id"].as_str().unwrap().to_owned();

    // A run of the approved script: its approval-gated writes need no new yes, only while it runs,
    // and only for the routine's creator.
    let start = now();
    fixture.set_due(&routine, start - 1_000).await;
    run_due(&fixture.agent, at(start)).await.unwrap();
    let (run, writes, _, conversation, root) = fixture.newest_run(&routine, false).await;
    assert_eq!(writes, "script");
    let database = (*fixture.database).clone();
    let granted = |user: Id| {
        let (conversation, root) = (conversation.clone(), root.clone());
        let database = database.clone();
        async move {
            orbit_server::scheduled::runs::approved_script_run(
                &database,
                &conversation,
                Some(&root),
                user,
            )
            .await
            .unwrap()
        }
    };
    assert_eq!(
        granted(fixture.owner.id).await.as_deref(),
        Some(run.as_str())
    );
    let other = fixture.add_person("other", "member").await;
    assert_eq!(granted(other.id).await, None);
    assert_eq!(
        fixture
            .finish(
                &run,
                json!({ "status": "done", "output": "1", "tool_calls": [], "state": { "n": 1 } })
            )
            .await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        granted(fixture.owner.id).await,
        None,
        "a finished run grants nothing"
    );
    // The script's state is kept for the next run.
    let state: Option<String> =
        sqlx::query_scalar("SELECT script_state FROM agent_routines WHERE id = $1")
            .bind(&routine)
            .fetch_one(fixture.database.pool())
            .await
            .unwrap();
    assert_eq!(state.as_deref(), Some(r#"{"n":1}"#));

    // New code (the same name, any case): the routine runs it without writes until a new yes.
    let changed = fixture
        .agent_tool(
            &bearer,
            &general,
            "save_script",
            json!({ "name": "digest", "code": "text(2)" }),
        )
        .await;
    assert_eq!(changed["result"]["id"], script.as_str());
    assert_eq!(changed["result"]["routines"][0]["approved"], false);
    assert_eq!(changed["result"]["description"], "Counts");
    let second_hash = changed["result"]["code_hash"].as_str().unwrap().to_owned();
    assert_ne!(first_hash, second_hash);
    fixture.set_due(&routine, start + 60_000).await;
    run_due(&fixture.agent, at(start + 61_000)).await.unwrap();
    let (_, writes, thread_key, conversation, root) = fixture.newest_run(&routine, false).await;
    assert_eq!(writes, "dry");
    // Its agent sessions are dry runs: Orbit refuses their writes.
    let (dry, is_dry) = fixture
        .thread_session(
            &fixture.owner,
            (&thread_key, &conversation, Some(&root)),
            true,
        )
        .await;
    assert!(is_dry);
    let (status, refused) = fixture.post_message(&dry, &general).await;
    assert_eq!(status, StatusCode::CONFLICT, "{refused}");
    assert_eq!(refused["code"], "agent_dry_run");

    // The approval is for exactly the code the creator looked at.
    let path = format!("/routines/{routine}");
    let (status, _) = fixture
        .api(
            &fixture.owner,
            "PATCH",
            &path,
            Some(json!({ "approve_script_hash": first_hash })),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let approved = fixture
        .ok(
            &fixture.owner,
            "PATCH",
            &path,
            Some(json!({ "approve_script_hash": second_hash })),
        )
        .await;
    assert_eq!(approved["script_approved"], true);
    // An edit of the code in Settings needs a new yes too.
    let edited = fixture
        .ok(
            &fixture.owner,
            "PATCH",
            &format!("/scripts/{script}"),
            Some(json!({ "code": "text(3)" })),
        )
        .await;
    assert_eq!(edited["routines"][0]["approved"], false);
    // Only the owner changes a script; a member does not even see it.
    let (status, _) = fixture
        .api(
            &other,
            "PATCH",
            &format!("/scripts/{script}"),
            Some(json!({ "code": "x" })),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // Deleting it takes the routine back to its instruction.
    let (status, _) = fixture
        .api(
            &fixture.owner,
            "DELETE",
            &format!("/scripts/{script}"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let routines = fixture.ok(&fixture.owner, "GET", "/routines", None).await;
    assert_eq!(routines["routines"][0]["script_id"], Value::Null);
    assert_eq!(routines["routines"][0]["script_approved"], false);
}

#[tokio::test]
async fn a_test_run_posts_only_in_the_creators_dm_and_writes_nothing() {
    let fixture = Fixture::new().await;
    fixture.connect_chatgpt(&fixture.owner).await;
    let general = fixture.general().await;
    let routine = fixture
        .routine(
            &fixture.owner,
            json!({ "name": "Standup", "prompt": "create task \"x\"", "schedule": "0 9 * * 1-5", "conversation_id": general }),
        )
        .await
        .1["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let in_general = || {
        fixture.count(
            "SELECT COUNT(*) FROM chat_messages WHERE conversation_id = $1",
            &general,
        )
    };
    let before = in_general().await;

    let run = fixture
        .ok(
            &fixture.owner,
            "POST",
            &format!("/routines/{routine}/test"),
            None,
        )
        .await;
    assert_eq!(run["test"], true);
    assert_eq!(run["writes"], "dry");
    assert_eq!(run["status"], "started");
    // Its message is in the creator's DM with the agent, marked as a test run; #general got nothing.
    let dm = run["conversation_id"].as_str().unwrap().to_owned();
    let root = run["message_id"].as_str().unwrap().to_owned();
    let kind: String = sqlx::query_scalar("SELECT kind FROM chat_conversations WHERE id = $1")
        .bind(&dm)
        .fetch_one(fixture.database.pool())
        .await
        .unwrap();
    assert_eq!(kind, "dm");
    let members: Vec<String> =
        sqlx::query_scalar("SELECT user_id FROM chat_members WHERE conversation_id = $1")
            .bind(&dm)
            .fetch_all(fixture.database.pool())
            .await
            .unwrap();
    assert_eq!(members.len(), 2);
    assert!(members.contains(&fixture.owner.id.to_string()));
    let body: String = sqlx::query_scalar("SELECT body FROM chat_messages WHERE id = $1")
        .bind(&root)
        .fetch_one(fixture.database.pool())
        .await
        .unwrap();
    assert!(body.starts_with("🧪 Test run"), "{body}");
    assert_eq!(in_general().await, before);
    // The agent gets it as the creator's run of the routine, in that DM thread.
    let input = sqlx::query(
        "SELECT author_id, routine_id, root_message_id, conversation_id FROM agent_inputs WHERE message_id = $1",
    )
    .bind(run["id"].as_str().unwrap())
    .fetch_one(fixture.database.pool())
    .await
    .unwrap();
    assert_eq!(
        input.get::<String, _>("author_id"),
        fixture.owner.id.to_string()
    );
    assert_eq!(input.get::<String, _>("routine_id"), routine);
    assert_eq!(input.get::<String, _>("root_message_id"), root);
    assert_eq!(input.get::<String, _>("conversation_id"), dm);
    // The sidecar reads it as a dry test run of the routine, with nothing to run but the instruction.
    let (status, queue) = fixture
        .request(
            "GET",
            "/api/internal/agent/inputs",
            ("authorization", &format!("Bearer {SECRET}")),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{queue}");
    let queued = queue["inputs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|input| input["message_id"] == run["id"])
        .unwrap();
    assert_eq!(queued["routine"]["test"], true);
    assert_eq!(queued["routine"]["writes"], "dry");
    assert_eq!(queued["routine"]["script"], Value::Null);
    assert_eq!(queued["body"], "create task \"x\"");
    // One test run at a time.
    let (status, _) = fixture
        .api(
            &fixture.owner,
            "POST",
            &format!("/routines/{routine}/test"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);

    // Orbit refuses every write of the run's agent sessions; reads work.
    let thread_key =
        sqlx::query_scalar::<_, String>("SELECT thread_key FROM agent_routine_runs WHERE id = $1")
            .bind(run["id"].as_str().unwrap())
            .fetch_one(fixture.database.pool())
            .await
            .unwrap();
    let (dry, is_dry) = fixture
        .thread_session(&fixture.owner, (&thread_key, &dm, Some(&root)), true)
        .await;
    assert!(is_dry);
    for conversation in [&dm, &general] {
        let (status, refused) = fixture.post_message(&dry, conversation).await;
        assert_eq!(status, StatusCode::CONFLICT, "{refused}");
        assert_eq!(refused["code"], "agent_dry_run");
    }
    let (status, _) = fixture
        .request(
            "GET",
            &format!(
                "/api/v1/workspaces/{}/chat/conversations",
                fixture.workspace_id
            ),
            ("authorization", &dry),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    // The agent tool route says what it would do and makes nothing.
    for (tool, arguments) in [
        (
            "remind_me",
            json!({ "body": "Call Ana", "send_at": chrono_like(now() + HOUR) }),
        ),
        ("save_script", json!({ "name": "x", "code": "text(1)" })),
        ("test_routine", json!({ "routine_id": routine })),
    ] {
        let answer = fixture
            .agent_tool_in(&dry, (&dm, &root), tool, arguments)
            .await;
        assert_eq!(answer["status"], "dry_run", "{tool}: {answer}");
        assert!(answer["action"].as_str().unwrap().starts_with(tool));
    }
    assert_eq!(
        fixture
            .count(
                "SELECT COUNT(*) FROM scheduled_messages WHERE author_id = $1",
                &fixture.owner.id.to_string()
            )
            .await,
        0
    );
    assert_eq!(
        fixture
            .count(
                "SELECT COUNT(*) FROM agent_scripts WHERE owner_id = $1",
                &fixture.owner.id.to_string()
            )
            .await,
        0
    );
    // A session without its thread (an older sidecar) is a dry run while the person drives one.
    let (_, is_dry) = fixture
        .thread_session(&fixture.owner, (&thread_key, &dm, Some(&root)), false)
        .await;
    assert!(is_dry);
    // The person's other threads write as before.
    let (normal, is_dry) = fixture
        .thread_session(&fixture.owner, (&general, &general, None), true)
        .await;
    assert!(!is_dry);
    let (status, value) = fixture.post_message(&normal, &general).await;
    assert!(status.is_success(), "{status} {value}");

    // Only the creator tests a routine: it runs as them.
    let member = fixture.add_person("member", "member").await;
    let theirs = fixture
        .routine(
            &member,
            json!({ "name": "Mine", "prompt": "list projects", "schedule": "0 9 * * 1-5" }),
        )
        .await
        .1["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let (status, _) = fixture
        .api(
            &fixture.owner,
            "POST",
            &format!("/routines/{theirs}/test"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = fixture
        .api(&member, "POST", &format!("/routines/{routine}/test"), None)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // Without ChatGPT there is nothing to test with.
    let (status, value) = fixture
        .api(&member, "POST", &format!("/routines/{theirs}/test"), None)
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{value}");

    // Guests have no scripts, test runs or run history.
    let guest = fixture.add_person("guest", "guest").await;
    for (method, path) in [
        ("GET", "/scripts".to_owned()),
        ("POST", format!("/routines/{routine}/test")),
        ("GET", format!("/routines/{routine}/runs")),
    ] {
        let (status, _) = fixture.api(&guest, method, &path, None).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {path}");
    }
}

#[tokio::test]
async fn run_history_keeps_the_last_runs_and_the_sidecars_report() {
    let fixture = Fixture::new().await;
    fixture.connect_chatgpt(&fixture.owner).await;
    let general = fixture.general().await;
    let routine = fixture
        .routine(
            &fixture.owner,
            json!({ "name": "Often", "prompt": "list projects", "schedule": "*/15 * * * *", "conversation_id": general }),
        )
        .await
        .1["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let start = now();
    for index in 0..22 {
        let due = start + index * 60_000;
        fixture.set_due(&routine, due - 1).await;
        run_due(&fixture.agent, at(due)).await.unwrap();
    }
    let runs = fixture
        .ok(
            &fixture.owner,
            "GET",
            &format!("/routines/{routine}/runs"),
            None,
        )
        .await;
    let runs = runs["runs"].as_array().unwrap().clone();
    assert_eq!(runs.len(), 20, "only the newest 20 stay");
    assert!(runs[0]["started_at"].as_i64() > runs[19]["started_at"].as_i64());
    assert_eq!(runs[0]["writes"], "normal");
    assert_eq!(runs[0]["test"], false);
    let newest = runs[0]["id"].as_str().unwrap().to_owned();

    let report = json!({
        "status": "done",
        "output": "x".repeat(5000),
        "tool_calls": [
            { "name": "list_projects", "args": "{}", "status": "ok" },
            { "name": "create_task", "args": "{\"title\":\"x\"}", "status": "dry_run", "would_do": "create_task {\"title\":\"x\"}" },
        ],
        "state": { "n": 1 },
    });
    assert_eq!(
        fixture.finish(&newest, report).await,
        StatusCode::NO_CONTENT
    );
    // A second report changes nothing.
    assert_eq!(
        fixture
            .finish(&newest, json!({ "status": "failed", "error": "late" }))
            .await,
        StatusCode::NO_CONTENT
    );
    let runs = fixture
        .ok(
            &fixture.owner,
            "GET",
            &format!("/routines/{routine}/runs"),
            None,
        )
        .await;
    let run = &runs["runs"][0];
    assert_eq!(run["status"], "done");
    assert_eq!(run["error"], Value::Null);
    assert_eq!(run["output"].as_str().unwrap().chars().count(), 4000);
    assert_eq!(run["tool_calls"][1]["status"], "dry_run");
    assert_eq!(
        run["tool_calls"][1]["would_do"],
        "create_task {\"title\":\"x\"}"
    );
    assert!(run["finished_at"].as_i64().is_some());
    // A run without a script keeps no state.
    let state: Option<String> =
        sqlx::query_scalar("SELECT script_state FROM agent_routines WHERE id = $1")
            .bind(&routine)
            .fetch_one(fixture.database.pool())
            .await
            .unwrap();
    assert_eq!(state, None);
    // Only the sidecar reports.
    let (status, _) = fixture
        .request(
            "PUT",
            &format!("/api/internal/agent/routine-runs/{newest}"),
            ("cookie", &fixture.owner.cookie),
            Some(json!({ "status": "done" })),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    // Deleting the routine deletes its history.
    fixture
        .ok(
            &fixture.owner,
            "DELETE",
            &format!("/routines/{routine}"),
            None,
        )
        .await;
    assert_eq!(
        fixture
            .count(
                "SELECT COUNT(*) FROM agent_routine_runs WHERE routine_id = $1",
                &routine
            )
            .await,
        0
    );
}
