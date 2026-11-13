//! Home cards: one layout per user, filtered by the caller's role in each workspace.

use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use orbit_platform::{AuthenticatedUser, Id, PasswordService, TestDatabase, TimestampMillis};
use orbit_server::auth_routes::CookieMode;
use orbit_server::repositories::identity::{IdentityRepository, SetupRequest};
use orbit_server::workspace_routes::{WorkspaceState, workspace_router};
use serde_json::{Value, json};
use tower::ServiceExt;

struct Fixture {
    database: TestDatabase,
    identity: Arc<IdentityRepository>,
    app: axum::Router,
    workspace_id: String,
    owner_cookie: String,
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
        let app = workspace_router(WorkspaceState::new(
            Arc::clone(&identity),
            "https://orbit.test".to_owned(),
            CookieMode::secure(),
        ));
        Self {
            database,
            identity,
            app,
            workspace_id: setup.workspace_id.to_string(),
            owner_cookie: format!("__Host-orbit_session={}", setup.session.token),
        }
    }

    /// A user with a session, with `role` in each of `workspaces`.
    async fn user(&self, email: &str, workspaces: &[(&str, &str)]) -> String {
        let id = Id::new_v7();
        let now = TimestampMillis::now();
        sqlx::query(
            "INSERT INTO users (id, email, normalized_email, display_name, password_hash, created_at, updated_at) \
             VALUES ($1, $2, $3, 'Person', 'unused', $4, $5)",
        )
        .bind(id.to_string())
        .bind(email)
        .bind(email)
        .bind(now.as_millis())
        .bind(now.as_millis())
        .execute(self.database.pool())
        .await
        .unwrap();
        for (workspace_id, role) in workspaces {
            sqlx::query(
                "INSERT INTO memberships (id, workspace_id, user_id, role, version, created_at, updated_at) \
                 VALUES ($1, $2, $3, $4, 0, $5, $6)",
            )
            .bind(Id::new_v7().to_string())
            .bind(*workspace_id)
            .bind(id.to_string())
            .bind(*role)
            .bind(now.as_millis())
            .bind(now.as_millis())
            .execute(self.database.pool())
            .await
            .unwrap();
        }
        let user = AuthenticatedUser {
            id,
            email: email.to_owned(),
            display_name: "Person".to_owned(),
        };
        let session = self.identity.create_session(&user, now).await.unwrap();
        format!("__Host-orbit_session={}", session.token)
    }

    /// A second workspace, made by the owner.
    async fn other_workspace(&self) -> String {
        let (status, workspace) = self
            .send(
                "POST",
                "/api/v1/workspaces",
                &self.owner_cookie,
                Some(json!({ "name": "Other" })),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED);
        workspace["id"].as_str().unwrap().to_owned()
    }

    async fn send(
        &self,
        method: &str,
        uri: &str,
        cookie: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let request = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::COOKIE, cookie)
            .header(header::CONTENT_TYPE, "application/json")
            .body(body.map_or_else(Body::empty, |body| Body::from(body.to_string())))
            .unwrap();
        let response = self.app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    fn path(&self, workspace_id: &str) -> String {
        format!("/api/v1/workspaces/{workspace_id}/home-layout")
    }

    async fn get(&self, workspace_id: &str, cookie: &str) -> (StatusCode, Value) {
        self.send("GET", &self.path(workspace_id), cookie, None)
            .await
    }

    async fn put(&self, workspace_id: &str, cookie: &str, cards: Value) -> (StatusCode, Value) {
        self.send(
            "PUT",
            &self.path(workspace_id),
            cookie,
            Some(json!({ "cards": cards })),
        )
        .await
    }
}

fn ids(layout: &Value) -> Vec<&str> {
    layout["cards"]
        .as_array()
        .unwrap()
        .iter()
        .map(|card| card["id"].as_str().unwrap())
        .collect()
}

#[tokio::test]
async fn a_member_gets_the_default_then_their_saved_layout() {
    let fixture = Fixture::new().await;
    let workspace = fixture.workspace_id.clone();
    let member = fixture
        .user("member@example.com", &[(&workspace, "member")])
        .await;

    let (status, layout) = fixture.get(&workspace, &member).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(layout["customized"], false);
    assert_eq!(
        ids(&layout),
        ["overdue", "due_soon", "inbox", "recent_docs", "unread_chat"]
    );
    assert_eq!(layout["available"].as_array().unwrap().len(), 10);

    let (status, saved) = fixture
        .put(
            &workspace,
            &member,
            json!([{ "id": "routines", "limit": 5 }, { "id": "my_tasks" }]),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(saved["customized"], true);
    assert_eq!(
        saved["cards"],
        json!([{ "id": "routines", "limit": 5 }, { "id": "my_tasks" }])
    );

    let (_, read) = fixture.get(&workspace, &member).await;
    assert_eq!(read["cards"], saved["cards"]);
    assert_eq!(read["customized"], true);

    // an empty layout is a saved choice, not the default
    let (_, empty) = fixture.put(&workspace, &member, json!([])).await;
    assert_eq!(empty["cards"], json!([]));
    let (_, read) = fixture.get(&workspace, &member).await;
    assert_eq!(read["cards"], json!([]));
    assert_eq!(read["customized"], true);
}

#[tokio::test]
async fn unknown_and_repeated_cards_are_dropped_and_limits_kept_in_range() {
    let fixture = Fixture::new().await;
    let workspace = fixture.workspace_id.clone();
    let member = fixture
        .user("member@example.com", &[(&workspace, "member")])
        .await;

    let (status, saved) = fixture
        .put(
            &workspace,
            &member,
            json!([
                { "id": "weather" },
                { "id": "inbox", "limit": 1000 },
                { "id": "inbox", "limit": 4 },
                { "id": "activity", "limit": 0 },
            ]),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        saved["cards"],
        json!([{ "id": "inbox", "limit": 20 }, { "id": "activity", "limit": 3 }])
    );

    // a row written before a card was removed from the catalog reads without it
    sqlx::query("UPDATE home_layouts SET cards_json = $1")
        .bind(r#"[{"id":"retired"},{"id":"inbox"}]"#)
        .execute(fixture.database.pool())
        .await
        .unwrap();
    let (_, read) = fixture.get(&workspace, &member).await;
    assert_eq!(read["cards"], json!([{ "id": "inbox" }]));

    // a broken row falls back to the default, Home still loads
    sqlx::query("UPDATE home_layouts SET cards_json = 'not json'")
        .execute(fixture.database.pool())
        .await
        .unwrap();
    let (status, read) = fixture.get(&workspace, &member).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(read["cards"], json!([]));
}

#[tokio::test]
async fn a_guest_only_sees_and_saves_chat_cards() {
    let fixture = Fixture::new().await;
    let workspace = fixture.workspace_id.clone();
    let guest = fixture
        .user("guest@example.com", &[(&workspace, "guest")])
        .await;

    let (status, layout) = fixture.get(&workspace, &guest).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(ids(&layout), ["unread_chat"]);
    assert_eq!(layout["available"], json!(["unread_chat"]));

    let (status, saved) = fixture
        .put(
            &workspace,
            &guest,
            json!([{ "id": "my_tasks" }, { "id": "unread_chat" }, { "id": "recent_files" }]),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(ids(&saved), ["unread_chat"]);
    let stored: String = sqlx::query_scalar("SELECT cards_json FROM home_layouts")
        .fetch_one(fixture.database.pool())
        .await
        .unwrap();
    assert_eq!(stored, r#"[{"id":"unread_chat"}]"#);
}

#[tokio::test]
async fn a_member_card_saved_elsewhere_is_hidden_from_a_guest_and_kept() {
    let fixture = Fixture::new().await;
    let home = fixture.workspace_id.clone();
    let other = fixture.other_workspace().await;
    let person = fixture
        .user("both@example.com", &[(&home, "member"), (&other, "guest")])
        .await;

    fixture
        .put(
            &home,
            &person,
            json!([{ "id": "my_tasks" }, { "id": "unread_chat" }]),
        )
        .await;
    // where the person is a guest, the member card does not show
    let (_, as_guest) = fixture.get(&other, &person).await;
    assert_eq!(ids(&as_guest), ["unread_chat"]);
    // a save there does not lose it
    let (_, saved) = fixture
        .put(
            &other,
            &person,
            json!([{ "id": "unread_chat", "limit": 4 }]),
        )
        .await;
    assert_eq!(saved["cards"], json!([{ "id": "unread_chat", "limit": 4 }]));
    let (_, as_member) = fixture.get(&home, &person).await;
    assert_eq!(
        as_member["cards"],
        json!([{ "id": "unread_chat", "limit": 4 }, { "id": "my_tasks" }])
    );
}

#[tokio::test]
async fn nobody_reads_or_writes_a_layout_through_a_workspace_they_are_not_in() {
    let fixture = Fixture::new().await;
    let workspace = fixture.workspace_id.clone();
    let member = fixture
        .user("member@example.com", &[(&workspace, "member")])
        .await;
    let outsider = fixture.user("outsider@example.com", &[]).await;
    fixture
        .put(&workspace, &member, json!([{ "id": "routines" }]))
        .await;

    let (status, _) = fixture.get(&workspace, &outsider).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = fixture
        .put(&workspace, &outsider, json!([{ "id": "inbox" }]))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = fixture.get(&workspace, "__Host-orbit_session=nope").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // a second member has their own layout: the first one's stays as it was
    let second = fixture
        .user("second@example.com", &[(&workspace, "member")])
        .await;
    let (_, theirs) = fixture.get(&workspace, &second).await;
    assert_eq!(theirs["customized"], false);
    fixture
        .put(&workspace, &second, json!([{ "id": "inbox" }]))
        .await;
    let (_, mine) = fixture.get(&workspace, &member).await;
    assert_eq!(mine["cards"], json!([{ "id": "routines" }]));
}

#[tokio::test]
async fn a_layout_has_at_most_forty_cards_and_a_known_shape() {
    let fixture = Fixture::new().await;
    let workspace = fixture.workspace_id.clone();
    let member = fixture
        .user("member@example.com", &[(&workspace, "member")])
        .await;

    let many: Vec<Value> = (0..41).map(|_| json!({ "id": "inbox" })).collect();
    let (status, body) = fixture.put(&workspace, &member, json!(many)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["code"], "invalid_home_layout");

    let (status, body) = fixture
        .send(
            "PUT",
            &fixture.path(&workspace),
            &member,
            Some(json!({ "cards": [{ "id": "inbox", "color": "red" }] })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "invalid_request");

    let (_, layout) = fixture.get(&workspace, &member).await;
    assert_eq!(layout["customized"], false);
}
