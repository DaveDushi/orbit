//! A workspace with an owner and the task routes, for tests that call them as people of
//! several roles. Runs on SQLite, or on Postgres with `ORBIT_TEST_POSTGRES_URL`.

#![allow(dead_code)]

use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use orbit_platform::{AuthenticatedUser, Id, PasswordService, TestDatabase, TimestampMillis};
use orbit_server::auth_routes::CookieMode;
use orbit_server::repositories::identity::{IdentityRepository, SetupRequest};
use orbit_server::task_routes::{TaskState, task_router};
use serde_json::{Value, json};
use tower::ServiceExt;

pub struct Fixture {
    pub database: TestDatabase,
    identity: Arc<IdentityRepository>,
    app: axum::Router,
    pub workspace_id: String,
    pub project_id: String,
    pub owner: Person,
}

pub struct Person {
    pub id: Id,
    pub cookie: String,
}

impl Fixture {
    pub async fn new() -> Self {
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
        let app = task_router(TaskState::new(Arc::clone(&identity), CookieMode::secure()));
        Self {
            database,
            identity,
            app,
            workspace_id: setup.workspace_id.to_string(),
            project_id: setup.project_id.to_string(),
            owner: Person {
                id: setup.user_id,
                cookie: format!("__Host-orbit_session={}", setup.session.token),
            },
        }
    }

    /// A user with a session and a membership of `role`.
    pub async fn person(&self, name: &str, role: &str) -> Person {
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
             VALUES ($1, $2, $3, $4, 0, $5, $6)",
        )
        .bind(Id::new_v7().to_string())
        .bind(&self.workspace_id)
        .bind(id.to_string())
        .bind(role)
        .bind(now.as_millis())
        .bind(now.as_millis())
        .execute(self.database.pool())
        .await
        .unwrap();
        let session = self
            .identity
            .create_session(
                &AuthenticatedUser {
                    id,
                    email,
                    display_name: name.to_owned(),
                },
                now,
            )
            .await
            .unwrap();
        Person {
            id,
            cookie: format!("__Host-orbit_session={}", session.token),
        }
    }

    /// Calls `/api/v1/workspaces/{workspace}{path}` as `person`.
    pub async fn call(
        &self,
        person: &Person,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let builder = Request::builder()
            .method(method)
            .uri(format!("/api/v1/workspaces/{}{path}", self.workspace_id))
            .header(header::COOKIE, &person.cookie);
        let request = match body {
            Some(body) => builder
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string())),
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

    /// A call that must succeed; returns its body.
    pub async fn ok(&self, person: &Person, method: &str, path: &str, body: Option<Value>) -> Value {
        let (status, value) = self.call(person, method, path, body).await;
        assert!(status.is_success(), "{method} {path}: {status} {value}");
        value
    }

    /// The first status of the General project in `category`.
    pub async fn status(&self, category: &str) -> String {
        sqlx::query_scalar(
            "SELECT id FROM task_statuses WHERE project_id = $1 AND category = $2 ORDER BY position LIMIT 1",
        )
        .bind(&self.project_id)
        .bind(category)
        .fetch_one(self.database.pool())
        .await
        .unwrap()
    }

    /// A new open task in General; `fields` adds to the body.
    pub async fn task(&self, person: &Person, title: &str, fields: Value) -> Value {
        let mut body = json!({
            "project_id": self.project_id,
            "status_id": self.status("unstarted").await,
            "title": title,
        });
        body.as_object_mut()
            .unwrap()
            .extend(fields.as_object().unwrap().clone());
        self.ok(person, "POST", "/tasks", Some(body)).await
    }

    /// Applies `change` to `task` at its current version; returns the updated task.
    pub async fn update(&self, person: &Person, task: &Value, change: Value) -> Value {
        let mut change = change;
        change["expected_version"] = task["version"].clone();
        self.ok(
            person,
            "PATCH",
            &format!("/tasks/{}", task["id"].as_str().unwrap()),
            Some(change),
        )
        .await
    }
}
