//! Label rules: a label assigns people, on every path that gives a task a label (web/API create
//! and update, GitHub sync, Discord), and only owners and admins manage the rules.

use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use orbit_platform::{AuthenticatedUser, Id, PasswordService, TestDatabase, TimestampMillis};
use orbit_server::auth_routes::CookieMode;
use orbit_server::label_rule_routes::label_rule_router;
use orbit_server::repositories::identity::{IdentityRepository, SetupRequest};
use orbit_server::repositories::tasks::{DiscordTask, GithubLabel, GithubWorkItem, TaskRepository};
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
    owner_id: Id,
}

struct Person {
    id: Id,
    cookie: String,
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
        let state = TaskState::new(Arc::clone(&identity), CookieMode::secure());
        let app = task_router(state.clone()).merge(label_rule_router(state));
        Self {
            database,
            identity,
            app,
            workspace_id: setup.workspace_id.to_string(),
            project_id: setup.project_id.to_string(),
            status_id,
            owner_cookie: format!("__Host-orbit_session={}", setup.session.token),
            owner_id: setup.user_id,
        }
    }

    async fn person(&self, email: &str, name: &str, role: &str) -> Person {
        let id = Id::new_v7();
        let now = TimestampMillis::now();
        sqlx::query(
            "INSERT INTO users (id, email, normalized_email, display_name, password_hash, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, 'unused', $5, $6)",
        )
        .bind(id.to_string())
        .bind(email)
        .bind(email)
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
                    email: email.to_owned(),
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

    async fn call(
        &self,
        method: &str,
        path: &str,
        cookie: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let builder = Request::builder()
            .method(method)
            .uri(format!("/api/v1/workspaces/{}{path}", self.workspace_id))
            .header(header::COOKIE, cookie);
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

    async fn label(&self, name: &str) -> String {
        let (status, label) = self
            .call(
                "POST",
                &format!("/projects/{}/labels", self.project_id),
                &self.owner_cookie,
                Some(json!({ "name": name, "color": "#ff0000" })),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{label}");
        label["id"].as_str().unwrap().to_owned()
    }

    async fn rule(&self, label_id: &str, user: Id) -> Value {
        let (status, rule) = self
            .call(
                "POST",
                &format!("/projects/{}/label-rules", self.project_id),
                &self.owner_cookie,
                Some(json!({ "label_id": label_id, "user_id": user.to_string() })),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{rule}");
        rule
    }

    async fn task(&self, labels: &[&str], assignees: &[Id]) -> Value {
        let (status, task) = self
            .call(
                "POST",
                "/tasks",
                &self.owner_cookie,
                Some(json!({
                    "project_id": self.project_id,
                    "status_id": self.status_id,
                    "title": "Task",
                    "label_ids": labels,
                    "assignee_ids": assignees.iter().map(ToString::to_string).collect::<Vec<_>>(),
                })),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{task}");
        task
    }

    async fn update(&self, task: &Value, change: Value) -> Value {
        let mut change = change;
        change["expected_version"] = task["version"].clone();
        let (status, task) = self
            .call(
                "PATCH",
                &format!("/tasks/{}", task["id"].as_str().unwrap()),
                &self.owner_cookie,
                Some(change),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{task}");
        task
    }

    async fn get_task(&self, task_id: &str) -> Value {
        let (status, task) = self
            .call(
                "GET",
                &format!("/tasks/{task_id}"),
                &self.owner_cookie,
                None,
            )
            .await;
        assert_eq!(status, StatusCode::OK);
        task
    }

    /// The task's activity, oldest first.
    async fn activity(&self, task_id: &str) -> Vec<Value> {
        let (status, page) = self
            .call(
                "GET",
                &format!("/tasks/{task_id}/activity"),
                &self.owner_cookie,
                None,
            )
            .await;
        assert_eq!(status, StatusCode::OK);
        let mut items = page["items"].as_array().unwrap().clone();
        items.reverse();
        items
    }

    async fn assigned_notifications(&self, user: Id, task_id: &str) -> i64 {
        sqlx::query_scalar(
            "SELECT COUNT(*) FROM notifications WHERE recipient_user_id = $1 AND task_id = $2 AND kind = 'task_assigned'",
        )
        .bind(user.to_string())
        .bind(task_id)
        .fetch_one(self.database.pool())
        .await
        .unwrap()
    }

    fn github_item(&self, number: i64, labels: &[&str]) -> GithubWorkItem {
        GithubWorkItem {
            repository: "acme/repo".to_owned(),
            number,
            project_id: self.project_id.parse().unwrap(),
            title: format!("Issue {number}"),
            description: String::new(),
            kind: "issue",
            state: "open",
            state_changed: false,
            labels: labels
                .iter()
                .map(|name| GithubLabel {
                    name: (*name).to_owned(),
                    color: "#00ff00".to_owned(),
                })
                .collect(),
        }
    }
}

fn assignees(task: &Value) -> Vec<String> {
    let mut ids: Vec<String> = task["assignee_ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|id| id.as_str().unwrap().to_owned())
        .collect();
    ids.sort();
    ids
}

fn ids(people: &[Id]) -> Vec<String> {
    let mut ids: Vec<String> = people.iter().map(ToString::to_string).collect();
    ids.sort();
    ids
}

fn rule_event(activity: &[Value]) -> &Value {
    activity
        .iter()
        .find(|event| event["action"] == "task.assigned_by_rule")
        .unwrap()
}

fn actions(activity: &[Value]) -> Vec<&str> {
    activity
        .iter()
        .map(|event| event["action"].as_str().unwrap())
        .collect()
}

#[tokio::test]
async fn a_rule_assigns_on_create_and_when_the_label_is_added_later() {
    let fixture = Fixture::new().await;
    let raymond = fixture
        .person("raymond@example.com", "Raymond", "member")
        .await;
    let bug = fixture.label("bug").await;
    fixture.rule(&bug, raymond.id).await;

    let task = fixture.task(&[&bug], &[]).await;
    let task_id = task["id"].as_str().unwrap();
    assert_eq!(assignees(&task), ids(&[raymond.id]));
    assert_eq!(fixture.assigned_notifications(raymond.id, task_id).await, 1);
    let activity = fixture.activity(task_id).await;
    let mut written = actions(&activity);
    written.sort_unstable();
    assert_eq!(written, ["task.assigned_by_rule", "task.created"]);
    let event = rule_event(&activity);
    // Same time as the write that added the label: the feed shows the rule's line after it.
    assert_eq!(event["occurred_at"], activity[0]["occurred_at"]);
    assert_eq!(event["occurred_at"], activity[1]["occurred_at"]);
    assert!(event["actor_id"].is_null());
    assert_eq!(event["metadata"]["label_name"], "bug");
    assert_eq!(event["metadata"]["assignee_name"], "Raymond");

    // Added later by an update.
    let later = fixture.task(&[], &[]).await;
    let later_id = later["id"].as_str().unwrap();
    assert!(assignees(&later).is_empty());
    let later = fixture.update(&later, json!({ "label_ids": [bug] })).await;
    assert_eq!(assignees(&later), ids(&[raymond.id]));
    assert_eq!(
        fixture.assigned_notifications(raymond.id, later_id).await,
        1
    );
    let activity = fixture.activity(later_id).await;
    assert!(actions(&activity).contains(&"task.assigned_by_rule"));
    // The person's own event names only what they changed, not the rule's assignee.
    let updated = activity
        .iter()
        .find(|event| event["action"] == "task.updated")
        .unwrap();
    assert!(updated["metadata"]["changes"].get("assignees").is_none());
    assert!(updated["metadata"]["changes"].get("labels").is_some());
}

#[tokio::test]
async fn rules_only_add_assignees() {
    let fixture = Fixture::new().await;
    let raymond = fixture
        .person("raymond@example.com", "Raymond", "member")
        .await;
    let erol = fixture.person("erol@example.com", "Erol", "member").await;
    let bug = fixture.label("bug").await;
    let other = fixture.label("other").await;
    fixture.rule(&bug, raymond.id).await;

    // A hand-set assignee stays; the rule adds its person.
    let task = fixture.task(&[&bug], &[erol.id]).await;
    assert_eq!(assignees(&task), ids(&[raymond.id, erol.id]));

    // Removing the rule's person by hand is final while the label stays on the task.
    let task = fixture
        .update(&task, json!({ "assignee_ids": [erol.id.to_string()] }))
        .await;
    assert_eq!(assignees(&task), ids(&[erol.id]));
    let task = fixture
        .update(&task, json!({ "label_ids": [bug, other] }))
        .await;
    assert_eq!(assignees(&task), ids(&[erol.id]));

    // Already assigned: no second row, no history line.
    let assigned = fixture.task(&[&bug], &[raymond.id]).await;
    assert_eq!(assignees(&assigned), ids(&[raymond.id]));
    let activity = fixture.activity(assigned["id"].as_str().unwrap()).await;
    assert!(!actions(&activity).contains(&"task.assigned_by_rule"));
}

#[tokio::test]
async fn every_matching_rule_assigns_and_unavailable_people_are_skipped() {
    let fixture = Fixture::new().await;
    let raymond = fixture
        .person("raymond@example.com", "Raymond", "member")
        .await;
    let erol = fixture.person("erol@example.com", "Erol", "member").await;
    let suspended = fixture.person("ds@example.com", "David S", "member").await;
    let gone = fixture.person("dd@example.com", "David D", "member").await;
    let paused = fixture.person("pat@example.com", "Pat", "member").await;
    let bug = fixture.label("bug").await;
    for person in [&raymond, &erol, &suspended, &gone] {
        fixture.rule(&bug, person.id).await;
    }
    let paused_rule = fixture.rule(&bug, paused.id).await;
    let (status, _) = fixture
        .call(
            "PATCH",
            &format!(
                "/projects/{}/label-rules/{}",
                fixture.project_id,
                paused_rule["id"].as_str().unwrap()
            ),
            &fixture.owner_cookie,
            Some(json!({ "enabled": false })),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    sqlx::query("UPDATE users SET suspended_at = 1 WHERE id = $1")
        .bind(suspended.id.to_string())
        .execute(fixture.database.pool())
        .await
        .unwrap();
    sqlx::query("DELETE FROM memberships WHERE user_id = $1")
        .bind(gone.id.to_string())
        .execute(fixture.database.pool())
        .await
        .unwrap();

    let task = fixture.task(&[&bug], &[]).await;
    assert_eq!(assignees(&task), ids(&[raymond.id, erol.id]));
}

#[tokio::test]
async fn owner_labels_assign_by_handle_or_name_only_when_switched_on() {
    let fixture = Fixture::new().await;
    let raymond = fixture
        .person("raymond@example.com", "Ray Mond", "member")
        .await;
    let david = fixture.person("ds@example.com", "David S", "member").await;
    let owner_raymond = fixture.label("owner:RAYMOND").await;
    let owner_david = fixture.label("owner:david-s").await;
    fixture.label("owner:nobody").await;
    let settings_path = format!("/projects/{}/label-rules", fixture.project_id);

    // Off by default: the label does nothing.
    let (_, settings) = fixture
        .call("GET", &settings_path, &fixture.owner_cookie, None)
        .await;
    assert_eq!(settings["owner_labels"], false);
    assert_eq!(settings["unmatched_owner_names"], json!(["nobody"]));
    let task = fixture.task(&[&owner_raymond], &[]).await;
    assert!(assignees(&task).is_empty());

    let (status, settings) = fixture
        .call(
            "PUT",
            &format!("/projects/{}/label-rule-settings", fixture.project_id),
            &fixture.owner_cookie,
            Some(json!({ "owner_labels": true })),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(settings["owner_labels"], true);
    let task = fixture.task(&[&owner_raymond, &owner_david], &[]).await;
    assert_eq!(assignees(&task), ids(&[raymond.id, david.id]));
    let activity = fixture.activity(task["id"].as_str().unwrap()).await;
    let names: Vec<&str> = activity
        .iter()
        .filter(|event| event["action"] == "task.assigned_by_rule")
        .map(|event| event["metadata"]["label_name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["owner:RAYMOND", "owner:david-s"]);

    // Two members with the same name: nobody is assigned, and the name shows as unmatched.
    fixture
        .person("david-s@example.org", "Someone", "member")
        .await;
    let task = fixture.task(&[&owner_david], &[]).await;
    assert!(assignees(&task).is_empty());
    let (_, settings) = fixture
        .call("GET", &settings_path, &fixture.owner_cookie, None)
        .await;
    assert_eq!(
        settings["unmatched_owner_names"],
        json!(["david-s", "nobody"])
    );
}

#[tokio::test]
async fn github_sync_runs_the_rules_on_create_and_on_new_labels() {
    let fixture = Fixture::new().await;
    let raymond = fixture
        .person("raymond@example.com", "Raymond", "member")
        .await;
    let erol = fixture.person("erol@example.com", "Erol", "member").await;
    let bug = fixture.label("bug").await;
    let urgent = fixture.label("urgent").await;
    fixture.rule(&bug, raymond.id).await;
    fixture.rule(&urgent, erol.id).await;
    let tasks = TaskRepository::new((*fixture.database).clone());
    let workspace_id = fixture.workspace_id.parse().unwrap();

    let task_id = tasks
        .sync_github_work_item(
            workspace_id,
            fixture.owner_id,
            fixture.github_item(7, &["Bug"]),
            "github-test",
            TimestampMillis::now(),
        )
        .await
        .unwrap();
    let task = fixture.get_task(&task_id.to_string()).await;
    assert_eq!(assignees(&task), ids(&[raymond.id]));
    let activity = fixture.activity(&task_id.to_string()).await;
    assert!(actions(&activity).contains(&"task.created"));
    assert!(actions(&activity).contains(&"task.assigned_by_rule"));

    tasks
        .sync_github_work_item(
            workspace_id,
            fixture.owner_id,
            fixture.github_item(7, &["Bug", "urgent"]),
            "github-test",
            TimestampMillis::now(),
        )
        .await
        .unwrap();
    let task = fixture.get_task(&task_id.to_string()).await;
    assert_eq!(assignees(&task), ids(&[raymond.id, erol.id]));
    assert_eq!(
        fixture
            .assigned_notifications(erol.id, &task_id.to_string())
            .await,
        1
    );
}

#[tokio::test]
async fn discord_tasks_run_the_rules_of_the_discord_label() {
    let fixture = Fixture::new().await;
    let raymond = fixture
        .person("raymond@example.com", "Raymond", "member")
        .await;
    let discord = fixture.label("Discord").await;
    fixture.rule(&discord, raymond.id).await;
    let (task, created) = TaskRepository::new((*fixture.database).clone())
        .create_discord_task(
            fixture.workspace_id.parse().unwrap(),
            fixture.owner_id,
            None,
            DiscordTask {
                source_url: "https://discord.com/channels/1/2/3".to_owned(),
                event_id: "event-1".to_owned(),
                payload_hash: [7; 32],
                project_id: fixture.project_id.parse().unwrap(),
                title: "From Discord".to_owned(),
                description: String::new(),
            },
            "discord-test",
            TimestampMillis::now(),
        )
        .await
        .unwrap();
    assert!(created);
    assert_eq!(task.assignee_ids, [raymond.id]);
}

#[tokio::test]
async fn only_owners_and_admins_manage_rules() {
    let fixture = Fixture::new().await;
    let member = fixture
        .person("member@example.com", "Member", "member")
        .await;
    let admin = fixture.person("admin@example.com", "Admin", "admin").await;
    let guest = fixture.person("guest@example.com", "Guest", "guest").await;
    let bug = fixture.label("bug").await;
    let rule = fixture.rule(&bug, member.id).await;
    let rules = format!("/projects/{}/label-rules", fixture.project_id);
    let one = format!("{rules}/{}", rule["id"].as_str().unwrap());
    let settings = format!("/projects/{}/label-rule-settings", fixture.project_id);
    let create = json!({ "label_id": bug, "user_id": admin.id.to_string() });

    for (method, path, body) in [
        ("GET", &rules, None),
        ("POST", &rules, Some(create.clone())),
        ("PATCH", &one, Some(json!({ "enabled": false }))),
        ("DELETE", &one, None),
        ("PUT", &settings, Some(json!({ "owner_labels": true }))),
    ] {
        let (status, _) = fixture.call(method, path, &member.cookie, body).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "member {method} {path}");
    }
    let (status, _) = fixture.call("GET", &rules, &guest.cookie, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, _) = fixture
        .call("POST", &rules, &admin.cookie, Some(create.clone()))
        .await;
    assert_eq!(status, StatusCode::CREATED);
    // One rule per label and person; only assignable members of this project's labels.
    let (status, _) = fixture
        .call("POST", &rules, &admin.cookie, Some(create))
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, _) = fixture
        .call(
            "POST",
            &rules,
            &admin.cookie,
            Some(json!({ "label_id": bug, "user_id": guest.id.to_string() })),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let (status, _) = fixture
        .call(
            "POST",
            &rules,
            &admin.cookie,
            Some(json!({ "label_id": Id::new_v7().to_string(), "user_id": member.id.to_string() })),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let (status, _) = fixture
        .call(
            "PATCH",
            &one,
            &admin.cookie,
            Some(json!({ "enabled": false })),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, listed) = fixture.call("GET", &rules, &admin.cookie, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(listed["rules"].as_array().unwrap().len(), 2);
    let (status, _) = fixture.call("DELETE", &one, &admin.cookie, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}
