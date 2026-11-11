use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use orbit_platform::{AuthenticatedUser, Id, PasswordService, TestDatabase, TimestampMillis};
use orbit_server::auth_routes::CookieMode;
use orbit_server::repositories::identity::{IdentityRepository, SetupRequest};
use orbit_server::task_routes::{TaskState, task_router};
use serde_json::{Value, json};
use tower::ServiceExt;

struct Fixture {
    database: TestDatabase,
    identity: Arc<IdentityRepository>,
    app: axum::Router,
    workspace_id: String,
    project_id: String,
    status_id: String,
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
        let status_id: String = sqlx::query_scalar(
            "SELECT id FROM task_statuses WHERE project_id = $1 ORDER BY position LIMIT 1",
        )
        .bind(setup.project_id.to_string())
        .fetch_one(database.pool())
        .await
        .unwrap();
        let app = task_router(TaskState::new(Arc::clone(&identity), CookieMode::secure()));
        Self {
            database,
            identity,
            app,
            workspace_id: setup.workspace_id.to_string(),
            project_id: setup.project_id.to_string(),
            status_id,
            owner_cookie: format!("__Host-orbit_session={}", setup.session.token),
        }
    }
}

/// The kinds in `cookie`'s inbox, newest first.
async fn kinds(fixture: &Fixture, cookie: &str) -> Vec<String> {
    list_notifications(fixture, cookie, false).await["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["kind"].as_str().unwrap().to_owned())
        .collect()
}

async fn create_task(fixture: &Fixture, assignees: &[Id]) -> String {
    let response = fixture
        .app
        .clone()
        .oneshot(json_request(
            "POST",
            &format!("/api/v1/workspaces/{}/tasks", fixture.workspace_id),
            &fixture.owner_cookie,
            json!({
                "project_id": fixture.project_id,
                "status_id": fixture.status_id,
                "title": "Followed work",
                "assignee_ids": assignees.iter().map(ToString::to_string).collect::<Vec<_>>(),
            }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    response_json(response).await["id"]
        .as_str()
        .unwrap()
        .to_owned()
}

async fn comment(fixture: &Fixture, task: &str, cookie: &str, mentioned: &[Id]) {
    let response = fixture
        .app
        .clone()
        .oneshot(json_request(
            "POST",
            &format!(
                "/api/v1/workspaces/{}/tasks/{task}/comments",
                fixture.workspace_id
            ),
            cookie,
            json!({
                "body": "a note",
                "mentioned_user_ids": mentioned.iter().map(ToString::to_string).collect::<Vec<_>>(),
            }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
}

/// `GET`, `PUT` or `DELETE` of the task's followers; `PUT` and `DELETE` go to `followers/me`.
async fn followers(
    fixture: &Fixture,
    task: &str,
    cookie: &str,
    method: &str,
) -> (StatusCode, Value) {
    let me = if method == "GET" { "" } else { "/me" };
    let response = fixture
        .app
        .clone()
        .oneshot(cookie_request(
            method,
            &format!(
                "/api/v1/workspaces/{}/tasks/{task}/followers{me}",
                fixture.workspace_id
            ),
            cookie,
        ))
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn follower_ids(fixture: &Fixture, task: &str) -> Vec<String> {
    let (status, value) = followers(fixture, task, &fixture.owner_cookie, "GET").await;
    assert_eq!(status, StatusCode::OK, "{value}");
    let mut ids: Vec<String> = value["user_ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|id| id.as_str().unwrap().to_owned())
        .collect();
    ids.sort();
    ids
}

fn sorted(ids: &[Id]) -> Vec<String> {
    let mut ids: Vec<String> = ids.iter().map(ToString::to_string).collect();
    ids.sort();
    ids
}

async fn owner_id(fixture: &Fixture) -> Id {
    sqlx::query_scalar::<_, String>("SELECT id FROM users WHERE email = 'owner@example.com'")
        .fetch_one(fixture.database.pool())
        .await
        .unwrap()
        .parse()
        .unwrap()
}

#[tokio::test]
async fn the_creator_assignees_note_authors_and_mentioned_members_follow() {
    let fixture = Fixture::new().await;
    let owner = owner_id(&fixture).await;
    let (assignee, _) = add_member(&fixture, "assignee@example.com").await;
    let (author, author_cookie) = add_member(&fixture, "author@example.com").await;
    let (mentioned, _) = add_member(&fixture, "mentioned@example.com").await;
    let (later, _) = add_member(&fixture, "later@example.com").await;
    let task = create_task(&fixture, &[assignee]).await;
    assert_eq!(
        follower_ids(&fixture, &task).await,
        sorted(&[owner, assignee])
    );

    comment(&fixture, &task, &author_cookie, &[mentioned]).await;
    assert_eq!(
        follower_ids(&fixture, &task).await,
        sorted(&[owner, assignee, author, mentioned])
    );

    // A member assigned later follows too.
    let (_, task_value) = get_task(&fixture, &task).await;
    let patched = fixture
        .app
        .clone()
        .oneshot(json_request(
            "PATCH",
            &format!("/api/v1/workspaces/{}/tasks/{task}", fixture.workspace_id),
            &fixture.owner_cookie,
            json!({ "expected_version": task_value["version"], "assignee_ids": [assignee.to_string(), later.to_string()] }),
        ))
        .await
        .unwrap();
    assert_eq!(patched.status(), StatusCode::OK);
    assert!(
        follower_ids(&fixture, &task)
            .await
            .contains(&later.to_string())
    );
}

#[tokio::test]
async fn a_note_notifies_the_followers_once_and_never_its_author() {
    let fixture = Fixture::new().await;
    let (assignee, assignee_cookie) = add_member(&fixture, "assignee@example.com").await;
    let (_author, author_cookie) = add_member(&fixture, "author@example.com").await;
    let task = create_task(&fixture, &[assignee]).await;

    // The note @mentions the assignee: a mention, not also a follower notification.
    comment(&fixture, &task, &author_cookie, &[assignee]).await;
    assert_eq!(
        kinds(&fixture, &assignee_cookie).await,
        ["comment_mentioned", "task_assigned"]
    );
    assert_eq!(
        kinds(&fixture, &fixture.owner_cookie).await,
        ["task_commented"]
    );
    assert!(kinds(&fixture, &author_cookie).await.is_empty());

    // The author follows now: the owner's note notifies them, and the assignee.
    comment(&fixture, &task, &fixture.owner_cookie, &[]).await;
    assert_eq!(kinds(&fixture, &author_cookie).await, ["task_commented"]);
    assert_eq!(
        kinds(&fixture, &assignee_cookie).await,
        ["task_commented", "comment_mentioned", "task_assigned"]
    );
    assert_eq!(
        kinds(&fixture, &fixture.owner_cookie).await,
        ["task_commented"]
    );
    let inbox = list_notifications(&fixture, &author_cookie, false).await;
    assert_eq!(inbox["items"][0]["task_id"], task);
    assert!(inbox["items"][0]["comment_id"].is_string());
}

#[tokio::test]
async fn unfollowed_members_and_guests_get_no_notifications() {
    let fixture = Fixture::new().await;
    let (assignee, assignee_cookie) = add_member(&fixture, "assignee@example.com").await;
    let (guest, guest_cookie) = add_person(&fixture, "guest@example.com", "guest").await;
    let task = create_task(&fixture, &[assignee]).await;

    let (status, value) = followers(&fixture, &task, &assignee_cookie, "DELETE").await;
    assert_eq!(status, StatusCode::OK, "{value}");
    assert_eq!(value["following"], false);
    comment(&fixture, &task, &fixture.owner_cookie, &[]).await;
    assert_eq!(kinds(&fixture, &assignee_cookie).await, ["task_assigned"]);

    // Following again brings the notifications back.
    let (status, value) = followers(&fixture, &task, &assignee_cookie, "PUT").await;
    assert_eq!(status, StatusCode::OK, "{value}");
    assert_eq!(value["following"], true);
    comment(&fixture, &task, &fixture.owner_cookie, &[]).await;
    assert_eq!(
        kinds(&fixture, &assignee_cookie).await,
        ["task_commented", "task_assigned"]
    );

    // A guest cannot follow, and a follower who became a guest is neither listed nor notified.
    for method in ["GET", "PUT", "DELETE"] {
        let (status, _) = followers(&fixture, &task, &guest_cookie, method).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{method}");
    }
    sqlx::query("INSERT INTO task_followers (task_id, user_id, created_at) VALUES ($1, $2, 1)")
        .bind(&task)
        .bind(guest.to_string())
        .execute(fixture.database.pool())
        .await
        .unwrap();
    assert!(
        !follower_ids(&fixture, &task)
            .await
            .contains(&guest.to_string())
    );
    comment(&fixture, &task, &fixture.owner_cookie, &[]).await;
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM notifications WHERE recipient_user_id = $1")
            .bind(guest.to_string())
            .fetch_one(fixture.database.pool())
            .await
            .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn a_status_change_notifies_the_followers_but_not_a_new_assignee_twice() {
    let fixture = Fixture::new().await;
    let (assignee, assignee_cookie) = add_member(&fixture, "assignee@example.com").await;
    let (newcomer, newcomer_cookie) = add_member(&fixture, "newcomer@example.com").await;
    let task = create_task(&fixture, &[assignee]).await;
    let other_status: String = sqlx::query_scalar(
        "SELECT id FROM task_statuses WHERE project_id = $1 AND id <> $2 ORDER BY position LIMIT 1",
    )
    .bind(&fixture.project_id)
    .bind(&fixture.status_id)
    .fetch_one(fixture.database.pool())
    .await
    .unwrap();
    let (_, value) = get_task(&fixture, &task).await;
    let patched = fixture
        .app
        .clone()
        .oneshot(json_request(
            "PATCH",
            &format!("/api/v1/workspaces/{}/tasks/{task}", fixture.workspace_id),
            &fixture.owner_cookie,
            json!({
                "expected_version": value["version"],
                "status_id": other_status,
                "assignee_ids": [assignee.to_string(), newcomer.to_string()],
            }),
        ))
        .await
        .unwrap();
    assert_eq!(patched.status(), StatusCode::OK);
    assert_eq!(
        kinds(&fixture, &assignee_cookie).await,
        ["task_status_changed", "task_assigned"]
    );
    assert_eq!(kinds(&fixture, &newcomer_cookie).await, ["task_assigned"]);
    assert!(kinds(&fixture, &fixture.owner_cookie).await.is_empty());

    // Another change notifies again; a PATCH without a status change does not.
    let (_, value) = get_task(&fixture, &task).await;
    let patched = fixture
        .app
        .clone()
        .oneshot(json_request(
            "PATCH",
            &format!("/api/v1/workspaces/{}/tasks/{task}", fixture.workspace_id),
            &fixture.owner_cookie,
            json!({ "expected_version": value["version"], "status_id": fixture.status_id, "title": "Renamed" }),
        ))
        .await
        .unwrap();
    assert_eq!(patched.status(), StatusCode::OK);
    let (_, value) = get_task(&fixture, &task).await;
    let patched = fixture
        .app
        .clone()
        .oneshot(json_request(
            "PATCH",
            &format!("/api/v1/workspaces/{}/tasks/{task}", fixture.workspace_id),
            &fixture.owner_cookie,
            json!({ "expected_version": value["version"], "title": "Renamed again" }),
        ))
        .await
        .unwrap();
    assert_eq!(patched.status(), StatusCode::OK);
    assert_eq!(
        kinds(&fixture, &newcomer_cookie).await,
        ["task_status_changed", "task_assigned"]
    );
}

#[tokio::test]
async fn nobody_follows_a_task_they_cannot_see() {
    let fixture = Fixture::new().await;
    let (member, member_cookie) = add_member(&fixture, "member@example.com").await;
    let task = create_task(&fixture, &[]).await;

    // Unknown task, then a trashed one.
    let (status, _) = followers(&fixture, &Id::new_v7().to_string(), &member_cookie, "PUT").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    sqlx::query("UPDATE tasks SET deleted_at = 1 WHERE id = $1")
        .bind(&task)
        .execute(fixture.database.pool())
        .await
        .unwrap();
    let (status, _) = followers(&fixture, &task, &member_cookie, "PUT").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    sqlx::query("UPDATE tasks SET deleted_at = NULL WHERE id = $1")
        .bind(&task)
        .execute(fixture.database.pool())
        .await
        .unwrap();

    // A removed member.
    sqlx::query("DELETE FROM memberships WHERE user_id = $1")
        .bind(member.to_string())
        .execute(fixture.database.pool())
        .await
        .unwrap();
    for method in ["GET", "PUT"] {
        let (status, _) = followers(&fixture, &task, &member_cookie, method).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{method}");
    }
    assert!(
        !follower_ids(&fixture, &task)
            .await
            .contains(&member.to_string())
    );
}

async fn get_task(fixture: &Fixture, task: &str) -> (StatusCode, Value) {
    let response = fixture
        .app
        .clone()
        .oneshot(cookie_request(
            "GET",
            &format!("/api/v1/workspaces/{}/tasks/{task}", fixture.workspace_id),
            &fixture.owner_cookie,
        ))
        .await
        .unwrap();
    let status = response.status();
    (status, response_json(response).await)
}

async fn list_notifications(fixture: &Fixture, cookie: &str, unread: bool) -> Value {
    let unread_query = if unread { "?unread=true" } else { "" };
    let response = fixture
        .app
        .clone()
        .oneshot(cookie_request(
            "GET",
            &format!(
                "/api/v1/workspaces/{}/notifications{unread_query}",
                fixture.workspace_id
            ),
            cookie,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    response_json(response).await
}

async fn add_member(fixture: &Fixture, email: &str) -> (Id, String) {
    add_person(fixture, email, "member").await
}

async fn add_person(fixture: &Fixture, email: &str, role: &str) -> (Id, String) {
    let id = Id::new_v7();
    let membership_id = Id::new_v7();
    let now = TimestampMillis::now();
    sqlx::query(
        "INSERT INTO users (id, email, normalized_email, display_name, password_hash, created_at, updated_at) \
         VALUES ($1, $2, $3, 'Member', 'unused', $4, $5)",
    )
    .bind(id.to_string())
    .bind(email)
    .bind(email)
    .bind(now.as_millis())
    .bind(now.as_millis())
    .execute(fixture.database.pool())
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO memberships (id, workspace_id, user_id, role, version, created_at, updated_at) \
         VALUES ($1, $2, $3, $6, 0, $4, $5)",
    )
    .bind(membership_id.to_string())
    .bind(&fixture.workspace_id)
    .bind(id.to_string())
    .bind(now.as_millis())
    .bind(now.as_millis())
    .bind(role)
    .execute(fixture.database.pool())
    .await
    .unwrap();
    let session = fixture
        .identity
        .create_session(
            &AuthenticatedUser {
                id,
                email: email.to_owned(),
                display_name: "Member".to_owned(),
            },
            now,
        )
        .await
        .unwrap();
    (id, format!("__Host-orbit_session={}", session.token))
}

fn json_request(method: &str, uri: &str, cookie: &str, value: Value) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::COOKIE, cookie)
        .body(Body::from(value.to_string()))
        .unwrap()
}

fn cookie_request(method: &str, uri: &str, cookie: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header(header::COOKIE, cookie)
        .body(Body::empty())
        .unwrap()
}

async fn response_json(response: axum::response::Response) -> Value {
    serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap()
}
