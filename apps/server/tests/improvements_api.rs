//! "Suggest a change": who may send and read requests, the switch, the limits, withdrawing, the
//! `orbit improvements` commands and the table's CHECKs, on SQLite and Postgres.

use std::sync::Arc;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use orbit_platform::{Database, Id, PasswordService, TestDatabase, TimestampMillis};
use orbit_server::auth_routes::CookieMode;
use orbit_server::improvements::{
    ImprovementError, ImprovementState, ImprovementsCommand, Status, router, run_command,
};
use orbit_server::repositories::api_tokens::{ApiTokenRepository, ApiTokenScope};
use orbit_server::repositories::identity::{IdentityRepository, SetupRequest};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tower::ServiceExt;

const PATH: &str = "/api/v1/admin/improvements";

struct Fixture {
    database: TestDatabase,
    app: Router,
    admin: Id,
    workspace: Id,
    project: Id,
}

impl Fixture {
    async fn new() -> Self {
        let database = TestDatabase::new().await.unwrap();
        let identity = IdentityRepository::new((*database).clone());
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
                    token: "setup".into(),
                    email: "root@example.com".into(),
                    display_name: "Root".into(),
                    password_hash: PasswordService::default()
                        .hash("correct horse battery")
                        .unwrap(),
                    workspace_name: "Orbit".into(),
                    project_name: "General".into(),
                },
                now,
            )
            .await
            .unwrap();
        // The production order: the agent scope layer runs before the routes.
        let app = router(ImprovementState::new(
            Arc::new(identity),
            CookieMode::secure(),
        ))
        .layer(axum::middleware::from_fn_with_state(
            (*database).clone(),
            orbit_server::agent::scope::layer,
        ));
        let fixture = Self {
            database,
            app,
            admin: setup.user_id,
            workspace: setup.workspace_id,
            project: setup.project_id,
        };
        fixture.session("admin-token", setup.user_id).await;
        fixture
    }

    fn db(&self) -> &Database {
        &self.database
    }

    async fn session(&self, token: &str, user: Id) -> Id {
        let id = Id::new_v7();
        let hash: [u8; 32] = Sha256::digest(token.as_bytes()).into();
        let far = TimestampMillis::now().as_millis() + 3_600_000;
        sqlx::query(
            "INSERT INTO sessions (id, token_hash, user_id, created_at, last_activity_at, \
             idle_expires_at, absolute_expires_at) VALUES ($1, $2, $3, 1, 1, $4, $4)",
        )
        .bind(id.to_string())
        .bind(hash.to_vec())
        .bind(user.to_string())
        .bind(far)
        .execute(self.db().pool())
        .await
        .unwrap();
        id
    }

    /// A signed-in account that is not an installation admin.
    async fn member(&self) -> String {
        let user = Id::new_v7();
        sqlx::query(
            "INSERT INTO users (id, email, normalized_email, display_name, password_hash, created_at, updated_at) \
             VALUES ($1, 'member@example.com', 'member@example.com', 'Member', 'unused', 1, 1)",
        )
        .bind(user.to_string())
        .execute(self.db().pool())
        .await
        .unwrap();
        self.session("member-token", user).await;
        cookie("member-token")
    }

    /// An agent session of the admin (as the sidecar gets one for a run).
    async fn agent_bearer(&self) -> String {
        let token = "orbit-agent.test-token";
        let session = self.session(token, self.admin).await;
        sqlx::query(
            "INSERT INTO agent_sessions (session_id, user_id, workspace_id, expires_at) VALUES ($1, $2, $3, $4)",
        )
        .bind(session.to_string())
        .bind(self.admin.to_string())
        .bind(self.workspace.to_string())
        .bind(TimestampMillis::now().as_millis() + 3_600_000)
        .execute(self.db().pool())
        .await
        .unwrap();
        format!("Bearer {token}")
    }

    /// An API token of the admin with MCP read access.
    async fn mcp_bearer(&self) -> String {
        let issued = ApiTokenRepository::new(self.db().clone())
            .create(
                self.workspace,
                self.admin,
                "MCP".into(),
                vec![self.project],
                &[ApiTokenScope::TasksRead],
                false,
                None,
                "test",
                TimestampMillis::now(),
            )
            .await
            .unwrap();
        format!("Bearer {}", issued.token)
    }

    async fn call(
        &self,
        auth: (header::HeaderName, &str),
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let request = Request::builder()
            .method(method)
            .uri(path)
            .header(auth.0, auth.1);
        let request = match body {
            Some(body) => request
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string())),
            None => request.body(Body::empty()),
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

    async fn admin_call(
        &self,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        self.call((header::COOKIE, &cookie("admin-token")), method, path, body)
            .await
    }

    async fn audited(&self, action: &str) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM audit_events WHERE action = $1")
            .bind(action)
            .fetch_one(self.db().pool())
            .await
            .unwrap()
    }
}

fn cookie(token: &str) -> String {
    format!("__Host-orbit_session={token}")
}

fn request(text: &str) -> Value {
    json!({ "text": text, "context": { "path": "/tasks", "viewport": "1440x900", "revision": "6af5609" } })
}

#[tokio::test]
async fn only_installation_admins_reach_the_routes() {
    let fixture = Fixture::new().await;
    let member = fixture.member().await;
    let agent = fixture.agent_bearer().await;
    let mcp = fixture.mcp_bearer().await;
    let routes = [
        ("GET", PATH.to_owned(), None),
        ("POST", PATH.to_owned(), Some(request("Make it pink"))),
        ("GET", format!("{PATH}/settings"), None),
        (
            "PUT",
            format!("{PATH}/settings"),
            Some(json!({ "enabled": true })),
        ),
        ("POST", format!("{PATH}/{}/withdraw", Id::new_v7()), None),
    ];
    for (method, path, body) in &routes {
        let (status, value) = fixture
            .call((header::COOKIE, &member), method, path, body.clone())
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "member {method} {path}");
        assert_eq!(value["code"], "installation_admin_required");
        // An agent session of an admin: refused before the route.
        let (status, value) = fixture
            .call((header::AUTHORIZATION, &agent), method, path, body.clone())
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "agent {method} {path}");
        assert_eq!(value["code"], "agent_scope");
        // An API token is no session.
        let (status, _) = fixture
            .call((header::AUTHORIZATION, &mcp), method, path, body.clone())
            .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "MCP {method} {path}");
        let (status, _) = fixture
            .call((header::COOKIE, "nothing=1"), method, path, body.clone())
            .await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "anonymous {method} {path}"
        );
    }
    assert_eq!(fixture.audited("instance.improvements_changed").await, 0);
    let rows: i64 = fixture
        .db()
        .scalar("SELECT COUNT(*) FROM improvement_requests")
        .await
        .unwrap();
    assert_eq!(rows, 0);
}

#[tokio::test]
async fn admins_send_requests_only_while_the_switch_is_on() {
    let fixture = Fixture::new().await;
    let settings = format!("{PATH}/settings");

    // Off by default: the list works, sending does not.
    let (status, value) = fixture.admin_call("GET", &settings, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(value, json!({ "enabled": false, "max_text_chars": 4000 }));
    let (status, value) = fixture
        .admin_call("POST", PATH, Some(request("Pink")))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(value["code"], "improvements_disabled");

    let (status, value) = fixture
        .admin_call("PUT", &settings, Some(json!({ "enabled": true })))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(value["enabled"], true);
    assert_eq!(fixture.audited("instance.improvements_changed").await, 1);
    let resource: String = fixture
        .db()
        .scalar(
            "SELECT resource_type FROM audit_events WHERE action = 'instance.improvements_changed'",
        )
        .await
        .unwrap();
    assert_eq!(resource, "installation", "as the other instance settings");

    // The limits.
    let mut invalid = vec![
        request(" \n "),
        request(&"a".repeat(4_001)),
        request("bell \u{0007}"),
    ];
    for context in [
        json!({ "path": "https://evil.test", "viewport": "1x1", "revision": "r" }),
        json!({ "path": "/", "viewport": "wide", "revision": "r" }),
        json!({ "path": "/", "viewport": "1x1", "revision": "<b>" }),
    ] {
        invalid.push(json!({ "text": "Pink", "context": context }));
    }
    for body in invalid {
        let (status, value) = fixture.admin_call("POST", PATH, Some(body.clone())).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert_eq!(value["code"], "validation_failed");
    }

    // 4,000 characters (not bytes) fit; markup is stored as it is, to be shown as plain text.
    let long = "é".repeat(4_000);
    let (status, first) = fixture.admin_call("POST", PATH, Some(request(&long))).await;
    assert_eq!(status, StatusCode::CREATED, "{first}");
    let (status, second) = fixture
        .admin_call(
            "POST",
            PATH,
            Some(request("  <img src=x onerror=alert(1)>\r\nline two ")),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(second["text"], "<img src=x onerror=alert(1)>\nline two");
    assert_eq!(second["status"], "new");
    assert_eq!(second["note"], Value::Null);
    assert_eq!(second["author"]["name"], "Root");
    assert_eq!(
        second["context"],
        json!({ "path": "/tasks", "viewport": "1440x900", "revision": "6af5609" })
    );
    assert_eq!(fixture.audited("improvement.created").await, 2);

    // Newest first.
    let (status, list) = fixture.admin_call("GET", PATH, None).await;
    assert_eq!(status, StatusCode::OK);
    let ids: Vec<&Value> = list["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| &item["id"])
        .collect();
    assert_eq!(ids, [&second["id"], &first["id"]]);

    // Off again: sending stops, the list stays.
    fixture
        .admin_call("PUT", &settings, Some(json!({ "enabled": false })))
        .await;
    let (status, _) = fixture
        .admin_call("POST", PATH, Some(request("Pink")))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, list) = fixture.admin_call("GET", PATH, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list["items"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn an_admin_withdraws_only_a_new_request() {
    let fixture = Fixture::new().await;
    fixture
        .admin_call(
            "PUT",
            &format!("{PATH}/settings"),
            Some(json!({ "enabled": true })),
        )
        .await;
    let (_, created) = fixture
        .admin_call("POST", PATH, Some(request("Pink")))
        .await;
    let id = created["id"].as_str().unwrap();
    let withdraw = format!("{PATH}/{id}/withdraw");

    let (status, value) = fixture.admin_call("POST", &withdraw, None).await;
    assert_eq!(status, StatusCode::OK, "{value}");
    assert_eq!(value["status"], "declined");
    assert_eq!(value["note"], "Withdrawn");
    let (status, value) = fixture.admin_call("POST", &withdraw, None).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(value["code"], "improvement_not_new");
    assert_eq!(fixture.audited("improvement.withdrawn").await, 1);

    // Once the agent has it, it stays.
    let (_, building) = fixture
        .admin_call("POST", PATH, Some(request("Blue")))
        .await;
    let building = building["id"].as_str().unwrap();
    orbit_server::improvements::update(
        fixture.db(),
        None,
        building,
        Status::Building,
        None,
        None,
        "test",
    )
    .await
    .unwrap();
    let (status, _) = fixture
        .admin_call("POST", &format!("{PATH}/{building}/withdraw"), None)
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    for missing in [Id::new_v7().to_string(), "not-an-id".to_owned()] {
        let (status, value) = fixture
            .admin_call("POST", &format!("{PATH}/{missing}/withdraw"), None)
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(value["code"], "improvement_not_found");
    }
}

fn list(status: Option<Status>) -> ImprovementsCommand {
    ImprovementsCommand::List { status, json: true }
}

#[tokio::test]
async fn the_cli_lists_by_status_and_updates_with_a_stable_json_shape() {
    let fixture = Fixture::new().await;
    fixture
        .admin_call(
            "PUT",
            &format!("{PATH}/settings"),
            Some(json!({ "enabled": true })),
        )
        .await;
    let (_, older) = fixture
        .admin_call("POST", PATH, Some(request("Older")))
        .await;
    let (_, newer) = fixture
        .admin_call("POST", PATH, Some(request("Newer \"quoted\"\n<b>")))
        .await;
    let older = older["id"].as_str().unwrap().to_owned();
    let newer = newer["id"].as_str().unwrap().to_owned();

    let printed = run_command(fixture.db(), &list(None)).await.unwrap();
    let items: Value = serde_json::from_str(&printed).unwrap();
    let items = items.as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["id"], newer.as_str());
    // The text is data: JSON-escaped, never interpreted.
    assert_eq!(items[0]["text"], "Newer \"quoted\"\n<b>");
    assert!(printed.contains(r#"Newer \"quoted\"\n<b>"#));
    let keys = |value: &Value| {
        value
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<Vec<_>>()
    };
    assert_eq!(
        keys(&items[0]),
        [
            "author",
            "context",
            "created_at",
            "id",
            "link",
            "note",
            "status",
            "text",
            "updated_at"
        ]
    );
    assert_eq!(keys(&items[0]["author"]), ["email", "id", "name"]);
    assert_eq!(keys(&items[0]["context"]), ["path", "revision", "viewport"]);
    assert_eq!(items[0]["author"]["email"], "root@example.com");
    assert!(items[0]["created_at"].as_str().unwrap().ends_with('Z'));

    let update =
        |id: &str, status, note: Option<&str>, link: Option<&str>| ImprovementsCommand::Update {
            id: id.to_owned(),
            status,
            note: note.map(str::to_owned),
            link: link.map(str::to_owned),
            json: true,
        };
    let printed = run_command(
        fixture.db(),
        &update(
            &older,
            Status::Building,
            Some("Working on it"),
            Some("feat/older"),
        ),
    )
    .await
    .unwrap();
    let updated: Value = serde_json::from_str(&printed).unwrap();
    assert_eq!(updated["status"], "building");
    assert_eq!(updated["note"], "Working on it");
    assert_eq!(updated["link"], "feat/older");
    assert_eq!(updated["text"], "Older");

    // Filter.
    let new_only: Value = serde_json::from_str(
        &run_command(fixture.db(), &list(Some(Status::New)))
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(new_only.as_array().unwrap().len(), 1);
    assert_eq!(new_only[0]["id"], newer.as_str());
    let building: Value = serde_json::from_str(
        &run_command(fixture.db(), &list(Some(Status::Building)))
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(building[0]["id"], older.as_str());
    assert_eq!(
        run_command(fixture.db(), &list(Some(Status::Live)))
            .await
            .unwrap(),
        "[]"
    );

    // Without --note the note stays; an empty one clears it.
    let kept: Value = serde_json::from_str(
        &run_command(fixture.db(), &update(&older, Status::Review, None, None))
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        (kept["status"].as_str(), kept["note"].as_str()),
        (Some("review"), Some("Working on it"))
    );
    let cleared: Value = serde_json::from_str(
        &run_command(
            fixture.db(),
            &update(&older, Status::Live, Some(""), Some(" ")),
        )
        .await
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        (&cleared["note"], &cleared["link"]),
        (&Value::Null, &Value::Null)
    );

    // Limits and unknown ids.
    let too_long = "n".repeat(2_001);
    assert!(matches!(
        run_command(
            fixture.db(),
            &update(&older, Status::Live, Some(&too_long), None)
        )
        .await,
        Err(ImprovementError::Invalid(_))
    ));
    assert!(matches!(
        run_command(
            fixture.db(),
            &update(&"x".repeat(5), Status::Live, None, None)
        )
        .await,
        Err(ImprovementError::NotFound)
    ));
    let plain = run_command(
        fixture.db(),
        &ImprovementsCommand::Update {
            id: newer.clone(),
            status: Status::Declined,
            note: None,
            link: None,
            json: false,
        },
    )
    .await
    .unwrap();
    assert_eq!(plain, format!("{newer} is declined"));

    // A request an admin withdrew stays withdrawn, even for an agent working from an older list.
    let (_, taken_back) = fixture
        .admin_call("POST", PATH, Some(request("Taken back")))
        .await;
    let taken_back = taken_back["id"].as_str().unwrap().to_owned();
    let (status, _) = fixture
        .admin_call("POST", &format!("{PATH}/{taken_back}/withdraw"), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(matches!(
        run_command(
            fixture.db(),
            &update(&taken_back, Status::Building, None, None)
        )
        .await,
        Err(ImprovementError::Withdrawn)
    ));
    let still: Value = serde_json::from_str(
        &run_command(fixture.db(), &list(Some(Status::Declined)))
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(
        still
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["id"] == taken_back.as_str() && item["note"] == "Withdrawn")
    );

    // Every status change is in the audit log, as the operator's.
    let changes: Vec<(Option<String>, String, String)> = sqlx::query_as(
        "SELECT actor_id, request_id, metadata_json FROM audit_events \
         WHERE action = 'improvement.status_changed' ORDER BY occurred_at, id",
    )
    .fetch_all(fixture.db().pool())
    .await
    .unwrap();
    assert_eq!(changes.len(), 4);
    assert_eq!(changes[0].0, None);
    assert_eq!(changes[0].1, "operator-cli");
    assert_eq!(
        serde_json::from_str::<Value>(&changes[0].2).unwrap(),
        json!({ "from": "new", "to": "building" })
    );
}

#[tokio::test]
async fn the_table_allows_only_the_five_statuses_and_the_limits() {
    let fixture = Fixture::new().await;
    let insert = |status: &'static str, body: String, note: Option<String>| {
        let database = fixture.db().clone();
        let admin = fixture.admin.to_string();
        async move {
            sqlx::query(
                "INSERT INTO improvement_requests (id, author_id, body, context_path, context_viewport, \
                 context_revision, status, status_note, created_at, updated_at) \
                 VALUES ($1, $2, $3, '/', '1x1', 'r', $4, $5, 1, 1)",
            )
            .bind(Id::new_v7().to_string())
            .bind(admin)
            .bind(body)
            .bind(status)
            .bind(note)
            .execute(database.pool())
            .await
        }
    };
    for status in ["new", "building", "review", "live", "declined"] {
        insert(status, "ok".into(), None).await.unwrap();
    }
    assert!(insert("done", "ok".into(), None).await.is_err());
    assert!(insert("new", String::new(), None).await.is_err());
    assert!(insert("new", "é".repeat(4_001), None).await.is_err());
    assert!(
        insert("new", "é".repeat(4_000), Some("n".repeat(2_000)))
            .await
            .is_ok()
    );
    assert!(
        insert("new", "ok".into(), Some("n".repeat(2_001)))
            .await
            .is_err()
    );
    assert!(
        insert("new", "ok".into(), Some(String::new()))
            .await
            .is_err()
    );
    // The switch is a 0/1 flag, off after the migration.
    assert!(
        sqlx::query("UPDATE instance_settings SET improvement_requests_enabled = 2")
            .execute(fixture.db().pool())
            .await
            .is_err()
    );
    let flag: i64 = fixture
        .db()
        .scalar("SELECT improvement_requests_enabled FROM instance_settings WHERE id = 1")
        .await
        .unwrap();
    assert_eq!(flag, 0);
}
