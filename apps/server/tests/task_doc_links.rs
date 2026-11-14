//! Docs pages linked to tasks: who may link, who sees which link (private projects, private
//! pages, guests, other workspaces), the task activity, and what deletes a link. Everything goes
//! through HTTP, as a client sees it.

use orbit_platform::{
    AuthenticatedUser, Config, EnvironmentMode, Id, PasswordService, TimestampMillis,
};
use orbit_server::app::App;
use orbit_server::repositories::identity::{IdentityRepository, SetupRequest};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

const COOKIE: &str = "orbit_session_dev";

struct Server {
    base: String,
    database: orbit_platform::Database,
    shutdown: CancellationToken,
    task: tokio::task::JoinHandle<()>,
    http: reqwest::Client,
    _root: TempDir,
}

impl Server {
    async fn start() -> Self {
        let root = tempfile::tempdir().unwrap();
        let mut config = Config {
            environment: EnvironmentMode::Development,
            app_key: Some(orbit_platform::AppKey::new([1; 32])),
            ..Config::default()
        };
        config.data.database = root.path().join("data/orbit.sqlite");
        config.data.attachments = root.path().join("data/attachments");
        config.data.backups = root.path().join("backups");
        config.rate_limits.general_per_minute = 100_000;
        let app = App::build(config).await.unwrap();
        let database = app.database().clone();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let shutdown = CancellationToken::new();
        let token = shutdown.clone();
        let task = tokio::spawn(async move {
            app.serve(listener, token).await.unwrap();
        });
        Self {
            base,
            database,
            shutdown,
            task,
            http: reqwest::Client::new(),
            _root: root,
        }
    }

    async fn stop(self) {
        self.shutdown.cancel();
        self.task.await.unwrap();
    }

    async fn call(&self, session: &str, method: &str, path: &str, body: Value) -> (u16, Value) {
        let mut request = self
            .http
            .request(method.parse().unwrap(), format!("{}{path}", self.base))
            .header("origin", &self.base)
            .header("cookie", format!("{COOKIE}={session}"));
        if method != "GET" && method != "DELETE" {
            request = request.json(&body);
        }
        let response = request.send().await.unwrap();
        let status = response.status().as_u16();
        let text = response.text().await.unwrap_or_default();
        (
            status,
            serde_json::from_str(&text).unwrap_or(Value::String(text)),
        )
    }

    async fn ok(&self, session: &str, method: &str, path: &str, body: Value) -> Value {
        let (status, response) = self.call(session, method, path, body).await;
        assert!(
            (200..300).contains(&status),
            "{method} {path}: {status} {response}"
        );
        response
    }

    /// A user with a session and a membership with `role` in `workspace`.
    async fn person(&self, workspace: &str, name: &str, role: &str) -> String {
        let id = Id::new_v7();
        let now = TimestampMillis::now().as_millis();
        let email = format!("{name}@example.com");
        sqlx::query(
            "INSERT INTO users (id, email, normalized_email, display_name, password_hash, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, 'unused', $5, $6)",
        )
        .bind(id.to_string())
        .bind(&email)
        .bind(&email)
        .bind(name)
        .bind(now)
        .bind(now)
        .execute(self.database.pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO memberships (id, workspace_id, user_id, role, version, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, 0, $5, $6)",
        )
        .bind(Id::new_v7().to_string())
        .bind(workspace)
        .bind(id.to_string())
        .bind(role)
        .bind(now)
        .bind(now)
        .execute(self.database.pool())
        .await
        .unwrap();
        IdentityRepository::new(self.database.clone())
            .create_session(
                &AuthenticatedUser {
                    id,
                    email,
                    display_name: name.to_owned(),
                },
                TimestampMillis::now(),
            )
            .await
            .unwrap()
            .token
    }

    async fn link_count(&self) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM task_doc_links")
            .fetch_one(self.database.pool())
            .await
            .unwrap()
    }
}

fn id(record: &Value) -> String {
    record["id"].as_str().unwrap().to_owned()
}

fn ids(list: &Value) -> Vec<String> {
    list.get("items")
        .unwrap_or(list)
        .as_array()
        .unwrap()
        .iter()
        .map(id)
        .collect()
}

fn task_ids(list: &Value) -> Vec<String> {
    list.as_array()
        .unwrap()
        .iter()
        .map(|item| id(&item["task"]))
        .collect()
}

/// A workspace with the public project General and the private project Secret (Alice is on it,
/// Bob is not), a guest, a teamspace page, and a private page of Alice and of Bob.
struct World {
    server: Server,
    base: String,
    owner: String,
    alice: String,
    bob: String,
    guest: String,
    /// A task in General.
    general_task: String,
    /// A task in Secret.
    secret_task: String,
    team_page: String,
    alice_page: String,
    bob_page: String,
}

impl World {
    async fn new() -> Self {
        let server = Server::start().await;
        let identity = IdentityRepository::new(server.database.clone());
        let now = TimestampMillis::now();
        identity
            .store_setup_token(
                "doc-links",
                TimestampMillis::from_millis(now.as_millis() + 60_000),
            )
            .await
            .unwrap();
        let setup = identity
            .complete_setup(
                SetupRequest {
                    token: "doc-links".to_owned(),
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
        let owner = setup.session.token.clone();
        let workspace = setup.workspace_id.to_string();
        let base = format!("/api/v1/workspaces/{workspace}");
        let alice = server.person(&workspace, "alice", "member").await;
        let bob = server.person(&workspace, "bob", "member").await;
        let guest = server.person(&workspace, "guest", "guest").await;

        let get = |session: &str, path: String| {
            let server = &server;
            let session = session.to_owned();
            async move { server.ok(&session, "GET", &path, Value::Null).await }
        };
        let general = ids(&get(&owner, format!("{base}/projects")).await).remove(0);
        let general_status =
            ids(&get(&owner, format!("{base}/projects/{general}/statuses")).await).remove(0);
        let secret = id(&server
            .ok(
                &alice,
                "POST",
                &format!("{base}/projects"),
                json!({"name": "Secret", "key": "SEC", "color": "#336699", "private": true}),
            )
            .await);
        let secret_status =
            ids(&get(&alice, format!("{base}/projects/{secret}/statuses")).await).remove(0);
        let general_task = id(&server
            .ok(
                &alice,
                "POST",
                &format!("{base}/tasks"),
                json!({"project_id": general, "status_id": general_status, "title": "Open work"}),
            )
            .await);
        let secret_task = id(&server
            .ok(
                &alice,
                "POST",
                &format!("{base}/tasks"),
                json!({"project_id": secret, "status_id": secret_status, "title": "Secret work"}),
            )
            .await);
        let page = |session: &str, title: &str, private: bool| {
            let server = &server;
            let (session, path) = (session.to_owned(), format!("{base}/pages"));
            let body = json!({"title": title, "private": private});
            async move { id(&server.ok(&session, "POST", &path, body).await) }
        };
        let team_page = page(&owner, "Team plan", false).await;
        let alice_page = page(&alice, "Alice notes", true).await;
        let bob_page = page(&bob, "Bob notes", true).await;
        Self {
            server,
            base,
            owner,
            alice,
            bob,
            guest,
            general_task,
            secret_task,
            team_page,
            alice_page,
            bob_page,
        }
    }

    async fn call(&self, session: &str, method: &str, suffix: &str, body: Value) -> (u16, Value) {
        let path = format!("{}{suffix}", self.base);
        self.server.call(session, method, &path, body).await
    }

    async fn link(&self, session: &str, task: &str, page: &str) -> (u16, Value) {
        self.call(
            session,
            "POST",
            &format!("/tasks/{task}/linked-docs"),
            json!({"page_id": page}),
        )
        .await
    }

    async fn unlink(&self, session: &str, task: &str, page: &str) -> u16 {
        self.call(
            session,
            "DELETE",
            &format!("/tasks/{task}/linked-docs/{page}"),
            Value::Null,
        )
        .await
        .0
    }

    async fn docs(&self, session: &str, task: &str) -> Vec<String> {
        let (status, body) = self
            .call(
                session,
                "GET",
                &format!("/tasks/{task}/linked-docs"),
                Value::Null,
            )
            .await;
        assert_eq!(status, 200, "{body}");
        ids(&body)
    }

    async fn tasks(&self, session: &str, page: &str) -> Vec<String> {
        let (status, body) = self
            .call(
                session,
                "GET",
                &format!("/pages/{page}/linked-tasks"),
                Value::Null,
            )
            .await;
        assert_eq!(status, 200, "{body}");
        task_ids(&body)
    }

    /// The task's doc link events, oldest first, as `session` sees them.
    async fn doc_events(&self, session: &str, task: &str) -> Vec<Value> {
        let (status, body) = self
            .call(
                session,
                "GET",
                &format!("/tasks/{task}/activity"),
                Value::Null,
            )
            .await;
        assert_eq!(status, 200, "{body}");
        let mut events: Vec<Value> = body["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|event| {
                event["action"]
                    .as_str()
                    .is_some_and(|action| action.starts_with("task.doc_"))
            })
            .cloned()
            .collect();
        events.reverse();
        events
    }

    async fn version(&self, session: &str, kind: &str, id: &str) -> u64 {
        let (status, body) = self
            .call(session, "GET", &format!("/{kind}/{id}"), Value::Null)
            .await;
        assert_eq!(status, 200, "{body}");
        body["version"].as_u64().unwrap()
    }
}

#[tokio::test]
async fn members_link_only_what_they_see_and_see_only_their_links() {
    let world = World::new().await;
    let (alice, bob) = (world.alice.as_str(), world.bob.as_str());
    let (general, secret) = (world.general_task.as_str(), world.secret_task.as_str());
    let (team, alice_page, bob_page) = (
        world.team_page.as_str(),
        world.alice_page.as_str(),
        world.bob_page.as_str(),
    );

    let (status, doc) = world.link(alice, general, team).await;
    assert_eq!(status, 201, "{doc}");
    assert_eq!(doc["id"], team);
    assert_eq!(doc["title"], "Team plan");
    assert_eq!(doc["private"], false);
    // Linking again changes nothing and records no second event.
    assert_eq!(world.link(alice, general, team).await.0, 201);
    assert_eq!(world.link(alice, general, alice_page).await.0, 201);
    assert_eq!(world.link(alice, secret, team).await.0, 201);
    assert_eq!(world.link(bob, general, bob_page).await.0, 201);
    assert_eq!(world.server.link_count().await, 4);

    // Each reader sees only the pages and tasks they see.
    assert_eq!(world.docs(alice, general).await, [team, alice_page]);
    assert_eq!(world.docs(bob, general).await, [team, bob_page]);
    assert_eq!(world.docs(&world.owner, general).await, [team]);
    assert_eq!(world.tasks(alice, team).await, [general, secret]);
    assert_eq!(world.tasks(bob, team).await, [general]);
    assert_eq!(world.tasks(&world.owner, team).await, [general, secret]);

    // Bob cannot reach Alice's private page or the private project's task.
    let linked = |page: &str| format!("/pages/{page}/linked-tasks");
    assert_eq!(
        world
            .call(bob, "GET", &linked(alice_page), Value::Null)
            .await
            .0,
        404
    );
    let (status, body) = world.link(bob, general, alice_page).await;
    assert_eq!(status, 422, "{body}");
    assert_eq!(body["code"], "validation_failed");
    assert_eq!(world.link(bob, secret, team).await.0, 404);
    assert_eq!(
        world
            .call(
                bob,
                "GET",
                &format!("/tasks/{secret}/linked-docs"),
                Value::Null
            )
            .await
            .0,
        404
    );
    assert_eq!(world.unlink(bob, general, alice_page).await, 404);
    assert_eq!(world.unlink(bob, secret, team).await, 404);
    assert_eq!(world.docs(alice, general).await, [team, alice_page]);
    // A malformed page id is a validation error, a page that does not exist too.
    assert_eq!(world.link(alice, general, "nope").await.0, 422);
    assert_eq!(
        world
            .link(alice, general, &Id::new_v7().to_string())
            .await
            .0,
        422
    );

    // A guest reaches no link at all.
    let guest = world.guest.as_str();
    assert_eq!(world.link(guest, general, team).await.0, 404);
    assert_eq!(world.unlink(guest, general, team).await, 404);
    assert_eq!(
        world
            .call(
                guest,
                "GET",
                &format!("/tasks/{general}/linked-docs"),
                Value::Null
            )
            .await
            .0,
        404
    );
    assert_eq!(
        world.call(guest, "GET", &linked(team), Value::Null).await.0,
        404
    );

    // The activity names a page only for readers who see it; the audit log never stores a title.
    let events = world.doc_events(bob, general).await;
    assert_eq!(events.len(), 3, "{events:?}");
    assert!(
        events
            .iter()
            .all(|event| event["action"] == "task.doc_linked")
    );
    assert_eq!(events[0]["metadata"]["page_id"], team);
    assert_eq!(events[0]["metadata"]["page_title"], "Team plan");
    assert_eq!(events[1]["metadata"]["page_id"], alice_page);
    assert!(events[1]["metadata"].get("page_title").is_none());
    assert_eq!(events[2]["metadata"]["page_title"], "Bob notes");
    let events = world.doc_events(alice, general).await;
    assert_eq!(events[1]["metadata"]["page_title"], "Alice notes");
    assert!(events[2]["metadata"].get("page_title").is_none());
    let stored: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM audit_events WHERE action LIKE 'task.doc_%' AND metadata_json LIKE '%notes%'",
    )
    .fetch_one(world.server.database.pool())
    .await
    .unwrap();
    assert_eq!(stored, 0);

    // A page of another workspace never links to this workspace's task, and its links stay there.
    let other = id(&world
        .server
        .ok(
            &world.owner,
            "POST",
            "/api/v1/workspaces",
            json!({"name": "Other"}),
        )
        .await);
    let other_page = id(&world
        .server
        .ok(
            &world.owner,
            "POST",
            &format!("/api/v1/workspaces/{other}/pages"),
            json!({"title": "Elsewhere"}),
        )
        .await);
    assert_eq!(world.link(&world.owner, general, &other_page).await.0, 422);
    let (status, _) = world
        .server
        .call(
            bob,
            "GET",
            &format!("/api/v1/workspaces/{other}/pages/{other_page}/linked-tasks"),
            Value::Null,
        )
        .await;
    assert_eq!(status, 404);
    assert_eq!(world.server.link_count().await, 4);

    world.server.stop().await;
}

#[tokio::test]
async fn unlinking_trashing_and_deleting_remove_links() {
    let world = World::new().await;
    let (alice, owner) = (world.alice.as_str(), world.owner.as_str());
    let general = world.general_task.as_str();
    let team = world.team_page.as_str();
    let alice_page = world.alice_page.as_str();

    assert_eq!(world.link(alice, general, team).await.0, 201);
    assert_eq!(world.unlink(alice, general, team).await, 204);
    assert_eq!(world.unlink(alice, general, team).await, 404);
    assert!(world.docs(alice, general).await.is_empty());
    let events = world.doc_events(alice, general).await;
    let actions: Vec<&str> = events
        .iter()
        .map(|e| e["action"].as_str().unwrap())
        .collect();
    assert_eq!(actions, ["task.doc_linked", "task.doc_unlinked"]);
    assert_eq!(events[1]["metadata"]["page_title"], "Team plan");

    // A trashed page is hidden until it is restored; deleting it forever removes the link.
    assert_eq!(world.link(alice, general, team).await.0, 201);
    let version = world.version(owner, "pages", team).await;
    world
        .server
        .ok(
            owner,
            "DELETE",
            &format!("{}/pages/{team}?expected_version={version}", world.base),
            Value::Null,
        )
        .await;
    assert!(world.docs(alice, general).await.is_empty());
    assert_eq!(world.link(alice, general, team).await.0, 422);
    assert_eq!(world.server.link_count().await, 1);
    world
        .server
        .ok(
            owner,
            "POST",
            &format!("{}/pages/{team}/restore", world.base),
            json!({"expected_version": version + 1}),
        )
        .await;
    assert_eq!(world.docs(alice, general).await, [team]);
    let version = world.version(owner, "pages", team).await;
    // The trash bumps the version that deleting forever expects.
    for (suffix, version) in [("", version), ("/permanent", version + 1)] {
        world
            .server
            .ok(
                owner,
                "DELETE",
                &format!(
                    "{}/pages/{team}{suffix}?expected_version={version}",
                    world.base
                ),
                Value::Null,
            )
            .await;
    }
    assert_eq!(world.server.link_count().await, 0);

    // A trashed task leaves the page's list; deleting it forever removes the link.
    assert_eq!(world.link(alice, general, alice_page).await.0, 201);
    assert_eq!(world.tasks(alice, alice_page).await, [general]);
    let version = world.version(alice, "tasks", general).await;
    world
        .server
        .ok(
            alice,
            "DELETE",
            &format!("{}/tasks/{general}?expected_version={version}", world.base),
            Value::Null,
        )
        .await;
    assert!(world.tasks(alice, alice_page).await.is_empty());
    assert_eq!(world.server.link_count().await, 1);
    world
        .server
        .ok(
            owner,
            "DELETE",
            &format!("{}/tasks/{general}/permanent", world.base),
            Value::Null,
        )
        .await;
    assert_eq!(world.server.link_count().await, 0);

    world.server.stop().await;
}
