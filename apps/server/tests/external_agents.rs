//! External agents (phase 1): an outside client runs an agent profile through MCP, as its bot
//! user. Its card (scopes, channels, projects, DMs) limits every tool family; it pulls its inputs
//! with `wait_for_messages` (at least once, acknowledged per agent), answers with `send_message`,
//! never joins a channel by a mention, starts no agent or task run, and loses every credential when
//! it is deleted. Owners and admins connect it by OAuth "as agent" or with an agent token. Runs on
//! SQLite, and on Postgres with `ORBIT_TEST_POSTGRES_URL`.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use base64::Engine;
use orbit_platform::{
    AttachmentMutationCoordinator, AuthenticatedUser, Id, LocalBlobStore, PasswordService,
    TestDatabase, TimestampMillis, UploadLimits, UploadService,
};
use orbit_server::agent::{AgentConfig, AgentState};
use orbit_server::auth_routes::CookieMode;
use orbit_server::chat_routes::{ChatState, chat_router};
use orbit_server::mcp::{McpState, router as mcp_router};
use orbit_server::oauth::{OAuthState, router as oauth_router};
use orbit_server::repositories::api_tokens::ApiTokenRepository;
use orbit_server::repositories::chat::ChatRepository;
use orbit_server::repositories::identity::{IdentityRepository, SetupRequest};
use orbit_server::repositories::pages::PageRepository;
use orbit_server::repositories::tasks::{CreateTask, TaskRepository};
use orbit_server::task_routes::{TaskState, task_router};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tower::ServiceExt;

const ORIGIN: &str = "https://orbit.test";
const SECRET: &str = "test-agent-secret-0123456789abcdef0123456789";
const CALLBACK: &str = "http://127.0.0.1:6274/callback";
const VERIFIER: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-._~";

struct Fixture {
    _root: tempfile::TempDir,
    database: TestDatabase,
    identity: Arc<IdentityRepository>,
    tasks: TaskRepository,
    app: axum::Router,
    workspace_id: Id,
    project_id: Id,
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
                "setup",
                TimestampMillis::from_millis(now.as_millis() + 60_000),
            )
            .await
            .unwrap();
        let setup = identity
            .complete_setup(
                SetupRequest {
                    token: "setup".to_owned(),
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
        let chat = ChatState::new(Arc::clone(&identity), uploads.clone(), CookieMode::secure());
        // Nothing listens on the sidecar's address: external agents do not need it.
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
        let oauth = OAuthState::new(Arc::clone(&identity), CookieMode::secure(), ORIGIN);
        let tasks = TaskRepository::new((*database).clone());
        let mcp = mcp_router(
            McpState::new(
                Arc::new(ApiTokenRepository::new((*database).clone())),
                Arc::new(tasks.clone()),
            )
            .with_content(
                Arc::new(PageRepository::new((*database).clone())),
                Arc::new(ChatRepository::new((*database).clone(), uploads)),
            )
            .with_oauth(oauth.clone())
            .with_agents(agent.clone()),
            ORIGIN,
        );
        let app = chat_router(chat)
            .merge(task_router(TaskState::new(
                Arc::clone(&identity),
                CookieMode::secure(),
            )))
            .merge(orbit_server::agent::router(agent))
            .merge(oauth_router(oauth))
            .merge(mcp);
        Self {
            _root: root,
            database,
            identity,
            tasks,
            app,
            workspace_id: setup.workspace_id,
            project_id: setup.project_id,
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
        .bind(self.workspace_id.to_string())
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

    async fn send_request(&self, request: Request<Body>) -> (StatusCode, Value) {
        let response = self.app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    /// A route as `member` (cookie); `path` starts at `/api/v1`.
    async fn call(
        &self,
        member: &Member,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let builder = Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, "orbit.test")
            .header("cookie", &member.cookie);
        let request = match body {
            Some(value) => builder
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(value.to_string())),
            None => builder.body(Body::empty()),
        }
        .unwrap();
        self.send_request(request).await
    }

    /// A workspace route as `member`.
    async fn api(
        &self,
        member: &Member,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let uri = format!("/api/v1/workspaces/{}{path}", self.workspace_id);
        self.call(member, method, &uri, body).await
    }

    async fn chat(&self, member: &Member, method: &str, path: &str, body: Option<Value>) -> Value {
        let (status, value) = self
            .api(member, method, &format!("/chat{path}"), body)
            .await;
        assert_eq!(status, StatusCode::OK, "{method} {path}: {value}");
        value
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
        self.reply(member, conversation, body, None).await
    }

    async fn reply(
        &self,
        member: &Member,
        conversation: &str,
        body: &str,
        thread: Option<&str>,
    ) -> String {
        let nonce = format!("n{}", self.nonce.fetch_add(1, Ordering::Relaxed));
        self.chat(
            member,
            "POST",
            &format!("/conversations/{conversation}/messages"),
            Some(json!({ "body": body, "nonce": nonce, "thread_root_id": thread })),
        )
        .await["result"]["id"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    async fn create_external(&self, body: Value) -> Value {
        let (status, agent) = self
            .api(&self.owner, "POST", "/agents/external", Some(body))
            .await;
        assert_eq!(status, StatusCode::OK, "{agent}");
        agent
    }

    async fn token(&self, agent: &Value) -> String {
        let (status, issued) = self
            .api(
                &self.owner,
                "POST",
                &format!("/agents/{}/tokens", agent["id"].as_str().unwrap()),
                Some(json!({ "expires_in_days": 30 })),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{issued}");
        issued["token"].as_str().unwrap().to_owned()
    }

    /// One JSON-RPC call to `/mcp` with `token`; (status, body).
    async fn rpc(&self, token: &str, method: &str, params: Value) -> (StatusCode, Value) {
        let request = Request::post("/mcp")
            .header(header::HOST, "orbit.test")
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::ACCEPT, "application/json, text/event-stream")
            .header("mcp-protocol-version", "2025-11-25")
            .body(Body::from(
                json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).to_string(),
            ))
            .unwrap();
        self.send_request(request).await
    }

    /// A tool call that must reach the tool; its `result` (or `error`).
    async fn tool(&self, token: &str, name: &str, arguments: Value) -> Value {
        let (status, body) = self
            .rpc(
                token,
                "tools/call",
                json!({"name": name, "arguments": arguments}),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{name}: {body}");
        if body.get("error").is_some() {
            return json!({"isError": true, "protocol": body["error"]});
        }
        body["result"].clone()
    }

    async fn tool_names(&self, token: &str) -> Vec<String> {
        let (status, body) = self.rpc(token, "tools/list", json!({})).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["name"].as_str().unwrap().to_owned())
            .collect()
    }

    async fn wait(&self, token: &str, timeout: u64, ack: &[&str]) -> Vec<Value> {
        let result = self
            .tool(
                token,
                "wait_for_messages",
                json!({"timeout_seconds": timeout, "ack": ack}),
            )
            .await;
        result["structuredContent"]["events"]
            .as_array()
            .unwrap_or_else(|| panic!("{result}"))
            .clone()
    }

    async fn project(&self, name: &str, key: &str, private: bool) -> Id {
        self.tasks
            .create_project(
                self.workspace_id,
                self.owner.id,
                name.into(),
                key.into(),
                "#ff0000".into(),
                private,
                "test",
                TimestampMillis::now(),
            )
            .await
            .unwrap()
            .id
    }

    async fn task(&self, project: Id, title: &str) -> Id {
        let status: String = sqlx::query_scalar(
            "SELECT id FROM task_statuses WHERE project_id = $1 ORDER BY position LIMIT 1",
        )
        .bind(project.to_string())
        .fetch_one(self.database.pool())
        .await
        .unwrap();
        self.tasks
            .create_task(
                self.workspace_id,
                self.owner.id,
                CreateTask {
                    project_id: project,
                    status_id: status.parse().unwrap(),
                    title: title.into(),
                    description: String::new(),
                    description_json: None,
                    source_url: None,
                    priority: "none".into(),
                    position: None,
                    assignee_ids: vec![],
                    label_ids: vec![],
                    due_start_at: None,
                    due_at: None,
                    parent_task_id: None,
                    milestone_id: None,
                    cycle_id: None,
                    estimate: None,
                },
                "test",
                TimestampMillis::now(),
            )
            .await
            .unwrap()
            .task
            .id
    }

    async fn count(&self, sql: &str, bind: &str) -> i64 {
        sqlx::query_scalar(sqlx::AssertSqlSafe(sql.to_owned()))
            .bind(bind.to_owned())
            .fetch_one(self.database.pool())
            .await
            .unwrap()
    }
}

fn not_available(result: &Value) -> bool {
    result["isError"] == true
}

#[tokio::test]
async fn owners_and_admins_make_external_agents_and_tokens_are_kept_as_hashes() {
    let fixture = Fixture::new().await;
    let member = fixture.add_person("Mia", "member").await;
    let (status, _) = fixture
        .api(
            &member,
            "POST",
            "/agents/external",
            Some(json!({ "name": "Muse" })),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = fixture
        .api(
            &fixture.owner,
            "POST",
            "/agents/external",
            Some(json!({ "name": "Muse", "scopes": ["drive:read"] })),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "no Drive scope");

    let agent = fixture.create_external(json!({ "name": "Muse" })).await;
    assert_eq!(agent["kind"], "external");
    assert_eq!(agent["all_channels"], false);
    assert_eq!(
        agent["external"]["scopes"],
        json!(["tasks:read", "chat:read", "chat:write"])
    );
    let id = agent["id"].as_str().unwrap();
    // The internal form cannot change it (no tools, model or "every channel" for it).
    let (status, _) = fixture
        .api(
            &fixture.owner,
            "PATCH",
            &format!("/agents/{id}"),
            Some(json!({ "all_channels": true })),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let (status, _) = fixture
        .api(
            &member,
            "POST",
            &format!("/agents/{id}/tokens"),
            Some(json!({ "expires_in_days": 90 })),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = fixture
        .api(
            &fixture.owner,
            "POST",
            &format!("/agents/{id}/tokens"),
            Some(json!({ "expires_in_days": 7 })),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    let token = fixture.token(&agent).await;
    assert!(token.starts_with("orb_agent_"));
    let stored: Vec<u8> =
        sqlx::query_scalar("SELECT token_hash FROM agent_tokens WHERE agent_id = $1")
            .bind(id)
            .fetch_one(fixture.database.pool())
            .await
            .unwrap();
    assert_eq!(stored, Sha256::digest(token.as_bytes()).to_vec());
    // The token never shows again: the card lists only its prefix, and only to managers.
    let (_, list) = fixture.api(&fixture.owner, "GET", "/agents", None).await;
    let card = &list["agents"][0]["external"];
    assert_eq!(card["tokens"].as_array().unwrap().len(), 1);
    assert!(!list.to_string().contains(&token));
    let (_, list) = fixture.api(&member, "GET", "/agents", None).await;
    assert!(
        list["agents"][0]["external"]["tokens"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    // The agent acts as its bot user.
    let me = fixture.tool(&token, "get_me", json!({})).await;
    assert_eq!(
        me["structuredContent"]["user"]["id"], agent["user_id"],
        "{me}"
    );
    let token_id = card["tokens"][0]["id"].as_str().unwrap();
    let (status, _) = fixture
        .api(
            &fixture.owner,
            "DELETE",
            &format!("/agents/{id}/tokens/{token_id}"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = fixture.rpc(&token, "tools/list", json!({})).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "a revoked token stops");
}

#[tokio::test]
async fn every_tool_family_keeps_to_the_card() {
    let fixture = Fixture::new().await;
    let on_card = fixture.channel("on-card", "public").await;
    let off_card = fixture.channel("off-card", "public").await;
    let secret_project = fixture.project("Secret", "SEC", true).await;
    let other_project = fixture.project("Other", "OTH", false).await;
    let agent = fixture
        .create_external(json!({
            "name": "Muse",
            "channel_ids": [on_card],
            "project_ids": [fixture.project_id, secret_project],
            "scopes": ["chat:read", "chat:write", "tasks:write", "docs:read", "docs:write"],
            "allow_dms": false,
        }))
        .await;
    let token = fixture.token(&agent).await;
    let bot = agent["user_id"].as_str().unwrap();
    let inside = fixture.send(&fixture.owner, &on_card, "hello inside").await;
    let outside = fixture
        .send(&fixture.owner, &off_card, "hello outside")
        .await;

    // Chat: lists and search filter in SQL; an ID outside answers like an unknown one.
    let conversations = fixture.tool(&token, "list_conversations", json!({})).await;
    let ids: Vec<&str> = conversations["structuredContent"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec![on_card.as_str()], "{conversations}");
    let read = fixture
        .tool(&token, "list_messages", json!({"conversation_id": on_card}))
        .await;
    assert!(read.to_string().contains("hello inside"));
    for (name, args) in [
        ("list_messages", json!({"conversation_id": off_card})),
        ("get_thread", json!({"message_id": outside})),
        (
            "search_messages",
            json!({"query": "hello", "conversation_id": off_card}),
        ),
        (
            "send_message",
            json!({"conversation_id": off_card, "text": "hi"}),
        ),
        (
            "set_status",
            json!({"conversation_id": off_card, "state": "working"}),
        ),
    ] {
        let unknown = fixture
            .tool(&token, name, {
                let mut unknown = args.clone();
                for key in ["conversation_id", "message_id"] {
                    if unknown.get(key).is_some() {
                        unknown[key] = json!(Id::new_v7());
                    }
                }
                unknown
            })
            .await;
        let refused = fixture.tool(&token, name, args).await;
        assert!(not_available(&refused), "{name}: {refused}");
        assert_eq!(refused, unknown, "{name}: no existence leak");
    }
    let found = fixture
        .tool(&token, "search_messages", json!({"query": "hello"}))
        .await;
    assert!(found.to_string().contains(&inside));
    assert!(!found.to_string().contains(&outside), "{found}");
    let thread = fixture
        .tool(&token, "get_thread", json!({"message_id": inside}))
        .await;
    assert!(!not_available(&thread), "{thread}");

    // DMs are off on the card: a DM with the bot is neither read nor answered, and makes no input.
    let dm = fixture
        .chat(
            &fixture.owner,
            "POST",
            "/dms",
            Some(json!({ "user_ids": [bot] })),
        )
        .await["result"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    fixture.send(&fixture.owner, &dm, "a private word").await;
    assert!(not_available(
        &fixture
            .tool(&token, "list_messages", json!({"conversation_id": dm}))
            .await
    ));
    assert!(not_available(
        &fixture
            .tool(
                &token,
                "send_message",
                json!({"conversation_id": dm, "text": "hi"})
            )
            .await
    ));
    assert_eq!(
        fixture
            .count(
                "SELECT COUNT(*) FROM agent_inputs WHERE agent_user_id = $1",
                bot
            )
            .await,
        0
    );

    // Tasks: the card's projects that the bot sees. A private project needs the bot on its list.
    let task = fixture.task(fixture.project_id, "On the card").await;
    let secret = fixture.task(secret_project, "Secret plan").await;
    let other = fixture.task(other_project, "Other work").await;
    let projects = fixture.tool(&token, "list_projects", json!({})).await;
    let names: Vec<&str> = projects["structuredContent"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["General"]);
    assert!(!not_available(
        &fixture
            .tool(&token, "get_task", json!({"task_id": task}))
            .await
    ));
    for id in [secret, other] {
        let refused = fixture
            .tool(&token, "get_task", json!({"task_id": id}))
            .await;
        let unknown = fixture
            .tool(&token, "get_task", json!({"task_id": Id::new_v7()}))
            .await;
        assert!(not_available(&refused), "{refused}");
        assert_eq!(refused, unknown);
    }
    let listed = fixture.tool(&token, "list_tasks", json!({})).await;
    assert!(listed.to_string().contains("On the card"));
    assert!(!listed.to_string().contains("Secret plan"));
    assert!(!listed.to_string().contains("Other work"));
    // Never deletes a task or makes a project, whatever its scopes.
    let names = fixture.tool_names(&token).await;
    for hidden in ["delete_task", "create_project", "drive_list", "drive_read"] {
        assert!(!names.iter().any(|name| name == hidden), "{hidden}");
    }
    assert!(names.iter().any(|name| name == "wait_for_messages"));
    assert!(not_available(
        &fixture
            .tool(
                &token,
                "delete_task",
                json!({"task_id": task, "expected_version": 1})
            )
            .await
    ));

    // Docs: only in the teamspaces of its projects.
    let now = TimestampMillis::now().as_millis();
    let (linked, other_space) = (Id::new_v7(), Id::new_v7());
    for (id, name, project, position) in [
        (linked, "General docs", fixture.project_id, 10_i64),
        (other_space, "Elsewhere", other_project, 11),
    ] {
        sqlx::query(
            "INSERT INTO teamspaces (id, workspace_id, name, position, version, created_by, created_at, updated_at, project_id) \
             VALUES ($1, $2, $3, $4, 0, $5, $6, $6, $7)",
        )
        .bind(id.to_string())
        .bind(fixture.workspace_id.to_string())
        .bind(name)
        .bind(position)
        .bind(fixture.owner.id.to_string())
        .bind(now)
        .bind(project.to_string())
        .execute(fixture.database.pool())
        .await
        .unwrap();
    }
    let created = fixture
        .tool(
            &token,
            "create_doc",
            json!({"title": "Plan", "teamspace_id": linked}),
        )
        .await;
    assert!(!not_available(&created), "{created}");
    for refused in [
        json!({"title": "X", "teamspace_id": other_space}),
        json!({"title": "X", "private": true}),
        json!({"title": "X"}),
    ] {
        let result = fixture.tool(&token, "create_doc", refused.clone()).await;
        assert!(not_available(&result), "{refused}: {result}");
    }
    let elsewhere_doc = {
        // The owner's doc in the other project's teamspace, made through the page repository.
        let pages = PageRepository::new((*fixture.database).clone());
        pages
            .create_page_with_content(
                fixture.workspace_id,
                fixture.owner.id,
                orbit_server::repositories::pages::CreatePage {
                    parent_id: None,
                    space: orbit_server::repositories::pages::SpaceRequest {
                        teamspace_id: Some(other_space),
                        private: None,
                    },
                    title: "Elsewhere doc".into(),
                    icon: None,
                    position: None,
                },
                &[],
                "test",
                TimestampMillis::now(),
            )
            .await
            .unwrap()
            .id
    };
    let refused = fixture
        .tool(&token, "get_doc", json!({"page_id": elsewhere_doc}))
        .await;
    assert!(not_available(&refused), "{refused}");
    let docs = fixture.tool(&token, "list_docs", json!({})).await;
    assert!(docs.to_string().contains("Plan"));
    assert!(!docs.to_string().contains("Elsewhere doc"), "{docs}");

    // Every write of the agent is in the audit log.
    assert!(
        fixture
            .count(
                "SELECT COUNT(*) FROM audit_events WHERE action = 'external_agent.write' AND actor_id = $1",
                bot,
            )
            .await
            >= 1
    );
}

#[tokio::test]
async fn a_doc_on_the_card_shows_no_titles_or_drive_names_from_outside_it() {
    let fixture = Fixture::new().await;
    let other_project = fixture.project("Other", "OTH", false).await;
    let agent = fixture
        .create_external(json!({
            "name": "Muse",
            "project_ids": [fixture.project_id],
            "scopes": ["docs:read", "docs:write"],
        }))
        .await;
    let token = fixture.token(&agent).await;
    let bot: Id = agent["user_id"].as_str().unwrap().parse().unwrap();
    let now = TimestampMillis::now().as_millis();
    let (linked, other_space) = (Id::new_v7(), Id::new_v7());
    for (id, name, project, position) in [
        (linked, "General docs", fixture.project_id, 10_i64),
        (other_space, "Elsewhere", other_project, 11),
    ] {
        sqlx::query(
            "INSERT INTO teamspaces (id, workspace_id, name, position, version, created_by, created_at, updated_at, project_id) \
             VALUES ($1, $2, $3, $4, 0, $5, $6, $6, $7)",
        )
        .bind(id.to_string())
        .bind(fixture.workspace_id.to_string())
        .bind(name)
        .bind(position)
        .bind(fixture.owner.id.to_string())
        .bind(now)
        .bind(project.to_string())
        .execute(fixture.database.pool())
        .await
        .unwrap();
    }
    // A Drive file of the workspace, which the bot user (a member) can see.
    let (folder, file, blob) = (Id::new_v7(), Id::new_v7(), Id::new_v7());
    sqlx::query(
        "INSERT INTO drive_blobs (id, workspace_id, r2_key, size, mime, status, created_at, updated_at) \
         VALUES ($1, $2, 'drive/salaries', 1, 'text/plain', 'ready', $3, $3)",
    )
    .bind(blob.to_string())
    .bind(fixture.workspace_id.to_string())
    .bind(now)
    .execute(fixture.database.pool())
    .await
    .unwrap();
    for (id, parent, kind, name, blob) in [
        (folder, None, "folder", "HR", None),
        (file, Some(folder), "file", "salaries-2026.xlsx", Some(blob)),
    ] {
        sqlx::query(
            "INSERT INTO drive_nodes (id, workspace_id, parent_id, kind, name, name_lower, blob_id, created_at, \
             updated_at) VALUES ($1, $2, $3, $4, $5, $5, $6, $7, $7)",
        )
        .bind(id.to_string())
        .bind(fixture.workspace_id.to_string())
        .bind(parent.map(|id: Id| id.to_string()))
        .bind(kind)
        .bind(name)
        .bind(blob.map(|id: Id| id.to_string()))
        .bind(now)
        .execute(fixture.database.pool())
        .await
        .unwrap();
    }
    let pages = PageRepository::new((*fixture.database).clone());
    let doc = |space: Id, title: &str, content: Vec<Value>| {
        let pages = &pages;
        let title = title.to_owned();
        async move {
            pages
                .create_page_with_content(
                    fixture.workspace_id,
                    fixture.owner.id,
                    orbit_server::repositories::pages::CreatePage {
                        parent_id: None,
                        space: orbit_server::repositories::pages::SpaceRequest {
                            teamspace_id: Some(space),
                            private: None,
                        },
                        title,
                        icon: None,
                        position: None,
                    },
                    &content,
                    "test",
                    TimestampMillis::now(),
                )
                .await
                .unwrap()
                .id
        }
    };
    let sibling = doc(linked, "Roadmap on the card", Vec::new()).await;
    let secret = doc(other_space, "Layoff plan", Vec::new()).await;
    let link = |id: Id, block: &str| {
        json!({"id": block, "type": "page", "props": {"pageId": id.to_string()}, "content": [], "children": []})
    };
    let shared = doc(
        linked,
        "Shared notes",
        vec![
            link(sibling, "b1"),
            link(secret, "b2"),
            orbit_server::repositories::drive_refs::drive_file_block(fixture.workspace_id, file),
        ],
    )
    .await;

    // The bot user alone would see both titles and the file's name.
    let visible = pages
        .pages(fixture.workspace_id, bot)
        .await
        .unwrap()
        .items
        .into_iter()
        .map(|page| page.title)
        .collect::<Vec<_>>();
    assert!(visible.contains(&"Layoff plan".to_owned()), "{visible:?}");
    let mut conn = fixture.database.pool().acquire().await.unwrap();
    let blocks = pages
        .get_page(fixture.workspace_id, shared, bot)
        .await
        .unwrap()
        .content;
    let refs = orbit_server::repositories::drive_refs::drive_refs(&blocks);
    let states = orbit_server::repositories::drive_refs::resolve_all(&mut conn, bot, &refs)
        .await
        .unwrap();
    assert!(
        states.values().all(|state| matches!(
            state,
            orbit_server::repositories::drive_refs::DriveRefState::Available(_)
        )),
        "{states:?}"
    );
    drop(conn);

    // Through the card: the title on the card, not the one outside, no Drive name.
    let read = fixture
        .tool(&token, "get_doc", json!({"page_id": shared}))
        .await;
    let markdown = read["structuredContent"]["markdown"].as_str().unwrap();
    assert!(markdown.contains("Roadmap on the card"), "{markdown}");
    assert!(markdown.contains(&secret.to_string()), "{markdown}");
    assert!(!markdown.contains("Layoff plan"), "{markdown}");
    assert!(!markdown.contains("salaries"), "{markdown}");
    assert!(!markdown.contains("HR"), "{markdown}");

    // Writing back what it read keeps the link and the file block as they are.
    let version = read["structuredContent"]["version"].clone();
    let updated = fixture
        .tool(
            &token,
            "update_doc",
            json!({"page_id": shared, "expected_version": version, "markdown": format!("{markdown}\n\nOne more line.")}),
        )
        .await;
    assert!(!not_available(&updated), "{updated}");
    let after = pages
        .get_page(fixture.workspace_id, shared, fixture.owner.id)
        .await
        .unwrap()
        .content;
    assert_eq!(
        orbit_server::repositories::drive_refs::drive_refs(&after).len(),
        1,
        "{after:?}"
    );
    assert!(after.iter().any(|block| block["props"]["pageId"] == secret.to_string()));
}

#[tokio::test]
async fn wait_for_messages_delivers_at_least_once_and_acks_only_its_own() {
    let fixture = Fixture::new().await;
    let channel = fixture.channel("ops", "public").await;
    let muse = fixture
        .create_external(json!({ "name": "Muse", "channel_ids": [channel] }))
        .await;
    let other = fixture
        .create_external(json!({ "name": "Echo", "channel_ids": [channel] }))
        .await;
    let (token, other_token) = (fixture.token(&muse).await, fixture.token(&other).await);
    let muse_id = muse["user_id"].as_str().unwrap();

    // Timeout without input: an empty list after about the timeout.
    let started = Instant::now();
    assert!(fixture.wait(&token, 1, &[]).await.is_empty());
    assert!(started.elapsed() >= Duration::from_millis(900));

    let message = fixture
        .send(
            &fixture.owner,
            &channel,
            &format!("<@{muse_id}> summarize this"),
        )
        .await;
    let events = fixture.wait(&token, 0, &[]).await;
    assert_eq!(events.len(), 1, "{events:?}");
    let event = &events[0];
    assert_eq!(event["type"], "app_mention");
    assert_eq!(event["conversation_id"], channel);
    // A mention in a channel is answered in a new thread under it.
    assert_eq!(event["thread_ts"], message);
    assert_eq!(event["author_id"], fixture.owner.id.to_string());
    assert!(event["text"].as_str().unwrap().contains("summarize this"));
    let event_id = event["event_id"].as_str().unwrap().to_owned();

    // Not acknowledged: it comes back. Another agent cannot acknowledge it.
    assert!(fixture.wait(&other_token, 0, &[&event_id]).await.is_empty());
    let again = fixture.wait(&token, 0, &[]).await;
    assert_eq!(again.len(), 1);
    assert_eq!(again[0]["event_id"], event_id.as_str());
    assert_eq!(again[0]["delivery_attempts"], 1);

    // The answer goes in the thread as the bot user; then the ack ends it.
    let sent = fixture
        .tool(
            &token,
            "send_message",
            json!({"conversation_id": channel, "text": "Here is the summary.", "thread_ts": message}),
        )
        .await;
    assert_eq!(sent["structuredContent"]["thread_ts"], message, "{sent}");
    let author: String = sqlx::query_scalar("SELECT author_id FROM chat_messages WHERE id = $1")
        .bind(sent["structuredContent"]["message_id"].as_str().unwrap())
        .fetch_one(fixture.database.pool())
        .await
        .unwrap();
    assert_eq!(author, muse_id);
    assert!(fixture.wait(&token, 0, &[&event_id]).await.is_empty());

    // A waiting call wakes up on a new input, well before its timeout.
    let waiting = {
        let fixture = &fixture;
        let token = token.clone();
        async move {
            let started = Instant::now();
            let events = fixture.wait(&token, 20, &[]).await;
            (events, started.elapsed())
        }
    };
    let post = async {
        tokio::time::sleep(Duration::from_millis(600)).await;
        fixture
            .send(&fixture.owner, &channel, &format!("<@{muse_id}> again"))
            .await
    };
    let ((events, elapsed), _) = tokio::join!(waiting, post);
    assert_eq!(events.len(), 1);
    assert!(elapsed < Duration::from_secs(5), "{elapsed:?}");

    // One open wait per credential: a second call ends the first one, empty.
    let first = async {
        fixture
            .wait(&token, 20, &[events[0]["event_id"].as_str().unwrap()])
            .await
    };
    let second = async {
        tokio::time::sleep(Duration::from_millis(700)).await;
        let started = Instant::now();
        let events = fixture.wait(&token, 1, &[]).await;
        (events, started.elapsed())
    };
    let started = Instant::now();
    let (first, _) = tokio::join!(first, second);
    assert!(first.is_empty());
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[tokio::test]
async fn mentions_outside_the_card_get_a_refusal_and_the_agent_never_joins() {
    let fixture = Fixture::new().await;
    let listed = fixture.channel("listed", "public").await;
    let public = fixture.channel("lobby", "public").await;
    let private = fixture.channel("board", "private").await;
    let muse = fixture
        .create_external(json!({ "name": "Muse", "channel_ids": [listed] }))
        .await;
    let token = fixture.token(&muse).await;
    let bot = muse["user_id"].as_str().unwrap();
    for channel in [&public, &private] {
        fixture
            .send(&fixture.owner, channel, &format!("<@{bot}> look here"))
            .await;
        assert_eq!(
            fixture
                .count(
                    &format!(
                        "SELECT COUNT(*) FROM chat_members WHERE conversation_id = '{channel}' AND user_id = $1"
                    ),
                    bot,
                )
                .await,
            0,
            "never joins by a mention"
        );
    }
    // Nothing reaches the agent; after the sidecar's grace time, the agent's own poll answers in
    // the author's DM.
    sqlx::query("UPDATE agent_inputs SET created_at = created_at - 10000 WHERE agent_user_id = $1")
        .bind(bot)
        .execute(fixture.database.pool())
        .await
        .unwrap();
    assert!(fixture.wait(&token, 0, &[]).await.is_empty());
    let refusals: Vec<String> = sqlx::query_scalar(
        "SELECT m.body FROM chat_messages m JOIN chat_conversations c ON c.id = m.conversation_id \
         WHERE c.kind = 'dm' AND m.author_id = $1",
    )
    .bind(bot)
    .fetch_all(fixture.database.pool())
    .await
    .unwrap();
    assert_eq!(refusals.len(), 2, "{refusals:?}");
    assert!(refusals[0].contains("#listed"));
    assert_eq!(
        fixture
            .count(
                "SELECT COUNT(*) FROM agent_inputs WHERE agent_user_id = $1 AND acked_at IS NULL",
                bot,
            )
            .await,
        0
    );

    // A guest's mention, even in a listed channel, never reaches it.
    let guest = fixture.add_person("Gus", "guest").await;
    sqlx::query(
        "INSERT INTO chat_members (conversation_id, user_id, workspace_id, notify, joined_at, read_count) \
         VALUES ($1, $2, $3, 'all', 0, 0)",
    )
    .bind(&listed)
    .bind(guest.id.to_string())
    .bind(fixture.workspace_id.to_string())
    .execute(fixture.database.pool())
    .await
    .unwrap();
    fixture
        .send(&guest, &listed, &format!("<@{bot}> do my work"))
        .await;
    assert!(fixture.wait(&token, 0, &[]).await.is_empty());
}

#[tokio::test]
async fn the_agents_messages_and_task_writes_start_nothing() {
    let fixture = Fixture::new().await;
    let channel = fixture.channel("ops", "public").await;
    let muse = fixture
        .create_external(json!({
            "name": "Muse",
            "channel_ids": [channel],
            "project_ids": [fixture.project_id],
            "scopes": ["chat:write", "tasks:write"],
        }))
        .await;
    let echo = fixture
        .create_external(json!({ "name": "Echo", "channel_ids": [channel], "project_ids": [fixture.project_id] }))
        .await;
    let token = fixture.token(&muse).await;
    let echo_id = echo["user_id"].as_str().unwrap();
    let muse_id = muse["user_id"].as_str().unwrap();
    let sent = fixture
        .tool(
            &token,
            "send_message",
            json!({"conversation_id": channel, "text": format!("<@{echo_id}> and <@{muse_id}> ping")}),
        )
        .await;
    assert!(!not_available(&sent), "{sent}");
    assert!(not_available(
        &fixture
            .tool(
                &token,
                "send_message",
                json!({"conversation_id": channel, "text": "<!channel> all"})
            )
            .await
    ));
    let task = fixture.task(fixture.project_id, "Shared").await;
    let view = fixture
        .tool(&token, "get_task", json!({"task_id": task}))
        .await;
    let updated = fixture
        .tool(
            &token,
            "update_task",
            json!({"task_id": task, "expected_version": view["structuredContent"]["version"], "assignee_ids": [echo_id]}),
        )
        .await;
    assert!(!not_available(&updated), "{updated}");
    fixture
        .tool(
            &token,
            "create_comment",
            json!({"task_id": task, "body": format!("<@{echo_id}> over to you")}),
        )
        .await;
    assert_eq!(
        fixture
            .count(
                "SELECT COUNT(*) FROM agent_inputs WHERE workspace_id = $1",
                &fixture.workspace_id.to_string()
            )
            .await,
        0
    );
    assert_eq!(
        fixture
            .count(
                "SELECT COUNT(*) FROM agent_task_runs WHERE workspace_id = $1",
                &fixture.workspace_id.to_string()
            )
            .await,
        0
    );
}

#[tokio::test]
async fn deleting_the_agent_revokes_its_tokens_grants_and_inputs() {
    let fixture = Fixture::new().await;
    let channel = fixture.channel("ops", "public").await;
    let muse = fixture
        .create_external(json!({ "name": "Muse", "channel_ids": [channel] }))
        .await;
    let token = fixture.token(&muse).await;
    let bot = muse["user_id"].as_str().unwrap();
    let id = muse["id"].as_str().unwrap();
    fixture
        .send(&fixture.owner, &channel, &format!("<@{bot}> pending work"))
        .await;
    let (status, _) = fixture
        .api(&fixture.owner, "DELETE", &format!("/agents/{id}"), None)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = fixture.rpc(&token, "tools/list", json!({})).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        fixture
            .count(
                "SELECT COUNT(*) FROM agent_tokens WHERE agent_id = $1 AND revoked_at IS NULL",
                id
            )
            .await,
        0
    );
    assert_eq!(
        fixture
            .count(
                "SELECT COUNT(*) FROM agent_inputs WHERE agent_user_id = $1 AND acked_at IS NULL",
                bot,
            )
            .await,
        0
    );
}

/// The authorization code of a consent request, approved with `body` by `member`.
async fn consent_code(
    fixture: &Fixture,
    member: &Member,
    client: &str,
    body: Value,
) -> (StatusCode, Value) {
    let challenge =
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(VERIFIER));
    let query = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs([
            ("client_id", client),
            ("response_type", "code"),
            ("redirect_uri", CALLBACK),
            ("code_challenge", challenge.as_str()),
            ("code_challenge_method", "S256"),
            ("state", "s"),
            ("resource", "https://orbit.test/mcp"),
        ])
        .finish();
    let response = fixture
        .app
        .clone()
        .oneshot(
            Request::get(format!("/oauth/authorize?{query}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let location = url::Url::parse(response.headers()[header::LOCATION].to_str().unwrap()).unwrap();
    let request = location
        .query_pairs()
        .find(|(key, _)| key == "request")
        .unwrap()
        .1
        .to_string();
    let (status, consent) = fixture
        .call(
            member,
            "GET",
            &format!("/api/v1/oauth/consent/{request}"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{consent}");
    let mut body = body;
    body["csrf_token"] = consent["csrf_token"].clone();
    body["approved"] = json!(true);
    let (status, approved) = fixture
        .call(
            member,
            "POST",
            &format!("/api/v1/oauth/consent/{request}"),
            Some(body),
        )
        .await;
    (status, json!({"consent": consent, "approved": approved}))
}

#[tokio::test]
async fn owners_connect_a_client_as_the_agent_and_members_cannot() {
    let fixture = Fixture::new().await;
    let channel = fixture.channel("ops", "public").await;
    let muse = fixture
        .create_external(json!({ "name": "Muse", "channel_ids": [channel] }))
        .await;
    let agent_id = muse["id"].as_str().unwrap();
    let (status, registered) = fixture
        .send_request(
            Request::post("/oauth/register")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json!({"client_name": "Claude Code", "redirect_uris": [CALLBACK], "token_endpoint_auth_method": "none"}).to_string()))
                .unwrap(),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{registered}");
    let client = registered["client_id"].as_str().unwrap();

    // A member sees no agent to connect as, and cannot approve one.
    let member = fixture.add_person("Mia", "member").await;
    let (status, result) =
        consent_code(&fixture, &member, client, json!({"agent_id": agent_id})).await;
    assert_eq!(result["consent"]["agents"], json!([]));
    assert_eq!(status, StatusCode::BAD_REQUEST, "{result}");

    let (status, result) = consent_code(
        &fixture,
        &fixture.owner,
        client,
        json!({"agent_id": agent_id}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["consent"]["agents"][0]["name"], "Muse");
    assert_eq!(result["consent"]["agents"][0]["channels"], json!(["ops"]));
    let callback = url::Url::parse(result["approved"]["redirect_uri"].as_str().unwrap()).unwrap();
    let code = callback
        .query_pairs()
        .find(|(key, _)| key == "code")
        .unwrap()
        .1
        .to_string();
    let form = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs([
            ("grant_type", "authorization_code"),
            ("client_id", client),
            ("code", code.as_str()),
            ("redirect_uri", CALLBACK),
            ("code_verifier", VERIFIER),
            ("resource", "https://orbit.test/mcp"),
        ])
        .finish();
    let (status, issued) = fixture
        .send_request(
            Request::post("/oauth/token")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(form))
                .unwrap(),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{issued}");
    let access = issued["access_token"].as_str().unwrap();
    let me = fixture.tool(access, "get_me", json!({})).await;
    assert_eq!(me["structuredContent"]["user"]["id"], muse["user_id"]);
    let grant: (String, Option<String>) =
        sqlx::query_as("SELECT user_id, approved_by FROM oauth_grants WHERE agent_profile_id = $1")
            .bind(agent_id)
            .fetch_one(fixture.database.pool())
            .await
            .unwrap();
    assert_eq!(grant.0, muse["user_id"].as_str().unwrap());
    assert_eq!(grant.1, Some(fixture.owner.id.to_string()));

    // The card is the limit, read on each request: without chat:write, no send_message.
    assert!(
        fixture
            .tool_names(access)
            .await
            .iter()
            .any(|name| name == "send_message")
    );
    let (status, _) = fixture
        .api(
            &fixture.owner,
            "PATCH",
            &format!("/agents/{agent_id}/external"),
            Some(json!({ "scopes": ["chat:read"] })),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        !fixture
            .tool_names(access)
            .await
            .iter()
            .any(|name| name == "send_message")
    );

    // Deleting the agent ends the connection.
    let (status, _) = fixture
        .api(
            &fixture.owner,
            "DELETE",
            &format!("/agents/{agent_id}"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = fixture.rpc(access, "tools/list", json!({})).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let revoked: Option<i64> =
        sqlx::query_scalar("SELECT revoked_at FROM oauth_grants WHERE agent_profile_id = $1")
            .bind(agent_id)
            .fetch_one(fixture.database.pool())
            .await
            .unwrap();
    assert!(revoked.is_some());
}
