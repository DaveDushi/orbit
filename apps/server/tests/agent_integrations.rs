//! Agent integrations: who sets them up, who may use them, the driver's approval of writes, URL
//! confinement and the address check. Local fake MCP and HTTP servers stand in for the outside.
//! Runs on SQLite, and on Postgres with `ORBIT_TEST_POSTGRES_URL`.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use axum::body::{Body, to_bytes};
use axum::extract::State;
use axum::http::{HeaderMap, Request, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, post};
use orbit_platform::{
    AttachmentMutationCoordinator, AuthenticatedUser, Id, LocalBlobStore, PasswordService,
    TestDatabase, TimestampMillis, UploadLimits, UploadService,
};
use orbit_server::agent::{AgentConfig, AgentState};
use orbit_server::auth_routes::CookieMode;
use orbit_server::chat_routes::{ChatState, chat_router};
use orbit_server::repositories::identity::{IdentityRepository, SetupRequest};
use serde_json::{Value, json};
use tower::ServiceExt;

const SECRET: &str = "test-agent-secret-0123456789abcdef0123456789";
const API_KEY: &str = "Bearer very-secret-api-key-4711";

/// What a fake server received: one JSON line per request.
type Seen = Arc<Mutex<Vec<Value>>>;

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
    async fn new(test_hosts: &[&str]) -> Self {
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
        )
        .with_integration_test_hosts(test_hosts);
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
            app,
            workspace_id: setup.workspace_id.to_string(),
            owner: Member {
                id: setup.user_id,
                cookie: format!("__Host-orbit_session={}", setup.session.token),
            },
            nonce: AtomicU64::new(0),
        }
    }

    /// A user with `role` in the workspace, or with no membership at all when `role` is `None`.
    async fn add_user(&self, name: &str, role: Option<&str>) -> Member {
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
        if let Some(role) = role {
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
        }
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

    /// An admin route as `member` (cookie).
    async fn admin(
        &self,
        member: &Member,
        method: &str,
        rest: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let uri = format!(
            "/api/v1/workspaces/{}/agent-integrations{rest}",
            self.workspace_id
        );
        self.request(method, &uri, ("cookie", &member.cookie), body)
            .await
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
        let (status, value) = self
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
        assert_eq!(status, StatusCode::OK, "{value}");
        value
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

    /// A thread rooted at a mention of the agent by `driver`, now running, and an agent session
    /// of `driver` as an `authorization` header value.
    async fn run(&self, driver: &Member) -> (String, String, String) {
        let (status, value) = self.internal("GET", "/inputs", None).await;
        assert_eq!(status, StatusCode::OK, "{value}");
        let bot = value["bot_user_id"].as_str().unwrap().to_owned();
        let general = self.general().await;
        let root = self
            .send(driver, &general, &format!("<@{bot}> use the tools"), None)
            .await;
        let (status, value) = self
            .internal(
                "PUT",
                &format!("/threads/{root}"),
                Some(json!({
                    "workspace_id": self.workspace_id,
                    "conversation_id": general,
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
                Some(
                    json!({ "user_id": driver.id.to_string(), "workspace_id": self.workspace_id }),
                ),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{session}");
        (
            general,
            root,
            format!("Bearer {}", session["token"].as_str().unwrap()),
        )
    }

    async fn tools(&self, bearer: &str) -> (StatusCode, Value) {
        self.request(
            "GET",
            &format!("/api/v1/workspaces/{}/agent-tools", self.workspace_id),
            ("authorization", bearer),
            None,
        )
        .await
    }

    async fn call(
        &self,
        bearer: &str,
        (conversation, root): (&str, &str),
        tool: &str,
        arguments: Value,
    ) -> (StatusCode, Value) {
        self.request(
            "POST",
            &format!("/api/v1/workspaces/{}/agent-tools/call", self.workspace_id),
            ("authorization", bearer),
            Some(json!({
                "tool": tool,
                "arguments": arguments,
                "conversation_id": conversation,
                "root_message_id": root,
            })),
        )
        .await
    }
}

/// Serves `router` on a free loopback port; returns its base URL.
async fn serve(router: axum::Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    format!("http://{address}")
}

/// A Streamable HTTP MCP server with a read-only tool and a write tool. It answers `tools/call`
/// as an event stream and everything else as JSON.
async fn fake_mcp() -> (String, Seen) {
    async fn handle(
        State(seen): State<Seen>,
        headers: HeaderMap,
        body: axum::body::Bytes,
    ) -> Response {
        let message: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
        seen.lock().unwrap().push(json!({
            "method": message["method"],
            "params": message["params"],
            "auth": headers.get("authorization").and_then(|value| value.to_str().ok()),
            "session": headers.get("mcp-session-id").and_then(|value| value.to_str().ok()),
        }));
        let id = message["id"].clone();
        let reply = |result: Value| json!({ "jsonrpc": "2.0", "id": id, "result": result });
        match message["method"].as_str().unwrap_or_default() {
            "initialize" => (
                [("mcp-session-id", "session-1")],
                axum::Json(reply(json!({
                    "protocolVersion": "2025-06-18",
                    "capabilities": { "tools": {} },
                    "serverInfo": { "name": "fake", "version": "1" },
                }))),
            )
                .into_response(),
            "notifications/initialized" => StatusCode::ACCEPTED.into_response(),
            "tools/list" => axum::Json(reply(json!({ "tools": [
                {
                    "name": "get_zone",
                    "description": "Read a zone.",
                    "inputSchema": { "type": "object", "properties": { "zone": { "type": "string" } } },
                    "annotations": { "readOnlyHint": true },
                },
                {
                    "name": "delete_record",
                    "description": "Delete a DNS record.",
                    "inputSchema": { "type": "object", "properties": { "id": { "type": "string" } } },
                    "annotations": { "destructiveHint": true },
                },
            ] })))
            .into_response(),
            "tools/call" => {
                let text = format!(
                    "{} ran with {}",
                    message["params"]["name"].as_str().unwrap_or_default(),
                    message["params"]["arguments"]
                );
                let event = format!(
                    "event: message\ndata: {}\n\n",
                    reply(json!({ "content": [{ "type": "text", "text": text }] }))
                );
                ([(header::CONTENT_TYPE, "text/event-stream")], event).into_response()
            }
            _ => StatusCode::BAD_REQUEST.into_response(),
        }
    }
    let seen: Seen = Arc::default();
    let router = axum::Router::new()
        .route("/mcp", post(handle).delete(|| async { StatusCode::OK }))
        .with_state(Arc::clone(&seen));
    (format!("{}/mcp", serve(router).await), seen)
}

/// An HTTP API that echoes what it got.
async fn fake_api() -> (String, Seen) {
    async fn handle(
        State(seen): State<Seen>,
        method: axum::http::Method,
        uri: Uri,
        headers: HeaderMap,
        body: axum::body::Bytes,
    ) -> axum::Json<Value> {
        let line = json!({
            "method": method.as_str(),
            "uri": uri.to_string(),
            "auth": headers.get("x-api-key").and_then(|value| value.to_str().ok()),
            "body": serde_json::from_slice::<Value>(&body).ok(),
        });
        seen.lock().unwrap().push(line.clone());
        axum::Json(line)
    }
    let seen: Seen = Arc::default();
    let router = axum::Router::new()
        .route("/{*rest}", any(handle))
        .with_state(Arc::clone(&seen));
    (format!("{}/api/v1", serve(router).await), seen)
}

fn calls_of(seen: &Seen, method: &str) -> Vec<Value> {
    seen.lock()
        .unwrap()
        .iter()
        .filter(|line| line["method"] == method)
        .cloned()
        .collect()
}

#[tokio::test]
async fn only_owners_and_admins_manage_integrations_and_the_secret_never_comes_back() {
    let fixture = Fixture::new(&["127.0.0.1"]).await;
    let (api, _) = fake_api().await;
    let member = fixture.add_user("member", Some("member")).await;
    let admin = fixture.add_user("admin", Some("admin")).await;
    let outsider = fixture.add_user("outsider", None).await;
    let input = json!({
        "kind": "http",
        "name": "Shop API",
        "url": api,
        "auth_header": "X-Api-Key",
        "secret": API_KEY,
    });

    let (status, _) = fixture
        .admin(&member, "POST", "", Some(input.clone()))
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = fixture.admin(&member, "GET", "", None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = fixture.admin(&outsider, "GET", "", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, created) = fixture.admin(&admin, "POST", "", Some(input)).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["slug"], "shop_api");
    assert_eq!(created["secret_set"], true);
    assert_eq!(created["auth_header"], "X-Api-Key");
    let id = created["id"].as_str().unwrap().to_owned();

    // A change without a secret keeps it; no answer ever carries it.
    let (status, updated) = fixture
        .admin(
            &fixture.owner,
            "PATCH",
            &format!("/{id}"),
            Some(json!({ "allowed_methods": ["GET", "post"] })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(updated["allowed_methods"], json!(["GET", "POST"]));
    assert_eq!(updated["secret_set"], true);
    let (_, listed) = fixture.admin(&fixture.owner, "GET", "", None).await;
    let (_, tested) = fixture
        .admin(&fixture.owner, "POST", &format!("/{id}/test"), None)
        .await;
    assert_eq!(tested["ok"], true, "{tested}");
    for answer in [&created, &updated, &listed, &tested] {
        assert!(
            !answer.to_string().contains("very-secret-api-key"),
            "{answer}"
        );
    }
    let sealed: Vec<u8> =
        sqlx::query_scalar("SELECT sealed_secret FROM agent_integrations WHERE id = $1")
            .bind(&id)
            .fetch_one(fixture.database.pool())
            .await
            .unwrap();
    assert!(!String::from_utf8_lossy(&sealed).contains("very-secret-api-key"));

    // An agent session never reaches the admin routes, also an owner's.
    let (_, _, bearer) = fixture.run(&fixture.owner).await;
    for (method, rest) in [
        ("GET", String::new()),
        ("POST", format!("/{id}/test")),
        ("DELETE", format!("/{id}")),
    ] {
        let (status, value) = fixture
            .request(
                method,
                &format!(
                    "/api/v1/workspaces/{}/agent-integrations{rest}",
                    fixture.workspace_id
                ),
                ("authorization", &bearer),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {rest}");
        assert_eq!(value["code"], "agent_scope");
    }

    let (status, _) = fixture
        .admin(&member, "DELETE", &format!("/{id}"), None)
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = fixture
        .admin(&admin, "DELETE", &format!("/{id}"), None)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn mcp_tools_follow_roles_and_a_write_needs_the_drivers_yes_once() {
    let fixture = Fixture::new(&["127.0.0.1"]).await;
    let (mcp, seen) = fake_mcp().await;
    let member = fixture.add_user("member", Some("member")).await;
    let teammate = fixture.add_user("teammate", Some("member")).await;

    let (status, created) = fixture
        .admin(
            &fixture.owner,
            "POST",
            "",
            Some(json!({ "kind": "mcp", "name": "Cloudflare", "url": mcp, "secret": API_KEY })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let id = created["id"].as_str().unwrap().to_owned();
    let (_, tested) = fixture
        .admin(&fixture.owner, "POST", &format!("/{id}/test"), None)
        .await;
    assert_eq!(tested["ok"], true, "{tested}");
    // Read-only tools are on at first, the others off.
    let tools = tested["integration"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 2);
    assert_eq!(tools[0]["name"], "get_zone");
    assert_eq!(tools[0]["enabled"], true);
    assert_eq!(tools[1]["enabled"], false);
    assert_eq!(
        calls_of(&seen, "tools/list")[0]["auth"],
        API_KEY,
        "the credential is sent to the server"
    );

    let (_, _, owner_bearer) = fixture.run(&fixture.owner).await;
    let (general, root, bearer) = fixture.run(&member).await;
    let thread = (general.as_str(), root.as_str());
    let (status, listed) = fixture.tools(&bearer).await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    assert_eq!(listed["tools"].as_array().unwrap().len(), 1);
    assert_eq!(listed["tools"][0]["name"], "cloudflare__get_zone");
    assert_eq!(listed["tools"][0]["approval"], "never");

    // A tool that is off cannot be called.
    let (_, refused) = fixture
        .call(
            &bearer,
            thread,
            "cloudflare__delete_record",
            json!({ "id": "r1" }),
        )
        .await;
    assert_eq!(refused["status"], "refused", "{refused}");

    let (status, read) = fixture
        .call(
            &bearer,
            thread,
            "cloudflare__get_zone",
            json!({ "zone": "example.com" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{read}");
    assert_eq!(read["status"], "ok", "{read}");
    assert!(
        read["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("get_zone ran")
    );

    // The owner turns the write tool on.
    let (status, _) = fixture
        .admin(
            &fixture.owner,
            "PATCH",
            &format!("/{id}"),
            Some(json!({ "enabled_tools": ["get_zone", "delete_record"] })),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let write = json!({ "id": "r1" });
    let (_, asked) = fixture
        .call(&bearer, thread, "cloudflare__delete_record", write.clone())
        .await;
    assert_eq!(asked["status"], "approval_required", "{asked}");
    assert!(asked["action"].as_str().unwrap().contains("delete_record"));
    let approval = asked["approval_id"].clone();

    // A teammate's yes and the driver's other words do not count.
    fixture.send(&teammate, &general, "yes", Some(&root)).await;
    fixture
        .send(&member, &general, "hmm, wait", Some(&root))
        .await;
    let (_, again) = fixture
        .call(&bearer, thread, "cloudflare__delete_record", write.clone())
        .await;
    assert_eq!(again["status"], "approval_required");
    assert_eq!(again["approval_id"], approval);
    // The driver's yes does not cover other arguments.
    fixture.send(&member, &general, "yes", Some(&root)).await;
    let (_, other) = fixture
        .call(
            &bearer,
            thread,
            "cloudflare__delete_record",
            json!({ "id": "r2" }),
        )
        .await;
    assert_eq!(other["status"], "approval_required");
    assert_ne!(other["approval_id"], approval);
    assert!(
        calls_of(&seen, "tools/call")
            .iter()
            .all(|call| call["params"]["name"] != "delete_record")
    );
    let (_, done) = fixture
        .call(&bearer, thread, "cloudflare__delete_record", write.clone())
        .await;
    assert_eq!(done["status"], "ok", "{done}");
    let deletes: Vec<Value> = calls_of(&seen, "tools/call")
        .into_iter()
        .filter(|call| call["params"]["name"] == "delete_record")
        .collect();
    assert_eq!(deletes.len(), 1);
    assert_eq!(deletes[0]["params"]["arguments"], write);
    assert_eq!(deletes[0]["session"], "session-1");

    // Used once: the same write asks again.
    let (_, reuse) = fixture
        .call(&bearer, thread, "cloudflare__delete_record", write.clone())
        .await;
    assert_eq!(reuse["status"], "approval_required");
    // An expired approval does not count, even after a yes.
    fixture.send(&member, &general, "yes", Some(&root)).await;
    sqlx::query("UPDATE agent_integration_approvals SET expires_at = 0 WHERE id = $1")
        .bind(reuse["approval_id"].as_str().unwrap())
        .execute(fixture.database.pool())
        .await
        .unwrap();
    let (_, expired) = fixture
        .call(&bearer, thread, "cloudflare__delete_record", write.clone())
        .await;
    assert_eq!(expired["status"], "approval_required");
    assert_ne!(expired["approval_id"], reuse["approval_id"]);

    // Every external call is in the audit log, without the arguments.
    let audited: Vec<String> = sqlx::query_scalar(
        "SELECT metadata_json FROM audit_events WHERE action = 'agent_integration.call'",
    )
    .fetch_all(fixture.database.pool())
    .await
    .unwrap();
    assert_eq!(audited.len(), 2);
    assert!(
        audited
            .iter()
            .all(|metadata| !metadata.contains("r1") && !metadata.contains("example.com"))
    );

    // The tools only run in a thread the caller drives now.
    let (status, value) = fixture
        .call(&owner_bearer, thread, "cloudflare__get_zone", json!({}))
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{value}");
    assert_eq!(value["code"], "agent_run_required");
    // People call them only through the agent.
    let (status, value) = fixture
        .request(
            "GET",
            &format!("/api/v1/workspaces/{}/agent-tools", fixture.workspace_id),
            ("cookie", &member.cookie),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{value}");
    // Another workspace is out of the session's scope.
    let (status, _) = fixture
        .request(
            "GET",
            &format!("/api/v1/workspaces/{}/agent-tools", Id::new_v7()),
            ("authorization", &bearer),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // A role that is not allowed sees nothing and calls nothing.
    let (status, _) = fixture
        .admin(
            &fixture.owner,
            "PATCH",
            &format!("/{id}"),
            Some(json!({ "allowed_roles": ["owner", "admin"] })),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (_, listed) = fixture.tools(&bearer).await;
    assert_eq!(listed["tools"], json!([]));
    let (status, value) = fixture
        .call(&bearer, thread, "cloudflare__get_zone", json!({}))
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{value}");
    assert_eq!(value["code"], "agent_integration_role");
}

#[tokio::test]
async fn http_api_calls_stay_below_the_base_url_and_writes_need_approval() {
    let fixture = Fixture::new(&["127.0.0.1"]).await;
    let (api, seen) = fake_api().await;
    let (status, created) = fixture
        .admin(
            &fixture.owner,
            "POST",
            "",
            Some(json!({
                "kind": "http",
                "name": "Shop",
                "url": api,
                "auth_header": "X-Api-Key",
                "secret": API_KEY,
                "allowed_methods": ["GET", "POST"],
            })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let (general, root, bearer) = fixture.run(&fixture.owner).await;
    let thread = (general.as_str(), root.as_str());
    let (_, listed) = fixture.tools(&bearer).await;
    assert_eq!(listed["tools"][0]["name"], "shop__api_request");
    assert_eq!(listed["tools"][0]["approval"], "writes");

    let (_, read) = fixture
        .call(
            &bearer,
            thread,
            "shop__api_request",
            json!({ "method": "GET", "path": "/orders", "query": { "limit": 5, "status": "open" } }),
        )
        .await;
    assert_eq!(read["status"], "ok", "{read}");
    assert_eq!(read["response"]["status"], 200);
    assert_eq!(
        read["response"]["body"]["uri"],
        "/api/v1/orders?limit=5&status=open"
    );
    assert_eq!(read["response"]["body"]["auth"], API_KEY);

    for path in [
        "/../admin",
        "/%2e%2e/admin",
        "//evil.example.com/x",
        "/x?y=1",
        "orders",
    ] {
        let (_, refused) = fixture
            .call(
                &bearer,
                thread,
                "shop__api_request",
                json!({ "method": "GET", "path": path }),
            )
            .await;
        assert_eq!(refused["status"], "refused", "{path}: {refused}");
    }
    let (_, refused) = fixture
        .call(
            &bearer,
            thread,
            "shop__api_request",
            json!({ "method": "DELETE", "path": "/orders/1" }),
        )
        .await;
    assert_eq!(refused["status"], "refused", "a method that is not allowed");
    assert_eq!(seen.lock().unwrap().len(), 1, "nothing refused left Orbit");

    let order = json!({ "method": "POST", "path": "/orders", "body": { "item": "mug" } });
    let (_, asked) = fixture
        .call(&bearer, thread, "shop__api_request", order.clone())
        .await;
    assert_eq!(asked["status"], "approval_required");
    assert!(asked["action"].as_str().unwrap().contains("POST"));
    fixture
        .send(&fixture.owner, &general, "Yes, go ahead", Some(&root))
        .await;
    let (_, done) = fixture
        .call(&bearer, thread, "shop__api_request", order)
        .await;
    assert_eq!(done["status"], "ok", "{done}");
    assert_eq!(done["response"]["body"]["body"], json!({ "item": "mug" }));
}

#[tokio::test]
async fn local_and_private_addresses_are_refused_without_the_test_setting() {
    let fixture = Fixture::new(&[]).await;
    let (api, seen) = fake_api().await;
    for url in [
        api.clone(),
        "https://127.0.0.1/".to_owned(),
        "https://169.254.169.254/latest".to_owned(),
        "https://[::1]/".to_owned(),
    ] {
        let (status, value) = fixture
            .admin(
                &fixture.owner,
                "POST",
                "",
                Some(json!({ "kind": "http", "name": "Local", "url": url })),
            )
            .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{url}: {value}");
    }
    // A host name is checked when it resolves.
    let port = api
        .trim_start_matches("http://127.0.0.1:")
        .split('/')
        .next()
        .unwrap()
        .to_owned();
    let (status, created) = fixture
        .admin(
            &fixture.owner,
            "POST",
            "",
            Some(json!({ "kind": "mcp", "name": "Local", "url": format!("https://localhost:{port}/mcp") })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let (_, tested) = fixture
        .admin(
            &fixture.owner,
            "POST",
            &format!("/{}/test", created["id"].as_str().unwrap()),
            None,
        )
        .await;
    assert_eq!(tested["ok"], false, "{tested}");
    assert!(
        tested["detail"]
            .as_str()
            .unwrap()
            .contains("local or private"),
        "{tested}"
    );
    assert!(seen.lock().unwrap().is_empty());
}
