//! The iPhone app's push devices (APNs): registration, ownership, delivery and cleanup.

use std::sync::{Arc, Mutex};

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use futures_util::future::BoxFuture;
use orbit_platform::{AuthenticatedUser, Id, PasswordService, TestDatabase, TimestampMillis};
use orbit_server::auth_routes::{AuthState, CookieMode, auth_router};
use orbit_server::push::apns::DeviceSender;
use orbit_server::push::{Notice, PushSender, PushService, SendError, Subscription, Vapid};
use orbit_server::repositories::identity::{IdentityRepository, SetupRequest};
use orbit_server::task_routes::{TaskState, task_router};
use serde_json::{Value, json};
use tower::ServiceExt;

/// Records the device pushes. A token that starts with "dead" answers as APNs does for a removed
/// app (410), one with "bad" is refused every time, and one with "busy" is busy the first time.
#[derive(Default)]
struct FakeApns {
    sent: Mutex<Vec<(String, Notice)>>,
    tried: Mutex<Vec<String>>,
}

impl DeviceSender for FakeApns {
    fn send(&self, token: String, notice: Notice) -> BoxFuture<'static, Result<(), SendError>> {
        let earlier = {
            let mut tried = self.tried.lock().unwrap();
            tried.push(token.clone());
            tried.iter().filter(|tried| **tried == token).count() - 1
        };
        let result = if token.starts_with("dead") {
            Err(SendError::Gone)
        } else if token.starts_with("bad") {
            Err(SendError::Failed("refused".to_owned()))
        } else if token.starts_with("b0") && earlier == 0 {
            Err(SendError::Busy("busy".to_owned()))
        } else {
            self.sent.lock().unwrap().push((token, notice));
            Ok(())
        };
        Box::pin(async move { result })
    }
}

/// No browser subscribes in these tests.
struct NoWebPush;

impl PushSender for NoWebPush {
    fn send(
        &self,
        _: Vapid,
        _: Subscription,
        _: Vec<u8>,
    ) -> BoxFuture<'static, Result<(), SendError>> {
        Box::pin(async { Ok(()) })
    }
}

struct Fixture {
    database: TestDatabase,
    identity: Arc<IdentityRepository>,
    apns: Arc<FakeApns>,
    push: PushService,
    app: axum::Router,
    workspace_id: Id,
    owner_id: Id,
    owner_cookie: String,
}

/// A device token: 64 hex characters that start with `prefix` (hex too).
fn token(prefix: &str) -> String {
    format!("{prefix}{}", "0".repeat(64 - prefix.len()))
}

impl Fixture {
    async fn new() -> Self {
        let database = TestDatabase::new().await.unwrap();
        let apns = Arc::new(FakeApns::default());
        let push = PushService::with_senders(&database, Arc::new(NoWebPush), apns.clone());
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
        let owner_id: String = sqlx::query_scalar("SELECT id FROM users")
            .fetch_one(database.pool())
            .await
            .unwrap();
        let app = auth_router(AuthState::new(Arc::clone(&identity), CookieMode::secure())).merge(
            task_router(TaskState::new(Arc::clone(&identity), CookieMode::secure())),
        );
        Self {
            database,
            identity,
            apns,
            push,
            app,
            workspace_id: setup.workspace_id,
            owner_id: owner_id.parse().unwrap(),
            owner_cookie: format!("__Host-orbit_session={}", setup.session.token),
        }
    }

    async fn member(&self, name: &str) -> (Id, String) {
        let id = Id::new_v7();
        let now = TimestampMillis::now();
        let email = format!("{name}@example.com");
        sqlx::query(
            "INSERT INTO users (id, email, normalized_email, display_name, password_hash, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, 'unused', $5, $6)",
        )
        .bind(id.to_string())
        .bind(&email)
        .bind(&email)
        .bind(name)
        .bind(now.as_millis())
        .bind(now.as_millis())
        .execute(self.database.pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO memberships (id, workspace_id, user_id, role, version, created_at, updated_at) \
             VALUES ($1, $2, $3, 'member', 0, $4, $5)",
        )
        .bind(Id::new_v7().to_string())
        .bind(self.workspace_id.to_string())
        .bind(id.to_string())
        .bind(now.as_millis())
        .bind(now.as_millis())
        .execute(self.database.pool())
        .await
        .unwrap();
        let user = AuthenticatedUser {
            id,
            email,
            display_name: name.to_owned(),
        };
        let session = self.identity.create_session(&user, now).await.unwrap();
        (id, format!("__Host-orbit_session={}", session.token))
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

    async fn register(&self, cookie: &str, token: &str) -> (StatusCode, Value) {
        let body = json!({ "token": token, "label": "iPhone" });
        self.send("POST", "/api/v1/push/devices", cookie, Some(body))
            .await
    }

    /// The tokens of the user's device rows.
    async fn devices(&self, user_id: Id) -> Vec<String> {
        sqlx::query_scalar("SELECT token FROM push_devices WHERE user_id = $1 ORDER BY token")
            .bind(user_id.to_string())
            .fetch_all(self.database.pool())
            .await
            .unwrap()
    }

    fn sent(&self) -> Vec<(String, Notice)> {
        std::mem::take(&mut self.apns.sent.lock().unwrap())
    }
}

fn notice() -> Notice {
    Notice {
        title: "Ada".to_owned(),
        body: "Hello".to_owned(),
        url: "/chat/1".to_owned(),
        tag: "chat:1".to_owned(),
        sound: "mention",
        icon: None,
    }
}

#[tokio::test]
async fn a_device_is_registered_for_its_user_and_only_its_user_removes_it() {
    let fixture = Fixture::new().await;
    let (ada, ada_cookie) = fixture.member("ada").await;
    let owner = &fixture.owner_cookie;

    // Only an APNs token is taken (it names the URL the server sends to).
    for bad in ["abc", "../../3/device/x", &"zz".repeat(32)] {
        let (status, problem) = fixture.register(owner, bad).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{bad}");
        assert_eq!(problem["code"], "invalid_device");
    }
    let phone = token("a1");
    let (status, record) = fixture.register(owner, &phone.to_uppercase()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(record["label"], "iPhone");
    // The same phone again: still one row, stored in lowercase.
    fixture.register(owner, &phone).await;
    assert_eq!(
        fixture.devices(fixture.owner_id).await,
        std::slice::from_ref(&phone)
    );

    // Somebody else cannot remove it (no error tells them it exists).
    let uri = format!("/api/v1/push/devices/{phone}");
    let (status, _) = fixture.send("DELETE", &uri, &ada_cookie, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(
        fixture.devices(fixture.owner_id).await,
        std::slice::from_ref(&phone)
    );

    // The phone signs in as Ada: the row moves to her, and the owner gets nothing more on it.
    fixture.register(&ada_cookie, &phone).await;
    assert!(fixture.devices(fixture.owner_id).await.is_empty());
    assert_eq!(
        fixture
            .push
            .push(fixture.owner_id, &notice())
            .await
            .unwrap(),
        0
    );
    assert_eq!(fixture.push.push(ada, &notice()).await.unwrap(), 1);
    assert_eq!(fixture.sent()[0].0, phone);

    // Its user removes it.
    let (status, _) = fixture.send("DELETE", &uri, &ada_cookie, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(fixture.devices(ada).await.is_empty());
    let (status, _) = fixture
        .send(
            "DELETE",
            "/api/v1/push/devices/not-a-token",
            &ada_cookie,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    // Without a session, nothing.
    let (status, _) = fixture.register("", &phone).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_signed_out_session_stops_its_devices_pushes() {
    let fixture = Fixture::new().await;
    let (ada, ada_cookie) = fixture.member("ada").await;
    let phone = token("c0");
    fixture.register(&ada_cookie, &phone).await;
    assert_eq!(fixture.push.push(ada, &notice()).await.unwrap(), 1);
    fixture.sent();

    let (status, _) = fixture
        .send("POST", "/api/v1/auth/logout", &ada_cookie, None)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(fixture.push.push(ada, &notice()).await.unwrap(), 0);
    assert!(fixture.sent().is_empty());
}

#[tokio::test]
async fn apns_cleanup_removes_a_device_it_no_longer_knows_and_retries_a_busy_one() {
    let fixture = Fixture::new().await;
    let (ada, ada_cookie) = fixture.member("ada").await;
    let (good, dead, busy, bad) = (token("a0"), token("dead"), token("b0"), token("bad"));
    for phone in [&good, &dead, &busy, &bad] {
        fixture.register(&ada_cookie, phone).await;
    }

    // The good one and the busy one (on its second try) take it; the removed app's row goes.
    assert_eq!(fixture.push.push(ada, &notice()).await.unwrap(), 2);
    let mut sent: Vec<String> = fixture.sent().into_iter().map(|(token, _)| token).collect();
    sent.sort();
    assert_eq!(sent, [good.clone(), busy.clone()]);
    let mut left = vec![good.clone(), busy.clone(), bad.clone()];
    left.sort();
    assert_eq!(fixture.devices(ada).await, left);

    // A device that is refused again and again goes after a few pushes.
    for _ in 0..5 {
        fixture.push.push(ada, &notice()).await.unwrap();
    }
    assert!(!fixture.devices(ada).await.contains(&bad));
    assert!(fixture.devices(ada).await.contains(&good));
}

#[tokio::test]
async fn nobody_gets_a_push_for_what_they_did_themselves() {
    let fixture = Fixture::new().await;
    let (ada, ada_cookie) = fixture.member("ada").await;
    fixture.register(&ada_cookie, &token("a2")).await;
    let project: String = sqlx::query_scalar("SELECT id FROM projects LIMIT 1")
        .fetch_one(fixture.database.pool())
        .await
        .unwrap();
    let status: String = sqlx::query_scalar(
        "SELECT id FROM task_statuses WHERE project_id = $1 ORDER BY position LIMIT 1",
    )
    .bind(&project)
    .fetch_one(fixture.database.pool())
    .await
    .unwrap();
    let (created, task) = fixture
        .send(
            "POST",
            &format!("/api/v1/workspaces/{}/tasks", fixture.workspace_id),
            &fixture.owner_cookie,
            Some(json!({ "project_id": project, "status_id": status, "title": "Ship it" })),
        )
        .await;
    assert_eq!(created, StatusCode::CREATED);
    // Ada's own action as an inbox row (a writer that forgot to skip her), then the owner's.
    for (actor, key) in [(ada, "self"), (fixture.owner_id, "owner")] {
        sqlx::query(
            "INSERT INTO notifications (id, workspace_id, recipient_user_id, actor_user_id, kind, \
             task_id, dedupe_key, created_at) VALUES ($1, $2, $3, $4, 'task_assigned', $5, $6, $7)",
        )
        .bind(Id::new_v7().to_string())
        .bind(fixture.workspace_id.to_string())
        .bind(ada.to_string())
        .bind(actor.to_string())
        .bind(task["id"].as_str().unwrap())
        .bind(key)
        .bind(TimestampMillis::now().as_millis())
        .execute(fixture.database.pool())
        .await
        .unwrap();
    }
    assert_eq!(fixture.push.push_inbox().await.unwrap(), 2);
    let sent = fixture.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].1.title, "Owner assigned you a task");
    assert_eq!(sent[0].1.url, "/activity");
}
