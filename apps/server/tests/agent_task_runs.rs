//! Agents work on tasks assigned to them (`agent::task_runs`): assigning a task to an agent, an
//! @mention of an agent in a task comment, or a person's comment on a task assigned to an agent
//! starts a run whose driver is that person (their permissions, their DM with the agent for
//! answers and approvals); the agent writes its progress and result as its own comment; its own
//! comments, agent sessions and guests start nothing; one run per task at a time; unassigning
//! the agent or deleting the task stops its run. Runs on SQLite, and on Postgres with
//! `ORBIT_TEST_POSTGRES_URL`.

use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use orbit_platform::{
    AttachmentMutationCoordinator, AuthenticatedUser, Id, LocalBlobStore, PasswordService, Row,
    TestDatabase, TimestampMillis, UploadLimits, UploadService,
};
use orbit_server::agent::{AgentConfig, AgentState};
use orbit_server::auth_routes::CookieMode;
use orbit_server::chat_routes::{ChatState, chat_router};
use orbit_server::repositories::identity::{IdentityRepository, SetupRequest};
use orbit_server::task_routes::{TaskState, task_router};
use serde_json::{Value, json};
use tower::ServiceExt;

const SECRET: &str = "test-agent-secret-0123456789abcdef0123456789";

struct Fixture {
    _root: tempfile::TempDir,
    database: TestDatabase,
    identity: Arc<IdentityRepository>,
    _agent: AgentState,
    app: axum::Router,
    workspace_id: String,
    owner: Member,
}

#[derive(Clone)]
struct Member {
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
        let root = tempfile::tempdir().unwrap();
        let uploads = UploadService::new(
            (*database).clone(),
            Arc::new(LocalBlobStore::new(root.path().join("attachments"))),
            AttachmentMutationCoordinator::default(),
            UploadLimits::default(),
        );
        let chat = ChatState::new(Arc::clone(&identity), uploads, CookieMode::secure());
        // Nothing listens on the sidecar's address: only the default model can be saved.
        let agent = AgentState::new(
            Some(AgentConfig {
                url: "http://127.0.0.1:9".to_owned(),
                secret: SECRET.to_owned(),
            }),
            chat.clone(),
            Arc::clone(&identity),
            CookieMode::secure(),
            Some([3; 32]),
        );
        let app = chat_router(chat)
            .merge(task_router(TaskState::new(
                Arc::clone(&identity),
                CookieMode::secure(),
            )))
            .merge(orbit_server::agent::router(agent.clone()))
            .layer(axum::middleware::from_fn_with_state(
                (*database).clone(),
                orbit_server::agent::scope::layer,
            ));
        Self {
            _root: root,
            database,
            identity,
            _agent: agent,
            app,
            workspace_id: setup.workspace_id.to_string(),
            owner: Member {
                id: setup.user_id,
                cookie: format!("__Host-orbit_session={}", setup.session.token),
            },
        }
    }

    async fn add_person(&self, name: &str, role: &str) -> Member {
        let id = Id::new_v7();
        let now = TimestampMillis::now().as_millis();
        let email = format!("{name}@example.com");
        sqlx::query(
            "INSERT INTO users (id, email, normalized_email, display_name, password_hash, created_at, updated_at) \
             VALUES ($1, $2, $2, $3, 'unused', $4, $4)",
        )
        .bind(id.to_string())
        .bind(&email)
        .bind(name)
        .bind(now)
        .execute(self.database.pool())
        .await
        .unwrap();
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
        let session = self
            .identity
            .create_session(
                &AuthenticatedUser {
                    id,
                    email,
                    display_name: name.to_owned(),
                },
                TimestampMillis::now(),
            )
            .await
            .unwrap();
        Member {
            id,
            cookie: format!("__Host-orbit_session={}", session.token),
        }
    }

    async fn request(
        &self,
        method: &str,
        uri: &str,
        auth: (&str, &str),
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let builder = Request::builder()
            .method(method)
            .uri(uri)
            .header(auth.0, auth.1);
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

    /// A workspace route as `member` (cookie).
    async fn api(
        &self,
        member: &Member,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let uri = format!("/api/v1/workspaces/{}{path}", self.workspace_id);
        self.request(method, &uri, ("cookie", &member.cookie), body)
            .await
    }

    async fn chat(&self, member: &Member, method: &str, path: &str, body: Option<Value>) -> Value {
        let (status, value) = self
            .api(member, method, &format!("/chat{path}"), body)
            .await;
        assert_eq!(status, StatusCode::OK, "{method} {path}: {value}");
        value
    }

    async fn internal(&self, method: &str, path: &str, body: Option<Value>) -> (StatusCode, Value) {
        self.request(
            method,
            &format!("/api/internal/agent{path}"),
            ("authorization", &format!("Bearer {SECRET}")),
            body,
        )
        .await
    }

    /// The queued inputs (also answers refusals and makes the default bot).
    async fn pending(&self) -> Vec<Value> {
        let (status, value) = self.internal("GET", "/inputs", None).await;
        assert_eq!(status, StatusCode::OK, "{value}");
        value["inputs"].as_array().unwrap().clone()
    }

    async fn create_agent(&self, body: Value) -> Value {
        let (status, agent) = self.api(&self.owner, "POST", "/agents", Some(body)).await;
        assert_eq!(status, StatusCode::OK, "{agent}");
        agent
    }

    async fn set_thread(&self, input: &Value, status: &str, driver: &Member) {
        let (code, value) = self
            .internal(
                "PUT",
                &format!("/threads/{}", input["thread_key"].as_str().unwrap()),
                Some(json!({
                    "workspace_id": self.workspace_id,
                    "conversation_id": input["conversation_id"],
                    "root_message_id": input["root_message_id"],
                    "status": status,
                    "driver_user_id": driver.id.to_string(),
                    "waiting_for_user_id": null,
                    "agent_user_id": input["agent"]["user_id"],
                })),
            )
            .await;
        assert_eq!(code, StatusCode::NO_CONTENT, "{value}");
    }

    async fn session(&self, member: &Member, agent: Option<&str>) -> (StatusCode, String) {
        let (status, session) = self
            .internal(
                "POST",
                "/sessions",
                Some(json!({
                    "user_id": member.id.to_string(),
                    "workspace_id": self.workspace_id,
                    "agent_user_id": agent,
                })),
            )
            .await;
        (
            status,
            format!("Bearer {}", session["token"].as_str().unwrap_or_default()),
        )
    }

    async fn as_agent(
        &self,
        token: &str,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let uri = format!("/api/v1/workspaces/{}{path}", self.workspace_id);
        self.request(method, &uri, ("authorization", token), body)
            .await
    }

    async fn ack(&self, input: &Value) {
        let (status, _) = self
            .internal(
                "POST",
                &format!("/inputs/{}/ack", input["message_id"].as_str().unwrap()),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
    }

    /// The default agent's bot user (made by the first poll).
    async fn donna(&self) -> String {
        let (status, value) = self.internal("GET", "/inputs", None).await;
        assert_eq!(status, StatusCode::OK, "{value}");
        value["bot_user_id"].as_str().unwrap().to_owned()
    }

    async fn ok(&self, member: &Member, method: &str, path: &str, body: Option<Value>) -> Value {
        let (status, value) = self.api(member, method, path, body).await;
        assert!(status.is_success(), "{method} {path}: {status} {value}");
        value
    }

    /// The first project and its first status.
    async fn general(&self) -> (String, String) {
        let projects = self.ok(&self.owner, "GET", "/projects", None).await;
        let project = projects.get("items").unwrap_or(&projects)[0]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let statuses = self
            .ok(
                &self.owner,
                "GET",
                &format!("/projects/{project}/statuses"),
                None,
            )
            .await;
        let status = statuses.get("items").unwrap_or(&statuses)[0]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        (project, status)
    }

    /// A new task in `project` (the first one when `None`), assigned to `assignees`.
    async fn task(
        &self,
        member: &Member,
        project: Option<(&str, &str)>,
        assignees: &[&str],
    ) -> String {
        let (general, status) = self.general().await;
        let (project, status) = project.unwrap_or((&general, &status));
        let created = self
            .ok(
                member,
                "POST",
                "/tasks",
                Some(json!({
                    "project_id": project,
                    "status_id": status,
                    "title": "Write the release notes",
                    "assignee_ids": assignees,
                })),
            )
            .await;
        created["id"].as_str().unwrap().to_owned()
    }

    async fn version(&self, task: &str) -> i64 {
        self.ok(&self.owner, "GET", &format!("/tasks/{task}"), None)
            .await["version"]
            .as_i64()
            .unwrap()
    }

    async fn assign(&self, member: &Member, task: &str, assignees: &[&str]) -> (StatusCode, Value) {
        let version = self.version(task).await;
        self.api(
            member,
            "PATCH",
            &format!("/tasks/{task}"),
            Some(json!({ "assignee_ids": assignees, "expected_version": version })),
        )
        .await
    }

    async fn comment(&self, member: &Member, task: &str, body: &str, mentioned: &[&str]) -> String {
        self.ok(
            member,
            "POST",
            &format!("/tasks/{task}/comments"),
            Some(json!({ "body": body, "mentioned_user_ids": mentioned })),
        )
        .await["id"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    /// The task's runs, oldest first: (status, driver, cause, agent).
    async fn runs(&self, task: &str) -> Vec<(String, String, String, String)> {
        sqlx::query(
            "SELECT status, driver_user_id, cause, agent_user_id FROM agent_task_runs \
             WHERE task_id = $1 ORDER BY created_at, id",
        )
        .bind(task)
        .fetch_all(self.database.pool())
        .await
        .unwrap()
        .iter()
        .map(|row| {
            (
                row.get("status"),
                row.get("driver_user_id"),
                row.get("cause"),
                row.get("agent_user_id"),
            )
        })
        .collect()
    }

    async fn comments(&self, task: &str) -> Vec<Value> {
        self.ok(&self.owner, "GET", &format!("/tasks/{task}/comments"), None)
            .await["items"]
            .as_array()
            .unwrap()
            .clone()
    }
}

#[tokio::test]
async fn an_assigned_agent_works_for_the_assigner_and_comments_its_result() {
    let fixture = Fixture::new().await;
    let mia = fixture.add_person("Mia", "member").await;
    let donna = fixture.donna().await;
    let task = fixture.task(&fixture.owner, None, &[]).await;
    assert!(fixture.pending().await.is_empty());

    // Mia assigns it: a run for her, at once.
    let (status, value) = fixture.assign(&mia, &task, &[&donna]).await;
    assert_eq!(status, StatusCode::OK, "{value}");
    assert_eq!(
        fixture.runs(&task).await,
        vec![(
            "queued".to_owned(),
            mia.id.to_string(),
            "assigned".to_owned(),
            donna.clone()
        )]
    );
    let inputs = fixture.pending().await;
    assert_eq!(inputs.len(), 1, "{inputs:?}");
    let input = &inputs[0];
    assert_eq!(fixture.runs(&task).await[0].0, "running");
    assert_eq!(input["author_id"], mia.id.to_string());
    assert_eq!(input["kind"], "instruction");
    assert_eq!(input["task"]["task_id"], task);
    assert_eq!(input["task"]["cause"], "assigned");
    assert_eq!(input["task"]["title"], "Write the release notes");
    assert!(
        input["task"]["identifier"]
            .as_str()
            .unwrap()
            .ends_with("-1")
    );
    assert_eq!(input["body"], "");
    assert!(input["agent"].is_null());

    // Its thread is under a header from the agent in Mia's DM with it: answers and approvals go
    // there, to her.
    assert_eq!(input["conversation_kind"], "dm");
    assert_eq!(input["conversation_members"], 2);
    let members: Vec<String> = sqlx::query_scalar(
        "SELECT user_id FROM chat_members WHERE conversation_id = $1 ORDER BY user_id",
    )
    .bind(input["conversation_id"].as_str().unwrap())
    .fetch_all(fixture.database.pool())
    .await
    .unwrap();
    let mut expected = vec![mia.id.to_string(), donna.clone()];
    expected.sort();
    assert_eq!(members, expected);
    let header = fixture
        .chat(
            &mia,
            "GET",
            &format!(
                "/conversations/{}/messages",
                input["conversation_id"].as_str().unwrap()
            ),
            None,
        )
        .await;
    let header = header.get("items").unwrap_or(&header)[0].clone();
    assert_eq!(header["id"], input["root_message_id"]);
    assert_eq!(header["author_id"], donna);
    assert!(
        header["body"].as_str().unwrap().contains("You assigned me"),
        "{header}"
    );

    // The run acts as Mia only: Orbit gives her a session, nobody else.
    fixture.set_thread(input, "running", &mia).await;
    let (status, token) = fixture.session(&mia, None).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = fixture.session(&fixture.owner, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, me) = fixture
        .as_agent(&token, "GET", &format!("/tasks/{task}"), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{me}");

    // Progress and result: one comment by the agent, replaced each time.
    let run = input["message_id"].as_str().unwrap();
    let (status, first) = fixture
        .internal(
            "PUT",
            &format!("/task-runs/{run}/comment"),
            Some(json!({ "body": "⏳ Working on it…" })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{first}");
    let (status, second) = fixture
        .internal(
            "PUT",
            &format!("/task-runs/{run}/comment"),
            Some(json!({ "body": "Done: the notes are in the description." })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{second}");
    assert_eq!(first["comment_id"], second["comment_id"]);
    let comments = fixture.comments(&task).await;
    assert_eq!(comments.len(), 1, "{comments:?}");
    assert_eq!(comments[0]["author_id"], donna);
    assert_eq!(
        comments[0]["body"],
        "Done: the notes are in the description."
    );
    // The agent's own comment starts nothing.
    assert_eq!(fixture.runs(&task).await.len(), 1);

    // The run ends: its task is free for the next one.
    fixture.ack(input).await;
    fixture.set_thread(input, "idle", &mia).await;
    assert_eq!(fixture.runs(&task).await[0].0, "done");
}

#[tokio::test]
async fn comments_and_mentions_start_runs_for_their_author_but_agents_never_start_one() {
    let fixture = Fixture::new().await;
    let mia = fixture.add_person("Mia", "member").await;
    let donna = fixture.donna().await;
    let writer = fixture
        .create_agent(
            json!({ "name": "Writer", "tools": ["get_task", "comment_on_task", "update_task"] }),
        )
        .await;
    let writer = writer["user_id"].as_str().unwrap().to_owned();

    // An @mention on a task that no agent is assigned to: a run of that agent, for the author.
    let task = fixture.task(&fixture.owner, None, &[]).await;
    let comment = fixture
        .comment(&mia, &task, "@Writer draft the notes", &[&writer])
        .await;
    let runs = fixture.runs(&task).await;
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].1, mia.id.to_string());
    assert_eq!(runs[0].2, "mentioned");
    assert_eq!(runs[0].3, writer);
    let inputs = fixture.pending().await;
    assert_eq!(inputs.len(), 1, "{inputs:?}");
    let input = &inputs[0];
    assert_eq!(input["body"], "@Writer draft the notes");
    assert_eq!(input["task"]["comment_id"], comment);
    assert_eq!(input["task"]["cause"], "mentioned");
    assert_eq!(input["agent"]["user_id"], writer);

    // A plain comment on a task with no agent starts nothing.
    let quiet = fixture.task(&fixture.owner, None, &[]).await;
    fixture.comment(&mia, &quiet, "Looks good", &[]).await;
    assert!(fixture.runs(&quiet).await.is_empty());

    // Through the run's agent session (the agent acting as Mia), comments and assignments start
    // nothing: no agent makes an agent work, itself included.
    fixture.set_thread(input, "running", &mia).await;
    let (status, token) = fixture.session(&mia, Some(&writer)).await;
    assert_eq!(status, StatusCode::OK);
    let (status, value) = fixture
        .as_agent(
            &token,
            "POST",
            &format!("/tasks/{quiet}/comments"),
            Some(json!({ "body": "@Orbit Agent please help", "mentioned_user_ids": [donna] })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{value}");
    let version = fixture.version(&quiet).await;
    let (status, value) = fixture
        .as_agent(
            &token,
            "PATCH",
            &format!("/tasks/{quiet}"),
            Some(json!({ "assignee_ids": [donna, writer], "expected_version": version })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{value}");
    assert!(fixture.runs(&quiet).await.is_empty());

    // The agent's own comment (its result) starts nothing either, also on a task assigned to it.
    let run = input["message_id"].as_str().unwrap();
    let (status, _) = fixture
        .internal(
            "PUT",
            &format!("/task-runs/{run}/comment"),
            Some(json!({ "body": "Here is a draft." })),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(fixture.runs(&task).await.len(), 1);

    // A person's comment on a task assigned to agents: a run of each agent, for the author.
    fixture
        .comment(&fixture.owner, &quiet, "What is the status?", &[])
        .await;
    let runs = fixture.runs(&quiet).await;
    assert_eq!(runs.len(), 2, "{runs:?}");
    assert!(
        runs.iter()
            .all(|run| run.1 == fixture.owner.id.to_string() && run.2 == "commented")
    );
}

#[tokio::test]
async fn one_run_per_task_and_later_triggers_wait_their_turn() {
    let fixture = Fixture::new().await;
    let mia = fixture.add_person("Mia", "member").await;
    let donna = fixture.donna().await;
    let task = fixture.task(&fixture.owner, None, &[&donna]).await;
    let first = fixture.pending().await;
    assert_eq!(first.len(), 1);
    fixture
        .set_thread(&first[0], "running", &fixture.owner)
        .await;
    fixture.ack(&first[0]).await;

    // Comments while it runs: they queue; Mia's second comment joins her queued run.
    fixture
        .comment(&mia, &task, "Also add the screenshots", &[])
        .await;
    let newest = fixture.comment(&mia, &task, "And the links", &[]).await;
    let runs = fixture.runs(&task).await;
    assert_eq!(
        runs.iter().map(|run| run.0.as_str()).collect::<Vec<_>>(),
        ["running", "queued"]
    );
    assert!(fixture.pending().await.is_empty());

    // A run that waits for an answer frees the task too; then the queued run starts, with the
    // newest comment.
    fixture
        .set_thread(&first[0], "waiting", &fixture.owner)
        .await;
    let next = fixture.pending().await;
    assert_eq!(next.len(), 1, "{next:?}");
    assert_eq!(next[0]["author_id"], mia.id.to_string());
    assert_eq!(next[0]["task"]["comment_id"], newest);
    assert_eq!(next[0]["body"], "And the links");
    assert_eq!(
        fixture
            .runs(&task)
            .await
            .iter()
            .map(|run| run.0.as_str())
            .collect::<Vec<_>>(),
        ["done", "running"]
    );
}

#[tokio::test]
async fn guests_never_start_or_drive_a_run() {
    let fixture = Fixture::new().await;
    let guest = fixture.add_person("Gus", "guest").await;
    let donna = fixture.donna().await;
    let task = fixture.task(&fixture.owner, None, &[]).await;
    let (project, status) = fixture.general().await;

    // A guest sees no task: no assignment, no comment, no new task with an agent.
    let (code, _) = fixture.assign(&guest, &task, &[&donna]).await;
    assert_eq!(code, StatusCode::NOT_FOUND);
    let (code, _) = fixture
        .api(
            &guest,
            "POST",
            &format!("/tasks/{task}/comments"),
            Some(json!({ "body": "hi", "mentioned_user_ids": [donna] })),
        )
        .await;
    assert_eq!(code, StatusCode::NOT_FOUND);
    let (code, _) = fixture
        .api(
            &guest,
            "POST",
            "/tasks",
            Some(json!({ "project_id": project, "status_id": status, "title": "x", "assignee_ids": [donna] })),
        )
        .await;
    assert!(code.is_client_error(), "{code}");
    assert!(fixture.runs(&task).await.is_empty());
    let any: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_task_runs")
        .fetch_one(fixture.database.pool())
        .await
        .unwrap();
    assert_eq!(any, 0);

    // A queued run whose driver is no longer a member who sees the task never starts.
    let mia = fixture.add_person("Mia", "member").await;
    let (code, _) = fixture.assign(&mia, &task, &[&donna]).await;
    assert_eq!(code, StatusCode::OK);
    sqlx::query("UPDATE memberships SET role = 'guest' WHERE user_id = $1")
        .bind(mia.id.to_string())
        .execute(fixture.database.pool())
        .await
        .unwrap();
    assert!(fixture.pending().await.is_empty());
    assert_eq!(fixture.runs(&task).await[0].0, "cancelled");
}

#[tokio::test]
async fn unassigning_the_agent_or_deleting_the_task_stops_its_run() {
    let fixture = Fixture::new().await;
    let donna = fixture.donna().await;
    let task = fixture.task(&fixture.owner, None, &[&donna]).await;
    let inputs = fixture.pending().await;
    let input = &inputs[0];
    fixture.set_thread(input, "running", &fixture.owner).await;
    let (_, token) = fixture.session(&fixture.owner, None).await;

    let (status, value) = fixture.assign(&fixture.owner, &task, &[]).await;
    assert_eq!(status, StatusCode::OK, "{value}");
    assert_eq!(fixture.runs(&task).await[0].0, "cancelled");
    // Orbit refuses the run at once: its session is gone, and no new one is given.
    let (status, _) = fixture
        .as_agent(&token, "GET", &format!("/tasks/{task}"), None)
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = fixture.session(&fixture.owner, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    // The sidecar's report of the thread cannot turn it on again.
    fixture.set_thread(input, "running", &fixture.owner).await;
    let (status, _) = fixture.session(&fixture.owner, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    // The sidecar is told to stop it, until it says it did.
    let (_, polled) = fixture.internal("GET", "/inputs", None).await;
    assert_eq!(polled["inputs"], json!([]));
    assert_eq!(polled["stops"][0]["run_id"], input["message_id"]);
    assert_eq!(
        polled["stops"][0]["reason"],
        "I was unassigned from this task"
    );
    let (status, _) = fixture
        .internal(
            "POST",
            &format!(
                "/task-runs/{}/stopped",
                input["message_id"].as_str().unwrap()
            ),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, polled) = fixture.internal("GET", "/inputs", None).await;
    assert_eq!(polled["stops"], json!([]));

    // Assigned again, then deleted: the new run stops too; a queued one is cancelled with it.
    let (status, _) = fixture.assign(&fixture.owner, &task, &[&donna]).await;
    assert_eq!(status, StatusCode::OK);
    let mia = fixture.add_person("Mia", "member").await;
    fixture.comment(&mia, &task, "please hurry", &[]).await;
    let again = fixture.pending().await;
    assert_eq!(again.len(), 1);
    let version = fixture.version(&task).await;
    let (status, _) = fixture
        .api(
            &fixture.owner,
            "DELETE",
            &format!("/tasks/{task}?expected_version={version}"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let runs = fixture.runs(&task).await;
    assert!(runs.iter().all(|run| run.0 == "cancelled"), "{runs:?}");
    let (_, polled) = fixture.internal("GET", "/inputs", None).await;
    assert_eq!(polled["inputs"], json!([]));
    assert_eq!(polled["stops"][0]["run_id"], again[0]["message_id"]);
    assert_eq!(polled["stops"][0]["reason"], "the task was deleted");
    // A deleted task takes no comment.
    let (status, _) = fixture
        .internal(
            "PUT",
            &format!(
                "/task-runs/{}/comment",
                again[0]["message_id"].as_str().unwrap()
            ),
            Some(json!({ "body": "Stopped." })),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_private_projects_agent_must_be_on_its_member_list() {
    let fixture = Fixture::new().await;
    let mia = fixture.add_person("Mia", "member").await;
    let bob = fixture.add_person("Bob", "member").await;
    let donna = fixture.donna().await;
    let secret = fixture
        .ok(
            &mia,
            "POST",
            "/projects",
            Some(json!({ "name": "Secret", "key": "SEC", "color": "#336699", "private": true })),
        )
        .await;
    let secret = secret["id"].as_str().unwrap().to_owned();
    let statuses = fixture
        .ok(&mia, "GET", &format!("/projects/{secret}/statuses"), None)
        .await;
    let status = statuses.get("items").unwrap_or(&statuses)[0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let task = fixture.task(&mia, Some((&secret, &status)), &[]).await;

    // Not on the list: it cannot be assigned, and a mention starts nothing.
    let (code, _) = fixture.assign(&mia, &task, &[&donna]).await;
    assert_eq!(code, StatusCode::UNPROCESSABLE_ENTITY);
    fixture
        .comment(&mia, &task, "@Orbit Agent help", &[&donna])
        .await;
    assert!(fixture.runs(&task).await.is_empty());

    // On the list: it works for Mia; Bob, who does not see the project, gets nothing from it.
    fixture
        .ok(
            &mia,
            "POST",
            &format!("/projects/{secret}/members"),
            Some(json!({ "user_id": donna })),
        )
        .await;
    let (code, value) = fixture.assign(&mia, &task, &[&donna]).await;
    assert_eq!(code, StatusCode::OK, "{value}");
    let inputs = fixture.pending().await;
    assert_eq!(inputs.len(), 1);
    assert_eq!(inputs[0]["author_id"], mia.id.to_string());
    fixture.set_thread(&inputs[0], "running", &mia).await;
    let (status, _) = fixture.session(&bob, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, token) = fixture.session(&mia, None).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = fixture
        .as_agent(&token, "GET", &format!("/tasks/{task}"), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    let (code, _) = fixture
        .api(
            &bob,
            "POST",
            &format!("/tasks/{task}/comments"),
            Some(json!({ "body": "me too" })),
        )
        .await;
    assert_eq!(code, StatusCode::NOT_FOUND);
    assert_eq!(fixture.runs(&task).await.len(), 1);
}
