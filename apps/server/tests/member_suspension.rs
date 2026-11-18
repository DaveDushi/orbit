//! Settings → Members: owners and admins suspend and resume a member. A suspension ends every way in at once and
//! resume lets the same password sign in again; the owner, yourself, other workspaces' members and accounts that
//! are also in another workspace are out of reach.

use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use orbit_platform::{AuthenticatedUser, Id, PasswordService, TestDatabase, TimestampMillis};
use orbit_server::auth_routes::{AuthState, CookieMode, auth_router};
use orbit_server::realtime::authorized;
use orbit_server::repositories::api_tokens::{ApiTokenRepository, ApiTokenScope};
use orbit_server::repositories::identity::{IdentityRepository, SetupRequest};
use orbit_server::workspace_routes::{WorkspaceState, workspace_router};
use serde_json::{Value, json};
use tower::ServiceExt;
use webauthn_authenticator_rs::WebauthnAuthenticator;
use webauthn_authenticator_rs::prelude::Url;
use webauthn_authenticator_rs::softpasskey::SoftPasskey;

const PASSWORD: &str = "correct horse battery";
const ORIGIN: &str = "https://orbit.test";

struct Fixture {
    db: TestDatabase,
    identity: Arc<IdentityRepository>,
    app: axum::Router,
    workspace: String,
    owner: Person,
}

#[derive(Clone)]
struct Person {
    id: Id,
    email: String,
    cookie: String,
    /// Their membership in the fixture's workspace (empty when they have none).
    membership: String,
}

impl Fixture {
    async fn new() -> Self {
        let db = TestDatabase::new().await.unwrap();
        let identity = Arc::new(IdentityRepository::new((*db).clone()));
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
                    email: "owner@example.com".into(),
                    display_name: "Owner".into(),
                    password_hash: PasswordService::default().hash(PASSWORD).unwrap(),
                    workspace_name: "Orbit".into(),
                    project_name: "General".into(),
                },
                now,
            )
            .await
            .unwrap();
        let workspace = setup.workspace_id.to_string();
        let app = auth_router(
            AuthState::new(Arc::clone(&identity), CookieMode::secure())
                .with_app_key([7; 32])
                .with_public_origin(ORIGIN),
        )
        .merge(workspace_router(WorkspaceState::new(
            Arc::clone(&identity),
            ORIGIN.to_owned(),
            CookieMode::secure(),
        )));
        let owner_membership: String = sqlx::query_scalar(
            "SELECT id FROM memberships WHERE workspace_id = $1 AND user_id = $2",
        )
        .bind(&workspace)
        .bind(setup.user_id.to_string())
        .fetch_one(db.pool())
        .await
        .unwrap();
        let owner = Person {
            id: setup.user_id,
            email: "owner@example.com".into(),
            cookie: format!("__Host-orbit_session={}", setup.session.token),
            membership: owner_membership,
        };
        Self {
            db,
            identity,
            app,
            workspace,
            owner,
        }
    }

    /// A new account with `PASSWORD` and a session, a member of `workspace` with `role` when given.
    async fn person(&self, email: &str, workspace: Option<(&str, &str)>) -> Person {
        let id = Id::new_v7();
        let now = TimestampMillis::now();
        sqlx::query(
            "INSERT INTO users (id, email, normalized_email, display_name, password_hash, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(id.to_string())
        .bind(email)
        .bind(email.to_ascii_lowercase())
        .bind(email.split('@').next().unwrap())
        .bind(PasswordService::default().hash(PASSWORD).unwrap())
        .bind(now.as_millis())
        .bind(now.as_millis())
        .execute(self.db.pool())
        .await
        .unwrap();
        let session = self
            .identity
            .create_session(
                &AuthenticatedUser {
                    id,
                    email: email.into(),
                    display_name: email.into(),
                },
                now,
            )
            .await
            .unwrap();
        let membership = match workspace {
            Some((workspace, role)) => self.join(id, workspace, role).await,
            None => String::new(),
        };
        Person {
            id,
            email: email.into(),
            cookie: format!("__Host-orbit_session={}", session.token),
            membership,
        }
    }

    async fn join(&self, user: Id, workspace: &str, role: &str) -> String {
        let membership = Id::new_v7().to_string();
        let now = TimestampMillis::now().as_millis();
        sqlx::query(
            "INSERT INTO memberships (id, workspace_id, user_id, role, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(&membership)
        .bind(workspace)
        .bind(user.to_string())
        .bind(role)
        .bind(now)
        .bind(now)
        .execute(self.db.pool())
        .await
        .unwrap();
        membership
    }

    async fn send(
        &self,
        method: &str,
        uri: &str,
        cookie: Option<&str>,
        body: Value,
    ) -> (StatusCode, Value) {
        let mut builder = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::ORIGIN, ORIGIN);
        if let Some(cookie) = cookie {
            builder = builder.header(header::COOKIE, cookie);
        }
        let response = self
            .app
            .clone()
            .oneshot(builder.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    async fn suspend(
        &self,
        actor: &Person,
        workspace: &str,
        membership: &str,
        suspended: bool,
    ) -> (StatusCode, Value) {
        self.send(
            "POST",
            &format!("/api/v1/workspaces/{workspace}/members/{membership}/suspension"),
            Some(&actor.cookie),
            json!({"suspended": suspended}),
        )
        .await
    }

    async fn login(&self, person: &Person) -> StatusCode {
        self.send(
            "POST",
            "/api/v1/auth/login",
            None,
            json!({"email": person.email, "password": PASSWORD}),
        )
        .await
        .0
    }

    async fn suspended(&self, person: &Person) -> bool {
        sqlx::query_scalar::<_, Option<i64>>("SELECT suspended_at FROM users WHERE id = $1")
            .bind(person.id.to_string())
            .fetch_one(self.db.pool())
            .await
            .unwrap()
            .is_some()
    }

    async fn audit(&self, action: &str, actor: &Person) -> Vec<Value> {
        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT outcome, metadata_json FROM audit_events WHERE workspace_id = $1 AND action = $2 \
             AND actor_id = $3 ORDER BY occurred_at",
        )
        .bind(&self.workspace)
        .bind(action)
        .bind(actor.id.to_string())
        .fetch_all(self.db.pool())
        .await
        .unwrap();
        rows.into_iter()
            .map(|(outcome, metadata)| json!({"outcome": outcome, "metadata": serde_json::from_str::<Value>(&metadata).unwrap()}))
            .collect()
    }

    async fn count(&self, sql: &str, user: Id) -> i64 {
        sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(sql.to_owned()))
            .bind(user.to_string())
            .fetch_one(self.db.pool())
            .await
            .unwrap()
    }

    /// Signs in with the soft passkey; returns the status of the finish step.
    async fn passkey_login(
        &self,
        authenticator: &mut SoftPasskey,
        credential_id: &str,
        user_handle: &Value,
    ) -> StatusCode {
        let (status, login) = self
            .send("POST", "/api/v1/auth/passkey/login/start", None, json!({}))
            .await;
        assert_eq!(status, StatusCode::OK);
        let mut options = login["options"].clone();
        options["publicKey"]["allowCredentials"] =
            json!([{"type": "public-key", "id": credential_id}]);
        let assertion = authenticator
            .do_authentication(
                Url::parse(ORIGIN).unwrap(),
                serde_json::from_value(options).unwrap(),
            )
            .unwrap();
        let mut answer = serde_json::to_value(assertion).unwrap();
        answer["response"]["userHandle"] = user_handle.clone();
        self.send(
            "POST",
            "/api/v1/auth/passkey/login/finish",
            None,
            json!({"challenge_token": login["challenge_token"], "credential": answer}),
        )
        .await
        .0
    }
}

#[tokio::test]
async fn a_suspension_ends_every_way_in_and_resume_lets_the_same_password_sign_in() {
    let f = Fixture::new().await;
    let workspace = f.workspace.clone();
    // An admin, so they can make an API token of their own.
    let member = f
        .person("apple-review@example.com", Some((&workspace, "admin")))
        .await;
    let session_id: String = sqlx::query_scalar("SELECT id FROM sessions WHERE user_id = $1")
        .bind(member.id.to_string())
        .fetch_one(f.db.pool())
        .await
        .unwrap();
    let session_id: Id = session_id.parse().unwrap();

    // A passkey of their own.
    let mut authenticator = SoftPasskey::new(true);
    let (status, registration) = f
        .send(
            "POST",
            "/api/v1/auth/passkeys/register/start",
            Some(&member.cookie),
            json!({"password": PASSWORD}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{registration}");
    let credential = authenticator
        .do_registration(
            Url::parse(ORIGIN).unwrap(),
            serde_json::from_value(registration["options"].clone()).unwrap(),
        )
        .unwrap();
    let (status, _) = f
        .send(
            "POST",
            "/api/v1/auth/passkeys/register/finish",
            Some(&member.cookie),
            json!({"challenge_token": registration["challenge_token"], "name": "Soft key", "credential": credential}),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let user_handle = registration["options"]["publicKey"]["user"]["id"].clone();

    // An API token, an MCP (OAuth) grant, a web push subscription and an iPhone push device.
    let project: String = sqlx::query_scalar(
        "SELECT id FROM projects WHERE workspace_id = $1 ORDER BY created_at LIMIT 1",
    )
    .bind(&workspace)
    .fetch_one(f.db.pool())
    .await
    .unwrap();
    let (status, created) = f
        .send(
            "POST",
            &format!("/api/v1/workspaces/{workspace}/api-tokens"),
            Some(&member.cookie),
            json!({"name": "Script", "project_ids": [project], "scopes": ["tasks:read"]}),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let token = created["token"].as_str().unwrap().to_owned();
    let now = TimestampMillis::now().as_millis();
    sqlx::query("INSERT INTO oauth_clients (id, name, redirect_uris, created_at) VALUES ('client', 'Codex', '[]', $1)")
        .bind(now)
        .execute(f.db.pool())
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO oauth_grants (id, client_id, user_id, workspace_id, project_ids, resource, scope, created_at) \
         VALUES ('grant', 'client', $1, $2, '[]', 'https://orbit.test/mcp', 'tasks:read', $3)",
    )
    .bind(member.id.to_string())
    .bind(&workspace)
    .bind(now)
    .execute(f.db.pool())
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO push_subscriptions (id, user_id, session_id, endpoint, p256dh, auth, label, created_at) \
         VALUES ('sub', $1, $2, 'https://push.example/1', 'key', 'auth', 'Firefox', $3)",
    )
    .bind(member.id.to_string())
    .bind(session_id.to_string())
    .bind(now)
    .execute(f.db.pool())
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO push_devices (id, user_id, session_id, token, label, created_at) \
         VALUES ('device', $1, $2, $3, 'iPhone', $4)",
    )
    .bind(member.id.to_string())
    .bind(session_id.to_string())
    .bind("ab".repeat(32))
    .bind(now)
    .execute(f.db.pool())
    .await
    .unwrap();
    let tokens = ApiTokenRepository::new((*f.db).clone());
    assert!(
        tokens
            .authenticate(
                &token,
                Some(ApiTokenScope::TasksRead),
                TimestampMillis::now()
            )
            .await
            .is_ok()
    );
    assert!(
        authorized(&f.db, workspace.parse().unwrap(), session_id)
            .await
            .unwrap()
    );

    let (status, problem) = f
        .suspend(&f.owner, &workspace, &member.membership, true)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{problem}");

    // Session (cookie and bearer), live socket, password, passkey, API token, MCP grant and push all end.
    let (status, _) = f
        .send("GET", "/api/v1/auth/me", Some(&member.cookie), Value::Null)
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(
        !authorized(&f.db, workspace.parse().unwrap(), session_id)
            .await
            .unwrap()
    );
    assert_eq!(f.login(&member).await, StatusCode::UNAUTHORIZED);
    assert_ne!(
        f.passkey_login(&mut authenticator, &credential.id, &user_handle)
            .await,
        StatusCode::OK
    );
    assert!(
        tokens
            .authenticate(
                &token,
                Some(ApiTokenScope::TasksRead),
                TimestampMillis::now()
            )
            .await
            .is_err()
    );
    assert_eq!(
        f.count(
            "SELECT COUNT(*) FROM oauth_grants WHERE user_id = $1 AND revoked_at IS NULL",
            member.id
        )
        .await,
        0
    );
    assert_eq!(
        f.count(
            "SELECT COUNT(*) FROM push_subscriptions WHERE user_id = $1",
            member.id
        )
        .await,
        0
    );
    assert_eq!(
        f.count(
            "SELECT COUNT(*) FROM push_devices WHERE user_id = $1",
            member.id
        )
        .await,
        0
    );
    assert_eq!(
        f.count(
            "SELECT COUNT(*) FROM sessions WHERE user_id = $1 AND revoked_at IS NULL",
            member.id
        )
        .await,
        0
    );
    let suspended = f.audit("membership.suspended", &f.owner).await;
    assert_eq!(suspended.len(), 1);
    assert_eq!(suspended[0]["metadata"]["user_id"], member.id.to_string());

    // The member list shows the suspension, and the owner may resume.
    let (_, members) = f
        .send(
            "GET",
            &format!("/api/v1/workspaces/{workspace}/members"),
            Some(&f.owner.cookie),
            Value::Null,
        )
        .await;
    let row = members["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["user_id"] == member.id.to_string())
        .unwrap();
    assert!(row["suspended_at"].is_string());
    assert_eq!(row["can_suspend"], true);

    let (status, _) = f
        .suspend(&f.owner, &workspace, &member.membership, false)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(!f.suspended(&member).await);
    assert_eq!(f.login(&member).await, StatusCode::OK);
    assert_eq!(
        f.passkey_login(&mut authenticator, &credential.id, &user_handle)
            .await,
        StatusCode::OK
    );
    // What the suspension ended stays ended.
    assert!(
        tokens
            .authenticate(
                &token,
                Some(ApiTokenScope::TasksRead),
                TimestampMillis::now()
            )
            .await
            .is_err()
    );
    assert!(
        !authorized(&f.db, workspace.parse().unwrap(), session_id)
            .await
            .unwrap()
    );
    assert_eq!(f.audit("membership.resumed", &f.owner).await.len(), 1);
}

#[tokio::test]
async fn only_owners_and_admins_suspend_and_never_the_owner_themselves_or_another_workspace() {
    let f = Fixture::new().await;
    let workspace = f.workspace.clone();
    let admin = f
        .person("admin@example.com", Some((&workspace, "admin")))
        .await;
    let other_admin = f
        .person("admin2@example.com", Some((&workspace, "admin")))
        .await;
    let member = f
        .person("member@example.com", Some((&workspace, "member")))
        .await;
    let target = f
        .person("target@example.com", Some((&workspace, "member")))
        .await;

    // A second workspace with its own owner and member.
    let stranger_owner = f.person("other-owner@example.com", None).await;
    let (status, second) = f
        .send(
            "POST",
            "/api/v1/workspaces",
            Some(&stranger_owner.cookie),
            json!({"name": "Second"}),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{second}");
    let second = second["id"].as_str().unwrap().to_owned();
    let outsider = f
        .person("outsider@example.com", Some((&second, "member")))
        .await;

    let forbidden = StatusCode::FORBIDDEN;
    // A member may not; nobody suspends the owner or themselves; an admin may not suspend an admin.
    assert_eq!(
        f.suspend(&member, &workspace, &target.membership, true)
            .await
            .0,
        forbidden
    );
    assert_eq!(
        f.suspend(&admin, &workspace, &f.owner.membership, true)
            .await
            .0,
        forbidden
    );
    assert_eq!(
        f.suspend(&f.owner, &workspace, &f.owner.membership, true)
            .await
            .0,
        forbidden
    );
    assert_eq!(
        f.suspend(&admin, &workspace, &admin.membership, true)
            .await
            .0,
        forbidden
    );
    assert_eq!(
        f.suspend(&admin, &workspace, &other_admin.membership, true)
            .await
            .0,
        forbidden
    );
    // IDOR: another workspace's membership is not found through this workspace, and this admin has no role there.
    assert_eq!(
        f.suspend(&admin, &workspace, &outsider.membership, true)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_ne!(
        f.suspend(&admin, &second, &outsider.membership, true)
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    for person in [&f.owner, &admin, &other_admin, &target, &outsider] {
        assert!(!f.suspended(person).await, "{}", person.email);
    }
    let denied = f.audit("membership.suspension_denied", &admin).await;
    assert_eq!(denied.len(), 3);
    assert!(denied.iter().all(|event| event["outcome"] == "failure"));

    // The owner may suspend an admin; an admin may suspend a member.
    assert_eq!(
        f.suspend(&f.owner, &workspace, &other_admin.membership, true)
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        f.suspend(&admin, &workspace, &target.membership, true)
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert!(f.suspended(&other_admin).await && f.suspended(&target).await);
    assert_eq!(f.audit("membership.suspended", &admin).await.len(), 1);

    // Someone who is also in another workspace is not locked out by this one.
    let shared = f
        .person("shared@example.com", Some((&workspace, "member")))
        .await;
    f.join(shared.id, &second, "member").await;
    let (status, problem) = f
        .suspend(&f.owner, &workspace, &shared.membership, true)
        .await;
    assert_eq!(
        (status, problem["code"].as_str()),
        (StatusCode::CONFLICT, Some("member_elsewhere"))
    );
    assert!(!f.suspended(&shared).await);

    // An instance admin's suspension is not the workspace's to lift.
    let (status, _) = f
        .send(
            "POST",
            &format!("/api/v1/admin/users/{}/suspension", member.id),
            Some(&f.owner.cookie),
            json!({"suspended": true}),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, problem) = f
        .suspend(&f.owner, &workspace, &member.membership, false)
        .await;
    assert_eq!(
        (status, problem["code"].as_str()),
        (StatusCode::CONFLICT, Some("suspended_by_admin"))
    );
    assert!(f.suspended(&member).await);
    // While a workspace suspension is lifted by Admin → Users too.
    let (status, _) = f
        .send(
            "POST",
            &format!("/api/v1/admin/users/{}/suspension", target.id),
            Some(&f.owner.cookie),
            json!({"suspended": false}),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(!f.suspended(&target).await);
}

impl Fixture {
    async fn remove(&self, actor: &Person, membership: &str) -> StatusCode {
        let version: i64 = sqlx::query_scalar("SELECT version FROM memberships WHERE id = $1")
            .bind(membership)
            .fetch_one(self.db.pool())
            .await
            .unwrap();
        self.send(
            "DELETE",
            &format!(
                "/api/v1/workspaces/{}/members/{membership}?expected_version={version}",
                self.workspace
            ),
            Some(&actor.cookie),
            Value::Null,
        )
        .await
        .0
    }
}

#[tokio::test]
async fn removing_a_member_lifts_this_workspaces_suspension_so_they_sign_in_again() {
    let f = Fixture::new().await;
    let workspace = f.workspace.clone();
    let member = f
        .person("member@example.com", Some((&workspace, "member")))
        .await;
    assert_eq!(
        f.suspend(&f.owner, &workspace, &member.membership, true)
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(f.login(&member).await, StatusCode::UNAUTHORIZED);

    assert_eq!(
        f.remove(&f.owner, &member.membership).await,
        StatusCode::NO_CONTENT
    );
    assert!(!f.suspended(&member).await);
    let workspace_of_suspension: Option<String> =
        sqlx::query_scalar("SELECT suspended_workspace_id FROM users WHERE id = $1")
            .bind(member.id.to_string())
            .fetch_one(f.db.pool())
            .await
            .unwrap();
    assert_eq!(workspace_of_suspension, None);
    // The same password signs in (as the iPhone app does, with a bearer token), and the session reaches the API.
    let call = |request: Request<Body>| async {
        let response = f.app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, serde_json::from_slice::<Value>(&bytes).unwrap())
    };
    let (status, login) = call(
        Request::builder()
            .method("POST")
            .uri("/api/v1/auth/login")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                json!({"email": member.email, "password": PASSWORD}).to_string(),
            ))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{login}");
    let token = login["session_token"].as_str().unwrap();
    let (status, me) = call(
        Request::builder()
            .uri("/api/v1/auth/me")
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{me}");
    let removed = f.audit("membership.removed", &f.owner).await;
    assert_eq!(removed.len(), 1);
    assert_eq!(removed[0]["metadata"]["suspension_lifted"], true);
}

#[tokio::test]
async fn removing_a_member_keeps_an_instance_admins_suspension() {
    let f = Fixture::new().await;
    let workspace = f.workspace.clone();
    let member = f
        .person("member@example.com", Some((&workspace, "member")))
        .await;
    let (status, _) = f
        .send(
            "POST",
            &format!("/api/v1/admin/users/{}/suspension", member.id),
            Some(&f.owner.cookie),
            json!({"suspended": true}),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    assert_eq!(
        f.remove(&f.owner, &member.membership).await,
        StatusCode::NO_CONTENT
    );
    assert!(f.suspended(&member).await);
    assert_eq!(f.login(&member).await, StatusCode::UNAUTHORIZED);
    let removed = f.audit("membership.removed", &f.owner).await;
    assert_eq!(removed[0]["metadata"]["suspension_lifted"], false);
}

#[tokio::test]
async fn removing_a_member_keeps_another_workspaces_suspension() {
    let f = Fixture::new().await;
    let workspace = f.workspace.clone();
    // A second workspace suspends its only-workspace member, who then joins this workspace too.
    let other_owner = f.person("other-owner@example.com", None).await;
    let (status, second) = f
        .send(
            "POST",
            "/api/v1/workspaces",
            Some(&other_owner.cookie),
            json!({"name": "Second"}),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{second}");
    let second = second["id"].as_str().unwrap().to_owned();
    let member = f
        .person("member@example.com", Some((&second, "member")))
        .await;
    assert_eq!(
        f.suspend(&other_owner, &second, &member.membership, true)
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    let here = f.join(member.id, &workspace, "member").await;

    assert_eq!(f.remove(&f.owner, &here).await, StatusCode::NO_CONTENT);
    assert!(f.suspended(&member).await);
    let workspace_of_suspension: Option<String> =
        sqlx::query_scalar("SELECT suspended_workspace_id FROM users WHERE id = $1")
            .bind(member.id.to_string())
            .fetch_one(f.db.pool())
            .await
            .unwrap();
    assert_eq!(workspace_of_suspension, Some(second));
    assert_eq!(f.login(&member).await, StatusCode::UNAUTHORIZED);
}
