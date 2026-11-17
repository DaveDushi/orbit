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
