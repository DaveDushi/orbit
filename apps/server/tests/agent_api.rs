//! The agent's side in Orbit: which chat messages become agent inputs, the bot's messages, agent
//! sessions and their scope, and sealed ChatGPT credentials. Runs on SQLite, and on Postgres with
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
use orbit_server::repositories::identity::{IdentityRepository, SetupRequest};
use orbit_server::task_routes::{TaskState, task_router};
use serde_json::{Value, json};
use tower::ServiceExt;

const SECRET: &str = "test-agent-secret-0123456789abcdef0123456789";

struct Fixture {
    _root: tempfile::TempDir,
    database: TestDatabase,
    identity: Arc<IdentityRepository>,
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
        // Nothing listens here: the login routes see an offline sidecar.
        Self::with_sidecar("http://127.0.0.1:9").await
    }

    async fn with_sidecar(sidecar_url: &str) -> Self {
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
                url: sidecar_url.to_owned(),
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
            .merge(orbit_server::agent::router(agent))
            .layer(axum::middleware::from_fn_with_state(
                (*database).clone(),
                orbit_server::agent::scope::layer,
            ));
        Self {
            _root: root,
            database,
            identity,
            app,
            workspace_id: setup.workspace_id.to_string(),
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
        .bind(&self.workspace_id)
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

    async fn chat(&self, member: &Member, method: &str, path: &str, body: Option<Value>) -> Value {
        let uri = format!("/api/v1/workspaces/{}/chat{path}", self.workspace_id);
        let (status, value) = self
            .request(method, &uri, ("cookie", &member.cookie), body)
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

    async fn send(
        &self,
        member: &Member,
        conversation: &str,
        body: &str,
        root: Option<&str>,
    ) -> String {
        let nonce = format!("n{}", self.nonce.fetch_add(1, Ordering::Relaxed));
        self.chat(
            member,
            "POST",
            &format!("/conversations/{conversation}/messages"),
            Some(json!({ "body": body, "thread_root_id": root, "nonce": nonce })),
        )
        .await["result"]["id"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    /// The queued inputs as `(message_id, kind, thread_key)`; also makes the bot.
    async fn pending(&self) -> (String, Vec<(String, String, String)>) {
        let (status, value) = self.internal("GET", "/inputs", None).await;
        assert_eq!(status, StatusCode::OK, "{value}");
        let inputs = value["inputs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|input| {
                (
                    input["message_id"].as_str().unwrap().to_owned(),
                    input["kind"].as_str().unwrap().to_owned(),
                    input["thread_key"].as_str().unwrap().to_owned(),
                )
            })
            .collect();
        (value["bot_user_id"].as_str().unwrap().to_owned(), inputs)
    }

    async fn set_thread(&self, key: &str, conversation: &str, status: &str, driver: &Member) {
        let (code, value) = self
            .internal(
                "PUT",
                &format!("/threads/{key}"),
                Some(json!({
                    "workspace_id": self.workspace_id,
                    "conversation_id": conversation,
                    "root_message_id": key,
                    "status": status,
                    "driver_user_id": driver.id.to_string(),
                    "waiting_for_user_id": null,
                })),
            )
            .await;
        assert_eq!(code, StatusCode::NO_CONTENT, "{value}");
    }
}

#[tokio::test]
async fn internal_routes_need_the_secret() {
    let fixture = Fixture::new().await;
    for secret in ["", "wrong"] {
        let (status, _) = fixture
            .request(
                "GET",
                "/api/internal/agent/inputs",
                ("authorization", &format!("Bearer {secret}")),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
    let (status, value) = fixture
        .internal(
            "GET",
            &format!("/workspaces/{}/people", fixture.workspace_id),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(value["people"][0]["name"], "Owner");
    // A person's session is not the secret either.
    let (status, _) = fixture
        .request(
            "GET",
            "/api/internal/agent/inputs",
            ("cookie", &fixture.owner.cookie),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn mentions_answers_and_steers_become_inputs() {
    let fixture = Fixture::new().await;
    let (bot, none) = fixture.pending().await;
    assert!(none.is_empty());
    let teammate = fixture.add_member("teammate").await;
    let general = fixture.general().await;

    // Plain channel messages are not for the agent.
    fixture
        .send(&fixture.owner, &general, "hello team", None)
        .await;
    let mention = fixture
        .send(
            &fixture.owner,
            &general,
            &format!("<@{bot}> make a task"),
            None,
        )
        .await;
    let (_, inputs) = fixture.pending().await;
    assert_eq!(
        inputs,
        vec![(mention.clone(), "instruction".to_owned(), mention.clone())]
    );

    // While the run goes, only the driver's plain replies count.
    fixture
        .set_thread(&mention, &general, "running", &fixture.owner)
        .await;
    fixture
        .send(&teammate, &general, "just chatting", Some(&mention))
        .await;
    let steer = fixture
        .send(&fixture.owner, &general, "also add a label", Some(&mention))
        .await;
    // While it waits for an answer, any reply is the answer.
    fixture
        .set_thread(&mention, &general, "waiting", &fixture.owner)
        .await;
    let answer = fixture
        .send(&teammate, &general, "use project General", Some(&mention))
        .await;
    // Once idle, plain replies are ignored again; a mention is always an instruction.
    fixture
        .set_thread(&mention, &general, "idle", &fixture.owner)
        .await;
    fixture
        .send(&teammate, &general, "thanks", Some(&mention))
        .await;
    let again = fixture
        .send(
            &teammate,
            &general,
            &format!("<@{bot}> now close it"),
            Some(&mention),
        )
        .await;

    let (_, inputs) = fixture.pending().await;
    let kinds: Vec<(String, String)> = inputs
        .iter()
        .map(|(id, kind, key)| {
            assert_eq!(key, &mention, "every input of a thread has its root as key");
            (id.clone(), kind.clone())
        })
        .collect();
    assert_eq!(
        kinds,
        vec![
            (mention.clone(), "instruction".to_owned()),
            (steer, "steer".to_owned()),
            (answer, "answer".to_owned()),
            (again, "instruction".to_owned()),
        ]
    );

    let (status, _) = fixture
        .internal("POST", &format!("/inputs/{mention}/ack"), None)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, inputs) = fixture.pending().await;
    assert_eq!(inputs.len(), 3);
    assert!(inputs.iter().all(|(id, _, _)| id != &mention));
}

#[tokio::test]
async fn a_mention_brings_the_agent_into_a_private_channel_and_it_replies_in_the_thread() {
    let fixture = Fixture::new().await;
    let (bot, _) = fixture.pending().await;
    let channel = fixture
        .chat(
            &fixture.owner,
            "POST",
            "/conversations",
            Some(json!({ "name": "secret", "kind": "private", "member_ids": [] })),
        )
        .await["result"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let mention = fixture
        .send(&fixture.owner, &channel, &format!("<@{bot}> hi"), None)
        .await;
    let (_, inputs) = fixture.pending().await;
    assert_eq!(inputs.len(), 1);

    let post = json!({
        "workspace_id": fixture.workspace_id,
        "conversation_id": channel,
        "root_message_id": mention,
        "body": "Working on it",
        "nonce": format!("agent:{mention}"),
    });
    let (status, first) = fixture
        .internal("POST", "/messages", Some(post.clone()))
        .await;
    assert_eq!(status, StatusCode::OK, "{first}");
    // Sent again with the same nonce, it is the same message.
    let (_, second) = fixture.internal("POST", "/messages", Some(post)).await;
    assert_eq!(first["id"], second["id"]);
    let id = first["id"].as_str().unwrap();
    let (status, _) = fixture
        .internal(
            "PATCH",
            &format!("/messages/{id}"),
            Some(json!({ "workspace_id": fixture.workspace_id, "body": "Done" })),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let thread = fixture
        .chat(&fixture.owner, "GET", &format!("/threads/{mention}"), None)
        .await;
    let replies = thread["items"].as_array().unwrap();
    assert_eq!(replies.len(), 1);
    assert_eq!(replies[0]["author_id"], bot);
    assert_eq!(replies[0]["body"], "Done");
    // The agent's own messages are never inputs.
    let (_, inputs) = fixture.pending().await;
    assert_eq!(inputs.len(), 1);
}

#[tokio::test]
async fn agent_sessions_act_as_the_driver_and_only_on_work_routes() {
    let fixture = Fixture::new().await;
    let (bot, _) = fixture.pending().await;
    let member = fixture.add_member("member").await;
    let general = fixture.general().await;
    let issue = |user: &Member| json!({ "user_id": user.id.to_string(), "workspace_id": fixture.workspace_id });

    // No run, no session.
    let (status, _) = fixture
        .internal("POST", "/sessions", Some(issue(&member)))
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let mention = fixture
        .send(&member, &general, &format!("<@{bot}> list tasks"), None)
        .await;
    fixture
        .set_thread(&mention, &general, "running", &member)
        .await;
    // Only the driver gets one.
    let (status, _) = fixture
        .internal("POST", "/sessions", Some(issue(&fixture.owner)))
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, session) = fixture
        .internal("POST", "/sessions", Some(issue(&member)))
        .await;
    assert_eq!(status, StatusCode::OK, "{session}");
    let token = session["token"].as_str().unwrap();
    assert!(token.starts_with("orbit-agent."));
    let bearer = format!("Bearer {token}");
    let auth = ("authorization", bearer.as_str());
    let workspace = &fixture.workspace_id;

    // Work routes of its workspace answer as the member.
    let (status, value) = fixture
        .request(
            "GET",
            &format!("/api/v1/workspaces/{workspace}/tasks"),
            auth,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{value}");
    let (status, written) = fixture
        .request(
            "POST",
            &format!("/api/v1/workspaces/{workspace}/chat/conversations/{general}/messages"),
            auth,
            Some(json!({ "body": "as the member", "nonce": "agent-session" })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{written}");
    assert_eq!(written["result"]["author_id"], member.id.to_string());

    // The member's own limits still hold: the owner's message is not theirs to edit.
    let owners = fixture
        .send(&fixture.owner, &general, "owner's words", None)
        .await;
    let (status, _) = fixture
        .request(
            "PATCH",
            &format!("/api/v1/workspaces/{workspace}/chat/messages/{owners}"),
            auth,
            Some(json!({ "body": "changed by the agent" })),
        )
        .await;
    assert!(status.is_client_error(), "{status}");

    // Account routes and other workspaces are out of scope.
    for (method, path) in [
        ("GET", "/api/v1/agent".to_owned()),
        ("DELETE", "/api/v1/agent/chatgpt".to_owned()),
        ("GET", format!("/api/v1/workspaces/{}/tasks", Id::new_v7())),
        ("POST", format!("/api/v1/workspaces/{workspace}/api-tokens")),
    ] {
        let (status, value) = fixture.request(method, &path, auth, None).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {path}");
        assert_eq!(value["code"], "agent_scope");
    }
    // A made-up agent token is refused before any route.
    let (status, _) = fixture
        .request(
            "GET",
            &format!("/api/v1/workspaces/{workspace}/tasks"),
            ("authorization", "Bearer orbit-agent.made-up"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn credentials_are_sealed_and_only_the_status_reaches_the_person() {
    let fixture = Fixture::new().await;
    let user = fixture.owner.id;
    let credential = json!({ "type": "oauth", "access": "access-token-value", "refresh": "refresh-token-value", "expires": 1 });
    let (status, _) = fixture
        .internal(
            "PUT",
            &format!("/credentials/{user}"),
            Some(json!({ "credential": credential, "account_id": "acct-1" })),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let sealed: Vec<u8> =
        sqlx::query_scalar("SELECT sealed FROM agent_credentials WHERE user_id = $1")
            .bind(user.to_string())
            .fetch_one(fixture.database.pool())
            .await
            .unwrap();
    assert!(!String::from_utf8_lossy(&sealed).contains("access-token-value"));

    let (_, read) = fixture
        .internal("GET", &format!("/credentials/{user}"), None)
        .await;
    assert_eq!(read["credential"], credential);

    let (status, value) = fixture
        .request(
            "GET",
            "/api/v1/agent",
            ("cookie", &fixture.owner.cookie),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{value}");
    assert_eq!(value["chatgpt"]["connected"], true);
    assert_eq!(value["chatgpt"]["account_id"], "acct-1");
    assert_eq!(value["online"], false);
    assert!(!value.to_string().contains("access-token-value"));

    // A failed refresh makes the person connect again.
    fixture
        .internal("POST", &format!("/credentials/{user}/invalid"), None)
        .await;
    let (_, read) = fixture
        .internal("GET", &format!("/credentials/{user}"), None)
        .await;
    assert_eq!(read["credential"], Value::Null);
    let (_, value) = fixture
        .request(
            "GET",
            "/api/v1/agent",
            ("cookie", &fixture.owner.cookie),
            None,
        )
        .await;
    assert_eq!(value["chatgpt"]["invalid"], true);

    let (status, _) = fixture
        .request(
            "DELETE",
            "/api/v1/agent/chatgpt",
            ("cookie", &fixture.owner.cookie),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, value) = fixture
        .request(
            "GET",
            "/api/v1/agent",
            ("cookie", &fixture.owner.cookie),
            None,
        )
        .await;
    assert_eq!(value["chatgpt"]["connected"], false);
}

// ---------------------------------------------------------------------------------------------
// Memory.

impl Fixture {
    /// An agent session of `member`, who drives a run in #general.
    async fn agent_session(&self, member: &Member) -> String {
        let (bot, _) = self.pending().await;
        let general = self.general().await;
        let mention = self
            .send(member, &general, &format!("<@{bot}> remember"), None)
            .await;
        self.set_thread(&mention, &general, "running", member).await;
        let (status, session) = self
            .internal(
                "POST",
                "/sessions",
                Some(
                    json!({ "user_id": member.id.to_string(), "workspace_id": self.workspace_id }),
                ),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{session}");
        format!("Bearer {}", session["token"].as_str().unwrap())
    }

    async fn memory(
        &self,
        auth: (&str, &str),
        method: &str,
        rest: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let uri = format!(
            "/api/v1/workspaces/{}/agent-memory/{rest}",
            self.workspace_id
        );
        self.request(method, &uri, auth, body).await
    }

    async fn note(&self, member: &Member, scope: &str, body: &str) -> Value {
        let (status, value) = self
            .memory(
                ("cookie", &member.cookie),
                "POST",
                scope,
                Some(json!({ "body": body })),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{value}");
        value
    }

    async fn wake(&self, member: &Member, scope: &str, bytes: usize) -> Value {
        let (status, value) = self
            .memory(
                ("cookie", &member.cookie),
                "GET",
                &format!("{scope}/wake?bytes={bytes}"),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{value}");
        value
    }

    async fn summarize(&self, member: &Member, block: &str, claim: &Value, body: &str) -> Value {
        let (status, value) = self
            .memory(
                ("cookie", &member.cookie),
                "POST",
                "team/summaries",
                Some(json!({ "block": block, "claim": claim, "body": body })),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{value}");
        value
    }
}

#[tokio::test]
async fn private_memory_is_only_its_owners_and_team_memory_only_the_workspaces() {
    let fixture = Fixture::new().await;
    let member = fixture.add_member("member").await;
    let owner = fixture.owner.clone();
    fixture
        .note(&owner, "team", "Releases ship on Thursdays")
        .await;
    fixture
        .note(&owner, "private", "Owner prefers short answers")
        .await;
    fixture
        .note(&member, "private", "Member is on leave in May")
        .await;

    // Every member reads the team memory; `private` is always the caller's own.
    let (_, team) = fixture
        .memory(("cookie", &member.cookie), "GET", "team", None)
        .await;
    assert_eq!(team["items"][0]["body"], "Releases ship on Thursdays");
    assert_eq!(team["items"][0]["author_name"], "Owner");
    assert_eq!(team["can_delete"], false);
    let (_, mine) = fixture
        .memory(("cookie", &member.cookie), "GET", "private", None)
        .await;
    assert_eq!(mine["items"].as_array().unwrap().len(), 1);
    assert_eq!(mine["items"][0]["body"], "Member is on leave in May");
    assert_eq!(mine["can_delete"], true);
    let view = fixture.wake(&member, "private", 4_000).await;
    assert!(!view["text"].as_str().unwrap().contains("Owner prefers"));
    let (_, search) = fixture
        .memory(
            ("cookie", &member.cookie),
            "GET",
            "private?query=short%20answers",
            None,
        )
        .await;
    assert_eq!(search["items"].as_array().unwrap().len(), 0);

    // A member cannot delete team memory; the owner can. Nobody reaches another's private one.
    let (status, _) = fixture
        .memory(
            ("cookie", &member.cookie),
            "DELETE",
            "team/memories/0",
            None,
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = fixture
        .memory(
            ("cookie", &member.cookie),
            "DELETE",
            "private/memories/1",
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "the member has only #0");

    // Outsiders and other workspaces get nothing.
    let outsider = fixture.add_member("outsider").await;
    sqlx::query("DELETE FROM memberships WHERE user_id = $1")
        .bind(outsider.id.to_string())
        .execute(fixture.database.pool())
        .await
        .unwrap();
    for rest in ["team", "team/wake", "private"] {
        let (status, _) = fixture
            .memory(("cookie", &outsider.cookie), "GET", rest, None)
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{rest}");
    }
    let (status, _) = fixture
        .request(
            "GET",
            &format!("/api/v1/workspaces/{}/agent-memory/team", Id::new_v7()),
            ("cookie", &member.cookie),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // The agent acts as its driver: it reads the driver's private memory, never the owner's,
    // notes with the driver as author, and cannot delete.
    let bearer = fixture.agent_session(&member).await;
    let agent = ("authorization", bearer.as_str());
    let (status, view) = fixture
        .memory(agent, "GET", "private/wake?bytes=4000", None)
        .await;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert!(view["text"].as_str().unwrap().contains("on leave in May"));
    assert!(!view["text"].as_str().unwrap().contains("Owner prefers"));
    let general = fixture.general().await;
    let (status, noted) = fixture
        .memory(
            agent,
            "POST",
            "team",
            Some(json!({ "body": "Staging is at staging.example.com", "source": { "conversation_id": general } })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{noted}");
    let (_, team) = fixture
        .memory(("cookie", &owner.cookie), "GET", "team", None)
        .await;
    assert_eq!(team["items"][0]["author_id"], member.id.to_string());
    assert_eq!(team["items"][0]["source"]["conversation_id"], general);
    for rest in [
        "private/memories/0",
        "team/memories/0",
        "team/summaries/0-1",
    ] {
        let (status, value) = fixture.memory(agent, "DELETE", rest, None).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{rest}");
        assert_eq!(value["code"], "agent_scope");
    }
    // A source from another workspace is refused.
    let (status, _) = fixture
        .memory(
            agent,
            "POST",
            "team",
            Some(json!({ "body": "x", "source": { "conversation_id": Id::new_v7().to_string() } })),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn summaries_are_claimed_once_and_deleting_a_memory_drops_its_summaries() {
    let fixture = Fixture::new().await;
    let owner = fixture.owner.clone();
    let member = fixture.add_member("member").await;

    assert_eq!(
        fixture
            .note(&owner, "team", "m0 deploys use blue-green")
            .await["due"],
        Value::Null
    );
    let first = fixture
        .note(&owner, "team", "m1 the API freeze is on Friday")
        .await;
    assert_eq!(first["position"], 1);
    assert_eq!(
        (first["due"]["lo"].clone(), first["due"]["hi"].clone()),
        (json!(0), json!(1))
    );
    assert_eq!(
        first["due"]["halves"][1]["body"],
        "m1 the API freeze is on Friday"
    );
    let claim_a = first["due"]["claim"].clone();

    // While #0-1 is claimed, nobody else is asked for it.
    assert_eq!(
        fixture.note(&member, "team", "m2 Ann owns billing").await["due"],
        Value::Null
    );
    let third = fixture.note(&member, "team", "m3 Bob owns search").await;
    assert_eq!(third["due"]["lo"], 2);
    let claim_b = third["due"]["claim"].clone();

    // Only the claim's holder saves; #0-3 waits for both halves.
    let wrong = fixture.summarize(&member, "0-1", &claim_b, "stolen").await;
    assert_eq!(wrong["saved"], false);
    let saved = fixture
        .summarize(&owner, "0-1", &claim_a, "S01 blue-green; freeze Friday")
        .await;
    assert_eq!(saved["saved"], true);
    assert_eq!(saved["due"], Value::Null, "#2-3 is claimed by the member");
    let saved = fixture
        .summarize(&member, "2-3", &claim_b, "S23 Ann billing, Bob search")
        .await;
    assert_eq!(saved["saved"], true);
    assert_eq!(saved["due"]["lo"], 0);
    assert_eq!(saved["due"]["hi"], 3);
    let top = fixture
        .summarize(
            &member,
            "0-3",
            &saved["due"]["claim"],
            "S03 process and owners",
        )
        .await;
    assert_eq!(top["saved"], true);
    let again = fixture
        .summarize(&member, "0-3", &saved["due"]["claim"], "twice")
        .await;
    assert_eq!(again["saved"], false, "a claim is used once");

    // A big budget reads every memory; a small one reads the summary.
    let full = fixture.wake(&owner, "team", 4_000).await;
    assert_eq!(full["lines"].as_array().unwrap().len(), 4);
    let small = fixture.wake(&owner, "team", 40).await;
    assert_eq!(small["text"], "#0-3 S03 process and owners");
    let (_, zoom) = fixture
        .memory(("cookie", &owner.cookie), "GET", "team/summaries/0-3", None)
        .await;
    assert_eq!(zoom["lines"][0]["body"], "S01 blue-green; freeze Friday");
    assert_eq!(zoom["lines"][1]["body"], "S23 Ann billing, Bob search");

    // Deleting #1 erases it and drops #0-1 and #0-3, which come due again.
    let (status, _) = fixture
        .memory(("cookie", &owner.cookie), "DELETE", "team/memories/1", None)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let body: String = sqlx::query_scalar(
        "SELECT m.body FROM agent_memories m JOIN agent_memory_scopes s ON s.id = m.scope_id \
         WHERE s.owner_user_id IS NULL AND m.position = 1",
    )
    .fetch_one(fixture.database.pool())
    .await
    .unwrap();
    assert_eq!(body, "");
    let view = fixture.wake(&owner, "team", 200).await;
    let text = view["text"].as_str().unwrap();
    assert!(
        text.contains("m0 deploys") && text.contains("m3 Bob"),
        "{text}"
    );
    assert!(!text.contains("freeze") && !text.contains("S03"), "{text}");
    let (_, list) = fixture
        .memory(("cookie", &owner.cookie), "GET", "team", None)
        .await;
    assert_eq!(list["items"].as_array().unwrap().len(), 3);
    let next = fixture
        .note(&owner, "team", "m4 the office closes at 6")
        .await;
    assert_eq!(
        (next["due"]["lo"].clone(), next["due"]["hi"].clone()),
        (json!(0), json!(1))
    );
    assert_eq!(next["due"]["halves"][1]["body"], "(deleted)");

    // Forgetting a summary drops it and its parents.
    let (status, _) = fixture
        .memory(
            ("cookie", &owner.cookie),
            "DELETE",
            "team/summaries/2-3",
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let left: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM agent_memory_summaries WHERE body IS NOT NULL")
            .fetch_one(fixture.database.pool())
            .await
            .unwrap();
    assert_eq!(left, 0);
}

#[tokio::test]
async fn parallel_notes_get_every_position_once_and_each_block_one_claim() {
    let fixture = Fixture::new().await;
    let member = fixture.add_member("member").await;
    let people = [fixture.owner.clone(), member];
    let mut jobs = Vec::new();
    for index in 0..24 {
        let app = fixture.app.clone();
        let cookie = people[index % 2].cookie.clone();
        let uri = format!(
            "/api/v1/workspaces/{}/agent-memory/team",
            fixture.workspace_id
        );
        jobs.push(tokio::spawn(async move {
            let request = Request::builder()
                .method("POST")
                .uri(uri)
                .header("cookie", cookie)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({ "body": format!("fact {index}") }).to_string(),
                ))
                .unwrap();
            let response = app.oneshot(request).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            serde_json::from_slice::<Value>(&bytes).unwrap()
        }));
    }
    let mut positions = Vec::new();
    let mut claimed = Vec::new();
    for job in jobs {
        let noted = job.await.unwrap();
        positions.push(noted["position"].as_u64().unwrap());
        if noted["due"].is_object() {
            claimed.push((
                noted["due"]["lo"].as_u64().unwrap(),
                noted["due"]["hi"].as_u64().unwrap(),
            ));
        }
    }
    positions.sort_unstable();
    assert_eq!(positions, (0..24).collect::<Vec<_>>());
    let mut unique = claimed.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), claimed.len(), "{claimed:?}");
    // All 12 pairs were handed out, each once.
    assert_eq!(unique.iter().filter(|(lo, hi)| hi - lo == 1).count(), 12);
}

#[tokio::test]
async fn a_member_who_leaves_loses_their_private_memory_and_the_team_keeps_theirs() {
    let fixture = Fixture::new().await;
    let member = fixture.add_member("member").await;
    fixture.note(&member, "private", "member's own note").await;
    fixture.note(&member, "team", "a work fact").await;
    let membership: String = sqlx::query_scalar("SELECT id FROM memberships WHERE user_id = $1")
        .bind(member.id.to_string())
        .fetch_one(fixture.database.pool())
        .await
        .unwrap();
    orbit_server::repositories::workspaces::WorkspaceRepository::new((*fixture.database).clone())
        .remove_member(
            fixture.workspace_id.parse().unwrap(),
            membership.parse().unwrap(),
            fixture.owner.id,
            0,
            "test",
            TimestampMillis::now(),
        )
        .await
        .unwrap();
    let private: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM agent_memories m JOIN agent_memory_scopes s ON s.id = m.scope_id \
         WHERE s.owner_user_id = $1",
    )
    .bind(member.id.to_string())
    .fetch_one(fixture.database.pool())
    .await
    .unwrap();
    assert_eq!(private, 0);
    let (_, team) = fixture
        .memory(("cookie", &fixture.owner.cookie), "GET", "team", None)
        .await;
    assert_eq!(team["items"][0]["body"], "a work fact");
    assert_eq!(team["items"][0]["author_name"], "member");
}

// ---------------------------------------------------------------------------------------------
// Name and team instructions.

impl Fixture {
    async fn cookie(
        &self,
        member: &Member,
        method: &str,
        uri: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        self.request(method, uri, ("cookie", &member.cookie), body)
            .await
    }
}

#[tokio::test]
async fn installation_admins_rename_the_agent_but_not_to_a_members_name() {
    let fixture = Fixture::new().await;
    let (bot, _) = fixture.pending().await;
    let member = fixture.add_member("Teammate").await;
    let rename = |name: &str| Some(json!({ "name": name }));

    let (status, value) = fixture
        .cookie(&fixture.owner, "GET", "/api/v1/admin/agent", None)
        .await;
    assert_eq!(status, StatusCode::OK, "{value}");
    assert_eq!(value["name"], "Orbit Agent");
    assert_eq!(value["bot_user_id"], bot.as_str());

    // Only an installation admin, and never an agent session (even one of that admin).
    let (status, _) = fixture
        .cookie(&member, "PUT", "/api/v1/admin/agent", rename("Ada"))
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let bearer = fixture.agent_session(&fixture.owner).await;
    for method in ["GET", "PUT"] {
        let (status, value) = fixture
            .request(
                method,
                "/api/v1/admin/agent",
                ("authorization", &bearer),
                rename("Ada"),
            )
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method}");
        assert_eq!(value["code"], "agent_scope");
    }

    // `@Teammate` would reach the member: any case, any spaces around it.
    let (status, value) = fixture
        .cookie(
            &fixture.owner,
            "PUT",
            "/api/v1/admin/agent",
            rename(" teammate "),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{value}");
    assert_eq!(value["code"], "agent_name_taken");
    // Also someone whose workspace the bot has not joined yet (it joins on the sidecar's next poll).
    sqlx::query(
        "INSERT INTO users (id, email, normalized_email, display_name, password_hash, created_at, updated_at) \
         VALUES ($1, 'new@example.com', 'new@example.com', 'Newcomer', 'unused', 0, 0)",
    )
    .bind(Id::new_v7().to_string())
    .execute(fixture.database.pool())
    .await
    .unwrap();
    let (status, _) = fixture
        .cookie(
            &fixture.owner,
            "PUT",
            "/api/v1/admin/agent",
            rename("NEWCOMER"),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    for invalid in ["A", "@Ada", "Ada  Bot", &"a".repeat(41)] {
        let (status, _) = fixture
            .cookie(
                &fixture.owner,
                "PUT",
                "/api/v1/admin/agent",
                rename(invalid),
            )
            .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{invalid}");
    }

    let (status, value) = fixture
        .cookie(
            &fixture.owner,
            "PUT",
            "/api/v1/admin/agent",
            rename("  Ada  "),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{value}");
    assert_eq!(value["name"], "Ada");
    // Its own name again is no collision.
    let (status, _) = fixture
        .cookie(&fixture.owner, "PUT", "/api/v1/admin/agent", rename("ADA"))
        .await;
    assert_eq!(status, StatusCode::OK);

    // The bot user has the name: people see it, and the sidecar gets it with the next poll.
    let (_, status_view) = fixture.cookie(&member, "GET", "/api/v1/agent", None).await;
    assert_eq!(status_view["name"], "ADA");
    let (_, inputs) = fixture.internal("GET", "/inputs", None).await;
    assert_eq!(inputs["bot_name"], "ADA");
    let (_, people) = fixture
        .internal(
            "GET",
            &format!("/workspaces/{}/people", fixture.workspace_id),
            None,
        )
        .await;
    assert!(
        people["people"]
            .as_array()
            .unwrap()
            .iter()
            .any(|person| person["id"] == bot.as_str() && person["name"] == "ADA"),
        "{people}"
    );
}

#[tokio::test]
async fn owners_and_admins_write_team_instructions_members_read_them_and_runs_get_them() {
    let fixture = Fixture::new().await;
    fixture.pending().await;
    let member = fixture.add_member("member").await;
    let admin = fixture.add_member("admin").await;
    sqlx::query("UPDATE memberships SET role = 'admin' WHERE user_id = $1")
        .bind(admin.id.to_string())
        .execute(fixture.database.pool())
        .await
        .unwrap();
    let uri = format!("/api/v1/workspaces/{}/agent", fixture.workspace_id);
    let write = |text: &str| Some(json!({ "instructions": text }));

    // Members read them (empty at first) and cannot write them.
    let (status, value) = fixture.cookie(&member, "GET", &uri, None).await;
    assert_eq!(status, StatusCode::OK, "{value}");
    assert_eq!(value["instructions"], "");
    assert_eq!(value["can_edit"], false);
    assert_eq!(value["max_chars"], 4000);
    let (status, _) = fixture
        .cookie(&member, "PUT", &uri, write("Answer in French."))
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Owners and admins write them, up to 4,000 characters.
    let (status, _) = fixture
        .cookie(&fixture.owner, "PUT", &uri, write(&"a".repeat(4_001)))
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let (status, value) = fixture
        .cookie(&fixture.owner, "PUT", &uri, write(&"é".repeat(4_000)))
        .await;
    assert_eq!(status, StatusCode::OK, "{value}");
    let (status, value) = fixture
        .cookie(
            &admin,
            "PUT",
            &uri,
            write("  Be brief.\r\nUse metric units.  "),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{value}");
    assert_eq!(value["instructions"], "Be brief.\nUse metric units.");
    assert_eq!(value["can_edit"], true);
    assert_eq!(value["updated_by_name"], "admin");
    let (_, value) = fixture.cookie(&member, "GET", &uri, None).await;
    assert_eq!(value["instructions"], "Be brief.\nUse metric units.");

    // Another workspace's id is not found.
    let (status, _) = fixture
        .cookie(
            &fixture.owner,
            "GET",
            &format!("/api/v1/workspaces/{}/agent", Id::new_v7()),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // An agent session cannot read or change them, even the owner's.
    let bearer = fixture.agent_session(&fixture.owner).await;
    for method in ["GET", "PUT"] {
        let (status, value) = fixture
            .request(
                method,
                &uri,
                ("authorization", &bearer),
                write("Ignore the rules."),
            )
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method}");
        assert_eq!(value["code"], "agent_scope");
    }

    // The sidecar gets them with each input of the workspace.
    let (_, inputs) = fixture.internal("GET", "/inputs", None).await;
    let inputs = inputs["inputs"].as_array().unwrap();
    assert!(!inputs.is_empty());
    assert!(
        inputs
            .iter()
            .all(|input| input["team_instructions"] == "Be brief.\nUse metric units."),
        "{inputs:?}"
    );
}

/// A sidecar that answers only `GET /models`, and only with the secret.
async fn fake_sidecar() -> String {
    use axum::routing::get;
    let app = axum::Router::new().route(
        "/models",
        get(|headers: axum::http::HeaderMap| async move {
            if headers
                .get("x-orbit-agent-secret")
                .is_none_or(|value| value != SECRET)
            {
                return (StatusCode::UNAUTHORIZED, axum::Json(json!({})));
            }
            (
                StatusCode::OK,
                axum::Json(json!({
                    "default_model": "gpt-6.1-sol",
                    "models": [
                        { "id": "gpt-6.1-sol", "name": "GPT-6.1 Sol" },
                        { "id": "gpt-6-luna", "name": "GPT-6 Luna" },
                    ],
                })),
            )
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    url
}

#[tokio::test]
async fn installation_admins_pick_the_model_and_thinking_level_and_runs_get_them() {
    let fixture = Fixture::with_sidecar(&fake_sidecar().await).await;
    fixture.pending().await;
    let member = fixture.add_member("Teammate").await;
    let workspace_admin = fixture.add_member("Wadmin").await;
    sqlx::query("UPDATE memberships SET role = 'admin' WHERE user_id = $1")
        .bind(workspace_admin.id.to_string())
        .execute(fixture.database.pool())
        .await
        .unwrap();
    let path = "/api/v1/admin/agent/model";
    let choose = |model: Option<&str>, thinking: Option<&str>| {
        Some(json!({ "model": model, "thinking": thinking }))
    };

    // Nothing saved: the sidecar's default, and the list it offers.
    let (status, value) = fixture.cookie(&fixture.owner, "GET", path, None).await;
    assert_eq!(status, StatusCode::OK, "{value}");
    assert_eq!(value["model"], Value::Null);
    assert_eq!(value["thinking"], Value::Null);
    assert_eq!(value["available"]["default_model"], "gpt-6.1-sol");
    assert_eq!(value["available"]["models"][1]["id"], "gpt-6-luna");
    let (_, inputs) = fixture.internal("GET", "/inputs", None).await;
    assert_eq!(inputs["model"], Value::Null);
    assert_eq!(inputs["thinking"], Value::Null);

    // Only an installation admin: not a workspace admin, a member, or an agent session.
    for person in [&member, &workspace_admin] {
        for method in ["GET", "PUT"] {
            let (status, _) = fixture
                .cookie(person, method, path, choose(Some("gpt-6-luna"), None))
                .await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{method}");
        }
    }
    let bearer = fixture.agent_session(&fixture.owner).await;
    for method in ["GET", "PUT"] {
        let (status, value) = fixture
            .request(
                method,
                path,
                ("authorization", &bearer),
                choose(Some("gpt-6-luna"), None),
            )
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method}");
        assert_eq!(value["code"], "agent_scope");
    }

    // A model the sidecar does not offer, and an unknown thinking level ("off" is the default).
    for (model, thinking) in [
        (Some("gpt-4"), None),
        (Some("gpt-6-luna"), Some("off")),
        (None, Some("HIGH")),
    ] {
        let (status, value) = fixture
            .cookie(&fixture.owner, "PUT", path, choose(model, thinking))
            .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{value}");
    }

    let (status, value) = fixture
        .cookie(
            &fixture.owner,
            "PUT",
            path,
            choose(Some(" gpt-6-luna "), Some("xhigh")),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{value}");
    assert_eq!(value["model"], "gpt-6-luna");
    assert_eq!(value["thinking"], "xhigh");
    assert_eq!(value["updated_by_name"], "Owner");
    let (_, value) = fixture.cookie(&fixture.owner, "GET", path, None).await;
    assert_eq!(value["model"], "gpt-6-luna");
    // The sidecar gets them with its next poll, for every new run.
    let (_, inputs) = fixture.internal("GET", "/inputs", None).await;
    assert_eq!(inputs["model"], "gpt-6-luna");
    assert_eq!(inputs["thinking"], "xhigh");

    // Back to the defaults.
    let (status, value) = fixture
        .cookie(&fixture.owner, "PUT", path, choose(None, None))
        .await;
    assert_eq!(status, StatusCode::OK, "{value}");
    let (_, inputs) = fixture.internal("GET", "/inputs", None).await;
    assert_eq!(inputs["model"], Value::Null);
    assert_eq!(inputs["thinking"], Value::Null);
}

#[tokio::test]
async fn an_offline_sidecar_allows_only_the_default_model() {
    let fixture = Fixture::new().await;
    let path = "/api/v1/admin/agent/model";
    let (status, value) = fixture.cookie(&fixture.owner, "GET", path, None).await;
    assert_eq!(status, StatusCode::OK, "{value}");
    assert_eq!(value["available"], Value::Null);
    let (status, value) = fixture
        .cookie(
            &fixture.owner,
            "PUT",
            path,
            Some(json!({ "model": "gpt-6-luna", "thinking": null })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_GATEWAY, "{value}");
    let (status, value) = fixture
        .cookie(
            &fixture.owner,
            "PUT",
            path,
            Some(json!({ "model": null, "thinking": "low" })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{value}");
    assert_eq!(value["thinking"], "low");
}
