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
                // Nothing listens here: the login routes see an offline sidecar.
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
            ("x-orbit-agent-secret", SECRET),
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
                ("x-orbit-agent-secret", secret),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
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
