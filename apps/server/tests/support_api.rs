//! Support mailboxes (docs/support.md): inbound email makes and threads tickets, replies are
//! queued and sent, tickets move between Open, Pending, Solved and Closed, and every route checks
//! its ids. Runs on SQLite, or on Postgres with `ORBIT_TEST_POSTGRES_URL`.

use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use orbit_platform::{
    AttachmentMutationCoordinator, AuthenticatedUser, Id, LocalBlobStore, PasswordService,
    TestDatabase, TimestampMillis, UploadLimits, UploadService,
};
use orbit_server::auth_routes::CookieMode;
use orbit_server::repositories::identity::{IdentityRepository, SetupRequest};
use orbit_server::support::outbound::SupportSender;
use orbit_server::support_routes::{SupportState, support_router};
use orbit_server::task_routes::{TaskState, task_router};
use serde_json::{Value, json};
use tempfile::TempDir;
use tower::ServiceExt;

const KEY: [u8; 32] = [7; 32];

struct Fixture {
    database: TestDatabase,
    _files: TempDir,
    identity: Arc<IdentityRepository>,
    app: axum::Router,
    workspace_id: String,
    project_id: String,
    owner: Person,
}

struct Person {
    id: Id,
    cookie: String,
}

/// A connected mailbox: its id and inbound path.
struct Mailbox {
    hook: String,
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
                    project_name: "Support".to_owned(),
                },
                now,
            )
            .await
            .unwrap();
        let files = TempDir::new().unwrap();
        let uploads = UploadService::new(
            (*database).clone(),
            Arc::new(LocalBlobStore::new(files.path().join("attachments"))),
            AttachmentMutationCoordinator::default(),
            UploadLimits::default(),
        );
        let tasks = TaskState::new(Arc::clone(&identity), CookieMode::secure());
        let app = task_router(tasks.clone()).merge(support_router(SupportState::new(
            tasks,
            uploads,
            Some(KEY),
            "https://hooks.orbit.test",
            true,
        )));
        Self {
            database,
            _files: files,
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

    async fn person(&self, name: &str, role: &str) -> Person {
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

    /// A new workspace owned by `person`; returns its id.
    async fn workspace_owned_by(&self, person: &Person) -> String {
        let (workspace, membership) = (Id::new_v7(), Id::new_v7());
        let now = TimestampMillis::now().as_millis();
        // A workspace and its owner membership reference each other (deferred foreign key).
        let mut tx = self.database.pool().begin().await.unwrap();
        sqlx::query(
            "INSERT INTO workspaces (id, name, owner_membership_id, created_at, updated_at) VALUES ($1, 'Other', $2, $3, $3)",
        )
        .bind(workspace.to_string())
        .bind(membership.to_string())
        .bind(now)
        .execute(&mut *tx)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO memberships (id, workspace_id, user_id, role, created_at, updated_at) VALUES ($1, $2, $3, 'owner', $4, $4)",
        )
        .bind(membership.to_string())
        .bind(workspace.to_string())
        .bind(person.id.to_string())
        .bind(now)
        .execute(&mut *tx)
        .await
        .unwrap();
        tx.commit().await.unwrap();
        workspace.to_string()
    }

    async fn send(&self, request: Request<Body>) -> (StatusCode, Value, String) {
        let response = self.app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let text = String::from_utf8_lossy(&bytes).into_owned();
        (
            status,
            serde_json::from_str(&text).unwrap_or(Value::Null),
            text,
        )
    }

    /// Calls `/api/v1/workspaces/{workspace}{path}` as `person`.
    async fn call(
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
        let (status, value, _) = self.send(request).await;
        (status, value)
    }

    async fn ok(&self, person: &Person, method: &str, path: &str, body: Option<Value>) -> Value {
        let (status, value) = self.call(person, method, path, body).await;
        assert!(status.is_success(), "{method} {path}: {status} {value}");
        value
    }

    async fn connect(&self, move_to_pending: bool) -> Mailbox {
        let created = self
            .ok(
                &self.owner,
                "POST",
                &format!("/projects/{}/support-mailbox", self.project_id),
                Some(json!({
                    "address": "Support@Acme.test",
                    "display_name": "Acme Support",
                    "smtp_host": "127.0.0.1",
                    "smtp_port": 1025,
                    "smtp_security": "none",
                    "smtp_password": "smtp secret",
                    "move_to_pending": move_to_pending,
                })),
            )
            .await;
        let url = created["inbound_url"].as_str().unwrap();
        let hook = url
            .strip_prefix("https://hooks.orbit.test")
            .unwrap()
            .to_owned();
        assert_eq!(created["mailbox"]["address"], "support@acme.test");
        assert_eq!(created["mailbox"]["smtp_password_set"], true);
        Mailbox { hook }
    }

    /// POSTs a raw email to `hook`; returns the status and the text answer.
    async fn deliver(&self, hook: &str, raw: &str) -> (StatusCode, String) {
        let request = Request::builder()
            .method("POST")
            .uri(hook)
            .header(header::CONTENT_TYPE, "message/rfc822")
            .body(Body::from(raw.to_owned()))
            .unwrap();
        let (status, _, text) = self.send(request).await;
        (status, text)
    }

    async fn scalar(&self, sql: &'static str, bind: &str) -> i64 {
        sqlx::query_scalar(sql)
            .bind(bind)
            .fetch_one(self.database.pool())
            .await
            .unwrap()
    }

    /// The tasks of the project, newest first.
    async fn tickets(&self) -> Vec<String> {
        sqlx::query_scalar(
            "SELECT task_id FROM support_tickets ORDER BY created_at DESC, task_id DESC",
        )
        .fetch_all(self.database.pool())
        .await
        .unwrap()
    }

    async fn status_name(&self, task_id: &str) -> String {
        sqlx::query_scalar(
            "SELECT task_statuses.name FROM tasks JOIN task_statuses ON task_statuses.id = tasks.status_id WHERE tasks.id = $1",
        )
        .bind(task_id)
        .fetch_one(self.database.pool())
        .await
        .unwrap()
    }

    async fn status_id(&self, name: &str) -> String {
        sqlx::query_scalar("SELECT id FROM task_statuses WHERE project_id = $1 AND name = $2")
            .bind(&self.project_id)
            .bind(name)
            .fetch_one(self.database.pool())
            .await
            .unwrap()
    }

    async fn set_status(&self, task_id: &str, name: &str) {
        let task = self
            .ok(&self.owner, "GET", &format!("/tasks/{task_id}"), None)
            .await;
        let status = self.status_id(name).await;
        self.ok(
            &self.owner,
            "PATCH",
            &format!("/tasks/{task_id}"),
            Some(json!({ "expected_version": task["version"], "status_id": status })),
        )
        .await;
    }

    /// Sends the queued replies with a recording sender; returns the formatted emails.
    async fn flush(&self) -> Vec<String> {
        let (sender, sent) = SupportSender::recording((*self.database).clone(), KEY);
        let queued: Vec<String> = sqlx::query_scalar(
            "SELECT id FROM support_messages WHERE send_state = 'queued' ORDER BY created_at",
        )
        .fetch_all(self.database.pool())
        .await
        .unwrap();
        for id in queued {
            let _ = sender.deliver(id.parse().unwrap(), true).await;
        }
        let sent = sent.lock().unwrap();
        sent.iter()
            .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
            .collect()
    }
}

fn email(from: &str, subject: &str, message_id: &str, extra: &str, body: &str) -> String {
    format!(
        "From: {from}\r\nTo: support@acme.test\r\nSubject: {subject}\r\nMessage-ID: <{message_id}>\r\n{extra}\
         Content-Type: text/plain; charset=utf-8\r\n\r\n{body}\r\n"
    )
}

/// The header value of a formatted email.
fn header_of(raw: &str, name: &str) -> String {
    raw.lines()
        .find_map(|line| line.strip_prefix(&format!("{name}: ")))
        .unwrap_or_default()
        .trim()
        .to_owned()
}

#[tokio::test]
async fn an_email_makes_a_ticket_in_open_with_its_customer() {
    let fixture = Fixture::new().await;
    let mailbox = fixture.connect(true).await;
    // the support statuses were added to the project
    for name in ["Open", "Pending", "Solved", "Closed"] {
        fixture.status_id(name).await;
    }
    let (status, answer) = fixture
        .deliver(
            &mailbox.hook,
            &email(
                "Ada Lovelace <ada@customer.test>",
                "Cannot log in",
                "first@customer.test",
                "",
                "Help please",
            ),
        )
        .await;
    assert_eq!((status, answer.as_str()), (StatusCode::OK, "ok"));
    let tickets = fixture.tickets().await;
    assert_eq!(tickets.len(), 1);
    let task_id = &tickets[0];
    assert_eq!(fixture.status_name(task_id).await, "Open");
    let task = fixture
        .ok(&fixture.owner, "GET", &format!("/tasks/{task_id}"), None)
        .await;
    assert_eq!(task["title"], "Cannot log in");
    let view = fixture
        .ok(
            &fixture.owner,
            "GET",
            &format!("/tasks/{task_id}/support"),
            None,
        )
        .await;
    assert_eq!(view["ticket"]["contact"]["email"], "ada@customer.test");
    assert_eq!(view["ticket"]["contact"]["name"], "Ada Lovelace");
    assert_eq!(view["ticket"]["can_reply"], true);
    assert_eq!(view["messages"][0]["direction"], "in");
    assert!(
        view["messages"][0]["body_text"]
            .as_str()
            .unwrap()
            .contains("Help please")
    );

    // a redelivery of the same Message-ID is filed once
    fixture
        .deliver(
            &mailbox.hook,
            &email(
                "ada@customer.test",
                "Cannot log in",
                "first@customer.test",
                "",
                "Help please",
            ),
        )
        .await;
    assert_eq!(
        fixture
            .scalar(
                "SELECT COUNT(*) FROM support_messages WHERE task_id = $1",
                task_id
            )
            .await,
        1
    );
}

#[tokio::test]
async fn html_and_attachments_are_cleaned_and_stored_as_task_attachments() {
    let fixture = Fixture::new().await;
    let mailbox = fixture.connect(true).await;
    let raw = "From: ada@customer.test\r\nSubject: Screenshot\r\nMessage-ID: <html@customer.test>\r\n\
MIME-Version: 1.0\r\nContent-Type: multipart/related; boundary=\"rel\"\r\n\r\n\
--rel\r\nContent-Type: text/html; charset=utf-8\r\n\r\n\
<p onclick=\"alert(1)\">See <img src=\"cid:shot@x\"></p><script>alert(2)</script><img src=\"https://tracker.test/p.gif\">\r\n\
--rel\r\nContent-Type: image/png\r\nContent-ID: <shot@x>\r\nContent-Disposition: inline; filename=\"shot.png\"\r\n\
Content-Transfer-Encoding: base64\r\n\r\niVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==\r\n\
--rel\r\nContent-Type: text/html\r\nContent-Disposition: attachment; filename=\"page.html\"\r\n\r\n<script>alert(3)</script>\r\n\
--rel--\r\n";
    assert_eq!(fixture.deliver(&mailbox.hook, raw).await.0, StatusCode::OK);
    let task_id = fixture.tickets().await.remove(0);
    let view = fixture
        .ok(
            &fixture.owner,
            "GET",
            &format!("/tasks/{task_id}/support"),
            None,
        )
        .await;
    let message = &view["messages"][0];
    let html = message["body_html"].as_str().unwrap();
    assert!(
        !html.contains("onclick") && !html.contains("script"),
        "{html}"
    );
    let attachments = message["attachments"].as_array().unwrap();
    assert_eq!(attachments.len(), 2);
    let image = attachments
        .iter()
        .find(|a| a["name"] == "shot.png")
        .unwrap();
    assert_eq!(image["inline"], true);
    assert_eq!(image["media_type"], "image/png");
    let image_id = image["id"].as_str().unwrap();
    assert!(
        html.contains(&format!("/tasks/{task_id}/attachments/{image_id}/download")),
        "{html}"
    );
    // the HTML file is a download, never a page
    let page = attachments
        .iter()
        .find(|a| a["name"] == "page.html")
        .unwrap();
    assert_eq!(page["inline"], false);
    assert_ne!(page["media_type"], "image/png");
    assert_eq!(
        fixture
            .scalar(
                "SELECT COUNT(*) FROM attachment_references WHERE task_id = $1",
                &task_id
            )
            .await,
        2
    );
}

#[tokio::test]
async fn reply_queues_an_email_moves_to_pending_and_a_customer_reply_reopens() {
    let fixture = Fixture::new().await;
    let mailbox = fixture.connect(true).await;
    fixture
        .deliver(
            &mailbox.hook,
            &email("ada@customer.test", "Login", "c1@customer.test", "", "Help"),
        )
        .await;
    let task_id = fixture.tickets().await.remove(0);

    // an internal note is a comment: no email
    fixture
        .ok(
            &fixture.owner,
            "POST",
            &format!("/tasks/{task_id}/comments"),
            Some(json!({ "body": "internal: check the logs" })),
        )
        .await;
    assert!(fixture.flush().await.is_empty());

    let reply = fixture
        .ok(
            &fixture.owner,
            "POST",
            &format!("/tasks/{task_id}/support/replies"),
            Some(json!({ "body": "Please try again now." })),
        )
        .await;
    assert_eq!(reply["send_state"], "queued");
    assert_eq!(reply["to"], json!(["ada@customer.test"]));
    assert_eq!(fixture.status_name(&task_id).await, "Pending");

    let sent = fixture.flush().await;
    assert_eq!(sent.len(), 1);
    let out = &sent[0];
    assert!(out.contains("Please try again now."), "{out}");
    assert_eq!(header_of(out, "In-Reply-To"), "<c1@customer.test>");
    assert!(header_of(out, "Subject").contains("[#"), "{out}");
    let our_id = header_of(out, "Message-ID");
    let view = fixture
        .ok(
            &fixture.owner,
            "GET",
            &format!("/tasks/{task_id}/support"),
            None,
        )
        .await;
    assert_eq!(view["messages"][1]["send_state"], "sent");

    // the customer answers our reply: same ticket, back to Open
    let (status, _) = fixture
        .deliver(
            &mailbox.hook,
            &email(
                "ada@customer.test",
                "Re: Login",
                "c2@customer.test",
                &format!("In-Reply-To: {our_id}\r\nReferences: <c1@customer.test> {our_id}\r\n"),
                "Works, thanks",
            ),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(fixture.tickets().await.len(), 1);
    assert_eq!(fixture.status_name(&task_id).await, "Open");

    // Solved reopens; Closed makes a new ticket
    fixture.set_status(&task_id, "Solved").await;
    fixture
        .deliver(
            &mailbox.hook,
            &email(
                "ada@customer.test",
                "Re: Login",
                "c3@customer.test",
                &format!("In-Reply-To: {our_id}\r\n"),
                "Again",
            ),
        )
        .await;
    assert_eq!(fixture.tickets().await.len(), 1);
    assert_eq!(fixture.status_name(&task_id).await, "Open");
    fixture.set_status(&task_id, "Closed").await;
    fixture
        .deliver(
            &mailbox.hook,
            &email(
                "ada@customer.test",
                "Re: Login",
                "c4@customer.test",
                &format!("In-Reply-To: {our_id}\r\n"),
                "Once more",
            ),
        )
        .await;
    let tickets = fixture.tickets().await;
    assert_eq!(tickets.len(), 2);
    assert_ne!(tickets[0], task_id);
    assert_eq!(fixture.status_name(&tickets[0]).await, "Open");
    assert_eq!(fixture.status_name(&task_id).await, "Closed");
}

#[tokio::test]
async fn a_reply_without_the_pending_setting_keeps_the_status() {
    let fixture = Fixture::new().await;
    let mailbox = fixture.connect(false).await;
    fixture
        .deliver(
            &mailbox.hook,
            &email("ada@customer.test", "Login", "p1@customer.test", "", "Help"),
        )
        .await;
    let task_id = fixture.tickets().await.remove(0);
    fixture
        .ok(
            &fixture.owner,
            "POST",
            &format!("/tasks/{task_id}/support/replies"),
            Some(json!({ "body": "On it." })),
        )
        .await;
    assert_eq!(fixture.status_name(&task_id).await, "Open");
}

#[tokio::test]
async fn forged_message_ids_and_foreign_subject_tags_make_new_tickets() {
    let fixture = Fixture::new().await;
    let mailbox = fixture.connect(true).await;
    fixture
        .deliver(
            &mailbox.hook,
            &email("ada@customer.test", "Mine", "a1@customer.test", "", "x"),
        )
        .await;
    let ada_ticket = fixture.tickets().await.remove(0);
    let identifier = {
        let task = fixture
            .ok(&fixture.owner, "GET", &format!("/tasks/{ada_ticket}"), None)
            .await;
        let key: String = sqlx::query_scalar("SELECT project_key FROM projects WHERE id = $1")
            .bind(&fixture.project_id)
            .fetch_one(fixture.database.pool())
            .await
            .unwrap();
        format!("{key}-{}", task["number"])
    };

    // an id in Orbit's format with Ada's task id but a made-up tag
    let forged = format!("orbit.{ada_ticket}.0123.{}@acme.test", "ab".repeat(16));
    fixture
        .deliver(
            &mailbox.hook,
            &email(
                "eve@attacker.test",
                "Hi",
                "e1@attacker.test",
                &format!("In-Reply-To: <{forged}>\r\n"),
                "x",
            ),
        )
        .await;
    assert_eq!(fixture.tickets().await.len(), 2);

    // the subject tag of Ada's ticket from another sender
    fixture
        .deliver(
            &mailbox.hook,
            &email(
                "eve@attacker.test",
                &format!("Re: [#{identifier}]"),
                "e2@attacker.test",
                "",
                "x",
            ),
        )
        .await;
    assert_eq!(fixture.tickets().await.len(), 3);
    assert_eq!(
        fixture
            .scalar(
                "SELECT COUNT(*) FROM support_messages WHERE task_id = $1",
                &ada_ticket
            )
            .await,
        1
    );

    // the subject tag from Ada herself threads
    fixture
        .deliver(
            &mailbox.hook,
            &email(
                "ada@customer.test",
                &format!("Re: Mine [#{identifier}]"),
                "a2@customer.test",
                "",
                "x",
            ),
        )
        .await;
    assert_eq!(fixture.tickets().await.len(), 3);
    assert_eq!(
        fixture
            .scalar(
                "SELECT COUNT(*) FROM support_messages WHERE task_id = $1",
                &ada_ticket
            )
            .await,
        2
    );
}

#[tokio::test]
async fn auto_replies_and_bounces_are_dropped() {
    let fixture = Fixture::new().await;
    let mailbox = fixture.connect(true).await;
    for (from, extra) in [
        ("ada@customer.test", "Auto-Submitted: auto-replied\r\n"),
        ("ada@customer.test", "Precedence: bulk\r\n"),
        ("MAILER-DAEMON@customer.test", ""),
    ] {
        let (status, answer) = fixture
            .deliver(
                &mailbox.hook,
                &email(
                    from,
                    "Out of office",
                    &format!("{}@x", Id::new_v7()),
                    extra,
                    "x",
                ),
            )
            .await;
        assert_eq!((status, answer.as_str()), (StatusCode::OK, "dropped"));
    }
    assert!(fixture.tickets().await.is_empty());
}

#[tokio::test]
async fn the_inbound_url_needs_the_mailbox_and_its_current_secret() {
    let fixture = Fixture::new().await;
    let mailbox = fixture.connect(true).await;
    let raw = email("ada@customer.test", "x", "s1@customer.test", "", "x");
    let (mailbox_id, secret) = mailbox
        .hook
        .strip_prefix("/api/v1/hooks/support/")
        .unwrap()
        .split_once('/')
        .unwrap();
    for hook in [
        format!("/api/v1/hooks/support/{mailbox_id}/wrong-secret"),
        format!("/api/v1/hooks/support/{}/{secret}", Id::new_v7()),
        format!("/api/v1/hooks/support/not-an-id/{secret}"),
    ] {
        assert_eq!(
            fixture.deliver(&hook, &raw).await.0,
            StatusCode::NOT_FOUND,
            "{hook}"
        );
    }
    assert_eq!(
        fixture.deliver(&mailbox.hook, "not an email").await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );

    let rotated = fixture
        .ok(
            &fixture.owner,
            "POST",
            &format!("/projects/{}/support-mailbox/rotate", fixture.project_id),
            None,
        )
        .await;
    assert_eq!(
        fixture.deliver(&mailbox.hook, &raw).await.0,
        StatusCode::NOT_FOUND
    );
    let new_hook = rotated["inbound_url"]
        .as_str()
        .unwrap()
        .strip_prefix("https://hooks.orbit.test")
        .unwrap()
        .to_owned();
    assert_eq!(fixture.deliver(&new_hook, &raw).await.0, StatusCode::OK);
    // only a hash of the secret is stored
    let stored: Vec<u8> = sqlx::query_scalar("SELECT secret_hash FROM support_mailboxes")
        .fetch_one(fixture.database.pool())
        .await
        .unwrap();
    assert_eq!(stored.len(), 32);
    let password: Vec<u8> = sqlx::query_scalar("SELECT smtp_password FROM support_mailboxes")
        .fetch_one(fixture.database.pool())
        .await
        .unwrap();
    assert!(!String::from_utf8_lossy(&password).contains("smtp secret"));
}

#[tokio::test]
async fn routes_check_workspace_project_and_role_for_every_id() {
    let fixture = Fixture::new().await;
    let mailbox = fixture.connect(true).await;
    fixture
        .deliver(
            &mailbox.hook,
            &email("ada@customer.test", "x", "i1@customer.test", "", "x"),
        )
        .await;
    let task_id = fixture.tickets().await.remove(0);
    let member = fixture.person("member", "member").await;
    let guest = fixture.person("guest", "guest").await;
    let mailbox_path = format!("/projects/{}/support-mailbox", fixture.project_id);

    // members do not manage mailboxes; guests see nothing
    assert_eq!(
        fixture.call(&member, "GET", &mailbox_path, None).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        fixture
            .call(&member, "POST", &format!("{mailbox_path}/rotate"), None)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        fixture.call(&member, "DELETE", &mailbox_path, None).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        fixture.call(&guest, "GET", &mailbox_path, None).await.0,
        StatusCode::NOT_FOUND
    );
    let support = format!("/tasks/{task_id}/support");
    assert_eq!(
        fixture.call(&guest, "GET", &support, None).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        fixture
            .call(
                &guest,
                "POST",
                &format!("{support}/replies"),
                Some(json!({ "body": "x" }))
            )
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    // a workspace member sees a ticket of a workspace project
    assert_eq!(
        fixture.call(&member, "GET", &support, None).await.0,
        StatusCode::OK
    );

    // a private project hides its tickets from members it does not list
    sqlx::query("UPDATE projects SET private = 1 WHERE id = $1")
        .bind(&fixture.project_id)
        .execute(fixture.database.pool())
        .await
        .unwrap();
    assert_eq!(
        fixture.call(&member, "GET", &support, None).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        fixture
            .call(
                &member,
                "POST",
                &format!("{support}/replies"),
                Some(json!({ "body": "x" }))
            )
            .await
            .0,
        StatusCode::NOT_FOUND
    );

    // the member owns another workspace: its id with this workspace's task, project or message
    // ids finds nothing
    let other_workspace = fixture.workspace_owned_by(&member).await;
    let reply = fixture
        .ok(
            &fixture.owner,
            "POST",
            &format!("{support}/replies"),
            Some(json!({ "body": "x" })),
        )
        .await;
    let message_id = reply["id"].as_str().unwrap().to_owned();
    sqlx::query("UPDATE support_messages SET send_state = 'failed' WHERE id = $1")
        .bind(&message_id)
        .execute(fixture.database.pool())
        .await
        .unwrap();
    for (method, path) in [
        ("GET", support.clone()),
        ("GET", mailbox_path.clone()),
        ("POST", format!("/support/messages/{message_id}/retry")),
    ] {
        let request = Request::builder()
            .method(method)
            .uri(format!("/api/v1/workspaces/{other_workspace}{path}"))
            .header(header::COOKIE, &member.cookie)
            .body(Body::empty())
            .unwrap();
        let (status, _, _) = fixture.send(request).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
    }
    // a message of a task the member does not see (private project)
    let retry = format!("/support/messages/{message_id}/retry");
    assert_eq!(
        fixture.call(&member, "POST", &retry, None).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        fixture.call(&guest, "POST", &retry, None).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        fixture.call(&fixture.owner, "POST", &retry, None).await.0,
        StatusCode::OK
    );
}

#[tokio::test]
async fn the_daily_send_cap_fails_the_reply_and_retry_queues_it_again() {
    let fixture = Fixture::new().await;
    let mailbox = fixture.connect(true).await;
    fixture
        .deliver(
            &mailbox.hook,
            &email("ada@customer.test", "x", "q1@customer.test", "", "x"),
        )
        .await;
    let task_id = fixture.tickets().await.remove(0);
    let mailbox_id: String = sqlx::query_scalar("SELECT id FROM support_mailboxes")
        .fetch_one(fixture.database.pool())
        .await
        .unwrap();
    let now = TimestampMillis::now().as_millis();
    for index in 0..500 {
        sqlx::query(
            "INSERT INTO email_sends (scope, subject, sent_at) VALUES ('support_mailbox', $1, $2)",
        )
        .bind(&mailbox_id)
        .bind(now - 1_000 - index)
        .execute(fixture.database.pool())
        .await
        .unwrap();
    }
    let reply = fixture
        .ok(
            &fixture.owner,
            "POST",
            &format!("/tasks/{task_id}/support/replies"),
            Some(json!({ "body": "x" })),
        )
        .await;
    assert!(fixture.flush().await.is_empty());
    let view = fixture
        .ok(
            &fixture.owner,
            "GET",
            &format!("/tasks/{task_id}/support"),
            None,
        )
        .await;
    assert_eq!(view["messages"][1]["send_state"], "failed");
    assert!(
        view["messages"][1]["send_error"]
            .as_str()
            .unwrap()
            .contains("500")
    );

    sqlx::query("DELETE FROM email_sends")
        .execute(fixture.database.pool())
        .await
        .unwrap();
    let retried = fixture
        .ok(
            &fixture.owner,
            "POST",
            &format!("/support/messages/{}/retry", reply["id"].as_str().unwrap()),
            None,
        )
        .await;
    assert_eq!(retried["send_state"], "queued");
    assert_eq!(fixture.flush().await.len(), 1);
    // a sent reply cannot be retried
    assert_eq!(
        fixture
            .call(
                &fixture.owner,
                "POST",
                &format!("/support/messages/{}/retry", reply["id"].as_str().unwrap()),
                None,
            )
            .await
            .0,
        StatusCode::CONFLICT
    );
}

#[tokio::test]
async fn the_default_assignee_and_followers_are_notified_of_customer_email() {
    let fixture = Fixture::new().await;
    let member = fixture.person("agent", "member").await;
    let guest = fixture.person("guest", "guest").await;
    // a guest cannot be the default assignee
    let (status, _) = fixture
        .call(
            &fixture.owner,
            "POST",
            &format!("/projects/{}/support-mailbox", fixture.project_id),
            Some(json!({ "address": "support@acme.test", "default_assignee_id": guest.id.to_string() })),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let created = fixture
        .ok(
            &fixture.owner,
            "POST",
            &format!("/projects/{}/support-mailbox", fixture.project_id),
            Some(json!({ "address": "support@acme.test", "default_assignee_id": member.id.to_string() })),
        )
        .await;
    let hook = created["inbound_url"]
        .as_str()
        .unwrap()
        .strip_prefix("https://hooks.orbit.test")
        .unwrap()
        .to_owned();
    fixture
        .deliver(
            &hook,
            &email("ada@customer.test", "x", "n1@customer.test", "", "x"),
        )
        .await;
    let task_id = fixture.tickets().await.remove(0);
    let task = fixture
        .ok(&fixture.owner, "GET", &format!("/tasks/{task_id}"), None)
        .await;
    assert_eq!(task["assignee_ids"], json!([member.id.to_string()]));
    let notifications: Vec<(String, String)> = sqlx::query_as(
        "SELECT recipient_user_id, actor_github_login FROM notifications WHERE task_id = $1",
    )
    .bind(&task_id)
    .fetch_all(fixture.database.pool())
    .await
    .unwrap();
    assert_eq!(
        notifications,
        vec![(member.id.to_string(), "ada@customer.test".to_owned())]
    );
    // without an SMTP server the ticket cannot be answered
    let view = fixture
        .ok(&member, "GET", &format!("/tasks/{task_id}/support"), None)
        .await;
    assert_eq!(view["ticket"]["can_reply"], false);
    assert_eq!(
        fixture
            .call(
                &member,
                "POST",
                &format!("/tasks/{task_id}/support/replies"),
                Some(json!({ "body": "x" }))
            )
            .await
            .0,
        StatusCode::CONFLICT
    );
}

/// A writer that keeps every log line, to check what the server logs.
#[derive(Clone, Default)]
struct Logs(Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for Logs {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// An SMTP server that answers every sign-in as Microsoft 365 does when SMTP AUTH is off. It
/// keeps the AUTH lines it got, so the test knows the password really went to the server.
async fn smtp_auth_disabled() -> (u16, Arc<std::sync::Mutex<Vec<String>>>) {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let auth_lines = Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen = Arc::clone(&auth_lines);
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let seen = Arc::clone(&seen);
            tokio::spawn(async move {
                let (read, mut write) = stream.into_split();
                let mut lines = BufReader::new(read).lines();
                write
                    .write_all(b"220 fake.outlook.test ESMTP\r\n")
                    .await
                    .unwrap();
                while let Ok(Some(line)) = lines.next_line().await {
                    let upper = line.to_ascii_uppercase();
                    let answer: &[u8] = if upper.starts_with("EHLO") || upper.starts_with("HELO") {
                        b"250-fake.outlook.test\r\n250 AUTH PLAIN LOGIN\r\n"
                    } else if upper.starts_with("AUTH") {
                        seen.lock().unwrap().push(line.clone());
                        b"535 5.7.139 Authentication unsuccessful, SmtpClientAuthentication is disabled for the Mailbox. Visit https://aka.ms/smtp_auth_disabled for more information.\r\n"
                    } else if upper.starts_with("QUIT") {
                        let _ = write.write_all(b"221 Bye\r\n").await;
                        break;
                    } else {
                        b"250 OK\r\n"
                    };
                    if write.write_all(answer).await.is_err() {
                        break;
                    }
                }
            });
        }
    });
    (port, auth_lines)
}

#[tokio::test]
async fn the_smtp_password_never_reaches_responses_logs_or_error_texts() {
    const PASSWORD: &str = "Pa55-Secret-xyz!";
    let logs = Logs::default();
    let writer = logs.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_writer(move || writer.clone())
        .finish();
    let _logging = tracing::subscriber::set_default(subscriber);

    let fixture = Fixture::new().await;
    let (port, auth_lines) = smtp_auth_disabled().await;
    let mailbox_path = format!("/projects/{}/support-mailbox", fixture.project_id);
    let mut bodies = Vec::new();
    let created = fixture
        .ok(
            &fixture.owner,
            "POST",
            &mailbox_path,
            Some(json!({
                "address": "support@acme.test",
                "smtp_host": "127.0.0.1",
                "smtp_port": port,
                "smtp_security": "none",
                "smtp_password": PASSWORD,
            })),
        )
        .await;
    // without a user name, the mailbox address signs in (Microsoft 365)
    assert_eq!(created["mailbox"]["smtp_username"], "support@acme.test");
    bodies.push(created.to_string());
    bodies.push(
        fixture
            .ok(&fixture.owner, "GET", &mailbox_path, None)
            .await
            .to_string(),
    );

    // Test connection: refused with the fix steps
    let tested = fixture
        .ok(
            &fixture.owner,
            "POST",
            &format!("{mailbox_path}/test"),
            None,
        )
        .await;
    assert_eq!(tested["ok"], false);
    assert!(
        tested["message"]
            .as_str()
            .unwrap()
            .contains("Authenticated SMTP"),
        "{tested}"
    );
    bodies.push(tested.to_string());
    let member = fixture.person("member", "member").await;
    assert_eq!(
        fixture
            .call(&member, "POST", &format!("{mailbox_path}/test"), None)
            .await
            .0,
        StatusCode::FORBIDDEN
    );

    // a reply that the server refuses: the reason, not the password
    let hook = created["inbound_url"]
        .as_str()
        .unwrap()
        .strip_prefix("https://hooks.orbit.test")
        .unwrap()
        .to_owned();
    fixture
        .deliver(
            &hook,
            &email("ada@customer.test", "x", "pw1@customer.test", "", "x"),
        )
        .await;
    let task_id = fixture.tickets().await.remove(0);
    fixture
        .ok(
            &fixture.owner,
            "POST",
            &format!("/tasks/{task_id}/support/replies"),
            Some(json!({ "body": "Hello" })),
        )
        .await;
    let sender = SupportSender::new((*fixture.database).clone(), Some(KEY), true);
    let queued: String =
        sqlx::query_scalar("SELECT id FROM support_messages WHERE direction = 'out'")
            .fetch_one(fixture.database.pool())
            .await
            .unwrap();
    let failure = sender
        .deliver(queued.parse().unwrap(), true)
        .await
        .unwrap_err();
    assert!(!failure.retry);
    let view = fixture
        .ok(
            &fixture.owner,
            "GET",
            &format!("/tasks/{task_id}/support"),
            None,
        )
        .await;
    assert_eq!(view["messages"][1]["send_state"], "failed");
    assert!(
        view["messages"][1]["send_error"]
            .as_str()
            .unwrap()
            .contains("5.7.139")
    );
    bodies.push(view.to_string());

    // the password did reach the SMTP server (so the checks below mean something) ...
    assert!(!auth_lines.lock().unwrap().is_empty());
    // ... and nowhere else
    let logged = String::from_utf8_lossy(&logs.0.lock().unwrap()).into_owned();
    for text in bodies.iter().chain([&logged]) {
        assert!(!text.contains(PASSWORD), "password in {text}");
    }
    let stored: Vec<u8> = sqlx::query_scalar("SELECT smtp_password FROM support_mailboxes")
        .fetch_one(fixture.database.pool())
        .await
        .unwrap();
    assert!(!String::from_utf8_lossy(&stored).contains(PASSWORD));
}

#[tokio::test]
async fn a_forward_from_the_support_mailbox_files_the_original_customer() {
    let fixture = Fixture::new().await;
    let mailbox = fixture.connect(true).await;
    // an Outlook "Forward" rule on the support mailbox: From is the mailbox
    fixture
        .deliver(
            &mailbox.hook,
            &email(
                "Acme Support <support@acme.test>",
                "FW: Export broken",
                "fw1@acme.test",
                "",
                "\r\n________________________________\r\nFrom: Ada Lovelace <ada@customer.test>\r\nSent: Monday\r\nSubject: Export broken\r\n\r\nHelp",
            ),
        )
        .await;
    let task_id = fixture.tickets().await.remove(0);
    let view = fixture
        .ok(
            &fixture.owner,
            "GET",
            &format!("/tasks/{task_id}/support"),
            None,
        )
        .await;
    assert_eq!(view["ticket"]["contact"]["email"], "ada@customer.test");
    assert_eq!(view["ticket"]["contact"]["name"], "Ada Lovelace");
    let reply = fixture
        .ok(
            &fixture.owner,
            "POST",
            &format!("/tasks/{task_id}/support/replies"),
            Some(json!({ "body": "On it" })),
        )
        .await;
    assert_eq!(reply["to"], json!(["ada@customer.test"]));

    // a forward with no customer in it: no reply to the support address itself (a loop)
    fixture
        .deliver(
            &mailbox.hook,
            &email("support@acme.test", "FW: ?", "fw2@acme.test", "", "x"),
        )
        .await;
    let looped = fixture.tickets().await.remove(0);
    assert_ne!(looped, task_id);
    assert_eq!(
        fixture
            .call(
                &fixture.owner,
                "POST",
                &format!("/tasks/{looped}/support/replies"),
                Some(json!({ "body": "x" }))
            )
            .await
            .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
}
