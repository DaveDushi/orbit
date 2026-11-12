//! GitHub comment sync, both ways: `issue_comment` webhooks become notes, and notes written in
//! Orbit queue `github.comment` jobs that post, edit and delete the GitHub comment (against a
//! local stand-in for the GitHub API).

use std::sync::{Arc, Mutex};

use axum::body::{Body, Bytes};
use axum::http::{Method, Request, StatusCode, Uri, header};
use axum::response::IntoResponse;
use chacha20poly1305::aead::{Aead, AeadCore, KeyInit, OsRng};
use chacha20poly1305::{ChaCha20Poly1305, Key};
use hmac::{Hmac, Mac};
use orbit_platform::{
    HttpPlatformLayer, Id, OriginPolicy, PasswordService, Row, TestDatabase, TimestampMillis,
};
use orbit_server::auth_routes::CookieMode;
use orbit_server::github_comments::{GithubApi, sync_note};
use orbit_server::integration_routes::{IntegrationState, integration_router};
use orbit_server::repositories::api_tokens::ApiTokenRepository;
use orbit_server::repositories::identity::{IdentityRepository, SetupRequest};
use orbit_server::repositories::tasks::{CommentContent, TaskRepository};
use serde_json::{Value, json};
use sha2::Sha256;
use tower::ServiceExt;

const KEY: [u8; 32] = [7u8; 32];
/// A throwaway 2048-bit key in GitHub's PKCS#1 format.
const APP_PEM: &str = "-----BEGIN RSA PRIVATE KEY-----\nMIIEpAIBAAKCAQEApKLylRa0dcJ1tFxZ7NEfhv9gUizg4zzEq3KyrmfoeetuwaSA\nSygPUi6kIOPzshcD7EsHQTWStyH+Vr0ATIWK5TOkkxD4ejqS9ycZ1II+HV4ab4gi\nod1HuCs6rQEk4PT+Ii5sKW1Irg9DHmSTsOS3C6FN5Rw7Veq+bXjcS4VQB5ofQSyv\nrSYJEg0sCMvJUek5emr24Kwn6iN7mnvfo40G1TIM5/AQgC/zvhwXHtYa+2TPlcdi\nvUXHxYV0YWDDmIhKTaEGpCl8Ob6QPUp85hwZDJTxe0ZUVXucmj3tzNOZqR0wMlAd\nnk/OfDroxhrPlP5tckoMal9h0l+b9rj73eOO3wIDAQABAoIBABrt1Z9R8GtBRyly\nNaSebCQTtM3xg0iypuSi79vGfpMlUZ92Pc837+bxtBehJuT0Dh3iA9TEY4l1k6Qy\naVBwQ3t2tVj3h1J5hD0i9dRPCBlGTC1UEijHCLUfdQsuoGP0uXYegRihmdLGsk7Q\nBWGuIiFSiBWNRvNTC0TdZlh//MrRRT+nPMIIr/72sPxghbm7h+GmmTfdvS9d+6Fw\nVL4EVxGpDrtiXsi9k47Tumik+oS+t1a1W+RZIKD6kt8uUPFn77Ij+Sb0FBNawo9A\ncnpQ7Pm+d5iL/q0GIccn9OV561FTlCTE1isseNzs8Sfj2KQQZ0LOF0fOtOtUq3js\nLGa2q9kCgYEA2OnEvh/+Au+DEAfApM0BrDIOdy7LQDYBnBXiRqm/URyikZ/sx+MV\nIDlbvdaPXQisEr7lZH2EI+FKhaHM4mLeWvTU5162UeDOXr1KoxizHmudlXRRcpjB\nFHQxCQtrIQPWBJyZcC1y454XCi0uRtlPliWNp2k23fuF8hEX38nU0W0CgYEAwk2m\nnUJ8E933u+FLB98TBm+yxvo9ca31fmHq0Ezww/53UfjR4pL0XsLR8ZSlxyIUp6mU\nQs+4enBeeLuNmptqyERnEr5IsBKUbWj4PTXLMi08EusIe6jzls4N0NQACP6WtzoE\nhjIUXHAZfLY13GxnYVpQ5QbfyXPVBPtxA0zYffsCgYAJZlf4RTqktexgDXXzrk6c\nP68Qyx5TsgNv2CLuHP0FbtA8r0l7gAOyWcn3+AxOY8nRJiSarjXDuEeacuCGDvLD\nzkhLnncYJT3MLTfGyZM5hkBV8ax14ti7Ii3dznjd/WSQ3phnnBCWfItTDZIrz877\nGrWAw7Z0+CldzA/UytJGIQKBgQCOdIHwkFsJ8dZ8r/vjpIB9Wo13ZuZ/nyv64e1X\nlOF8wOdYeRXgiR+TDkEAXpyAeBMurZzesw6s7g/FcrSqIsJSXBw52LrQJH8gDDLc\nJcF5Y0LFF/6bM06MRTxJ2/S+o3F6mpXQkYzuDN4S2kB5AB13edEPaU/PkhYe1f6Q\njdm32wKBgQCoKa89jnCtn7mHsJpwWp5n1QAjtJ0hy/zmBDSFcYAN+mMg9jVae/JO\neFIIgEo8FmKV3p+t8he2Gqxtr6XebDjHiJfoqM+SWleXZmafd3KHPTlGD/DemwKp\nbLhf1MiDcbItnjyH3Q8atTizkuuw+yMT0s4sszpCRYBGERtKlwt8AA==\n-----END RSA PRIVATE KEY-----";

struct Fixture {
    app: axum::Router,
    database: TestDatabase,
    tasks: TaskRepository,
    user_id: Id,
    workspace_id: Id,
}

/// Workspace with app 77 (`orbit-test`, secret `hook-secret`) installed as 1234 and connected to
/// `acme/repo` with the label `Orbit`.
async fn fixture() -> Fixture {
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
                token: "setup".to_owned(),
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
    let tasks = TaskRepository::new((*database).clone());
    let state = IntegrationState::new(
        Arc::new(ApiTokenRepository::new((*database).clone())),
        Arc::new(tasks.clone()),
    )
    .with_github_settings(
        Arc::new(identity),
        CookieMode::secure(),
        "https://orbit.test".to_owned(),
        false,
        Some(KEY),
    );
    let workspace = setup.workspace_id.to_string();
    let pool = database.pool();
    sqlx::query("INSERT INTO github_apps (workspace_id, app_id, slug, private_key_encrypted, webhook_secret_encrypted, created_at) VALUES ($1, 77, 'orbit-test', $2, $3, 0)")
        .bind(&workspace).bind(encrypt(APP_PEM)).bind(encrypt("hook-secret"))
        .execute(pool).await.unwrap();
    sqlx::query("INSERT INTO github_installations (installation_id, workspace_id, account_login) VALUES (1234, $1, 'acme')")
        .bind(&workspace).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO github_project_connections (project_id, workspace_id, installation_id, repository, label) VALUES ($1, $2, 1234, 'acme/repo', 'Orbit')")
        .bind(setup.project_id.to_string()).bind(&workspace).execute(pool).await.unwrap();
    let app = integration_router(state).layer(HttpPlatformLayer::new(OriginPolicy::new(
        "https://orbit.test",
    )));
    Fixture {
        app,
        database,
        tasks,
        user_id: setup.user_id,
        workspace_id: setup.workspace_id,
    }
}

fn encrypt(value: &str) -> Vec<u8> {
    let cipher = ChaCha20Poly1305::new(Key::from_slice(&KEY));
    let nonce = ChaCha20Poly1305::generate_nonce(&mut OsRng);
    let mut encrypted = nonce.to_vec();
    encrypted.extend(cipher.encrypt(&nonce, value.as_bytes()).unwrap());
    encrypted
}

fn signed(event: &str, body: &Value) -> Request<Body> {
    let body = body.to_string();
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(b"hook-secret").unwrap();
    mac.update(body.as_bytes());
    let digest: String = mac
        .finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    Request::builder()
        .method("POST")
        .uri("/api/v1/integrations/github/webhook")
        .header(header::CONTENT_TYPE, "application/json")
        .header("x-github-event", event)
        .header("x-hub-signature-256", format!("sha256={digest}"))
        .header("x-github-hook-installation-target-type", "integration")
        .header("x-github-hook-installation-target-id", "77")
        .body(Body::from(body))
        .unwrap()
}

impl Fixture {
    async fn send(&self, event: &str, body: Value) -> StatusCode {
        self.app
            .clone()
            .oneshot(signed(event, &body))
            .await
            .unwrap()
            .status()
    }

    /// Opens issue (or pull request) `number` with the sync label; returns its task.
    async fn linked(&self, kind: &str, number: i64) -> Id {
        let item = json!({"number": number, "title": format!("Item {number}"), "body": "", "state": "open", "labels": [{"name": "Orbit"}]});
        let event = if kind == "issue" {
            "issues"
        } else {
            "pull_request"
        };
        let status = self
            .send(event, json!({"action": "opened", "installation": {"id": 1234}, "repository": {"full_name": "acme/repo"}, kind: item}))
            .await;
        assert_eq!(status, StatusCode::OK);
        sqlx::query_scalar::<_, String>(
            "SELECT task_id FROM github_issue_links WHERE issue_number = $1",
        )
        .bind(number)
        .fetch_one(self.database.pool())
        .await
        .unwrap()
        .parse()
        .unwrap()
    }

    async fn notes(&self, task: Id) -> Vec<(String, Option<String>, Option<i64>)> {
        sqlx::query("SELECT body, github_author_login, github_comment_id FROM task_comments WHERE task_id = $1 ORDER BY created_at, id")
            .bind(task.to_string())
            .fetch_all(self.database.pool())
            .await
            .unwrap()
            .into_iter()
            .map(|row| (row.get("body"), row.get("github_author_login"), row.get("github_comment_id")))
            .collect()
    }

    /// Runs and removes the queued `github.comment` jobs, oldest first; returns how many ran.
    async fn run_jobs(&self, api: &GithubApi) -> usize {
        let rows = sqlx::query("SELECT id, payload_json FROM jobs WHERE kind = 'github.comment' ORDER BY created_at, id")
            .fetch_all(self.database.pool())
            .await
            .unwrap();
        for row in &rows {
            let payload: Value =
                serde_json::from_str(&row.get::<String, _>("payload_json")).unwrap();
            sync_note(&self.database, Some(KEY), api, &payload)
                .await
                .unwrap();
            sqlx::query("DELETE FROM jobs WHERE id = $1")
                .bind(row.get::<String, _>("id"))
                .execute(self.database.pool())
                .await
                .unwrap();
        }
        rows.len()
    }

    async fn note(&self, task: Id, text: &str) -> orbit_server::repositories::tasks::CommentRecord {
        self.tasks
            .create_comment(
                self.workspace_id,
                task,
                self.user_id,
                None,
                CommentContent::Markdown {
                    body: text.to_owned(),
                    mentioned_user_ids: Vec::new(),
                },
                "test",
                TimestampMillis::now(),
            )
            .await
            .unwrap()
    }
}

fn comment_event(action: &str, number: i64, id: i64, login: &str, body: &str, pull: bool) -> Value {
    let mut issue =
        json!({"number": number, "title": "Item", "state": "open", "labels": [{"name": "Orbit"}]});
    if pull {
        issue["pull_request"] = json!({"url": "https://api.github.com/repos/acme/repo/pulls/1"});
    }
    json!({"action": action, "installation": {"id": 1234}, "repository": {"full_name": "acme/repo"},
        "issue": issue, "sender": {"login": login},
        "comment": {"id": id, "body": body, "user": {"login": login, "avatar_url": "https://avatars.githubusercontent.com/u/583231?v=4"}}})
}

/// A stand-in for the GitHub API: records each call and answers like GitHub.
#[derive(Clone, Default)]
struct FakeGithub {
    calls: Arc<Mutex<Vec<(String, String, Value)>>>,
}

impl FakeGithub {
    async fn start(&self) -> GithubApi {
        let calls = Arc::clone(&self.calls);
        let router = axum::Router::new().fallback(
            move |method: Method, uri: Uri, headers: axum::http::HeaderMap, body: Bytes| {
                let calls = Arc::clone(&calls);
                async move {
                    let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
                    let auth = headers
                        .get("authorization")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("")
                        .to_owned();
                    let path = uri.path().to_owned();
                    calls
                        .lock()
                        .unwrap()
                        .push((method.to_string(), path.clone(), body));
                    if path.ends_with("/access_tokens") {
                        // the App JWT: header.claims.signature
                        assert_eq!(auth.trim_start_matches("Bearer ").split('.').count(), 3);
                        return (
                            StatusCode::CREATED,
                            axum::Json(json!({"token": "installation-token"})),
                        )
                            .into_response();
                    }
                    assert_eq!(auth, "Bearer installation-token");
                    match method {
                        Method::POST => {
                            (StatusCode::CREATED, axum::Json(json!({"id": 9001}))).into_response()
                        }
                        Method::DELETE => StatusCode::NO_CONTENT.into_response(),
                        _ => axum::Json(json!({})).into_response(),
                    }
                }
            },
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        GithubApi::new(&format!("http://{address}"))
    }

    /// The calls other than token requests.
    fn comment_calls(&self) -> Vec<(String, String, Value)> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, path, _)| !path.ends_with("/access_tokens"))
            .cloned()
            .collect()
    }
}

#[tokio::test]
async fn github_comments_become_notes_and_follow_edits_and_deletes() {
    let fixture = fixture().await;
    let task = fixture.linked("issue", 7).await;

    let created = comment_event(
        "created",
        7,
        1001,
        "octocat",
        "Hello **from** GitHub",
        false,
    );
    assert_eq!(
        fixture.send("issue_comment", created.clone()).await,
        StatusCode::OK
    );
    assert_eq!(
        fixture.notes(task).await,
        [(
            "Hello **from** GitHub".to_owned(),
            Some("octocat".to_owned()),
            Some(1001)
        )]
    );
    // The note reads as GitHub's: its login and avatar, the GitHub service account, not editable.
    let listed = fixture
        .tasks
        .comments(fixture.workspace_id, task, fixture.user_id, None, 50)
        .await
        .unwrap();
    let note = &listed.items[0];
    assert_eq!(note.github_author_login.as_deref(), Some("octocat"));
    assert_eq!(
        note.github_author_avatar_url.as_deref(),
        Some("https://avatars.githubusercontent.com/u/583231?v=4")
    );
    assert_eq!(note.author_service_account_name.as_deref(), Some("GitHub"));
    assert!(!note.can_edit);
    // A redelivery adds nothing.
    assert_eq!(
        fixture.send("issue_comment", created).await,
        StatusCode::ACCEPTED
    );
    assert_eq!(fixture.notes(task).await.len(), 1);

    let edited = comment_event("edited", 7, 1001, "octocat", "Hello again", false);
    assert_eq!(fixture.send("issue_comment", edited).await, StatusCode::OK);
    assert_eq!(fixture.notes(task).await[0].0, "Hello again");

    // A note from GitHub is never sent back.
    let queued: i64 = sqlx::query_scalar(
        "SELECT CAST(COUNT(*) AS BIGINT) FROM jobs WHERE kind = 'github.comment'",
    )
    .fetch_one(fixture.database.pool())
    .await
    .unwrap();
    assert_eq!(queued, 0);

    let deleted = comment_event("deleted", 7, 1001, "octocat", "Hello again", false);
    assert_eq!(fixture.send("issue_comment", deleted).await, StatusCode::OK);
    assert!(fixture.notes(task).await.is_empty());

    // Author fields are untrusted: an odd login is shown as "ghost", a foreign avatar URL is dropped.
    let mut odd = comment_event("created", 7, 1004, "x\"><img>", "Odd author", false);
    odd["comment"]["user"]["avatar_url"] = json!("https://evil.example/a.png");
    assert_eq!(fixture.send("issue_comment", odd).await, StatusCode::OK);
    let author: (Option<String>, Option<String>) = sqlx::query_as(
        "SELECT github_author_login, github_author_avatar_url FROM task_comments WHERE github_comment_id = 1004",
    )
    .fetch_one(fixture.database.pool())
    .await
    .unwrap();
    assert_eq!(author, (Some("ghost".to_owned()), None));

    // A comment GitHub names as made by the App is ignored too.
    let mut via_app = comment_event("created", 7, 1005, "someone", "Via the App", false);
    via_app["comment"]["performed_via_github_app"] = json!({"id": 77});
    assert_eq!(
        fixture.send("issue_comment", via_app).await,
        StatusCode::ACCEPTED
    );

    // An item Orbit does not sync gets no note.
    let unknown = comment_event("created", 99, 1003, "octocat", "Elsewhere", false);
    assert_eq!(
        fixture.send("issue_comment", unknown).await,
        StatusCode::ACCEPTED
    );
}

#[tokio::test]
async fn pull_request_conversation_comments_sync() {
    let fixture = fixture().await;
    let task = fixture.linked("pull_request", 8).await;
    let created = comment_event("created", 8, 2001, "octocat", "Looks good", true);
    assert_eq!(fixture.send("issue_comment", created).await, StatusCode::OK);
    assert_eq!(
        fixture.notes(task).await,
        [(
            "Looks good".to_owned(),
            Some("octocat".to_owned()),
            Some(2001)
        )]
    );
    // The same number as an issue comment does not match a pull request link.
    let as_issue = comment_event("created", 8, 2002, "octocat", "Wrong kind", false);
    assert_eq!(
        fixture.send("issue_comment", as_issue).await,
        StatusCode::ACCEPTED
    );
    assert_eq!(fixture.notes(task).await.len(), 1);
}

#[tokio::test]
async fn orbit_notes_are_posted_edited_and_deleted_on_github_without_echo() {
    let fixture = fixture().await;
    let github = FakeGithub::default();
    let api = github.start().await;
    let task = fixture.linked("issue", 7).await;

    // A member mention goes as code, so it notifies no GitHub user of that name.
    let note = fixture.note(task, "Fixed in **main**, thanks @Owner").await;
    assert_eq!(fixture.run_jobs(&api).await, 1);
    let calls = github.comment_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        (calls[0].0.as_str(), calls[0].1.as_str()),
        ("POST", "/repos/acme/repo/issues/7/comments")
    );
    assert_eq!(
        calls[0].2["body"],
        "Fixed in **main**, thanks `@Owner`\n\n— Owner via Orbit"
    );
    assert_eq!(fixture.notes(task).await[0].2, Some(9001));

    // GitHub's webhook for that comment (written by the App's bot) adds no copy.
    let echo = comment_event(
        "created",
        7,
        9001,
        "orbit-test[bot]",
        "Fixed in **main**\n\n— Owner via Orbit",
        false,
    );
    assert_eq!(
        fixture.send("issue_comment", echo).await,
        StatusCode::ACCEPTED
    );
    let echo_edit = comment_event("edited", 7, 9001, "orbit-test[bot]", "changed", false);
    assert_eq!(
        fixture.send("issue_comment", echo_edit).await,
        StatusCode::ACCEPTED
    );
    assert_eq!(fixture.notes(task).await.len(), 1);
    assert_eq!(
        fixture.notes(task).await[0].0,
        "Fixed in **main**, thanks @Owner"
    );

    let edited = fixture
        .tasks
        .update_comment(
            fixture.workspace_id,
            task,
            note.id,
            fixture.user_id,
            CommentContent::Markdown {
                body: "Fixed in **release**".to_owned(),
                mentioned_user_ids: Vec::new(),
            },
            note.version,
            "test",
            TimestampMillis::now(),
        )
        .await
        .unwrap();
    assert_eq!(fixture.run_jobs(&api).await, 1);
    let calls = github.comment_calls();
    assert_eq!(
        (calls[1].0.as_str(), calls[1].1.as_str()),
        ("PATCH", "/repos/acme/repo/issues/comments/9001")
    );
    assert_eq!(
        calls[1].2["body"],
        "Fixed in **release**\n\n— Owner via Orbit"
    );

    fixture
        .tasks
        .delete_comment(
            fixture.workspace_id,
            task,
            note.id,
            fixture.user_id,
            edited.version,
            "test",
            TimestampMillis::now(),
        )
        .await
        .unwrap();
    assert_eq!(fixture.run_jobs(&api).await, 1);
    let calls = github.comment_calls();
    assert_eq!(
        (calls[2].0.as_str(), calls[2].1.as_str()),
        ("DELETE", "/repos/acme/repo/issues/comments/9001")
    );

    // A note deleted before its job ran is never posted.
    let gone = fixture.note(task, "Short-lived").await;
    fixture
        .tasks
        .delete_comment(
            fixture.workspace_id,
            task,
            gone.id,
            fixture.user_id,
            0,
            "test",
            TimestampMillis::now(),
        )
        .await
        .unwrap();
    assert_eq!(fixture.run_jobs(&api).await, 2);
    assert_eq!(github.comment_calls().len(), 3);
}

#[tokio::test]
async fn paused_sync_moves_no_comments_either_way() {
    let fixture = fixture().await;
    let github = FakeGithub::default();
    let api = github.start().await;
    let task = fixture.linked("issue", 7).await;
    // Queued while the sync ran, sent after it was paused.
    fixture.note(task, "Before the pause").await;

    // The sync label is removed: the sync pauses.
    let unlabeled = json!({"action": "unlabeled", "installation": {"id": 1234}, "repository": {"full_name": "acme/repo"},
        "issue": {"number": 7, "title": "Item 7", "body": "", "state": "open", "labels": []}});
    assert_eq!(
        fixture.send("issues", unlabeled).await,
        StatusCode::ACCEPTED
    );

    assert_eq!(fixture.run_jobs(&api).await, 1);
    fixture.note(task, "During the pause").await;
    assert_eq!(fixture.run_jobs(&api).await, 0);
    assert!(github.comment_calls().is_empty());

    let created = comment_event("created", 7, 3001, "octocat", "From GitHub", false);
    assert_eq!(
        fixture.send("issue_comment", created).await,
        StatusCode::ACCEPTED
    );
    assert!(
        fixture
            .notes(task)
            .await
            .iter()
            .all(|(_, login, _)| login.is_none())
    );
}
