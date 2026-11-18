//! Agent sandboxes, Orbit's side (`agent::sandbox`, docs/agent-sandbox-design.md): the ticket and call token for one
//! sandbox call (who may get one), the call's end (once, token gone, CPU time counted), and egress decisions in the
//! order of section 4 (per thread and driver). Runs on SQLite, and on Postgres with `ORBIT_TEST_POSTGRES_URL`.

use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, Mac};
use orbit_platform::{
    AttachmentMutationCoordinator, Id, LocalBlobStore, PasswordService, Row, TestDatabase,
    TimestampMillis, UploadLimits, UploadService,
};
use orbit_server::agent::sandbox::SandboxKeys;
use orbit_server::agent::{AgentConfig, AgentState};
use orbit_server::auth_routes::CookieMode;
use orbit_server::chat_routes::ChatState;
use orbit_server::repositories::identity::{IdentityRepository, SetupRequest};
use orbit_server::task_routes::{TaskState, task_router};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use tower::ServiceExt;

const SECRET: &str = "test-agent-secret-0123456789abcdef0123456789";
const KEY: &[u8] = b"test-sandbox-broker-key-0123456789abcdef";

fn hmac(key: &[u8], parts: &[&[u8]]) -> Vec<u8> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).unwrap();
    for part in parts {
        mac.update(part);
    }
    mac.finalize().into_bytes().to_vec()
}

fn service_bearer() -> String {
    let key: String = hmac(KEY, &[b"orbit-sandbox/service"])
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("Bearer {key}")
}

/// What the broker does with a ticket: check the MAC, then open the sealed secrets.
fn open_ticket(ticket: &str) -> (Value, Value) {
    let parts: Vec<&str> = ticket.split('.').collect();
    assert_eq!(parts.len(), 3);
    assert_eq!(parts[0], "v1");
    let mac = hmac(
        &hmac(KEY, &[b"orbit-sandbox/ticket"]),
        &[b"v1.", parts[1].as_bytes()],
    );
    assert_eq!(URL_SAFE_NO_PAD.decode(parts[2]).unwrap(), mac, "the MAC");
    let claims: Value = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(parts[1]).unwrap()).unwrap();
    let sealed = URL_SAFE_NO_PAD
        .decode(claims["sealed"].as_str().unwrap())
        .unwrap();
    let (nonce, data) = sealed.split_at(16);
    let seal = hmac(KEY, &[b"orbit-sandbox/seal"]);
    let plain: Vec<u8> = data
        .chunks(32)
        .enumerate()
        .flat_map(|(index, chunk)| {
            let block = hmac(
                &seal,
                &[nonce, &u32::try_from(index).unwrap().to_be_bytes()],
            );
            chunk
                .iter()
                .zip(block)
                .map(|(a, b)| a ^ b)
                .collect::<Vec<_>>()
        })
        .collect();
    (claims, serde_json::from_slice(&plain).unwrap())
}

struct Fixture {
    _root: tempfile::TempDir,
    database: TestDatabase,
    app: axum::Router,
    workspace_id: String,
    conversation: String,
}

impl Fixture {
    async fn new() -> Self {
        Self::with_keys(Some(SandboxKeys::new(KEY).unwrap())).await
    }

    async fn with_keys(keys: Option<SandboxKeys>) -> Self {
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
            chat,
            Arc::clone(&identity),
            CookieMode::secure(),
            Some([3; 32]),
        )
        .with_public_origin("https://orbit.example.com")
        .with_sandbox(keys);
        let app = task_router(TaskState::new(Arc::clone(&identity), CookieMode::secure()))
            .merge(orbit_server::agent::router(agent))
            .layer(axum::middleware::from_fn_with_state(
                (*database).clone(),
                orbit_server::agent::scope::layer,
            ));
        let workspace_id = setup.workspace_id.to_string();
        let conversation = sqlx::query_scalar::<_, String>(
            "SELECT id FROM chat_conversations WHERE workspace_id = $1 ORDER BY id LIMIT 1",
        )
        .bind(&workspace_id)
        .fetch_one(database.pool())
        .await
        .unwrap();
        Self {
            _root: root,
            database,
            app,
            workspace_id,
            conversation,
        }
    }

    async fn exec(&self, sql: &'static str, binds: &[&str]) {
        let mut query = sqlx::query(sql);
        for bind in binds {
            query = query.bind(*bind);
        }
        query.execute(self.database.pool()).await.unwrap();
    }

    async fn add_user(&self, name: &str, role: Option<&str>) -> Id {
        let id = Id::new_v7();
        let now = TimestampMillis::now().as_millis();
        sqlx::query(
            "INSERT INTO users (id, email, normalized_email, display_name, password_hash, created_at, updated_at) \
             VALUES ($1, $2, $2, $3, 'unused', $4, $4)",
        )
        .bind(id.to_string())
        .bind(format!("{name}@example.com"))
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
        id
    }

    /// A custom agent with `tools`, and its sandbox settings (`enabled`).
    async fn add_agent(&self, tools: &str, enabled: bool) -> Id {
        let bot = self
            .add_user(
                &format!("bot{}", Id::new_v7().to_string().replace('-', "")),
                None,
            )
            .await;
        let now = TimestampMillis::now().as_millis();
        sqlx::query(
            "INSERT INTO agent_profiles (id, workspace_id, user_id, tools, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $5)",
        )
        .bind(Id::new_v7().to_string())
        .bind(&self.workspace_id)
        .bind(bot.to_string())
        .bind(tools)
        .bind(now)
        .execute(self.database.pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO agent_sandbox_settings (agent_user_id, workspace_id, enabled, updated_at) VALUES ($1, $2, $3, $4)",
        )
        .bind(bot.to_string())
        .bind(&self.workspace_id)
        .bind(i64::from(enabled))
        .bind(now)
        .execute(self.database.pool())
        .await
        .unwrap();
        bot
    }

    async fn thread(&self, agent: Id, driver: Id, status: &str) -> String {
        let key = Id::new_v7().to_string();
        sqlx::query(
            "INSERT INTO agent_threads (thread_key, workspace_id, conversation_id, status, driver_user_id, \
             updated_at, agent_user_id) VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(&key)
        .bind(&self.workspace_id)
        .bind(&self.conversation)
        .bind(status)
        .bind(driver.to_string())
        .bind(TimestampMillis::now().as_millis())
        .bind(agent.to_string())
        .execute(self.database.pool())
        .await
        .unwrap();
        key
    }

    async fn request(
        &self,
        method: &str,
        uri: &str,
        auth: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let builder = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::AUTHORIZATION, auth);
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

    async fn issue(&self, driver: Id, agent: Id, thread: &str) -> (StatusCode, Value) {
        self.request(
            "POST",
            "/api/internal/agent/sandbox/calls",
            &format!("Bearer {SECRET}"),
            Some(json!({
                "workspace_id": self.workspace_id,
                "user_id": driver.to_string(),
                "agent_user_id": agent.to_string(),
                "thread_key": thread,
                "timeout_seconds": 120,
                "script_sha256": "a".repeat(64),
            })),
        )
        .await
    }

    /// A running call: its id, call token and proxy password.
    async fn call(&self, driver: Id, agent: Id, thread: &str) -> (String, String, String) {
        let (status, issued) = self.issue(driver, agent, thread).await;
        assert_eq!(status, StatusCode::OK, "{issued}");
        let (_, sealed) = open_ticket(issued["ticket"].as_str().unwrap());
        (
            issued["call_id"].as_str().unwrap().to_owned(),
            sealed["token"].as_str().unwrap().to_owned(),
            sealed["proxy_password"].as_str().unwrap().to_owned(),
        )
    }

    async fn end(&self, call: &str, cpu_ms: i64) -> (StatusCode, Value) {
        self.request(
            "POST",
            &format!("/api/internal/agent/sandbox/calls/{call}/end"),
            &service_bearer(),
            Some(json!({ "exit": 0, "timed_out": false, "cpu_ms": cpu_ms })),
        )
        .await
    }

    async fn check(&self, call: &str, password: &str, host: &str, port: i64) -> Value {
        let (status, value) = self
            .request(
                "POST",
                "/api/internal/agent/egress/check",
                &service_bearer(),
                Some(json!({ "call_id": call, "proxy_password": password, "host": host, "port": port })),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{value}");
        value
    }

    async fn rule(&self, agent: Id, host: &str, port: i64, action: &str) {
        sqlx::query(
            "INSERT INTO agent_egress_rules (id, workspace_id, agent_user_id, host, port, action, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(Id::new_v7().to_string())
        .bind(&self.workspace_id)
        .bind(agent.to_string())
        .bind(host)
        .bind(port)
        .bind(action)
        .bind(TimestampMillis::now().as_millis())
        .execute(self.database.pool())
        .await
        .unwrap();
    }

    /// The driver's answer to a pending request (the card buttons come in a later phase).
    async fn answer(&self, request_id: &str, status: &str) {
        sqlx::query(
            "UPDATE agent_egress_requests SET status = $2, decided_at = $3, expires_at = $4 WHERE id = $1",
        )
        .bind(request_id)
        .bind(status)
        .bind(TimestampMillis::now().as_millis())
        .bind(TimestampMillis::now().as_millis() + 30 * 60 * 1000)
        .execute(self.database.pool())
        .await
        .unwrap();
    }

    async fn tasks_with(&self, token: &str) -> StatusCode {
        self.request(
            "GET",
            &format!("/api/v1/workspaces/{}/tasks", self.workspace_id),
            &format!("Bearer {token}"),
            None,
        )
        .await
        .0
    }
}

const TOOLS: &str = r#"["search_tasks", "sandbox_shell"]"#;

#[tokio::test]
async fn a_ticket_carries_a_sealed_call_token_that_dies_at_the_call_end() {
    let fixture = Fixture::new().await;
    let driver = fixture.add_user("driver", Some("member")).await;
    let agent = fixture.add_agent(TOOLS, true).await;
    let thread = fixture.thread(agent, driver, "running").await;

    let (status, issued) = fixture.issue(driver, agent, &thread).await;
    assert_eq!(status, StatusCode::OK, "{issued}");
    let ticket = issued["ticket"].as_str().unwrap();
    let (claims, sealed) = open_ticket(ticket);
    assert_eq!(claims["call"], issued["call_id"]);
    assert_eq!(claims["agent"], agent.to_string());
    assert_eq!(claims["limits"]["timeout_s"], 120);
    assert_eq!(claims["limits"]["idle_minutes"], 30);
    let work = claims["work"].as_str().unwrap();
    assert!(work.len() == 32 && work.bytes().all(|b| b.is_ascii_hexdigit()));
    // The sidecar sees neither the token nor the proxy password; Orbit keeps only hashes.
    let token = sealed["token"].as_str().unwrap();
    let password = sealed["proxy_password"].as_str().unwrap();
    assert!(token.starts_with("orbit-agent."));
    assert!(!issued.to_string().contains(token) && !issued.to_string().contains(password));
    let stored = sqlx::query(
        "SELECT proxy_password_hash, status, work_id FROM agent_sandbox_calls WHERE id = $1",
    )
    .bind(issued["call_id"].as_str().unwrap())
    .fetch_one(fixture.database.pool())
    .await
    .unwrap();
    assert_eq!(
        stored.get::<Vec<u8>, _>("proxy_password_hash"),
        Sha256::digest(password.as_bytes()).to_vec()
    );
    assert_eq!(stored.get::<String, _>("work_id"), work);

    // The same thread and driver keep their sandbox; another driver or thread gets another.
    let (_, again) = fixture.issue(driver, agent, &thread).await;
    assert_eq!(
        open_ticket(again["ticket"].as_str().unwrap()).0["work"],
        work
    );
    let other_thread = fixture.thread(agent, driver, "running").await;
    let (_, other) = fixture.issue(driver, agent, &other_thread).await;
    assert_ne!(
        open_ticket(other["ticket"].as_str().unwrap()).0["work"],
        work
    );

    // The token acts as the driver within the agent's tools, until the call ends.
    assert_eq!(fixture.tasks_with(token).await, StatusCode::OK);
    let (status, ended) = fixture.end(issued["call_id"].as_str().unwrap(), 1500).await;
    assert_eq!(status, StatusCode::OK, "{ended}");
    assert_eq!(fixture.tasks_with(token).await, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn only_the_driver_of_a_running_thread_gets_a_ticket_for_an_agent_with_the_sandbox_on() {
    let fixture = Fixture::new().await;
    let driver = fixture.add_user("driver", Some("member")).await;
    let other = fixture.add_user("other", Some("admin")).await;
    let guest = fixture.add_user("guest", Some("guest")).await;
    let agent = fixture.add_agent(TOOLS, true).await;
    let thread = fixture.thread(agent, driver, "running").await;

    // The sidecar secret is needed.
    let (status, _) = fixture
        .request(
            "POST",
            "/api/internal/agent/sandbox/calls",
            "Bearer wrong",
            Some(json!({
                "workspace_id": fixture.workspace_id,
                "user_id": driver.to_string(),
                "agent_user_id": agent.to_string(),
                "thread_key": thread,
            })),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let refused = |value: &Value, code: &str| assert_eq!(value["code"], code, "{value}");

    // Another person (even an admin) cannot get a ticket for the driver's thread.
    let (status, value) = fixture.issue(other, agent, &thread).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    refused(&value, "sandbox_refused");
    // A thread that is not running; another agent's thread; a guest's thread.
    let idle = fixture.thread(agent, driver, "idle").await;
    refused(
        &fixture.issue(driver, agent, &idle).await.1,
        "sandbox_refused",
    );
    let second = fixture.add_agent(TOOLS, true).await;
    refused(
        &fixture.issue(driver, second, &thread).await.1,
        "sandbox_refused",
    );
    let guests = fixture.thread(agent, guest, "running").await;
    refused(
        &fixture.issue(guest, agent, &guests).await.1,
        "sandbox_refused",
    );
    // An agent without the tool, or with the sandbox off.
    let untick = fixture.add_agent(r#"["search_tasks"]"#, true).await;
    let thread_untick = fixture.thread(untick, driver, "running").await;
    refused(
        &fixture.issue(driver, untick, &thread_untick).await.1,
        "sandbox_refused",
    );
    let off = fixture.add_agent(TOOLS, false).await;
    let thread_off = fixture.thread(off, driver, "running").await;
    refused(
        &fixture.issue(driver, off, &thread_off).await.1,
        "sandbox_off",
    );

    // The month's CPU budget spent.
    let month = jiff::Timestamp::now().strftime("%Y-%m").to_string();
    sqlx::query(
        "INSERT INTO agent_sandbox_usage (agent_user_id, month, cpu_ms) VALUES ($1, $2, $3)",
    )
    .bind(agent.to_string())
    .bind(&month)
    .bind(600_i64 * 60_000)
    .execute(fixture.database.pool())
    .await
    .unwrap();
    refused(
        &fixture.issue(driver, agent, &thread).await.1,
        "sandbox_budget_spent",
    );
    fixture
        .exec(
            "DELETE FROM agent_sandbox_usage WHERE agent_user_id = $1",
            &[&agent.to_string()],
        )
        .await;
    assert_eq!(
        fixture.issue(driver, agent, &thread).await.0,
        StatusCode::OK
    );

    // A suspended driver, and a deleted agent.
    fixture
        .exec(
            "UPDATE users SET suspended_at = 1 WHERE id = $1",
            &[&driver.to_string()],
        )
        .await;
    refused(
        &fixture.issue(driver, agent, &thread).await.1,
        "sandbox_refused",
    );
    fixture
        .exec(
            "UPDATE users SET suspended_at = NULL WHERE id = $1",
            &[&driver.to_string()],
        )
        .await;
    fixture
        .exec(
            "UPDATE agent_profiles SET deleted_at = 1 WHERE user_id = $1",
            &[&agent.to_string()],
        )
        .await;
    refused(
        &fixture.issue(driver, agent, &thread).await.1,
        "sandbox_refused",
    );
}

#[tokio::test]
async fn without_the_broker_key_the_sandbox_is_off() {
    let fixture = Fixture::with_keys(None).await;
    let driver = fixture.add_user("driver", Some("member")).await;
    let agent = fixture.add_agent(TOOLS, true).await;
    let thread = fixture.thread(agent, driver, "running").await;
    let (status, value) = fixture.issue(driver, agent, &thread).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(value["code"], "sandbox_off");
}

#[tokio::test]
async fn a_call_ends_once_even_when_ends_race_and_its_cpu_time_counts_once() {
    let fixture = Fixture::new().await;
    let driver = fixture.add_user("driver", Some("member")).await;
    let agent = fixture.add_agent(TOOLS, true).await;
    let thread = fixture.thread(agent, driver, "running").await;
    let (call, _, _) = fixture.call(driver, agent, &thread).await;

    // The broker's key, not the sidecar's secret.
    for auth in [format!("Bearer {SECRET}"), "Bearer wrong".to_owned()] {
        let (status, _) = fixture
            .request(
                "POST",
                &format!("/api/internal/agent/sandbox/calls/{call}/end"),
                &auth,
                Some(json!({ "cpu_ms": 1 })),
            )
            .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
    let ends = (0..6).map(|_| fixture.end(&call, 2_000));
    for (status, value) in futures_util::future::join_all(ends).await {
        assert_eq!(status, StatusCode::OK, "{value}");
    }
    let used: i64 =
        sqlx::query_scalar("SELECT cpu_ms FROM agent_sandbox_usage WHERE agent_user_id = $1")
            .bind(agent.to_string())
            .fetch_one(fixture.database.pool())
            .await
            .unwrap();
    assert_eq!(used, 2_000);
    let row = sqlx::query("SELECT status, session_id FROM agent_sandbox_calls WHERE id = $1")
        .bind(&call)
        .fetch_one(fixture.database.pool())
        .await
        .unwrap();
    assert_eq!(row.get::<String, _>("status"), "done");
    assert_eq!(row.get::<Option<String>, _>("session_id"), None);
    assert_eq!(
        fixture.end(&Id::new_v7().to_string(), 1).await.0,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn a_call_without_an_end_becomes_lost_and_its_token_dies() {
    let fixture = Fixture::new().await;
    let driver = fixture.add_user("driver", Some("member")).await;
    let agent = fixture.add_agent(TOOLS, true).await;
    let thread = fixture.thread(agent, driver, "running").await;
    let (call, token, _) = fixture.call(driver, agent, &thread).await;
    assert_eq!(fixture.tasks_with(&token).await, StatusCode::OK);
    // Started long ago: past its 120 s timeout and the 5 minutes of grace.
    sqlx::query("UPDATE agent_sandbox_calls SET started_at = $2 WHERE id = $1")
        .bind(&call)
        .bind(TimestampMillis::now().as_millis() - 8 * 60 * 1000)
        .execute(fixture.database.pool())
        .await
        .unwrap();
    fixture.call(driver, agent, &thread).await;
    let status: String = sqlx::query_scalar("SELECT status FROM agent_sandbox_calls WHERE id = $1")
        .bind(&call)
        .fetch_one(fixture.database.pool())
        .await
        .unwrap();
    assert_eq!(status, "lost");
    assert_eq!(fixture.tasks_with(&token).await, StatusCode::UNAUTHORIZED);
    // A late end changes nothing and counts nothing.
    assert_eq!(fixture.end(&call, 9_999).await.0, StatusCode::OK);
    let used: Option<i64> =
        sqlx::query_scalar("SELECT cpu_ms FROM agent_sandbox_usage WHERE agent_user_id = $1")
            .bind(agent.to_string())
            .fetch_optional(fixture.database.pool())
            .await
            .unwrap();
    assert_eq!(used, None);
}

#[tokio::test]
async fn egress_decisions_follow_the_order_and_stay_in_their_thread_and_driver() {
    let fixture = Fixture::new().await;
    let driver = fixture.add_user("driver", Some("member")).await;
    let teammate = fixture.add_user("teammate", Some("member")).await;
    let agent = fixture.add_agent(TOOLS, true).await;
    let thread = fixture.thread(agent, driver, "running").await;
    let (call, _, password) = fixture.call(driver, agent, &thread).await;
    let decide = |host: &'static str, port: i64| {
        let (call, password) = (call.clone(), password.clone());
        let fixture = &fixture;
        async move { fixture.check(&call, &password, host, port).await }
    };

    // Only the proxy, and only with the call's password.
    let (status, _) = fixture
        .request(
            "POST",
            "/api/internal/agent/egress/check",
            &format!("Bearer {SECRET}"),
            Some(json!({ "call_id": call, "proxy_password": password, "host": "a.example.com", "port": 443 })),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = fixture
        .request(
            "POST",
            "/api/internal/agent/egress/check",
            &service_bearer(),
            Some(json!({ "call_id": call, "proxy_password": "wrong", "host": "a.example.com", "port": 443 })),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // No rule: ask, one pending request however often it is asked.
    let first = decide("API.Other.NET.", 443).await;
    assert_eq!(first["decision"], "ask");
    assert_eq!(first["host"], "api.other.net");
    assert_eq!(
        decide("api.other.net", 443).await["request_id"],
        first["request_id"]
    );
    // Never: IP literals, Orbit's own host, orbit.internal, odd ports.
    for host in ["1.1.1.1", "[::1]", "orbit.example.com", "orbit.internal"] {
        assert_eq!(decide(host, 443).await["decision"], "deny", "{host}");
    }
    assert_eq!(decide("api.other.net", 70_000).await["decision"], "deny");

    // Rules: a wildcard covers subdomains only, and only its port; deny beats allow.
    fixture.rule(agent, "*.example.com", 443, "allow").await;
    assert_eq!(decide("a.b.example.com", 443).await["decision"], "allow");
    assert_eq!(decide("example.com", 443).await["decision"], "ask");
    assert_eq!(decide("a.example.com", 8443).await["decision"], "ask");
    fixture.rule(agent, "bad.example.com", 443, "deny").await;
    assert_eq!(decide("bad.example.com", 443).await["decision"], "deny");
    // A risky host asks even under an allow rule; Allow once (this thread, this driver) lets it through.
    fixture
        .rule(agent, "*.s3.amazonaws.com", 443, "allow")
        .await;
    let risky = decide("bucket.s3.amazonaws.com", 443).await;
    assert_eq!(risky["decision"], "ask");
    fixture
        .answer(risky["request_id"].as_str().unwrap(), "allowed")
        .await;
    assert_eq!(
        decide("bucket.s3.amazonaws.com", 443).await["decision"],
        "allow"
    );

    // Allow once in this thread does not reach another thread of the same driver, nor another driver.
    fixture
        .answer(first["request_id"].as_str().unwrap(), "allowed")
        .await;
    assert_eq!(decide("api.other.net", 443).await["decision"], "allow");
    let other_thread = fixture.thread(agent, driver, "running").await;
    let (other_call, _, other_password) = fixture.call(driver, agent, &other_thread).await;
    assert_eq!(
        fixture
            .check(&other_call, &other_password, "api.other.net", 443)
            .await["decision"],
        "ask"
    );
    let theirs = fixture.thread(agent, teammate, "running").await;
    let (their_call, _, their_password) = fixture.call(teammate, agent, &theirs).await;
    let asked = fixture
        .check(&their_call, &their_password, "api.other.net", 443)
        .await;
    assert_eq!(asked["decision"], "ask");
    // A Deny holds for its thread and driver.
    fixture
        .answer(asked["request_id"].as_str().unwrap(), "denied")
        .await;
    assert_eq!(
        fixture
            .check(&their_call, &their_password, "api.other.net", 443)
            .await["decision"],
        "deny"
    );

    // The pending requests come back with the call's end; after the end the call gets nothing.
    let (_, ended) = fixture.end(&other_call, 10).await;
    let pending = ended["egress_pending"].as_array().unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0]["host"], "api.other.net");
    assert_eq!(
        fixture
            .check(&other_call, &other_password, "api.other.net", 443)
            .await["decision"],
        "deny"
    );

    // Off, or the budget spent: deny, also for allowed hosts.
    let month = jiff::Timestamp::now().strftime("%Y-%m").to_string();
    sqlx::query(
        "INSERT INTO agent_sandbox_usage (agent_user_id, month, egress_bytes) VALUES ($1, $2, $3) \
         ON CONFLICT (agent_user_id, month) DO UPDATE SET egress_bytes = $3",
    )
    .bind(agent.to_string())
    .bind(&month)
    .bind(2_147_483_648_i64)
    .execute(fixture.database.pool())
    .await
    .unwrap();
    assert_eq!(decide("a.b.example.com", 443).await["decision"], "deny");
    fixture
        .exec(
            "DELETE FROM agent_sandbox_usage WHERE agent_user_id = $1",
            &[&agent.to_string()],
        )
        .await;
    fixture
        .exec(
            "UPDATE agent_sandbox_settings SET enabled = 0 WHERE agent_user_id = $1",
            &[&agent.to_string()],
        )
        .await;
    assert_eq!(decide("a.b.example.com", 443).await["decision"], "deny");

    // Every decision is logged: host and port only.
    let logged: Vec<(String, String)> =
        sqlx::query("SELECT host, decision FROM agent_egress_log WHERE call_id = $1 ORDER BY id")
            .bind(&call)
            .fetch_all(fixture.database.pool())
            .await
            .unwrap()
            .iter()
            .map(|row| (row.get("host"), row.get("decision")))
            .collect();
    assert!(logged.contains(&("api.other.net".to_owned(), "ask".to_owned())));
    assert!(logged.contains(&("bucket.s3.amazonaws.com".to_owned(), "allow".to_owned())));
    assert!(logged.contains(&("1.1.1.1".to_owned(), "deny".to_owned())));
    assert!(logged.len() >= 15);
}

// ---------------------------------------------------------------------------------------------
// The egress proxy (`agent::egress_proxy`) against these routes on a real port.

mod proxy {
    use std::collections::HashMap;
    use std::io;
    use std::net::{IpAddr, SocketAddr};
    use std::path::Path;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD;
    use futures_util::future::BoxFuture;
    use orbit_server::agent::egress_proxy::{Dialer, ProxyConfig, serve};
    use sha2::{Digest, Sha256};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpSocket, TcpStream};
    use tokio_util::sync::CancellationToken;

    use super::*;

    /// Fake DNS (counts lookups) and a dialer that sends every connection to a local echo server.
    pub struct FakeDialer {
        pub answers: HashMap<&'static str, Vec<IpAddr>>,
        pub resolved: Mutex<Vec<String>>,
        pub connected: Mutex<Vec<SocketAddr>>,
        pub echo: SocketAddr,
    }

    impl Dialer for FakeDialer {
        fn resolve<'a>(
            &'a self,
            host: &'a str,
            port: u16,
        ) -> BoxFuture<'a, io::Result<Vec<SocketAddr>>> {
            self.resolved.lock().unwrap().push(host.to_owned());
            let found = self.answers.get(host).map(|ips| {
                ips.iter()
                    .map(|ip| SocketAddr::new(*ip, port))
                    .collect::<Vec<_>>()
            });
            Box::pin(async move { found.ok_or_else(|| io::ErrorKind::NotFound.into()) })
        }

        fn connect(&self, address: SocketAddr) -> BoxFuture<'_, io::Result<TcpStream>> {
            self.connected.lock().unwrap().push(address);
            Box::pin(TcpStream::connect(self.echo))
        }
    }

    async fn echo_server() -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let (mut read, mut write) = stream.split();
                    let _ = tokio::io::copy(&mut read, &mut write).await;
                });
            }
        });
        address
    }

    pub struct Running {
        pub address: SocketAddr,
        pub dialer: Arc<FakeDialer>,
        pub calls: tempfile::TempDir,
        _stop: tokio_util::sync::DropGuard,
    }

    /// Orbit (the fixture's routes) on a port, and a proxy in front of it.
    pub async fn start(fixture: &Fixture, max_bytes: u64, max_connections: usize) -> Running {
        let orbit = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let orbit_address = orbit.local_addr().unwrap();
        let app = fixture.app.clone();
        tokio::spawn(async move { axum::serve(orbit, app).await });
        let dialer = Arc::new(FakeDialer {
            answers: HashMap::from([
                ("api.example.com", vec!["93.184.215.14".parse().unwrap()]),
                ("private.example.com", vec!["10.0.0.5".parse().unwrap()]),
                (
                    "rebind.example.com",
                    vec![
                        "93.184.215.14".parse().unwrap(),
                        "127.0.0.1".parse().unwrap(),
                    ],
                ),
                ("meta.example.com", vec!["169.254.169.254".parse().unwrap()]),
                ("ula.example.com", vec!["fd00::1".parse().unwrap()]),
            ]),
            resolved: Mutex::default(),
            connected: Mutex::default(),
            echo: echo_server().await,
        });
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let calls = tempfile::tempdir().unwrap();
        let stop = CancellationToken::new();
        let token = service_bearer().trim_start_matches("Bearer ").to_owned();
        tokio::spawn(serve(
            ProxyConfig {
                orbit_url: format!("http://{orbit_address}"),
                service_token: token,
                calls_dir: calls.path().to_owned(),
                orbit_internal: orbit_address,
                call_max_bytes: max_bytes,
                call_max_connections: max_connections,
            },
            Arc::clone(&dialer) as Arc<dyn Dialer>,
            listener,
            stop.clone(),
        ));
        Running {
            address,
            dialer,
            calls,
            _stop: stop.drop_guard(),
        }
    }

    /// What the broker writes for a running call.
    pub fn call_file(dir: &Path, call: &str, sandbox: &str, password: &str) {
        let hash: String = Sha256::digest(password.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        std::fs::write(
            dir.join(call),
            json!({ "addr": sandbox, "password_sha256": hash }).to_string(),
        )
        .unwrap();
    }

    /// A request from `source` with `call:password`. The status, the JSON body (when not 200), and the open
    /// stream after a 200.
    pub async fn send(
        proxy: SocketAddr,
        source: &str,
        credentials: (&str, &str),
        line: &str,
        extra: &str,
    ) -> (u16, Value, TcpStream) {
        let socket = TcpSocket::new_v4().unwrap();
        socket
            .bind(SocketAddr::new(source.parse().unwrap(), 0))
            .unwrap();
        let mut stream = socket.connect(proxy).await.unwrap();
        let auth = STANDARD.encode(format!("{}:{}", credentials.0, credentials.1));
        stream
            .write_all(
                format!("{line}\r\nHost: x\r\nProxy-Authorization: Basic {auth}\r\n{extra}\r\n")
                    .as_bytes(),
            )
            .await
            .unwrap();
        let mut head = Vec::new();
        while !head.ends_with(b"\r\n\r\n") {
            let mut byte = [0_u8; 1];
            if stream.read(&mut byte).await.unwrap() == 0 {
                break;
            }
            head.push(byte[0]);
        }
        let head = String::from_utf8(head).unwrap();
        let status: u16 = head
            .split(' ')
            .nth(1)
            .and_then(|code| code.parse().ok())
            .unwrap_or(0);
        if status == 200 && line.starts_with("CONNECT") {
            return (status, Value::Null, stream);
        }
        let mut body = Vec::new();
        let _ = tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut body)).await;
        (
            status,
            serde_json::from_slice(&body).unwrap_or(Value::Null),
            stream,
        )
    }

    /// A TLS ClientHello record that names `host`.
    pub fn hello(host: &str) -> Vec<u8> {
        let name = host.as_bytes();
        let n = u16::try_from(name.len()).unwrap();
        let mut sni = vec![0, 0];
        sni.extend_from_slice(&(n + 5).to_be_bytes());
        sni.extend_from_slice(&(n + 3).to_be_bytes());
        sni.push(0);
        sni.extend_from_slice(&n.to_be_bytes());
        sni.extend_from_slice(name);
        let mut body = vec![3, 3];
        body.extend_from_slice(&[7; 32]);
        body.extend_from_slice(&[0, 0, 2, 0x13, 0x01, 1, 0]);
        body.extend_from_slice(&u16::try_from(sni.len()).unwrap().to_be_bytes());
        body.extend_from_slice(&sni);
        let mut record = vec![22, 3, 1];
        record.extend_from_slice(&u16::try_from(body.len() + 4).unwrap().to_be_bytes());
        record.extend_from_slice(&[1, 0]);
        record.extend_from_slice(&u16::try_from(body.len()).unwrap().to_be_bytes());
        record.extend_from_slice(&body);
        record
    }

    /// Sends `payload` through an open tunnel and reads the echo back.
    pub async fn round_trip(stream: &mut TcpStream, payload: &[u8]) -> io::Result<Vec<u8>> {
        stream.write_all(payload).await?;
        let mut back = vec![0_u8; payload.len()];
        tokio::time::timeout(Duration::from_secs(5), stream.read_exact(&mut back))
            .await
            .map_err(|_| io::Error::from(io::ErrorKind::TimedOut))??;
        Ok(back)
    }

    /// Whether the proxy closed the tunnel (EOF or reset) within `wait`.
    pub async fn closed_within(stream: &mut TcpStream, wait: Duration) -> bool {
        let mut byte = [0_u8; 1];
        matches!(
            tokio::time::timeout(wait, stream.read(&mut byte)).await,
            Ok(Ok(0) | Err(_))
        )
    }
}

/// One log row of a call's connection to `host`, once the proxy reported it (or as it is after 5 s).
async fn logged(fixture: &Fixture, call: &str, host: &str) -> (String, i64, i64, bool) {
    for _ in 0..50 {
        let row = sqlx::query(
            "SELECT decision, bytes_up, bytes_down, done_at FROM agent_egress_log WHERE call_id = $1 AND host = $2 \
             ORDER BY id DESC LIMIT 1",
        )
        .bind(call)
        .bind(host)
        .fetch_one(fixture.database.pool())
        .await
        .unwrap();
        let done = row.get::<Option<i64>, _>("done_at").is_some();
        if done || row.get::<String, _>("decision") != "allow" {
            return (
                row.get("decision"),
                row.get("bytes_up"),
                row.get("bytes_down"),
                done,
            );
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("the proxy did not report the connection to {host}");
}

#[tokio::test]
async fn the_proxy_resolves_only_after_orbit_says_allow_reaches_only_public_addresses_and_counts_bytes()
 {
    let fixture = Fixture::new().await;
    let driver = fixture.add_user("driver", Some("member")).await;
    let agent = fixture.add_agent(TOOLS, true).await;
    let thread = fixture.thread(agent, driver, "running").await;
    let (call, _, password) = fixture.call(driver, agent, &thread).await;
    fixture.rule(agent, "*.example.com", 443, "allow").await;
    let running = proxy::start(&fixture, 100 * 1024 * 1024, 20).await;
    proxy::call_file(running.calls.path(), &call, "127.0.0.1", &password);
    let connect = |target: &str| {
        let line = format!("CONNECT {target} HTTP/1.1");
        let (address, call, password) = (running.address, call.clone(), password.clone());
        async move { proxy::send(address, "127.0.0.1", (&call, &password), &line, "").await }
    };

    // No rule: Orbit asks; no IP literal; nothing resolved or dialed before a yes. Plain HTTP never.
    let (status, body, _) = connect("api.other.net:443").await;
    assert_eq!(
        (status, body["error"].as_str()),
        (403, Some("egress_approval_required"))
    );
    for target in [
        "1.1.1.1:443",
        "[::1]:443",
        "169.254.169.254:80",
        "orbit.example.com:443",
    ] {
        let (status, body, _) = connect(target).await;
        assert_eq!(
            (status, body["error"].as_str()),
            (403, Some("egress_denied")),
            "{target}"
        );
    }
    assert!(running.dialer.resolved.lock().unwrap().is_empty());
    assert!(running.dialer.connected.lock().unwrap().is_empty());
    let (status, body, _) = proxy::send(
        running.address,
        "127.0.0.1",
        (&call, &password),
        "GET http://api.example.com/ HTTP/1.1",
        "",
    )
    .await;
    assert_eq!(
        (status, body["error"].as_str()),
        (405, Some("connect_only"))
    );

    // Allowed: resolved, dialed at the checked address, the ClientHello passes, bytes are counted once.
    let (status, _, mut tunnel) = connect("API.Example.com:443").await;
    assert_eq!(status, 200);
    let hello = proxy::hello("api.example.com");
    assert_eq!(proxy::round_trip(&mut tunnel, &hello).await.unwrap(), hello);
    let payload = vec![b'x'; 5000];
    assert_eq!(
        proxy::round_trip(&mut tunnel, &payload).await.unwrap(),
        payload
    );
    drop(tunnel);
    let sent = i64::try_from(hello.len() + payload.len()).unwrap();
    assert_eq!(
        logged(&fixture, &call, "api.example.com").await,
        ("allow".to_owned(), sent, sent, true)
    );
    assert_eq!(
        *running.dialer.connected.lock().unwrap(),
        vec!["93.184.215.14:443".parse::<std::net::SocketAddr>().unwrap()]
    );
    let counted = sqlx::query(
        "SELECT c.egress_bytes, u.egress_bytes AS month FROM agent_sandbox_calls c \
         JOIN agent_sandbox_usage u ON u.agent_user_id = c.agent_user_id WHERE c.id = $1",
    )
    .bind(&call)
    .fetch_one(fixture.database.pool())
    .await
    .unwrap();
    assert_eq!(counted.get::<i64, _>("egress_bytes"), 2 * sent);
    assert_eq!(counted.get::<i64, _>("month"), 2 * sent);

    // Allowed by the rules, but private, link-local, unique-local, or one private answer among public ones (DNS
    // rebinding): blocked before any connect, and logged as blocked.
    for host in [
        "private.example.com",
        "meta.example.com",
        "ula.example.com",
        "rebind.example.com",
    ] {
        let (status, body, _) = connect(&format!("{host}:443")).await;
        assert_eq!(
            (status, body["error"].as_str()),
            (403, Some("egress_blocked")),
            "{host}"
        );
        assert_eq!(logged(&fixture, &call, host).await.0, "blocked", "{host}");
    }
    assert_eq!(running.dialer.connected.lock().unwrap().len(), 1);

    // A ClientHello for another name (or none): the tunnel closes; blocked.
    let (status, _, mut tunnel) = connect("api.example.com:443").await;
    assert_eq!(status, 200);
    tunnel.write_all(&proxy::hello("evil.net")).await.unwrap();
    assert!(proxy::closed_within(&mut tunnel, std::time::Duration::from_secs(5)).await);
    assert_eq!(
        logged(&fixture, &call, "api.example.com").await.0,
        "blocked"
    );
    let (status, _, mut tunnel) = connect("api.example.com:443").await;
    assert_eq!(status, 200);
    tunnel.write_all(b"GET / HTTP/1.1\r\n\r\n").await.unwrap();
    assert!(proxy::closed_within(&mut tunnel, std::time::Duration::from_secs(5)).await);

    // A retried byte report counts nothing twice.
    let log_id: String = sqlx::query_scalar(
        "SELECT id FROM agent_egress_log WHERE call_id = $1 AND decision = 'allow' AND done_at IS NOT NULL",
    )
    .bind(&call)
    .fetch_one(fixture.database.pool())
    .await
    .unwrap();
    let (status, again) = fixture
        .request(
            "POST",
            "/api/internal/agent/egress/done",
            &service_bearer(),
            Some(json!({ "log_id": log_id, "call_id": call, "bytes_up": 999, "bytes_down": 999, "duration_ms": 1 })),
        )
        .await;
    assert_eq!(
        (status, again["counted"].as_bool()),
        (StatusCode::OK, Some(false))
    );
}

#[tokio::test]
async fn a_proxy_password_works_only_for_its_running_call_and_from_its_own_sandbox() {
    let fixture = Fixture::new().await;
    let driver = fixture.add_user("driver", Some("member")).await;
    let teammate = fixture.add_user("teammate", Some("member")).await;
    let agent = fixture.add_agent(TOOLS, true).await;
    let thread = fixture.thread(agent, driver, "running").await;
    let theirs = fixture.thread(agent, teammate, "running").await;
    let (call, token, password) = fixture.call(driver, agent, &thread).await;
    let (other, _, other_password) = fixture.call(teammate, agent, &theirs).await;
    fixture.rule(agent, "api.example.com", 443, "allow").await;
    let running = proxy::start(&fixture, 100 * 1024 * 1024, 20).await;
    proxy::call_file(running.calls.path(), &call, "127.0.0.1", &password);
    proxy::call_file(running.calls.path(), &other, "127.0.0.2", &other_password);
    let line = "CONNECT api.example.com:443 HTTP/1.1";
    let send = |source: &'static str, call: String, password: String| {
        let address = running.address;
        async move {
            proxy::send(address, source, (&call, &password), line, "")
                .await
                .0
        }
    };

    assert_eq!(
        send("127.0.0.1", call.clone(), "wrong".to_owned()).await,
        407
    );
    assert_eq!(
        send("127.0.0.1", call.clone(), other_password.clone()).await,
        407
    );
    // The other sandbox's own password, used from this sandbox's address.
    assert_eq!(
        send("127.0.0.1", other.clone(), other_password.clone()).await,
        407
    );
    assert_eq!(
        send("127.0.0.2", other.clone(), other_password.clone()).await,
        200
    );
    assert_eq!(send("127.0.0.1", call.clone(), password.clone()).await, 200);

    // orbit.internal reaches Orbit (no egress decision); the call token does the rest.
    let tasks = format!(
        "GET http://orbit.internal/api/v1/workspaces/{}/tasks HTTP/1.1",
        fixture.workspace_id
    );
    let (status, body, _) = proxy::send(
        running.address,
        "127.0.0.1",
        (&call, &password),
        &tasks,
        &format!("Authorization: Bearer {token}\r\n"),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    // One request per connection: a pipelined second request never reaches Orbit; a raw tunnel to Orbit is egress.
    let mut stream = tokio::net::TcpStream::connect(running.address)
        .await
        .unwrap();
    let auth = base64::engine::general_purpose::STANDARD.encode(format!("{call}:{password}"));
    let one = format!(
        "{tasks}\r\nHost: orbit.internal\r\nProxy-Authorization: Basic {auth}\r\nAuthorization: Bearer {token}\r\n\r\n"
    );
    stream
        .write_all(format!("{one}{one}").as_bytes())
        .await
        .unwrap();
    let mut answer = String::new();
    tokio::io::AsyncReadExt::read_to_string(&mut stream, &mut answer)
        .await
        .unwrap();
    assert_eq!(answer.matches("HTTP/1.1 ").count(), 1, "{answer}");
    let (status, body, _) = proxy::send(
        running.address,
        "127.0.0.1",
        (&call, &password),
        "CONNECT orbit.internal:80 HTTP/1.1",
        "",
    )
    .await;
    assert_eq!(
        (status, body["error"].as_str()),
        (403, Some("egress_denied"))
    );

    // The call ends in Orbit while the broker's file is still there: Orbit refuses.
    let (_, _, mut relaying) =
        proxy::send(running.address, "127.0.0.1", (&call, &password), line, "").await;
    let hello = proxy::hello("api.example.com");
    proxy::round_trip(&mut relaying, &hello).await.unwrap();
    let (_, _, mut waiting) =
        proxy::send(running.address, "127.0.0.1", (&call, &password), line, "").await;
    fixture.end(&call, 0).await;
    let (status, body, _) =
        proxy::send(running.address, "127.0.0.1", (&call, &password), line, "").await;
    assert_eq!(
        (status, body["error"].as_str()),
        (403, Some("egress_denied"))
    );
    std::fs::remove_file(running.calls.path().join(&call)).unwrap();
    // The broker deletes the file: the password is dead, and open tunnels close within the sweep.
    assert!(proxy::closed_within(&mut relaying, std::time::Duration::from_secs(3)).await);
    assert!(proxy::closed_within(&mut waiting, std::time::Duration::from_secs(1)).await);
    assert_eq!(send("127.0.0.1", call.clone(), password.clone()).await, 407);
    let (status, _, _) = proxy::send(
        running.address,
        "127.0.0.1",
        (&call, &password),
        &tasks,
        &format!("Authorization: Bearer {token}\r\n"),
    )
    .await;
    assert_eq!(status, 407);
    // The other call is not affected.
    assert_eq!(
        send("127.0.0.2", other.clone(), other_password.clone()).await,
        200
    );
}

#[tokio::test]
async fn a_call_gets_at_most_its_bytes_and_connections_through_the_proxy() {
    let fixture = Fixture::new().await;
    let driver = fixture.add_user("driver", Some("member")).await;
    let agent = fixture.add_agent(TOOLS, true).await;
    let thread = fixture.thread(agent, driver, "running").await;
    let (call, _, password) = fixture.call(driver, agent, &thread).await;
    fixture.rule(agent, "api.example.com", 443, "allow").await;
    let running = proxy::start(&fixture, 4096, 2).await;
    proxy::call_file(running.calls.path(), &call, "127.0.0.1", &password);
    let connect = || {
        let (address, call, password) = (running.address, call.clone(), password.clone());
        async move {
            proxy::send(
                address,
                "127.0.0.1",
                (&call, &password),
                "CONNECT api.example.com:443 HTTP/1.1",
                "",
            )
            .await
        }
    };

    let (_, _, mut first) = connect().await;
    let (_, _, second) = connect().await;
    let (status, body, _) = connect().await;
    assert_eq!(
        (status, body["error"].as_str()),
        (403, Some("egress_limit"))
    );
    drop(second);
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Past 4096 bytes up and down together, the tunnel is cut and no new one opens.
    let hello = proxy::hello("api.example.com");
    proxy::round_trip(&mut first, &hello).await.unwrap();
    let _ = proxy::round_trip(&mut first, &[b'y'; 8000]).await;
    assert!(proxy::closed_within(&mut first, std::time::Duration::from_secs(3)).await);
    let (up, down) = {
        let (_, up, down, _) = logged(&fixture, &call, "api.example.com").await;
        (up, down)
    };
    assert!(up + down <= 4096 + 16 * 1024, "{up} + {down}");
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let (status, body, _) = connect().await;
    assert_eq!(
        (status, body["error"].as_str()),
        (403, Some("egress_limit"))
    );
}

#[tokio::test]
async fn the_sandbox_migration_runs_after_productions_newest_migration() {
    // The catalog and its order are the same on Postgres; an own Postgres database here would stay behind.
    if orbit_platform::testing_postgres() {
        return;
    }
    // Production's newest migration when the sandbox migration was made (deployed 2026-11-18).
    const PRODUCTION: i64 = 20_261_118_104_258;
    const SANDBOX: i64 = 20_261_118_154_559;
    let directory = tempfile::tempdir().unwrap();
    let database = orbit_platform::Database::open(&orbit_platform::DatabaseConfig::new(
        directory.path().join("orbit.sqlite"),
    ))
    .await
    .unwrap();
    orbit_platform::MigrationRunner::embedded_through("test", PRODUCTION)
        .run(&database)
        .await
        .unwrap();
    let runner = orbit_platform::MigrationRunner::embedded("test");
    let pending: Vec<i64> = runner
        .pending(&database)
        .await
        .unwrap()
        .iter()
        .map(|migration| migration.version)
        .collect();
    assert!(pending.contains(&SANDBOX), "{pending:?}");
    assert!(
        pending.iter().all(|version| *version > PRODUCTION),
        "{pending:?}"
    );
    runner.run(&database).await.unwrap();
    sqlx::query("SELECT bytes_up, done_at FROM agent_egress_log")
        .fetch_all(database.pool())
        .await
        .unwrap();
}
