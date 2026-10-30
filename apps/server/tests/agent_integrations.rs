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

// ---------------------------------------------------------------------------------------------
// OAuth client credentials.

const CLIENT_ID: &str = "plain-client-id-1234";
const CLIENT_SECRET: &str = "plain-client-secret-5678";

/// One API call the fake saw.
#[derive(Clone, Debug)]
struct ApiCall {
    method: String,
    path: String,
    shopify: Option<String>,
    authorization: Option<String>,
}

/// A local token server and API. `{count}` in the token answer is the number of the request.
struct FakeOAuth {
    base: String,
    token_url: String,
    token_requests: AtomicU64,
    token_forms: Mutex<Vec<String>>,
    token_answer: Mutex<(StatusCode, String)>,
    redirect_followed: AtomicU64,
    /// Tokens the API refuses with 401.
    refused: Mutex<Vec<String>>,
    api_status: Mutex<StatusCode>,
    api_calls: Mutex<Vec<ApiCall>>,
    /// API calls wait here, so that concurrent calls overlap.
    barrier: Mutex<Option<Arc<tokio::sync::Barrier>>>,
}

fn token_answer(expires_in: Option<Value>) -> (StatusCode, String) {
    let mut answer = json!({ "access_token": "tok-{count}", "scope": "read_products" });
    if let Some(expires_in) = expires_in {
        answer["expires_in"] = expires_in;
    }
    (StatusCode::OK, answer.to_string())
}

async fn fake_oauth() -> Arc<FakeOAuth> {
    async fn token(State(fake): State<Arc<FakeOAuth>>, body: String) -> Response {
        let count = fake.token_requests.fetch_add(1, Ordering::SeqCst) + 1;
        fake.token_forms.lock().unwrap().push(body);
        // Long enough that a second concurrent request would start while this one runs.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let (status, answer) = fake.token_answer.lock().unwrap().clone();
        let answer = answer.replace("{count}", &count.to_string());
        (
            status,
            [
                (header::CONTENT_TYPE, "application/json"),
                (header::LOCATION, "/elsewhere"),
            ],
            answer,
        )
            .into_response()
    }
    async fn elsewhere(State(fake): State<Arc<FakeOAuth>>) -> axum::Json<Value> {
        fake.redirect_followed.fetch_add(1, Ordering::SeqCst);
        axum::Json(json!({ "access_token": "from-the-redirect", "expires_in": 86399 }))
    }
    async fn api(
        State(fake): State<Arc<FakeOAuth>>,
        method: axum::http::Method,
        uri: Uri,
        headers: HeaderMap,
    ) -> Response {
        let read = |name: &str| {
            headers
                .get(name)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned)
        };
        let call = ApiCall {
            method: method.to_string(),
            path: uri.path().to_owned(),
            shopify: read("x-shopify-access-token"),
            authorization: read("authorization"),
        };
        fake.api_calls.lock().unwrap().push(call.clone());
        let barrier = fake.barrier.lock().unwrap().clone();
        if let Some(barrier) = barrier {
            barrier.wait().await;
        }
        let token = call
            .shopify
            .or(call
                .authorization
                .map(|value| value.trim_start_matches("Bearer ").to_owned()))
            .unwrap_or_default();
        if fake.refused.lock().unwrap().contains(&token) {
            return StatusCode::UNAUTHORIZED.into_response();
        }
        let status = *fake.api_status.lock().unwrap();
        (status, axum::Json(json!({ "shop": "demo" }))).into_response()
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let root = format!("http://{}", listener.local_addr().unwrap());
    let fake = Arc::new(FakeOAuth {
        base: format!("{root}/admin/api/2026-10"),
        token_url: format!("{root}/admin/oauth/access_token"),
        token_requests: AtomicU64::new(0),
        token_forms: Mutex::default(),
        token_answer: Mutex::new(token_answer(Some(json!(86399)))),
        redirect_followed: AtomicU64::new(0),
        refused: Mutex::default(),
        api_status: Mutex::new(StatusCode::OK),
        api_calls: Mutex::default(),
        barrier: Mutex::default(),
    });
    let router = axum::Router::new()
        .route("/admin/oauth/access_token", post(token))
        .route("/elsewhere", any(elsewhere))
        .route("/admin/api/{*rest}", any(api))
        .with_state(Arc::clone(&fake));
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    fake
}

impl FakeOAuth {
    fn tokens(&self) -> u64 {
        self.token_requests.load(Ordering::SeqCst)
    }

    fn calls(&self) -> Vec<ApiCall> {
        self.api_calls.lock().unwrap().clone()
    }

    fn input(&self) -> Value {
        json!({
            "kind": "http",
            "name": "Shop",
            "url": self.base,
            "auth_header": "X-Shopify-Access-Token",
            "oauth_token_url": self.token_url,
            "oauth_client_id": CLIENT_ID,
            "oauth_client_secret": CLIENT_SECRET,
            "allowed_methods": ["GET", "POST"],
        })
    }
}

async fn oauth_fixture() -> (Fixture, Arc<FakeOAuth>, String) {
    let fixture = Fixture::new(&["127.0.0.1"]).await;
    let fake = fake_oauth().await;
    let (status, created) = fixture
        .admin(&fixture.owner, "POST", "", Some(fake.input()))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let id = created["id"].as_str().unwrap().to_owned();
    (fixture, fake, id)
}

fn read_call() -> Value {
    json!({ "method": "GET", "path": "/shop.json" })
}

#[tokio::test]
async fn a_cached_token_is_reused_and_renewed_60_seconds_before_expiry() {
    let (fixture, fake, _) = oauth_fixture().await;
    let (general, root, bearer) = fixture.run(&fixture.owner).await;
    let thread = (general.as_str(), root.as_str());
    // 62 seconds: usable for 2 seconds.
    *fake.token_answer.lock().unwrap() = token_answer(Some(json!(62)));
    for _ in 0..2 {
        let (_, read) = fixture
            .call(&bearer, thread, "shop__api_request", read_call())
            .await;
        assert_eq!(read["response"]["status"], 200, "{read}");
    }
    assert_eq!(fake.tokens(), 1);
    assert_eq!(fake.calls()[1].shopify.as_deref(), Some("tok-1"));
    assert_eq!(fake.calls()[1].authorization, None);
    let form = fake.token_forms.lock().unwrap()[0].clone();
    assert!(form.contains("grant_type=client_credentials"), "{form}");
    assert!(form.contains(&format!("client_id={CLIENT_ID}")), "{form}");
    assert!(!form.contains("scope"), "{form}");

    tokio::time::sleep(std::time::Duration::from_millis(2_100)).await;
    fixture
        .call(&bearer, thread, "shop__api_request", read_call())
        .await;
    assert_eq!(fake.tokens(), 2);
    assert_eq!(fake.calls()[2].shopify.as_deref(), Some("tok-2"));
}

#[tokio::test]
async fn a_token_without_a_usable_expiry_serves_one_call() {
    let (fixture, fake, _) = oauth_fixture().await;
    let (general, root, bearer) = fixture.run(&fixture.owner).await;
    let thread = (general.as_str(), root.as_str());
    let mut expected = 0;
    for expires_in in [None, Some(json!(60)), Some(json!(1)), Some(json!("soon"))] {
        *fake.token_answer.lock().unwrap() = token_answer(expires_in);
        for _ in 0..2 {
            let (_, read) = fixture
                .call(&bearer, thread, "shop__api_request", read_call())
                .await;
            assert_eq!(read["response"]["status"], 200, "{read}");
            expected += 1;
            assert_eq!(fake.tokens(), expected);
        }
    }
    // A long life is cached.
    *fake.token_answer.lock().unwrap() = token_answer(Some(json!(3600)));
    for _ in 0..3 {
        fixture
            .call(&bearer, thread, "shop__api_request", read_call())
            .await;
    }
    assert_eq!(fake.tokens(), expected + 1);
}

#[tokio::test]
async fn a_401_renews_and_retries_once_also_for_an_approved_write() {
    let (fixture, fake, _) = oauth_fixture().await;
    let (general, root, bearer) = fixture.run(&fixture.owner).await;
    let thread = (general.as_str(), root.as_str());

    // The first token is refused: a new one, and the retry works.
    fake.refused.lock().unwrap().push("tok-1".to_owned());
    let (_, read) = fixture
        .call(&bearer, thread, "shop__api_request", read_call())
        .await;
    assert_eq!(read["response"]["status"], 200, "{read}");
    assert_eq!(fake.tokens(), 2);
    assert_eq!(fake.calls().len(), 2);

    // An approved write that gets a 401 is sent again with no second approval.
    let write = json!({ "method": "POST", "path": "/orders.json", "body": { "item": "mug" } });
    let (_, asked) = fixture
        .call(&bearer, thread, "shop__api_request", write.clone())
        .await;
    assert_eq!(asked["status"], "approval_required", "{asked}");
    fixture
        .send(&fixture.owner, &general, "yes", Some(&root))
        .await;
    fake.refused.lock().unwrap().push("tok-2".to_owned());
    let (_, done) = fixture
        .call(&bearer, thread, "shop__api_request", write)
        .await;
    assert_eq!(done["response"]["status"], 200, "{done}");
    assert_eq!(fake.tokens(), 3);
    let posts = fake
        .calls()
        .into_iter()
        .filter(|call| call.method == "POST")
        .count();
    assert_eq!(posts, 2);

    // Still 401 with the new token: that failure comes back, with no more tries.
    for token in ["tok-3", "tok-4"] {
        fake.refused.lock().unwrap().push(token.to_owned());
    }
    let before = fake.calls().len();
    let (_, read) = fixture
        .call(&bearer, thread, "shop__api_request", read_call())
        .await;
    assert_eq!(read["response"]["status"], 401, "{read}");
    assert_eq!(fake.calls().len(), before + 2);
    assert_eq!(fake.tokens(), 4);

    // A 403 is not retried.
    *fake.api_status.lock().unwrap() = StatusCode::FORBIDDEN;
    let before = (fake.calls().len(), fake.tokens());
    let (_, read) = fixture
        .call(&bearer, thread, "shop__api_request", read_call())
        .await;
    assert_eq!(read["response"]["status"], 403, "{read}");
    assert_eq!(fake.calls().len(), before.0 + 1);
    *fake.api_status.lock().unwrap() = StatusCode::OK;

    // A failed token request is not retried: the call fails with its reason.
    fake.refused.lock().unwrap().push("tok-5".to_owned());
    *fake.token_answer.lock().unwrap() = (
        StatusCode::BAD_REQUEST,
        json!({ "error": "invalid_client", "error_description": "details-not-to-show" })
            .to_string(),
    );
    let (tokens, calls) = (fake.tokens(), fake.calls().len());
    let (_, failed) = fixture
        .call(&bearer, thread, "shop__api_request", read_call())
        .await;
    assert_eq!(failed["status"], "failed", "{failed}");
    assert!(
        failed["detail"]
            .as_str()
            .unwrap()
            .contains("invalid_client"),
        "{failed}"
    );
    assert!(!failed.to_string().contains("details-not-to-show"));
    assert_eq!(fake.tokens(), tokens + 1);
    assert_eq!(fake.calls().len(), calls + 1);
}

#[tokio::test]
async fn concurrent_calls_make_one_token_request_and_concurrent_401s_one_renewal() {
    let (fixture, fake, _) = oauth_fixture().await;
    let (general, root, bearer) = fixture.run(&fixture.owner).await;
    let thread = (general.as_str(), root.as_str());

    let first = fixture.call(&bearer, thread, "shop__api_request", read_call());
    let second = fixture.call(&bearer, thread, "shop__api_request", read_call());
    let (first, second) = tokio::join!(first, second);
    assert_eq!(first.1["response"]["status"], 200, "{}", first.1);
    assert_eq!(second.1["response"]["status"], 200, "{}", second.1);
    assert_eq!(fake.tokens(), 1);

    // Both calls hold tok-1 when the API refuses it.
    fake.refused.lock().unwrap().push("tok-1".to_owned());
    *fake.barrier.lock().unwrap() = Some(Arc::new(tokio::sync::Barrier::new(2)));
    let first = fixture.call(&bearer, thread, "shop__api_request", read_call());
    let second = fixture.call(&bearer, thread, "shop__api_request", read_call());
    let (first, second) = tokio::join!(first, second);
    assert_eq!(first.1["response"]["status"], 200, "{}", first.1);
    assert_eq!(second.1["response"]["status"], 200, "{}", second.1);
    assert_eq!(fake.tokens(), 2, "two 401s, one renewal");
}

#[tokio::test]
async fn a_config_change_drops_the_cached_token() {
    let (fixture, fake, id) = oauth_fixture().await;
    let (general, root, bearer) = fixture.run(&fixture.owner).await;
    let thread = (general.as_str(), root.as_str());
    fixture
        .call(&bearer, thread, "shop__api_request", read_call())
        .await;
    let (status, value) = fixture
        .admin(
            &fixture.owner,
            "PATCH",
            &format!("/{id}"),
            Some(json!({ "name": "Renamed shop" })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{value}");
    fixture
        .call(&bearer, thread, "shop__api_request", read_call())
        .await;
    assert_eq!(fake.tokens(), 2);
}

#[tokio::test]
async fn the_client_and_the_token_stay_sealed_and_out_of_every_view() {
    let (fixture, fake, id) = oauth_fixture().await;
    let (general, root, bearer) = fixture.run(&fixture.owner).await;
    let thread = (general.as_str(), root.as_str());
    fixture
        .call(&bearer, thread, "shop__api_request", read_call())
        .await;

    let (sealed, secret, url): (Vec<u8>, Option<Vec<u8>>, String) = {
        use orbit_platform::Row;
        let row = sqlx::query(
            "SELECT oauth_sealed_client, sealed_secret, oauth_token_url \
             FROM agent_integrations WHERE id = $1",
        )
        .bind(&id)
        .fetch_one(fixture.database.pool())
        .await
        .unwrap();
        (
            row.get("oauth_sealed_client"),
            row.get("sealed_secret"),
            row.get("oauth_token_url"),
        )
    };
    let sealed = String::from_utf8_lossy(&sealed).into_owned();
    assert!(!sealed.contains(CLIENT_ID) && !sealed.contains(CLIENT_SECRET));
    assert_eq!(secret, None, "OAuth data is not in sealed_secret");
    assert_eq!(url, fake.token_url);

    let (_, listed) = fixture.admin(&fixture.owner, "GET", "", None).await;
    assert_eq!(listed["items"][0]["auth_type"], "oauth");
    assert_eq!(listed["items"][0]["secret_set"], true);
    assert_eq!(listed["items"][0]["oauth_token_url"], fake.token_url);
    let (_, tested) = fixture
        .admin(&fixture.owner, "POST", &format!("/{id}/test"), None)
        .await;
    let (_, tools) = fixture.tools(&bearer).await;
    assert_eq!(tools["tools"][0]["name"], "shop__api_request");
    assert!(!tools.to_string().contains("oauth/access_token"), "{tools}");
    let audited: Vec<String> = sqlx::query_scalar(
        "SELECT metadata_json FROM audit_events WHERE action LIKE 'agent_integration.%'",
    )
    .fetch_all(fixture.database.pool())
    .await
    .unwrap();
    assert!(audited.len() >= 3, "{audited:?}");
    let texts = [
        listed.to_string(),
        tested.to_string(),
        tools.to_string(),
        audited.join("\n"),
    ];
    for text in texts {
        for hidden in [CLIENT_ID, CLIENT_SECRET, "tok-1", "tok-2"] {
            assert!(!text.contains(hidden), "{hidden} in {text}");
        }
    }
}

#[tokio::test]
async fn a_new_base_or_token_url_needs_a_new_client() {
    let (fixture, fake, id) = oauth_fixture().await;
    let path = format!("/{id}");
    let elsewhere = "http://127.0.0.1:9/steal";
    for change in [
        json!({ "url": elsewhere }),
        json!({ "oauth_token_url": elsewhere }),
        json!({ "url": elsewhere, "oauth_client_id": "other" }),
        json!({ "url": elsewhere, "oauth_client_secret": "other" }),
        json!({ "url": elsewhere, "oauth_client_id": "", "oauth_client_secret": "" }),
    ] {
        let (status, value) = fixture
            .admin(&fixture.owner, "PATCH", &path, Some(change.clone()))
            .await;
        assert_eq!(
            status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{change}: {value}"
        );
    }
    let (_, listed) = fixture.admin(&fixture.owner, "GET", "", None).await;
    assert_eq!(listed["items"][0]["url"], fake.base);
    assert_eq!(listed["items"][0]["oauth_token_url"], fake.token_url);

    // The same URLs are no change, and a change with a new client is allowed.
    let (status, value) = fixture
        .admin(
            &fixture.owner,
            "PATCH",
            &path,
            Some(json!({ "url": fake.base, "oauth_token_url": fake.token_url })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{value}");
    let (status, value) = fixture
        .admin(
            &fixture.owner,
            "PATCH",
            &path,
            Some(json!({
                "oauth_token_url": elsewhere,
                "oauth_client_id": "new-id",
                "oauth_client_secret": "new-secret",
            })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{value}");
    assert_eq!(value["oauth_token_url"], elsewhere);
}

#[tokio::test]
async fn oauth_is_http_only_and_excludes_a_fixed_credential() {
    let (fixture, fake, id) = oauth_fixture().await;
    let path = format!("/{id}");
    let mut mcp = fake.input();
    mcp["kind"] = json!("mcp");
    mcp.as_object_mut().unwrap().remove("allowed_methods");
    let (status, value) = fixture.admin(&fixture.owner, "POST", "", Some(mcp)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{value}");

    // Fixed replaces OAuth: the client is gone, the header is the default again.
    let (status, value) = fixture
        .admin(
            &fixture.owner,
            "PATCH",
            &path,
            Some(json!({ "secret": API_KEY })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{value}");
    assert_eq!(value["auth_type"], "header");
    assert_eq!(value["auth_header"], "Authorization");
    assert_eq!(value["oauth_token_url"], Value::Null);
    let client: Option<Vec<u8>> =
        sqlx::query_scalar("SELECT oauth_sealed_client FROM agent_integrations WHERE id = $1")
            .bind(&id)
            .fetch_one(fixture.database.pool())
            .await
            .unwrap();
    assert_eq!(client, None);

    // OAuth replaces fixed; without a header, the token goes in `Authorization` as a bearer token.
    let (status, value) = fixture
        .admin(
            &fixture.owner,
            "PATCH",
            &path,
            Some(json!({
                "oauth_token_url": fake.token_url,
                "oauth_client_id": CLIENT_ID,
                "oauth_client_secret": CLIENT_SECRET,
                "test_path": "/shop.json",
            })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{value}");
    assert_eq!(value["auth_type"], "oauth");
    assert_eq!(value["test_path"], "shop.json");
    let secret: Option<Vec<u8>> =
        sqlx::query_scalar("SELECT sealed_secret FROM agent_integrations WHERE id = $1")
            .bind(&id)
            .fetch_one(fixture.database.pool())
            .await
            .unwrap();
    assert_eq!(secret, None);
    let (_, tested) = fixture
        .admin(&fixture.owner, "POST", &format!("{path}/test"), None)
        .await;
    assert_eq!(tested["ok"], true, "{tested}");
    assert_eq!(
        fake.calls().last().unwrap().authorization.as_deref(),
        Some("Bearer tok-1")
    );
}

#[tokio::test]
async fn a_token_redirect_or_an_oversize_token_answer_is_refused() {
    let (fixture, fake, id) = oauth_fixture().await;
    let path = format!("/{id}/test");
    *fake.token_answer.lock().unwrap() = (
        StatusCode::FOUND,
        json!({ "access_token": "not-this-one", "error": "Has Upper Case" }).to_string(),
    );
    let (_, tested) = fixture.admin(&fixture.owner, "POST", &path, None).await;
    assert_eq!(tested["ok"], false, "{tested}");
    let detail = tested["detail"].as_str().unwrap();
    assert!(detail.contains("HTTP 302"), "{tested}");
    assert!(!detail.contains("Upper"), "{tested}");
    assert_eq!(fake.redirect_followed.load(Ordering::SeqCst), 0);

    *fake.token_answer.lock().unwrap() = (
        StatusCode::OK,
        json!({ "access_token": "x".repeat(1024 * 1024 + 1) }).to_string(),
    );
    let (_, tested) = fixture.admin(&fixture.owner, "POST", &path, None).await;
    assert_eq!(tested["ok"], false, "{tested}");
    assert!(
        tested["detail"].as_str().unwrap().contains("1 MiB"),
        "{tested}"
    );

    *fake.token_answer.lock().unwrap() = (
        StatusCode::BAD_REQUEST,
        json!({ "error": "shop_not_permitted", "error_description": "not-to-show" }).to_string(),
    );
    let (_, tested) = fixture.admin(&fixture.owner, "POST", &path, None).await;
    assert_eq!(tested["ok"], false, "{tested}");
    let detail = tested["detail"].as_str().unwrap();
    assert!(detail.contains("HTTP 400") && detail.contains("shop_not_permitted"));
    assert!(!tested.to_string().contains("not-to-show"), "{tested}");
    assert!(fake.calls().is_empty(), "the API was not called");
}

#[tokio::test]
async fn test_reports_the_token_and_the_test_path_apart() {
    let (fixture, fake, id) = oauth_fixture().await;
    let path = format!("/{id}");
    let test = format!("{path}/test");

    // Token works, no test path: the API is not called.
    let (_, tested) = fixture.admin(&fixture.owner, "POST", &test, None).await;
    assert_eq!(tested["ok"], true, "{tested}");
    assert!(
        tested["detail"]
            .as_str()
            .unwrap()
            .contains("API was not called"),
        "{tested}"
    );
    assert!(fake.calls().is_empty());

    // A test path that leaves the base is refused.
    for escape in [
        "../secret",
        "..",
        "%2e%2e/x",
        "/../x",
        "//evil.example.com/x",
        "a?b=1",
    ] {
        let (status, value) = fixture
            .admin(
                &fixture.owner,
                "PATCH",
                &path,
                Some(json!({ "test_path": escape })),
            )
            .await;
        assert_eq!(
            status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{escape}: {value}"
        );
    }

    let (status, value) = fixture
        .admin(
            &fixture.owner,
            "PATCH",
            &path,
            Some(json!({ "test_path": "shop.json" })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{value}");

    // Every Test gets a new token and leaves the cache alone.
    let before = fake.tokens();
    let (_, tested) = fixture.admin(&fixture.owner, "POST", &test, None).await;
    assert_eq!(tested["ok"], true, "{tested}");
    assert_eq!(fake.tokens(), before + 1);
    let call = fake.calls().last().unwrap().clone();
    assert_eq!(call.method, "GET");
    assert_eq!(call.path, "/admin/api/2026-10/shop.json");
    fixture.admin(&fixture.owner, "POST", &test, None).await;
    assert_eq!(fake.tokens(), before + 2);

    // The API refuses: the text says the token worked.
    *fake.api_status.lock().unwrap() = StatusCode::FORBIDDEN;
    let (_, tested) = fixture.admin(&fixture.owner, "POST", &test, None).await;
    assert_eq!(tested["ok"], false, "{tested}");
    let detail = tested["detail"].as_str().unwrap();
    assert!(
        detail.contains("403") && detail.contains("token request worked"),
        "{tested}"
    );

    // The token request fails: no call of the API.
    let calls = fake.calls().len();
    *fake.token_answer.lock().unwrap() = (StatusCode::UNAUTHORIZED, "{}".to_owned());
    let (_, tested) = fixture.admin(&fixture.owner, "POST", &test, None).await;
    assert_eq!(tested["ok"], false, "{tested}");
    assert!(
        tested["detail"].as_str().unwrap().contains("HTTP 401"),
        "{tested}"
    );
    assert_eq!(fake.calls().len(), calls);
}

#[tokio::test]
async fn a_fixed_credential_tests_the_base_url_or_its_test_path() {
    let fixture = Fixture::new(&["127.0.0.1"]).await;
    let (api, seen) = fake_api().await;
    let (_, created) = fixture
        .admin(
            &fixture.owner,
            "POST",
            "",
            Some(json!({ "kind": "http", "name": "Fixed", "url": api, "auth_header": "X-Api-Key", "secret": API_KEY })),
        )
        .await;
    assert_eq!(created["auth_type"], "header");
    let id = created["id"].as_str().unwrap();
    let (_, tested) = fixture
        .admin(&fixture.owner, "POST", &format!("/{id}/test"), None)
        .await;
    assert_eq!(tested["ok"], true, "{tested}");
    assert!(tested["detail"].as_str().unwrap().contains("base URL"));
    assert_eq!(seen.lock().unwrap()[0]["uri"], "/api/v1");

    let (status, value) = fixture
        .admin(
            &fixture.owner,
            "PATCH",
            &format!("/{id}"),
            Some(json!({ "test_path": "me" })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{value}");
    let (_, tested) = fixture
        .admin(&fixture.owner, "POST", &format!("/{id}/test"), None)
        .await;
    assert_eq!(tested["ok"], true, "{tested}");
    let line = seen.lock().unwrap()[1].clone();
    assert_eq!(line["uri"], "/api/v1/me");
    assert_eq!(line["auth"], API_KEY);
}
