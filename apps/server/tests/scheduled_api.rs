//! Routines, reminders and scheduled messages: who may make and change them, the runner (claim,
//! no double run, catch-up after a restart, a run a restart interrupted), pauses when the creator
//! loses access, limits, and send later posting once as its author. Runs on SQLite, and on
//! Postgres with `ORBIT_TEST_POSTGRES_URL`.

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

#[tokio::test]
async fn a_routine_is_checked_when_it_is_made() {
    let fixture = Fixture::new().await;
    let member = fixture.add_person("member", "member").await;
    let other = fixture.add_person("other", "member").await;
    let general = fixture.general().await;

    // The default target is the creator's DM with the agent, in their time zone (UTC without one).
    let (status, made) = fixture
        .routine(
            &member,
            json!({ "name": "Daily", "prompt": "list projects", "schedule": "0 9 * * 1-5" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{made}");
    assert_eq!(made["conversation_kind"], "dm");
    assert_eq!(made["time_zone"], "UTC");
    assert_eq!(made["status"], "active");
    assert!(made["next_run_at"].as_i64().unwrap() > now());
    assert_eq!(made["can_edit"], true);

    for (body, detail) in [
        (
            json!({ "name": "x", "prompt": "y", "schedule": "*/5 * * * *" }),
            "at most every 15 minutes",
        ),
        (
            json!({ "name": "x", "prompt": "y", "schedule": "every day" }),
            "cron expression",
        ),
        (
            json!({ "name": "x", "prompt": "y", "schedule": "0 9 * * *", "time_zone": "Mars/Base" }),
            "IANA",
        ),
        (
            json!({ "name": " ", "prompt": "y", "schedule": "0 9 * * *" }),
            "name",
        ),
    ] {
        let (status, value) = fixture.routine(&member, body).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{value}");
        assert!(
            value["detail"].as_str().unwrap().contains(detail),
            "{value}"
        );
    }

    // A channel the creator is in; not a private one they are not in, not someone else's DM.
    let (status, value) = fixture
        .routine(&member, json!({ "name": "x", "prompt": "y", "schedule": "0 9 * * *", "conversation_id": general }))
        .await;
    assert_eq!(status, StatusCode::OK, "{value}");
    let secret = fixture.private_channel(&fixture.owner, "secret", &[]).await;
    let (status, _) = fixture
        .routine(&member, json!({ "name": "x", "prompt": "y", "schedule": "0 9 * * *", "conversation_id": secret }))
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let dm = fixture
        .ok(
            &member,
            "POST",
            "/chat/dms",
            Some(json!({ "user_ids": [other.id.to_string()] })),
        )
        .await["result"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let (status, value) = fixture
        .routine(
            &member,
            json!({ "name": "x", "prompt": "y", "schedule": "0 9 * * *", "conversation_id": dm }),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{value}");

    // 25 routines per person.
    for index in 2..25 {
        let (status, _) = fixture
            .routine(
                &member,
                json!({ "name": format!("r{index}"), "prompt": "y", "schedule": "0 9 * * *" }),
            )
            .await;
        assert_eq!(status, StatusCode::OK);
    }
    let (status, value) = fixture
        .routine(
            &member,
            json!({ "name": "one more", "prompt": "y", "schedule": "0 9 * * *" }),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{value}");
    assert_eq!(value["code"], "limit_reached");
}

#[tokio::test]
async fn members_manage_their_own_routines_and_admins_pause_or_delete_any() {
    let fixture = Fixture::new().await;
    let member = fixture.add_person("member", "member").await;
    let teammate = fixture.add_person("teammate", "member").await;
    let admin = fixture.add_person("admin", "admin").await;
    let routine = fixture
        .routine(
            &member,
            json!({ "name": "Mine", "prompt": "list projects", "schedule": "0 9 * * *" }),
        )
        .await
        .1["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let path = format!("/routines/{routine}");

    // A teammate does not see it; an admin does.
    assert_eq!(
        fixture.ok(&teammate, "GET", "/routines", None).await["routines"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    let listed = fixture.ok(&admin, "GET", "/routines", None).await;
    assert_eq!(listed["routines"][0]["can_edit"], false);
    assert_eq!(listed["routines"][0]["can_stop"], true);
    let (status, _) = fixture
        .api(
            &teammate,
            "PATCH",
            &path,
            Some(json!({ "status": "paused" })),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = fixture.api(&teammate, "DELETE", &path, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // The admin pauses, but neither edits nor resumes: it runs with the creator's permissions.
    let paused = fixture
        .ok(&admin, "PATCH", &path, Some(json!({ "status": "paused" })))
        .await;
    assert_eq!(paused["status"], "paused");
    assert!(paused["next_run_at"].is_null());
    for change in [
        json!({ "status": "active" }),
        json!({ "prompt": "delete everything" }),
    ] {
        let (status, _) = fixture.api(&admin, "PATCH", &path, Some(change)).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    // The creator edits and resumes.
    let resumed = fixture
        .ok(&member, "PATCH", &path, Some(json!({ "status": "active", "schedule": "30 8 * * 1", "time_zone": "Europe/Berlin" })))
        .await;
    assert_eq!(resumed["status"], "active");
    assert_eq!(resumed["schedule"], "30 8 * * 1");
    assert!(resumed["next_run_at"].as_i64().unwrap() > now());

    fixture.ok(&admin, "DELETE", &path, None).await;
    assert_eq!(
        fixture.ok(&member, "GET", "/routines", None).await["routines"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
}

#[tokio::test]
async fn guests_have_no_routines_reminders_or_send_later() {
    let fixture = Fixture::new().await;
    let guest = fixture.add_person("guest", "guest").await;
    let channel = fixture
        .private_channel(&fixture.owner, "client", &[&guest])
        .await;
    for (method, path, body) in [
        ("GET", "/routines", None),
        (
            "POST",
            "/routines",
            Some(
                json!({ "name": "x", "prompt": "y", "schedule": "0 9 * * *", "conversation_id": channel }),
            ),
        ),
        ("GET", "/scheduled-messages", None),
        (
            "POST",
            "/scheduled-messages",
            Some(
                json!({ "kind": "message", "conversation_id": channel, "body": "hi", "send_at": "2099-01-01T10:00:00Z" }),
            ),
        ),
        (
            "POST",
            "/scheduled-messages",
            Some(json!({ "kind": "reminder", "body": "hi", "send_at": "2099-01-01T10:00:00Z" })),
        ),
    ] {
        let (status, value) = fixture.api(&guest, method, path, body).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {path}: {value}");
    }
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

#[tokio::test]
async fn agent_sessions_only_list_and_create_through_the_agent_tool() {
    let fixture = Fixture::new().await;
    let (bearer, general) = fixture.agent_run().await;
    let base = format!("/api/v1/workspaces/{}", fixture.workspace_id);
    // The normal create routes would skip the driver's approval.
    for (path, body) in [
        (
            "routines",
            json!({ "name": "Standup", "prompt": "list projects", "schedule": "0 9 * * 1-5", "conversation_id": general }),
        ),
        (
            "scheduled-messages",
            json!({ "kind": "message", "conversation_id": general, "body": "hi", "send_at": chrono_like(now() + HOUR) }),
        ),
    ] {
        let (status, value) = fixture
            .request(
                "POST",
                &format!("{base}/{path}"),
                ("authorization", &bearer),
                Some(body),
            )
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{path}: {value}");
    }
    let (status, _) = fixture
        .request(
            "GET",
            &format!("{base}/routines"),
            ("authorization", &bearer),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    // A reminder is a DM to the driver only: no approval.
    let reminder = fixture
        .agent_tool(
            &bearer,
            &general,
            "remind_me",
            // A local time without an offset: the driver's zone (UTC here).
            json!({ "body": "Call Ana", "send_at": &chrono_like(now() + 24 * HOUR)[..16] }),
        )
        .await;
    assert_eq!(reminder["status"], "ok", "{reminder}");
    assert_eq!(reminder["result"]["kind"], "reminder");
    // People change, pause and delete; the agent cannot.
    let routine = fixture
        .routine(
            &fixture.owner,
            json!({ "name": "Mine", "prompt": "list projects", "schedule": "0 9 * * 1-5" }),
        )
        .await
        .1;
    let id = routine["id"].as_str().unwrap();
    for method in ["PATCH", "DELETE"] {
        let (status, _) = fixture
            .request(
                method,
                &format!("{base}/routines/{id}"),
                ("authorization", &bearer),
                Some(json!({ "status": "paused" })),
            )
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method}");
    }
    // Only in a thread whose run the caller drives.
    let (status, _) = fixture
        .request(
            "POST",
            &format!("{base}/agent-tools/scheduled"),
            ("authorization", &bearer),
            Some(json!({ "tool": "remind_me", "arguments": { "body": "x", "send_at": chrono_like(now() + HOUR) }, "conversation_id": Id::new_v7().to_string(), "root_message_id": null })),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn a_routine_from_the_agent_needs_the_drivers_yes_to_exactly_it() {
    let fixture = Fixture::new().await;
    let (bearer, general) = fixture.agent_run().await;
    let owner = fixture.owner.id.to_string();
    let routines = || {
        fixture.count(
            "SELECT COUNT(*) FROM agent_routines WHERE creator_id = $1",
            &owner,
        )
    };
    let args = json!({ "name": "Standup", "prompt": "Summarize #support", "schedule": "0 9 * * 1-5", "time_zone": "Europe/Berlin", "conversation_id": general });
    let asked = fixture
        .agent_tool(&bearer, &general, "schedule_routine", args.clone())
        .await;
    assert_eq!(asked["status"], "approval_required", "{asked}");
    let action = asked["action"].as_str().unwrap();
    for part in [
        "\"Standup\"",
        "0 9 * * 1-5",
        "Europe/Berlin",
        "#general",
        "Summarize #support",
        "first on",
    ] {
        assert!(action.contains(part), "{part}: {action}");
    }
    assert_eq!(routines().await, 0);
    // An invalid routine is refused before anyone is asked.
    let refused = fixture
        .agent_tool(
            &bearer,
            &general,
            "schedule_routine",
            json!({ "name": "x", "prompt": "y", "schedule": "* * * * *" }),
        )
        .await;
    assert_eq!(refused["status"], "refused", "{refused}");

    // A "no" does not approve (as for Drive, a yes of someone else does not either: `agent_drive`).
    fixture.say(&fixture.owner, &general, "no").await;
    let again = fixture
        .agent_tool(&bearer, &general, "schedule_routine", args.clone())
        .await;
    assert_eq!(again["status"], "approval_required", "{again}");
    assert_eq!(again["approval_id"], asked["approval_id"]);
    assert_eq!(routines().await, 0);

    // After the driver's yes, other arguments still need their own approval.
    fixture.say(&fixture.owner, &general, "yes").await;
    let mut changed = args.clone();
    changed["schedule"] = json!("*/15 * * * *");
    let other = fixture
        .agent_tool(&bearer, &general, "schedule_routine", changed)
        .await;
    assert_eq!(other["status"], "approval_required", "{other}");
    assert_ne!(other["approval_id"], asked["approval_id"]);
    assert_eq!(routines().await, 0);

    // The same arguments run once.
    let made = fixture
        .agent_tool(&bearer, &general, "schedule_routine", args.clone())
        .await;
    assert_eq!(made["status"], "ok", "{made}");
    assert_eq!(made["result"]["schedule"], "0 9 * * 1-5");
    assert_eq!(made["result"]["time_zone"], "Europe/Berlin");
    assert_eq!(routines().await, 1);
    let next = fixture
        .agent_tool(&bearer, &general, "schedule_routine", args)
        .await;
    assert_eq!(
        next["status"], "approval_required",
        "the yes is used up: {next}"
    );
    assert_eq!(routines().await, 1);
}

#[tokio::test]
async fn a_message_sent_later_by_the_agent_needs_the_drivers_yes_to_exactly_it() {
    let fixture = Fixture::new().await;
    let (bearer, general) = fixture.agent_run().await;
    let owner = fixture.owner.id.to_string();
    let messages = || {
        fixture.count(
            "SELECT COUNT(*) FROM scheduled_messages WHERE author_id = $1 AND kind = 'message'",
            &owner,
        )
    };
    let at = chrono_like(now() + 2 * HOUR);
    let args = json!({ "conversation_id": general, "body": "Standup moved to 10", "send_at": at });
    let asked = fixture
        .agent_tool(&bearer, &general, "send_later", args.clone())
        .await;
    assert_eq!(asked["status"], "approval_required", "{asked}");
    let action = asked["action"].as_str().unwrap();
    for part in ["as you", "#general", "Standup moved to 10", "(UTC)"] {
        assert!(action.contains(part), "{part}: {action}");
    }
    assert_eq!(messages().await, 0);

    fixture.say(&fixture.owner, &general, "Yes, send it").await;
    let mut changed = args.clone();
    changed["body"] = json!("Standup is cancelled");
    let other = fixture
        .agent_tool(&bearer, &general, "send_later", changed)
        .await;
    assert_eq!(other["status"], "approval_required", "{other}");
    assert_eq!(messages().await, 0);

    let made = fixture
        .agent_tool(&bearer, &general, "send_later", args.clone())
        .await;
    assert_eq!(made["status"], "ok", "{made}");
    assert_eq!(made["result"]["body"], "Standup moved to 10");
    assert_eq!(made["result"]["conversation_id"], general.as_str());
    assert_eq!(messages().await, 1);
    let next = fixture
        .agent_tool(&bearer, &general, "send_later", args)
        .await;
    assert_eq!(
        next["status"], "approval_required",
        "the yes is used up: {next}"
    );
    assert_eq!(messages().await, 1);
}

#[tokio::test]
async fn a_due_routine_runs_once_as_its_creator_and_catches_up_once() {
    let fixture = Fixture::new().await;
    let member = fixture.add_person("member", "member").await;
    fixture.connect_chatgpt(&member).await;
    let general = fixture.general().await;
    let routine = fixture
        .routine(&member, json!({ "name": "Hourly", "prompt": "list projects", "schedule": "0 * * * *", "conversation_id": general }))
        .await
        .1["id"]
        .as_str()
        .unwrap()
        .to_owned();

    // Nothing is due yet.
    assert_eq!(
        run_due(&fixture.agent, at(now()))
            .await
            .unwrap()
            .routines_started,
        0
    );

    // Due three days ago (the server was down): two servers' loops run at once; it runs once.
    fixture.set_due(&routine, now() - 72 * HOUR).await;
    let moment = now();
    let (first, second) = tokio::join!(
        run_due(&fixture.agent, at(moment)),
        run_due(&fixture.agent, at(moment))
    );
    assert_eq!(
        first.unwrap().routines_started + second.unwrap().routines_started,
        1
    );
    assert_eq!(fixture.inputs_of(&routine).await, 1);
    let (status, _, next, last, pending) = fixture.routine_row(&routine).await;
    assert_eq!(status, "active");
    assert_eq!(last.as_deref(), Some("started"));
    assert!(pending.is_none());
    // The next run is the next hour after now, not one of the missed ones.
    let next = next.unwrap();
    assert!(next > moment && next <= moment + HOUR, "{next} vs {moment}");
    assert_eq!(
        run_due(&fixture.agent, at(moment))
            .await
            .unwrap()
            .routines_started,
        0
    );

    // The input is the creator's instruction, on the agent's message in the channel.
    let input = sqlx::query("SELECT author_id, kind, thread_key, root_message_id, conversation_id FROM agent_inputs WHERE routine_id = $1")
        .bind(&routine)
        .fetch_one(fixture.database.pool())
        .await
        .unwrap();
    assert_eq!(input.get::<String, _>("author_id"), member.id.to_string());
    assert_eq!(input.get::<String, _>("kind"), "instruction");
    assert_eq!(input.get::<String, _>("conversation_id"), general);
    let header: String = input.get("thread_key");
    assert_eq!(
        input.get::<Option<String>, _>("root_message_id").as_deref(),
        Some(header.as_str())
    );
    let author: String = sqlx::query_scalar("SELECT author_id FROM chat_messages WHERE id = $1")
        .bind(&header)
        .fetch_one(fixture.database.pool())
        .await
        .unwrap();
    assert_ne!(
        author,
        member.id.to_string(),
        "the agent posts the run's message"
    );

    // The sidecar gets the saved instruction and the routine.
    let (status, value) = fixture
        .request(
            "GET",
            "/api/internal/agent/inputs",
            ("authorization", &format!("Bearer {SECRET}")),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let pending = value["inputs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|input| input["routine"]["id"] == routine.as_str())
        .unwrap();
    assert_eq!(pending["body"], "list projects");
    assert_eq!(pending["routine"]["name"], "Hourly");
    assert_eq!(pending["author_id"], member.id.to_string());

    // The next hour runs again.
    let later = run_due(&fixture.agent, at(next + 1)).await.unwrap();
    assert_eq!(later.routines_started, 1);
    assert_eq!(fixture.inputs_of(&routine).await, 2);
}

#[tokio::test]
async fn a_run_that_a_restart_interrupted_is_finished_once() {
    let fixture = Fixture::new().await;
    let member = fixture.add_person("member", "member").await;
    fixture.connect_chatgpt(&member).await;
    let routine = fixture
        .routine(
            &member,
            json!({ "name": "Daily", "prompt": "list projects", "schedule": "0 9 * * *" }),
        )
        .await
        .1["id"]
        .as_str()
        .unwrap()
        .to_owned();
    // Claimed a minute ago, then the server stopped before it posted.
    let run = Id::new_v7().to_string();
    sqlx::query("UPDATE agent_routines SET pending_run_id = $1, last_run_at = $2, last_run_status = 'started' WHERE id = $3")
        .bind(&run)
        .bind(now() - 60_000)
        .bind(&routine)
        .execute(fixture.database.pool())
        .await
        .unwrap();
    let moment = now();
    assert_eq!(
        run_due(&fixture.agent, at(moment))
            .await
            .unwrap()
            .routines_started,
        1
    );
    assert_eq!(fixture.inputs_of(&routine).await, 1);
    assert_eq!(
        fixture
            .count(
                "SELECT COUNT(*) FROM agent_inputs WHERE message_id = $1",
                &run
            )
            .await,
        1
    );
    assert_eq!(
        fixture
            .count(
                "SELECT COUNT(*) FROM chat_messages WHERE nonce = $1",
                &format!("agent:{run}")
            )
            .await,
        1
    );
    assert!(fixture.routine_row(&routine).await.4.is_none());
    assert_eq!(
        run_due(&fixture.agent, at(moment))
            .await
            .unwrap()
            .routines_started,
        0
    );
}

#[tokio::test]
async fn a_routine_pauses_when_its_creator_loses_access_and_skips_without_chatgpt() {
    let fixture = Fixture::new().await;
    let member = fixture.add_person("member", "member").await;
    let channel = fixture
        .private_channel(&fixture.owner, "team", &[&member])
        .await;
    let make = |name: &str, conversation: Option<&str>| {
        let mut body = json!({ "name": name, "prompt": "list projects", "schedule": "0 9 * * *" });
        if let Some(conversation) = conversation {
            body["conversation_id"] = json!(conversation);
        }
        body
    };
    let in_channel = fixture
        .routine(&member, make("channel", Some(&channel)))
        .await
        .1["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let in_dm = fixture.routine(&member, make("dm", None)).await.1["id"]
        .as_str()
        .unwrap()
        .to_owned();

    // Without ChatGPT: skipped, still active, the next run moves on.
    fixture.set_due(&in_dm, now() - 1000).await;
    let round = run_due(&fixture.agent, at(now())).await.unwrap();
    assert_eq!(round.routines_skipped, 1);
    let (status, _, next, last, _) = fixture.routine_row(&in_dm).await;
    assert_eq!(
        (status.as_str(), last.as_deref()),
        ("active", Some("skipped"))
    );
    assert!(next.unwrap() > now());
    assert_eq!(fixture.inputs_of(&in_dm).await, 0);
    fixture.connect_chatgpt(&member).await;

    // Removed from the channel: that routine pauses with the reason and posts nothing.
    fixture
        .ok(
            &fixture.owner,
            "DELETE",
            &format!("/chat/conversations/{channel}/members/{}", member.id),
            None,
        )
        .await;
    fixture.set_due(&in_channel, now() - 1000).await;
    let round = run_due(&fixture.agent, at(now())).await.unwrap();
    assert_eq!(round.routines_paused, 1);
    let (status, reason, next, _, _) = fixture.routine_row(&in_channel).await;
    assert_eq!(status, "paused");
    assert!(reason.unwrap().contains("no longer in the conversation"));
    assert!(next.is_none());
    assert_eq!(fixture.inputs_of(&in_channel).await, 0);
    // Pausing is not a loop: the next round does nothing.
    assert_eq!(
        run_due(&fixture.agent, at(now())).await.unwrap(),
        Default::default()
    );

    // Made a guest: the other one pauses too.
    sqlx::query("UPDATE memberships SET role = 'guest' WHERE user_id = $1")
        .bind(member.id.to_string())
        .execute(fixture.database.pool())
        .await
        .unwrap();
    fixture.set_due(&in_dm, now() - 1000).await;
    assert_eq!(
        run_due(&fixture.agent, at(now()))
            .await
            .unwrap()
            .routines_paused,
        1
    );
    let (status, reason, ..) = fixture.routine_row(&in_dm).await;
    assert_eq!(status, "paused");
    assert!(reason.unwrap().contains("guest"));
}

#[tokio::test]
async fn a_scheduled_message_posts_once_as_its_author_and_cancel_works() {
    let fixture = Fixture::new().await;
    let member = fixture.add_person("member", "member").await;
    let general = fixture.general().await;
    let schedule = |body: &str, send_at: &str| json!({ "kind": "message", "conversation_id": general, "body": body, "send_at": send_at });
    let soon = chrono_like(now() + 60_000);
    let made = fixture
        .ok(
            &member,
            "POST",
            "/scheduled-messages",
            Some(schedule("Hello later", &soon)),
        )
        .await;
    let id = made["id"].as_str().unwrap().to_owned();
    let cancelled = fixture
        .ok(
            &member,
            "POST",
            "/scheduled-messages",
            Some(schedule("Never mind", &soon)),
        )
        .await;
    // Only its author sees or changes it.
    let other = fixture.add_person("other", "member").await;
    assert_eq!(
        fixture.ok(&other, "GET", "/scheduled-messages", None).await["scheduled"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    let (status, _) = fixture
        .api(&other, "DELETE", &format!("/scheduled-messages/{id}"), None)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // A time in the past is refused.
    let (status, _) = fixture
        .api(
            &member,
            "POST",
            "/scheduled-messages",
            Some(schedule("late", "2020-01-01T10:00:00Z")),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    fixture
        .ok(
            &member,
            "PATCH",
            &format!("/scheduled-messages/{id}"),
            Some(json!({ "body": "Hello, later" })),
        )
        .await;
    fixture
        .ok(
            &member,
            "DELETE",
            &format!("/scheduled-messages/{}", cancelled["id"].as_str().unwrap()),
            None,
        )
        .await;

    // Not yet due.
    assert_eq!(
        run_due(&fixture.agent, at(now()))
            .await
            .unwrap()
            .messages_sent,
        0
    );
    let due = at(now() + 120_000);
    let (first, second) = tokio::join!(run_due(&fixture.agent, due), run_due(&fixture.agent, due));
    assert!(first.unwrap().messages_sent + second.unwrap().messages_sent >= 1);
    assert_eq!(run_due(&fixture.agent, due).await.unwrap().messages_sent, 0);
    let posted = sqlx::query("SELECT author_id, body FROM chat_messages WHERE conversation_id = $1 AND nonce LIKE 'scheduled:%'")
        .bind(&general)
        .fetch_all(fixture.database.pool())
        .await
        .unwrap();
    assert_eq!(posted.len(), 1, "once, and not the cancelled one");
    assert_eq!(
        posted[0].get::<String, _>("author_id"),
        member.id.to_string()
    );
    assert_eq!(posted[0].get::<String, _>("body"), "Hello, later");
    assert_eq!(
        fixture
            .ok(&member, "GET", "/scheduled-messages", None)
            .await["scheduled"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
}

#[tokio::test]
async fn a_message_to_a_lost_channel_fails_visibly_and_a_reminder_comes_from_the_agent() {
    let fixture = Fixture::new().await;
    let member = fixture.add_person("member", "member").await;
    let channel = fixture
        .private_channel(&fixture.owner, "team", &[&member])
        .await;
    let soon = chrono_like(now() + 60_000);
    let message = fixture
        .ok(&member, "POST", "/scheduled-messages", Some(json!({ "kind": "message", "conversation_id": channel, "body": "hi", "send_at": soon })))
        .await;
    let reminder = fixture
        .ok(
            &member,
            "POST",
            "/scheduled-messages",
            Some(json!({ "kind": "reminder", "body": "Call Ana", "send_at": soon })),
        )
        .await;
    assert_eq!(reminder["kind"], "reminder");
    fixture
        .ok(
            &fixture.owner,
            "DELETE",
            &format!("/chat/conversations/{channel}/members/{}", member.id),
            None,
        )
        .await;

    let round = run_due(&fixture.agent, at(now() + 120_000)).await.unwrap();
    assert_eq!((round.messages_sent, round.messages_failed), (1, 1));
    let list = fixture
        .ok(&member, "GET", "/scheduled-messages", None)
        .await;
    let left = &list["scheduled"].as_array().unwrap()[0];
    assert_eq!(left["id"], message["id"]);
    assert_eq!(left["status"], "failed");
    assert!(
        left["error"]
            .as_str()
            .unwrap()
            .contains("no longer a member")
    );
    // Failed is final until the author picks a new time.
    assert_eq!(
        run_due(&fixture.agent, at(now() + 180_000)).await.unwrap(),
        Default::default()
    );

    // The reminder is the agent's message in the member's DM with it.
    let row = sqlx::query(
        "SELECT m.author_id, m.body, c.kind FROM chat_messages m JOIN chat_conversations c ON c.id = m.conversation_id \
         WHERE m.nonce = $1",
    )
    .bind(format!("scheduled:{}", reminder["id"].as_str().unwrap()))
    .fetch_one(fixture.database.pool())
    .await
    .unwrap();
    assert_eq!(row.get::<String, _>("kind"), "dm");
    assert_eq!(row.get::<String, _>("body"), "⏰ Reminder: Call Ana");
    assert_ne!(row.get::<String, _>("author_id"), member.id.to_string());
    let members: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM chat_members cm JOIN chat_messages m ON m.conversation_id = cm.conversation_id \
         WHERE m.nonce = $1 AND cm.user_id = $2",
    )
    .bind(format!("scheduled:{}", reminder["id"].as_str().unwrap()))
    .bind(member.id.to_string())
    .fetch_one(fixture.database.pool())
    .await
    .unwrap();
    assert_eq!(members, 1);
}

/// An RFC 3339 UTC time for `ms`.
fn chrono_like(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .unwrap()
        .to_rfc3339()
}
