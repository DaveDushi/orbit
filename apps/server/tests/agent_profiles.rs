//! Agent profiles (step 6): owners and admins make more agents; people reach one by mention in its
//! channels or by DM; it acts with the driver's permissions and only with its tools, enforced by
//! Orbit; guests cannot use it; each person's answer level reaches the run; routines run with any
//! agent; deleting an agent pauses its routines and keeps its DMs. Runs on SQLite, and on Postgres
//! with `ORBIT_TEST_POSTGRES_URL`.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

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
use orbit_server::task_routes::{TaskState, task_router};
use serde_json::{Value, json};
use tower::ServiceExt;

const SECRET: &str = "test-agent-secret-0123456789abcdef0123456789";

struct Fixture {
    _root: tempfile::TempDir,
    database: TestDatabase,
    identity: Arc<IdentityRepository>,
    agent: AgentState,
    app: axum::Router,
    workspace_id: String,
    owner: Member,
    nonce: AtomicU64,
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
        // Nothing listens on the sidecar's address: only the default model can be saved.
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
            .merge(orbit_server::agent::router(agent.clone()))
            .layer(axum::middleware::from_fn_with_state(
                (*database).clone(),
                orbit_server::agent::scope::layer,
            ));
        Self {
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
            nonce: AtomicU64::new(0),
        }
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

    /// A workspace route as `member` (cookie).
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

    async fn chat(&self, member: &Member, method: &str, path: &str, body: Option<Value>) -> Value {
        let (status, value) = self
            .api(member, method, &format!("/chat{path}"), body)
            .await;
        assert_eq!(status, StatusCode::OK, "{method} {path}: {value}");
        value
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

    async fn general(&self) -> String {
        self.chat(&self.owner, "GET", "/conversations", None)
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

    async fn channel(&self, name: &str, kind: &str) -> String {
        self.chat(
            &self.owner,
            "POST",
            "/conversations",
            Some(json!({ "name": name, "kind": kind, "member_ids": [] })),
        )
        .await["result"]["id"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    async fn send(&self, member: &Member, conversation: &str, body: &str) -> String {
        let nonce = format!("n{}", self.nonce.fetch_add(1, Ordering::Relaxed));
        self.chat(
            member,
            "POST",
            &format!("/conversations/{conversation}/messages"),
            Some(json!({ "body": body, "nonce": nonce })),
        )
        .await["result"]["id"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    /// The queued inputs (also answers refusals and makes the default bot).
    async fn pending(&self) -> Vec<Value> {
        let (status, value) = self.internal("GET", "/inputs", None).await;
        assert_eq!(status, StatusCode::OK, "{value}");
        value["inputs"].as_array().unwrap().clone()
    }

    async fn create_agent(&self, body: Value) -> Value {
        let (status, agent) = self.api(&self.owner, "POST", "/agents", Some(body)).await;
        assert_eq!(status, StatusCode::OK, "{agent}");
        agent
    }

    async fn set_thread(&self, input: &Value, status: &str, driver: &Member) {
        let (code, value) = self
            .internal(
                "PUT",
                &format!("/threads/{}", input["thread_key"].as_str().unwrap()),
                Some(json!({
                    "workspace_id": self.workspace_id,
                    "conversation_id": input["conversation_id"],
                    "root_message_id": input["root_message_id"],
                    "status": status,
                    "driver_user_id": driver.id.to_string(),
                    "waiting_for_user_id": null,
                    "agent_user_id": input["agent"]["user_id"],
                })),
            )
            .await;
        assert_eq!(code, StatusCode::NO_CONTENT, "{value}");
    }

    async fn session(&self, member: &Member, agent: Option<&str>) -> (StatusCode, String) {
        let (status, session) = self
            .internal(
                "POST",
                "/sessions",
                Some(json!({
                    "user_id": member.id.to_string(),
                    "workspace_id": self.workspace_id,
                    "agent_user_id": agent,
                })),
            )
            .await;
        (
            status,
            format!("Bearer {}", session["token"].as_str().unwrap_or_default()),
        )
    }

    async fn as_agent(
        &self,
        token: &str,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let uri = format!("/api/v1/workspaces/{}{path}", self.workspace_id);
        self.request(method, &uri, ("authorization", token), body)
            .await
    }

    async fn ack(&self, input: &Value) {
        let (status, _) = self
            .internal(
                "POST",
                &format!("/inputs/{}/ack", input["message_id"].as_str().unwrap()),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
    }
}

#[tokio::test]
async fn only_owners_and_admins_manage_agents_and_names_stay_unique() {
    let fixture = Fixture::new().await;
    let member = fixture.add_person("Mia", "member").await;
    let admin = fixture.add_person("Ada", "admin").await;
    let guest = fixture.add_person("Gus", "guest").await;
    let body = json!({ "name": "Blog Writer", "tools": ["read_page", "create_page"] });

    let (status, _) = fixture
        .api(&member, "POST", "/agents", Some(body.clone()))
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = fixture.api(&guest, "GET", "/agents", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, agent) = fixture
        .api(&admin, "POST", "/agents", Some(body.clone()))
        .await;
    assert_eq!(status, StatusCode::OK, "{agent}");
    assert_eq!(agent["tools"], json!(["read_page", "create_page"]));
    assert_eq!(agent["all_channels"], false);
    let id = agent["id"].as_str().unwrap().to_owned();

    // Members read the list (the routine form picks from it); it is never a guest's.
    let (status, list) = fixture.api(&member, "GET", "/agents", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list["can_manage"], false);
    assert_eq!(list["agents"].as_array().unwrap().len(), 1);
    assert!(list["tools"].as_array().unwrap().len() >= 40);

    // `@Name` must reach one account: a person's name, and another agent's, are taken.
    for taken in ["mia", "BLOG writer", "Owner"] {
        let (status, problem) = fixture
            .api(&admin, "POST", "/agents", Some(json!({ "name": taken })))
            .await;
        assert_eq!(status, StatusCode::CONFLICT, "{taken}: {problem}");
        assert_eq!(problem["code"], "agent_name_taken");
    }
    let (status, _) = fixture
        .api(
            &admin,
            "POST",
            "/agents",
            Some(json!({ "name": "Spy", "tools": ["delete_everything"] })),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    // A model needs the agent service, which is offline here.
    let (status, _) = fixture
        .api(
            &admin,
            "POST",
            "/agents",
            Some(json!({ "name": "Modelled", "model": "gpt-x" })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);

    let (status, _) = fixture
        .api(
            &member,
            "PATCH",
            &format!("/agents/{id}"),
            Some(json!({ "name": "Mine" })),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = fixture
        .api(&member, "DELETE", &format!("/agents/{id}"), None)
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, renamed) = fixture
        .api(
            &admin,
            "PATCH",
            &format!("/agents/{id}"),
            Some(json!({ "name": "Writer", "instructions": "Write blog posts." })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{renamed}");
    assert_eq!(renamed["name"], "Writer");
    assert_eq!(renamed["tools"], json!(["read_page", "create_page"]));
}

#[tokio::test]
async fn an_agent_answers_in_its_channels_and_dms_and_refuses_elsewhere_in_a_dm() {
    let fixture = Fixture::new().await;
    let member = fixture.add_person("Mia", "member").await;
    let blog = fixture.channel("blog", "public").await;
    // Not #general: every member, the agent too, is in it from the start.
    let general = fixture.channel("random", "public").await;
    fixture
        .chat(
            &member,
            "POST",
            &format!("/conversations/{general}/join"),
            None,
        )
        .await;
    let agent = fixture
        .create_agent(json!({
            "name": "Blog Writer",
            "instructions": "Write posts.",
            "tools": ["read_page", "create_page"],
            "channel_ids": [blog],
        }))
        .await;
    let bot = agent["user_id"].as_str().unwrap().to_owned();
    assert_eq!(agent["channels"][0]["id"], blog);
    let donna = fixture.pending().await;
    assert!(donna.is_empty());

    // In its channel: an input for this agent, with its profile, in a thread key of its own.
    let mention = fixture
        .send(&member, &blog, &format!("<@{bot}> draft a post"))
        .await;
    let inputs = fixture.pending().await;
    assert_eq!(inputs.len(), 1, "{inputs:?}");
    let input = &inputs[0];
    assert_eq!(input["message_id"], mention);
    assert_eq!(input["kind"], "instruction");
    assert_eq!(input["root_message_id"], mention);
    assert_ne!(input["thread_key"], mention);
    assert_eq!(input["agent"]["user_id"], bot);
    assert_eq!(input["agent"]["name"], "Blog Writer");
    assert_eq!(input["agent"]["instructions"], "Write posts.");
    assert_eq!(input["agent"]["tools"], json!(["read_page", "create_page"]));
    assert_eq!(input["answer_level"], "normal");
    fixture.ack(input).await;

    // Elsewhere: no run and no answer there; the agent says why in its DM with the author.
    let elsewhere = fixture
        .send(&member, &general, &format!("<@{bot}> draft here"))
        .await;
    assert!(fixture.pending().await.is_empty());
    let conversations = fixture.chat(&member, "GET", "/conversations", None).await;
    let dm = conversations
        .as_array()
        .unwrap()
        .iter()
        .find(|conversation| {
            conversation["kind"] == "dm"
                && conversation["member_ids"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|id| id == &bot)
        })
        .expect("the agent's DM with the author")["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let messages = fixture
        .chat(
            &member,
            "GET",
            &format!("/conversations/{dm}/messages"),
            None,
        )
        .await;
    let refusal = messages["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|message| message["author_id"] == bot)
        .expect("the refusal");
    let text = refusal["body"].as_str().unwrap();
    assert!(text.contains("#random") && text.contains("#blog"), "{text}");
    let general_members: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM chat_members WHERE conversation_id = $1 AND user_id = $2",
    )
    .bind(&general)
    .bind(&bot)
    .fetch_one(fixture.database.pool())
    .await
    .unwrap();
    assert_eq!(general_members, 0, "{elsewhere}");

    // A DM with it works.
    let hello = fixture.send(&member, &dm, "write about our launch").await;
    let inputs = fixture.pending().await;
    assert_eq!(inputs.len(), 1);
    assert_eq!(inputs[0]["message_id"], hello);
    assert_eq!(inputs[0]["agent"]["user_id"], bot);
    assert_eq!(inputs[0]["root_message_id"], Value::Null);
}

#[tokio::test]
async fn guests_cannot_use_an_agent() {
    let fixture = Fixture::new().await;
    let guest = fixture.add_person("Gus", "guest").await;
    let room = fixture.channel("clients", "private").await;
    fixture
        .chat(
            &fixture.owner,
            "POST",
            &format!("/conversations/{room}/members"),
            Some(json!({ "user_ids": [guest.id.to_string()] })),
        )
        .await;
    let agent = fixture
        .create_agent(json!({ "name": "Helper", "tools": ["read_page"], "all_channels": true }))
        .await;
    let bot = agent["user_id"].as_str().unwrap();
    fixture.send(&guest, &room, &format!("<@{bot}> help")).await;
    assert!(fixture.pending().await.is_empty());
    let (status, _) = fixture.api(&guest, "GET", "/agents", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn orbit_enforces_the_agents_tools_and_the_drivers_own_permissions() {
    let fixture = Fixture::new().await;
    let member = fixture.add_person("Mia", "member").await;
    let general = fixture.general().await;
    let reader = fixture
        .create_agent(
            json!({ "name": "Reader", "tools": ["read_page", "drive_list"], "all_channels": true }),
        )
        .await;
    let tasker = fixture
        .create_agent(json!({ "name": "Tasker", "tools": ["search_tasks", "get_task"], "all_channels": true }))
        .await;
    let reader_bot = reader["user_id"].as_str().unwrap();
    let tasker_bot = tasker["user_id"].as_str().unwrap();

    // No run of this agent, no session: a run of another agent does not count.
    fixture
        .send(&member, &general, &format!("<@{reader_bot}> read"))
        .await;
    let inputs = fixture.pending().await;
    fixture.set_thread(&inputs[0], "running", &member).await;
    let (status, _) = fixture.session(&member, Some(tasker_bot)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = fixture.session(&member, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, reader_token) = fixture.session(&member, Some(reader_bot)).await;
    assert_eq!(status, StatusCode::OK);

    // Only the routes of its tools; the workspace and its members always.
    for (method, path) in [
        ("GET", "/tasks"),
        ("POST", "/tasks"),
        ("GET", "/projects"),
        ("GET", "/chat/conversations"),
        ("POST", "/routines"),
        ("POST", "/agent-memory/team"),
        ("GET", "/agents"),
        ("GET", "/agent"),
    ] {
        let (status, problem) = fixture
            .as_agent(&reader_token, method, path, Some(json!({})))
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {path}: {problem}");
        assert_eq!(problem["code"], "agent_scope");
    }
    let (status, _) = fixture
        .as_agent(&reader_token, "GET", "/members", None)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // The Drive route checks the exact tool.
    let (status, refused) = fixture
        .as_agent(
            &reader_token,
            "POST",
            "/agent-tools/drive",
            Some(json!({ "tool": "drive_trash", "arguments": { "node_id": "x" }, "conversation_id": general, "root_message_id": null })),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(refused["status"], "refused");
    assert!(
        refused["detail"].as_str().unwrap().contains("tools"),
        "{refused}"
    );

    // An agent with task tools reads tasks, as the driver: a member sees what a member sees.
    fixture
        .send(&member, &general, &format!("<@{tasker_bot}> tasks"))
        .await;
    let inputs = fixture.pending().await;
    let tasker_input = inputs
        .iter()
        .find(|input| input["agent"]["user_id"] == tasker_bot)
        .unwrap();
    fixture.set_thread(tasker_input, "running", &member).await;
    let (status, tasker_token) = fixture.session(&member, Some(tasker_bot)).await;
    assert_eq!(status, StatusCode::OK);
    let (status, page) = fixture.as_agent(&tasker_token, "GET", "/tasks", None).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    let (status, _) = fixture
        .as_agent(
            &tasker_token,
            "POST",
            "/tasks",
            Some(json!({ "title": "x" })),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let session_user: String =
        sqlx::query_scalar("SELECT user_id FROM agent_sessions WHERE agent_user_id = $1")
            .bind(tasker_bot)
            .fetch_one(fixture.database.pool())
            .await
            .unwrap();
    assert_eq!(session_user, member.id.to_string());
}

#[tokio::test]
async fn each_persons_answer_level_is_stored_and_reaches_their_runs() {
    let fixture = Fixture::new().await;
    let member = fixture.add_person("Mia", "member").await;
    let general = fixture.general().await;
    let (status, level) = fixture
        .request(
            "GET",
            "/api/v1/agent/preferences",
            ("cookie", &member.cookie),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(level["answer_level"], "normal");
    let (status, _) = fixture
        .request(
            "PUT",
            "/api/v1/agent/preferences",
            ("cookie", &member.cookie),
            Some(json!({ "answer_level": "loud" })),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let (status, _) = fixture
        .request(
            "PUT",
            "/api/v1/agent/preferences",
            ("cookie", &member.cookie),
            Some(json!({ "answer_level": "short" })),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = fixture
        .request(
            "PUT",
            "/api/v1/agent/preferences",
            ("cookie", &fixture.owner.cookie),
            Some(json!({ "answer_level": "thorough" })),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    // The default agent and a custom one get the driver's level.
    fixture.pending().await;
    let bot: String = sqlx::query_scalar("SELECT user_id FROM agent_bot WHERE id = 1")
        .fetch_one(fixture.database.pool())
        .await
        .unwrap();
    let agent = fixture
        .create_agent(json!({ "name": "Helper", "tools": ["read_page"], "all_channels": true }))
        .await;
    let mention = fixture
        .send(&member, &general, &format!("<@{bot}> hi"))
        .await;
    fixture
        .send(
            &fixture.owner,
            &general,
            &format!("<@{}> hi", agent["user_id"].as_str().unwrap()),
        )
        .await;
    let inputs = fixture.pending().await;
    assert_eq!(inputs.len(), 2);
    assert_eq!(inputs[0]["message_id"], mention);
    assert_eq!(inputs[0]["agent"], Value::Null);
    assert_eq!(inputs[0]["answer_level"], "short");
    assert_eq!(inputs[1]["answer_level"], "thorough");

    // An answer in a waiting thread runs at the driver's level, not the answerer's.
    fixture.set_thread(&inputs[0], "waiting", &member).await;
    fixture.ack(&inputs[0]).await;
    fixture.ack(&inputs[1]).await;
    let nonce = format!("n{}", fixture.nonce.fetch_add(1, Ordering::Relaxed));
    fixture
        .chat(
            &fixture.owner,
            "POST",
            &format!("/conversations/{general}/messages"),
            Some(json!({ "body": "the answer", "thread_root_id": mention, "nonce": nonce })),
        )
        .await;
    let inputs = fixture.pending().await;
    assert_eq!(inputs.len(), 1);
    assert_eq!(inputs[0]["kind"], "answer");
    assert_eq!(inputs[0]["answer_level"], "short");
}

#[tokio::test]
async fn routines_run_with_the_chosen_agent_and_pause_when_it_is_deleted() {
    let fixture = Fixture::new().await;
    let member = fixture.add_person("Mia", "member").await;
    let blog = fixture.channel("blog", "public").await;
    let general = fixture.general().await;
    fixture
        .chat(
            &member,
            "POST",
            &format!("/conversations/{blog}/join"),
            None,
        )
        .await;
    let agent = fixture
        .create_agent(json!({ "name": "Blog Writer", "tools": ["read_page", "schedule_routine"], "channel_ids": [blog] }))
        .await;
    let bot = agent["user_id"].as_str().unwrap().to_owned();
    fixture.pending().await;

    let routine = |conversation: &str| json!({ "name": "Weekly post", "prompt": "Draft the weekly post.", "schedule": "0 9 * * 1", "conversation_id": conversation, "agent_user_id": bot });
    // Not in a channel that is not the agent's.
    let (status, problem) = fixture
        .api(&member, "POST", "/routines", Some(routine(&general)))
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
    let (status, made) = fixture
        .api(&member, "POST", "/routines", Some(routine(&blog)))
        .await;
    assert_eq!(status, StatusCode::OK, "{made}");
    assert_eq!(made["agent_user_id"], bot);
    assert_eq!(made["agent_name"], "Blog Writer");
    let routine_id = made["id"].as_str().unwrap().to_owned();

    // The run posts as the agent and queues its input for that agent.
    sqlx::query("UPDATE agent_routines SET next_run_at = $1 WHERE id = $2")
        .bind(TimestampMillis::now().as_millis() - 1_000)
        .bind(&routine_id)
        .execute(fixture.database.pool())
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO agent_credentials (user_id, sealed, account_id, created_at, updated_at) \
         VALUES ($1, $2, NULL, 0, 0)",
    )
    .bind(member.id.to_string())
    .bind(vec![1_u8, 2, 3])
    .execute(fixture.database.pool())
    .await
    .unwrap();
    let round = run_due(&fixture.agent, TimestampMillis::now())
        .await
        .unwrap();
    assert_eq!(round.routines_started, 1, "{round:?}");
    let inputs = fixture.pending().await;
    assert_eq!(inputs.len(), 1);
    assert_eq!(inputs[0]["agent"]["user_id"], bot);
    assert_eq!(inputs[0]["routine"]["id"], routine_id);
    assert_ne!(inputs[0]["thread_key"], inputs[0]["root_message_id"]);
    let header = sqlx::query("SELECT author_id FROM chat_messages WHERE id = $1")
        .bind(inputs[0]["root_message_id"].as_str().unwrap())
        .fetch_one(fixture.database.pool())
        .await
        .unwrap();
    assert_eq!(header.get::<String, _>("author_id"), bot);
    fixture.ack(&inputs[0]).await;

    // An agent's own session makes routines for itself, whatever the body says, only through
    // `/agent-tools/scheduled` and after the driver's yes.
    let mention = fixture
        .send(&member, &blog, &format!("<@{bot}> every monday"))
        .await;
    let inputs = fixture.pending().await;
    let input = inputs
        .iter()
        .find(|input| input["message_id"] == mention)
        .unwrap();
    fixture.set_thread(input, "running", &member).await;
    let (_, token) = fixture.session(&member, Some(&bot)).await;
    let mut own = routine(&blog);
    own.as_object_mut().unwrap().remove("agent_user_id");
    own["name"] = json!("Own");
    let (status, problem) = fixture
        .as_agent(&token, "POST", "/routines", Some(own.clone()))
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{problem}");
    let call = |tool: &str, arguments: &Value| json!({ "tool": tool, "arguments": arguments, "conversation_id": blog, "root_message_id": input["root_message_id"] });
    // Not a tool of this agent.
    let (status, refused) = fixture
        .as_agent(
            &token,
            "POST",
            "/agent-tools/scheduled",
            Some(call(
                "remind_me",
                &json!({ "body": "x", "send_at": "2099-01-01T09:00", "time_zone": "UTC" }),
            )),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(refused["status"], "refused", "{refused}");
    // Not in a channel that is not the agent's, before anyone is asked.
    let mut elsewhere = own.clone();
    elsewhere["conversation_id"] = json!(general);
    let (_, refused) = fixture
        .as_agent(
            &token,
            "POST",
            "/agent-tools/scheduled",
            Some(call("schedule_routine", &elsewhere)),
        )
        .await;
    assert_eq!(refused["status"], "refused", "{refused}");
    let (status, asked) = fixture
        .as_agent(
            &token,
            "POST",
            "/agent-tools/scheduled",
            Some(call("schedule_routine", &own)),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{asked}");
    assert_eq!(asked["status"], "approval_required", "{asked}");
    fixture
        .chat(
            &member,
            "POST",
            &format!("/conversations/{blog}/messages"),
            Some(json!({ "body": "yes", "thread_root_id": input["root_message_id"], "nonce": "yes-own" })),
        )
        .await;
    let (status, made) = fixture
        .as_agent(
            &token,
            "POST",
            "/agent-tools/scheduled",
            Some(call("schedule_routine", &own)),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{made}");
    assert_eq!(made["status"], "ok", "{made}");
    assert_eq!(made["result"]["agent_user_id"], bot);

    // Deleting the agent: routines pause, its DM stays readable, nobody reaches it, its sessions end.
    let dm = fixture
        .chat(&member, "POST", "/dms", Some(json!({ "user_ids": [bot] })))
        .await["result"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    fixture.send(&member, &dm, "hello").await;
    let (status, _) = fixture
        .api(
            &fixture.owner,
            "DELETE",
            &format!("/agents/{}", agent["id"].as_str().unwrap()),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, routines) = fixture.api(&member, "GET", "/routines", None).await;
    assert_eq!(status, StatusCode::OK);
    for routine in routines["routines"].as_array().unwrap() {
        assert_eq!(routine["status"], "paused", "{routine}");
        assert_eq!(routine["paused_reason"], "Paused: its agent was deleted.");
    }
    let messages = fixture
        .chat(
            &member,
            "GET",
            &format!("/conversations/{dm}/messages"),
            None,
        )
        .await;
    assert!(!messages["items"].as_array().unwrap().is_empty());
    assert!(fixture.pending().await.is_empty());
    fixture.send(&member, &dm, "still there?").await;
    fixture
        .send(&member, &blog, &format!("<@{bot}> hello"))
        .await;
    assert!(fixture.pending().await.is_empty());
    let (status, _) = fixture.as_agent(&token, "GET", "/members", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let name: String = sqlx::query_scalar("SELECT display_name FROM users WHERE id = $1")
        .bind(&bot)
        .fetch_one(fixture.database.pool())
        .await
        .unwrap();
    assert_eq!(name, "Blog Writer (deleted)");
    // The name is free again.
    fixture.create_agent(json!({ "name": "Blog Writer" })).await;
}

impl Fixture {
    /// An HTTP API integration of the workspace for these roles.
    async fn add_integration(&self, slug: &str, roles: &[&str]) -> String {
        let id = Id::new_v7().to_string();
        sqlx::query(
            "INSERT INTO agent_integrations (id, workspace_id, slug, name, kind, url, allowed_roles, created_at, updated_at) \
             VALUES ($1, $2, $3, $3, 'http', 'https://api.example.com', $4, 1, 1)",
        )
        .bind(&id)
        .bind(&self.workspace_id)
        .bind(slug)
        .bind(json!(roles).to_string())
        .execute(self.database.pool())
        .await
        .unwrap();
        id
    }

    /// The integration tools an agent session lists, by slug.
    async fn listed(&self, token: &str) -> Vec<String> {
        let (status, listed) = self.as_agent(token, "GET", "/agent-tools", None).await;
        assert_eq!(status, StatusCode::OK, "{listed}");
        listed["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| {
                tool["name"]
                    .as_str()
                    .unwrap()
                    .trim_end_matches("__api_request")
                    .to_owned()
            })
            .collect()
    }

    /// A call of an integration's tool in the run of `input`: the problem code, or "" when Orbit
    /// passed the agent and role checks. DELETE is not an allowed method, so nothing leaves Orbit.
    async fn call_code(&self, token: &str, input: &Value, slug: &str) -> String {
        let (_, answer) = self
            .as_agent(
                token,
                "POST",
                "/agent-tools/call",
                Some(json!({
                    "tool": format!("{slug}__api_request"),
                    "arguments": { "method": "DELETE", "path": "/orders" },
                    "conversation_id": input["conversation_id"],
                    "root_message_id": input["root_message_id"],
                })),
            )
            .await;
        answer["code"].as_str().unwrap_or_default().to_owned()
    }
}

#[tokio::test]
async fn the_default_agent_is_first_cannot_be_deleted_and_its_card_reaches_runs() {
    let fixture = Fixture::new().await;
    let admin = fixture.add_person("Ada", "admin").await;
    let member = fixture.add_person("Mia", "member").await;
    let general = fixture.general().await;

    let (status, list) = fixture.api(&member, "GET", "/agents", None).await;
    assert_eq!(status, StatusCode::OK, "{list}");
    let bot = list["default_agent"]["user_id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(list["default_agent"]["name"], "Orbit Agent");
    assert_eq!(list["default_agent"]["can_rename"], false);

    // It cannot be deleted, by its route or as a custom agent.
    let (status, _) = fixture
        .api(&fixture.owner, "DELETE", "/agents/default", None)
        .await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    let (status, _) = fixture
        .api(&fixture.owner, "DELETE", &format!("/agents/{bot}"), None)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Its name is the installation's: a workspace admin changes only the workspace's part.
    let (status, problem) = fixture
        .api(
            &admin,
            "PATCH",
            "/agents/default",
            Some(json!({ "name": "Donna", "instructions": "Be brief." })),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{problem}");
    assert_eq!(problem["code"], "installation_admin_required");
    let (status, _) = fixture
        .api(
            &member,
            "PATCH",
            "/agents/default",
            Some(json!({ "instructions": "x" })),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, saved) = fixture
        .api(
            &admin,
            "PATCH",
            "/agents/default",
            Some(json!({ "instructions": "Be brief." })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(saved["instructions"], "Be brief.");
    assert_eq!(saved["name"], "Orbit Agent");
    let (status, problem) = fixture
        .api(
            &fixture.owner,
            "PATCH",
            "/agents/default",
            Some(json!({ "name": "mia" })),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{problem}");
    let (status, renamed) = fixture
        .api(
            &fixture.owner,
            "PATCH",
            "/agents/default",
            Some(json!({ "name": "Donna" })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{renamed}");
    assert_eq!(renamed["name"], "Donna");
    assert_eq!(renamed["instructions"], "Be brief.");
    assert_eq!(renamed["can_rename"], true);

    // The name is the bot's: the sidecar and its runs read it, and the card's instructions reach
    // the default agent's inputs only.
    fixture
        .send(&member, &general, &format!("<@{bot}> hello"))
        .await;
    let (status, queue) = fixture.internal("GET", "/inputs", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(queue["bot_name"], "Donna");
    let input = &queue["inputs"][0];
    assert!(input["agent"].is_null(), "{input}");
    assert_eq!(input["default_agent"]["instructions"], "Be brief.");
    assert!(input["default_agent"]["model"].is_null());
    let (status, _) = fixture
        .request(
            "PUT",
            "/api/v1/admin/agent",
            ("cookie", &fixture.owner.cookie),
            Some(json!({ "name": "Old route" })),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "the old rename route is gone"
    );
}

#[tokio::test]
async fn each_agent_uses_only_its_ticked_integrations_and_the_role_rule_still_applies() {
    let fixture = Fixture::new().await;
    let member = fixture.add_person("Mia", "member").await;
    let general = fixture.general().await;
    let everyone = ["owner", "admin", "member"];
    let shop = fixture.add_integration("shop", &everyone).await;
    let tiktok = fixture.add_integration("tiktok", &["owner", "admin"]).await;
    let ads = fixture.add_integration("ads", &everyone).await;

    let seller = fixture
        .create_agent(json!({
            "name": "Seller",
            "tools": ["use_integration"],
            "all_channels": true,
            "integration_ids": [shop, tiktok],
        }))
        .await;
    assert_eq!(seller["integration_ids"].as_array().unwrap().len(), 2);
    let (status, _) = fixture
        .api(
            &fixture.owner,
            "PATCH",
            &format!("/agents/{}", seller["id"].as_str().unwrap()),
            Some(json!({ "integration_ids": ["not-ours"] })),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    // The default agent: everything but ads.
    let (status, card) = fixture
        .api(
            &fixture.owner,
            "PATCH",
            "/agents/default",
            Some(json!({ "integration_ids": [shop, tiktok] })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{card}");
    // Added later: on for the default agent, off for the custom one.
    let later = fixture.add_integration("later", &everyone).await;
    let (_, list) = fixture.api(&fixture.owner, "GET", "/agents", None).await;
    let mut default_ids: Vec<&str> = list["default_agent"]["integration_ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|id| id.as_str().unwrap())
        .collect();
    default_ids.sort_unstable();
    assert!(!default_ids.contains(&ads.as_str()));
    let mut expected = vec![shop.as_str(), tiktok.as_str(), later.as_str()];
    expected.sort_unstable();
    assert_eq!(default_ids, expected);
    assert_eq!(list["integrations"].as_array().unwrap().len(), 4);
    assert_eq!(
        list["agents"][0]["integration_ids"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let (_, members_view) = fixture.api(&member, "GET", "/agents", None).await;
    assert_eq!(members_view["integrations"], json!([]));

    // The custom agent, driven by a member: its ticks, within the member's role.
    let seller_bot = seller["user_id"].as_str().unwrap();
    fixture
        .send(&member, &general, &format!("<@{seller_bot}> orders"))
        .await;
    let inputs = fixture.pending().await;
    let seller_input = inputs
        .iter()
        .find(|input| input["agent"]["user_id"] == seller_bot)
        .unwrap()
        .clone();
    fixture.set_thread(&seller_input, "running", &member).await;
    let (_, token) = fixture.session(&member, Some(seller_bot)).await;
    assert_eq!(fixture.listed(&token).await, ["shop"]);
    for (slug, code) in [
        ("ads", "agent_integration_agent"),
        ("later", "agent_integration_agent"),
        ("tiktok", "agent_integration_role"),
    ] {
        assert_eq!(
            fixture.call_code(&token, &seller_input, slug).await,
            code,
            "{slug}"
        );
    }
    assert_eq!(fixture.call_code(&token, &seller_input, "shop").await, "");
    fixture.ack(&seller_input).await;
    fixture.set_thread(&seller_input, "idle", &member).await;

    // The default agent, driven by the owner: every role, all but the unticked one.
    let bot = list["default_agent"]["user_id"].as_str().unwrap();
    fixture
        .send(&fixture.owner, &general, &format!("<@{bot}> orders"))
        .await;
    let inputs = fixture.pending().await;
    let input = inputs
        .iter()
        .find(|input| input["agent"].is_null())
        .unwrap()
        .clone();
    fixture.set_thread(&input, "running", &fixture.owner).await;
    let (_, token) = fixture.session(&fixture.owner, None).await;
    let mut listed = fixture.listed(&token).await;
    listed.sort();
    assert_eq!(listed, ["later", "shop", "tiktok"]);
    assert_eq!(
        fixture.call_code(&token, &input, "ads").await,
        "agent_integration_agent"
    );

    // Deleting an integration takes its ticks with it.
    sqlx::query("DELETE FROM agent_integrations WHERE id = $1")
        .bind(&shop)
        .execute(fixture.database.pool())
        .await
        .unwrap();
    let (_, list) = fixture.api(&fixture.owner, "GET", "/agents", None).await;
    assert_eq!(list["agents"][0]["integration_ids"], json!([tiktok]));
}
