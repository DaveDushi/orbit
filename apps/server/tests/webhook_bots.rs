//! Webhook bots (step 10): owners and admins make bots; a mention in a bot's channel or a DM with it
//! is sent, signed, to the bot's URL; the reply comes in the HTTP answer or once through the reply
//! URL; without an answer in time the bot posts a note. Channel limits, guests, the rate limit,
//! bot loops and the address checks. A local HTTP server plays the bot. Runs on SQLite, and on
//! Postgres with `ORBIT_TEST_POSTGRES_URL`.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::{Body, Bytes, to_bytes};
use axum::extract::State;
use axum::http::{HeaderMap, Request, StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::post;
use orbit_platform::{
    AttachmentMutationCoordinator, AuthenticatedUser, Id, LocalBlobStore, PasswordService, Row,
    TestDatabase, TimestampMillis, UploadLimits, UploadService,
};
use orbit_server::auth_routes::CookieMode;
use orbit_server::chat_routes::{ChatState, chat_router};
use orbit_server::repositories::identity::{IdentityRepository, SetupRequest};
use orbit_server::webhook_bots::{self, BotState};
use serde_json::{Value, json};
use tower::ServiceExt;

/// What the bot server got: the path, the headers it checks and the raw body.
#[derive(Clone, Debug)]
struct Received {
    path: String,
    signature: String,
    timestamp: String,
    delivery: String,
    body: Bytes,
}

#[derive(Clone, Default)]
struct BotServer {
    received: Arc<Mutex<Vec<Received>>>,
}

impl BotServer {
    async fn start() -> (Self, String) {
        let server = Self::default();
        let app = axum::Router::new()
            .route("/sync", post(answer))
            .route("/async", post(answer))
            .route("/slow", post(answer))
            .route("/silent", post(answer))
            .route("/error", post(answer))
            .route("/redirect", post(answer))
            .with_state(server.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (server, format!("127.0.0.1:{}", address.port()))
    }

    fn received(&self) -> Vec<Received> {
        self.received.lock().unwrap().clone()
    }
}

async fn answer(
    State(server): State<BotServer>,
    uri: axum::http::Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let header = |name: &str| {
        headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned()
    };
    server.received.lock().unwrap().push(Received {
        path: uri.path().to_owned(),
        signature: header("x-orbit-signature"),
        timestamp: header("x-orbit-timestamp"),
        delivery: header("x-orbit-delivery"),
        body: body.clone(),
    });
    let payload: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    match uri.path() {
        "/sync" => axum::Json(json!({
            "text": format!("Echo: {} <!channel>", payload["message"]["text"].as_str().unwrap_or_default())
        }))
        .into_response(),
        "/async" => StatusCode::ACCEPTED.into_response(),
        "/slow" => {
            tokio::time::sleep(Duration::from_secs(4)).await;
            axum::Json(json!({ "text": "too late" })).into_response()
        }
        "/silent" => axum::Json(json!({})).into_response(),
        "/redirect" => Redirect::temporary("/sync").into_response(),
        _ => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

struct Fixture {
    _root: tempfile::TempDir,
    database: TestDatabase,
    identity: Arc<IdentityRepository>,
    bots: BotState,
    app: axum::Router,
    workspace_id: String,
    owner: Member,
    nonce: AtomicU64,
    server: BotServer,
    host: String,
}

#[derive(Clone)]
struct Member {
    id: Id,
    cookie: String,
}

impl Fixture {
    async fn new() -> Self {
        Self::with_window(webhook_bots::REPLY_WINDOW).await
    }

    async fn with_window(window: Duration) -> Self {
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
        let bots = BotState::new(chat.clone(), Some([5; 32]), "https://orbit.test")
            .with_test_hosts(&["127.0.0.1"])
            .with_reply_window(window);
        let app = chat_router(chat).merge(webhook_bots::router(bots.clone()));
        let (server, host) = BotServer::start().await;
        Self {
            _root: root,
            database,
            identity,
            bots,
            app,
            workspace_id: setup.workspace_id.to_string(),
            owner: Member {
                id: setup.user_id,
                cookie: format!("__Host-orbit_session={}", setup.session.token),
            },
            nonce: AtomicU64::new(0),
            server,
            host,
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
        cookie: Option<&str>,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut builder = Request::builder().method(method).uri(uri);
        if let Some(cookie) = cookie {
            builder = builder.header("cookie", cookie);
        }
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

    async fn api(
        &self,
        member: &Member,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let uri = format!("/api/v1/workspaces/{}{path}", self.workspace_id);
        self.request(method, &uri, Some(&member.cookie), body).await
    }

    async fn chat(&self, member: &Member, method: &str, path: &str, body: Option<Value>) -> Value {
        let (status, value) = self
            .api(member, method, &format!("/chat{path}"), body)
            .await;
        assert_eq!(status, StatusCode::OK, "{method} {path}: {value}");
        value
    }

    async fn general(&self) -> String {
        self.chat(&self.owner, "GET", "/conversations", None)
            .await
            .as_array()
            .unwrap()
            .iter()
            .find(|conversation| conversation["is_default"] == true)
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    async fn channel(&self, name: &str, kind: &str) -> String {
        self.chat(
            &self.owner,
            "POST",
            "/conversations",
            Some(json!({ "name": name, "kind": kind, "member_ids": [] })),
        )
        .await["result"]["id"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    async fn send(&self, member: &Member, conversation: &str, body: &str) -> String {
        let nonce = format!("n{}", self.nonce.fetch_add(1, Ordering::Relaxed));
        self.chat(
            member,
            "POST",
            &format!("/conversations/{conversation}/messages"),
            Some(json!({ "body": body, "nonce": nonce })),
        )
        .await["result"]["id"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    /// Makes a bot whose URL is `path` on the bot server; returns it and its signing secret.
    async fn create_bot(&self, name: &str, path: &str, channels: &[&str]) -> (Value, String) {
        let (status, created) = self
            .api(
                &self.owner,
                "POST",
                "/bots",
                Some(json!({ "name": name, "url": format!("http://{}{path}", self.host), "channel_ids": channels })),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{created}");
        (
            created["bot"].clone(),
            created["signing_secret"].as_str().unwrap().to_owned(),
        )
    }

    /// The thread replies under `root`, as `member` reads them.
    async fn thread(&self, member: &Member, root: &str) -> Vec<Value> {
        let thread = self
            .chat(member, "GET", &format!("/threads/{root}"), None)
            .await;
        thread["items"].as_array().cloned().unwrap()
    }

    async fn deliveries(&self) -> Vec<(String, Option<i64>, Option<String>)> {
        sqlx::query(
            "SELECT status, http_status, error FROM webhook_bot_deliveries ORDER BY created_at, id",
        )
        .fetch_all(self.database.pool())
        .await
        .unwrap()
        .iter()
        .map(|row| (row.get("status"), row.get("http_status"), row.get("error")))
        .collect()
    }

    async fn reply(&self, url: &str, body: Value) -> (StatusCode, Value) {
        let path = url.strip_prefix("https://orbit.test").unwrap();
        self.request("POST", path, None, Some(body)).await
    }
}

fn user_of(bot: &Value) -> &str {
    bot["user_id"].as_str().unwrap()
}

fn bodies_by(messages: &[Value], author: &str) -> Vec<String> {
    messages
        .iter()
        .filter(|message| message["author_id"] == author)
        .map(|message| message["body"].as_str().unwrap_or_default().to_owned())
        .collect()
}

#[tokio::test]
async fn only_owners_and_admins_manage_bots_and_the_secret_shows_once() {
    let fixture = Fixture::new().await;
    let member = fixture.add_person("Mia", "member").await;
    let guest = fixture.add_person("Gus", "guest").await;
    let general = fixture.general().await;
    let (bot, secret) = fixture.create_bot("Echo", "/sync", &[&general]).await;
    assert!(secret.starts_with("obs_") && secret.len() > 40);
    assert_eq!(bot["channels"][0]["id"], general.as_str());

    for (who, expected) in [
        (&member, StatusCode::FORBIDDEN),
        (&guest, StatusCode::NOT_FOUND),
    ] {
        let (status, _) = fixture.api(who, "GET", "/bots", None).await;
        assert_eq!(status, expected);
        let (status, _) = fixture
            .api(
                who,
                "POST",
                "/bots",
                Some(json!({ "name": "Mine", "url": "https://bot.example.com" })),
            )
            .await;
        assert_eq!(status, expected);
        let (status, _) = fixture
            .api(
                who,
                "POST",
                &format!("/bots/{}/secret", bot["id"].as_str().unwrap()),
                None,
            )
            .await;
        assert_eq!(status, expected);
    }

    // The list never has the secret, and the database keeps it encrypted.
    let (status, list) = fixture.api(&fixture.owner, "GET", "/bots", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!list.to_string().contains(&secret));
    let stored: Vec<u8> = sqlx::query_scalar("SELECT signing_secret FROM webhook_bots")
        .fetch_one(fixture.database.pool())
        .await
        .unwrap();
    assert!(!String::from_utf8_lossy(&stored).contains(&secret[4..]));

    let (status, rotated) = fixture
        .api(
            &fixture.owner,
            "POST",
            &format!("/bots/{}/secret", bot["id"].as_str().unwrap()),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let rotated = rotated["signing_secret"].as_str().unwrap();
    assert_ne!(rotated, secret);

    // The new secret signs the next delivery; the old one no longer verifies it.
    fixture
        .send(&member, &general, &format!("<@{}> hi", user_of(&bot)))
        .await;
    webhook_bots::dispatch(&fixture.bots).await.unwrap();
    let got = fixture.server.received().pop().unwrap();
    let now = TimestampMillis::now().as_millis() / 1000;
    assert!(webhook_bots::verify(rotated, &got.timestamp, &got.body, &got.signature, now).is_ok());
    assert!(webhook_bots::verify(&secret, &got.timestamp, &got.body, &got.signature, now).is_err());

    // A taken name is refused; a bot's name is free again after it is deleted.
    let (status, _) = fixture
        .api(
            &fixture.owner,
            "POST",
            "/bots",
            Some(json!({ "name": "mia", "url": "https://bot.example.com" })),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, _) = fixture
        .api(
            &fixture.owner,
            "DELETE",
            &format!("/bots/{}", bot["id"].as_str().unwrap()),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, list) = fixture.api(&fixture.owner, "GET", "/bots", None).await;
    assert_eq!(list["bots"].as_array().unwrap().len(), 0);
    fixture.create_bot("Echo", "/sync", &[]).await;
}

#[tokio::test]
async fn a_mention_is_sent_signed_and_the_answer_in_the_body_is_posted_as_the_bot() {
    let fixture = Fixture::new().await;
    let member = fixture.add_person("Mia", "member").await;
    let general = fixture.general().await;
    let (bot, secret) = fixture.create_bot("Echo", "/sync", &[&general]).await;
    let bot_user = user_of(&bot);
    let text = format!("<@{bot_user}> what is the status?");
    let message = fixture.send(&member, &general, &text).await;
    // A message without a mention reaches no bot.
    fixture.send(&member, &general, "just talking").await;

    assert_eq!(webhook_bots::dispatch(&fixture.bots).await.unwrap(), 1);
    let received = fixture.server.received();
    assert_eq!(received.len(), 1);
    let got = &received[0];
    let now = TimestampMillis::now().as_millis() / 1000;
    assert_eq!(
        webhook_bots::verify(&secret, &got.timestamp, &got.body, &got.signature, now),
        Ok(())
    );
    assert!(webhook_bots::verify(&secret, &got.timestamp, b"{}", &got.signature, now).is_err());
    let payload: Value = serde_json::from_slice(&got.body).unwrap();
    assert_eq!(payload["type"], "message");
    assert_eq!(payload["delivery_id"], got.delivery.as_str());
    assert_eq!(payload["workspace"]["id"], fixture.workspace_id.as_str());
    assert_eq!(payload["workspace"]["name"], "Orbit");
    assert_eq!(payload["conversation"]["id"], general.as_str());
    assert_eq!(payload["conversation"]["kind"], "public");
    assert_eq!(payload["thread_id"], message.as_str());
    assert_eq!(payload["message"]["id"], message.as_str());
    assert_eq!(payload["message"]["text"], text.as_str());
    assert_eq!(payload["author"]["id"], member.id.to_string());
    assert_eq!(payload["author"]["name"], "Mia");
    assert_eq!(payload["bot"]["name"], "Echo");
    let reply_url = payload["reply"]["url"].as_str().unwrap();
    assert!(reply_url.starts_with(&format!(
        "https://orbit.test/api/v1/hooks/bots/{}/replies/",
        bot["id"].as_str().unwrap()
    )));
    assert!(reply_url.ends_with(payload["reply"]["token"].as_str().unwrap()));
    // Never the secret.
    assert!(!String::from_utf8_lossy(&got.body).contains(&secret));

    // The answer is a plain message of the bot in the thread; it cannot notify the whole channel.
    let replies = fixture.thread(&member, &message).await;
    assert_eq!(
        bodies_by(&replies, bot_user),
        vec![format!("Echo: {text} @channel")]
    );
    assert_eq!(
        fixture.deliveries().await,
        vec![("answered".to_owned(), Some(200), None)]
    );

    // The bot's own message mentions it, but a bot's message never reaches a bot: no loop.
    assert_eq!(webhook_bots::dispatch(&fixture.bots).await.unwrap(), 0);
    assert_eq!(fixture.deliveries().await.len(), 1);

    // The reply URL of an answered delivery is used up.
    let (status, _) = fixture.reply(reply_url, json!({ "text": "again" })).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_dm_with_the_bot_reaches_it_without_a_mention_and_the_answer_stays_in_the_dm() {
    let fixture = Fixture::new().await;
    let member = fixture.add_person("Mia", "member").await;
    let (bot, _) = fixture.create_bot("Echo", "/sync", &[]).await;
    let dm = fixture
        .chat(
            &member,
            "POST",
            "/dms",
            Some(json!({ "user_ids": [user_of(&bot)] })),
        )
        .await["result"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    fixture.send(&member, &dm, "hello").await;
    webhook_bots::dispatch(&fixture.bots).await.unwrap();
    let payload: Value = serde_json::from_slice(&fixture.server.received()[0].body).unwrap();
    assert_eq!(payload["conversation"]["kind"], "dm");
    assert_eq!(payload["thread_id"], Value::Null);
    let messages = fixture
        .chat(
            &member,
            "GET",
            &format!("/conversations/{dm}/messages"),
            None,
        )
        .await;
    let messages = messages["items"].as_array().cloned().unwrap();
    assert_eq!(
        bodies_by(&messages, user_of(&bot)),
        vec!["Echo: hello @channel".to_owned()]
    );
}

#[tokio::test]
async fn an_answer_through_the_reply_url_is_posted_once() {
    let fixture = Fixture::new().await;
    let member = fixture.add_person("Mia", "member").await;
    let general = fixture.general().await;
    let (bot, _) = fixture.create_bot("Later", "/async", &[&general]).await;
    let (other, _) = fixture.create_bot("Other", "/async", &[&general]).await;
    let message = fixture
        .send(&member, &general, &format!("<@{}> report", user_of(&bot)))
        .await;
    webhook_bots::dispatch(&fixture.bots).await.unwrap();
    assert_eq!(
        fixture.deliveries().await,
        vec![("sending".to_owned(), Some(202), None)]
    );
    let payload: Value = serde_json::from_slice(&fixture.server.received()[0].body).unwrap();
    let url = payload["reply"]["url"].as_str().unwrap().to_owned();
    let token = payload["reply"]["token"].as_str().unwrap();

    // Another bot's path, a wrong token and a bad body do not use the token.
    let wrong_bot = url.replace(bot["id"].as_str().unwrap(), other["id"].as_str().unwrap());
    assert_eq!(
        fixture.reply(&wrong_bot, json!({ "text": "x" })).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        fixture
            .reply(&url.replace(token, "nope"), json!({ "text": "x" }))
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        fixture.reply(&url, json!({ "words": "x" })).await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        fixture.reply(&url, json!({ "text": "  " })).await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );

    let (status, posted) = fixture
        .reply(&url, json!({ "text": "**Report:** all green" }))
        .await;
    assert_eq!(status, StatusCode::OK, "{posted}");
    let replies = fixture.thread(&member, &message).await;
    assert_eq!(
        bodies_by(&replies, user_of(&bot)),
        vec!["**Report:** all green".to_owned()]
    );
    assert_eq!(fixture.deliveries().await[0].0, "answered");

    // Single use.
    let (status, _) = fixture.reply(&url, json!({ "text": "twice" })).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(fixture.thread(&member, &message).await.len(), 1);
}

#[tokio::test]
async fn without_an_answer_in_time_the_bot_posts_a_note() {
    let fixture = Fixture::with_window(Duration::from_millis(1_500)).await;
    let member = fixture.add_person("Mia", "member").await;
    let general = fixture.general().await;
    let (slow, _) = fixture.create_bot("Slow", "/slow", &[&general]).await;
    let (silent, _) = fixture.create_bot("Silent", "/silent", &[&general]).await;
    let (broken, _) = fixture.create_bot("Broken", "/error", &[&general]).await;
    let slow_message = fixture
        .send(&member, &general, &format!("<@{}> hi", user_of(&slow)))
        .await;
    let silent_message = fixture
        .send(&member, &general, &format!("<@{}> hi", user_of(&silent)))
        .await;
    let broken_message = fixture
        .send(&member, &general, &format!("<@{}> hi", user_of(&broken)))
        .await;

    let started = std::time::Instant::now();
    webhook_bots::dispatch(&fixture.bots).await.unwrap();
    // The slow bot's request is cut at the deadline, not after its 4 seconds.
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "{:?}",
        started.elapsed()
    );
    let note = |messages: &[Value], bot: &Value| bodies_by(messages, user_of(bot));
    assert_eq!(
        note(&fixture.thread(&member, &slow_message).await, &slow),
        vec!["_No answer: the bot did not reply within 30 seconds._".to_owned()]
    );
    assert_eq!(
        note(&fixture.thread(&member, &broken_message).await, &broken),
        vec!["_No answer: the bot could not be reached._".to_owned()]
    );
    // The silent bot took the message: its note comes at the deadline.
    assert!(fixture.thread(&member, &silent_message).await.is_empty());
    tokio::time::sleep(Duration::from_millis(1_600)).await;
    assert_eq!(webhook_bots::sweep(&fixture.bots).await.unwrap(), 1);
    assert_eq!(webhook_bots::sweep(&fixture.bots).await.unwrap(), 0);
    assert_eq!(
        note(&fixture.thread(&member, &silent_message).await, &silent),
        vec!["_No answer: the bot did not reply within 30 seconds._".to_owned()]
    );
    // Its reply URL expired with it.
    let payload = fixture
        .server
        .received()
        .into_iter()
        .find(|got| got.path == "/silent")
        .map(|got| serde_json::from_slice::<Value>(&got.body).unwrap())
        .unwrap();
    let (status, _) = fixture
        .reply(
            payload["reply"]["url"].as_str().unwrap(),
            json!({ "text": "late" }),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let mut states = fixture.deliveries().await;
    states.sort();
    assert_eq!(
        states,
        vec![
            (
                "failed".to_owned(),
                Some(500),
                Some("http_status".to_owned())
            ),
            ("late".to_owned(), None, Some("timeout".to_owned())),
            ("late".to_owned(), Some(200), Some("timeout".to_owned())),
        ]
    );
}

#[tokio::test]
async fn a_bot_is_not_reached_outside_its_channels_or_by_guests() {
    let fixture = Fixture::new().await;
    let member = fixture.add_person("Mia", "member").await;
    let guest = fixture.add_person("Gus", "guest").await;
    let general = fixture.general().await;
    let bots_channel = fixture.channel("bots", "public").await;
    let clients = fixture.channel("clients", "private").await;
    fixture
        .chat(
            &fixture.owner,
            "POST",
            &format!("/conversations/{clients}/members"),
            Some(json!({ "user_ids": [guest.id.to_string(), member.id.to_string()] })),
        )
        .await;
    let (bot, _) = fixture
        .create_bot("Echo", "/sync", &[&bots_channel, &clients])
        .await;
    let bot_user = user_of(&bot);

    // In #general, not one of its channels: nothing is sent; the bot tells the author in a DM.
    fixture
        .send(&member, &general, &format!("<@{bot_user}> help"))
        .await;
    // A guest's mention in a channel of the bot reaches nothing.
    fixture
        .send(&guest, &clients, &format!("<@{bot_user}> help"))
        .await;
    webhook_bots::dispatch(&fixture.bots).await.unwrap();
    assert!(fixture.server.received().is_empty());
    assert_eq!(
        fixture.deliveries().await,
        vec![("refused".to_owned(), None, None)]
    );
    let conversations = fixture.chat(&member, "GET", "/conversations", None).await;
    let dm = conversations
        .as_array()
        .unwrap()
        .iter()
        .find(|conversation| {
            conversation["kind"] == "dm"
                && conversation["member_ids"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|id| id == bot_user)
        })
        .expect("the refusal DM")["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let messages = fixture
        .chat(
            &member,
            "GET",
            &format!("/conversations/{dm}/messages"),
            None,
        )
        .await;
    let refusal = bodies_by(messages["items"].as_array().unwrap(), bot_user);
    assert_eq!(refusal.len(), 1);
    assert!(
        refusal[0].starts_with(
            "I can't answer in #general: I work only in #bots, #clients and in direct messages."
        ),
        "{}",
        refusal[0]
    );
    // The refusal is sent once.
    webhook_bots::dispatch(&fixture.bots).await.unwrap();
    let messages = fixture
        .chat(
            &member,
            "GET",
            &format!("/conversations/{dm}/messages"),
            None,
        )
        .await;
    assert_eq!(
        bodies_by(messages["items"].as_array().unwrap(), bot_user).len(),
        1
    );

    // A member's mention in the same private channel works, and the guest there reads the answer.
    let asked = fixture
        .send(&member, &clients, &format!("<@{bot_user}> status"))
        .await;
    webhook_bots::dispatch(&fixture.bots).await.unwrap();
    assert_eq!(fixture.server.received().len(), 1);
    assert_eq!(
        bodies_by(&fixture.thread(&guest, &asked).await, bot_user).len(),
        1
    );

    // A guest cannot open a DM with the bot to reach it either.
    let (status, _) = fixture
        .api(
            &guest,
            "POST",
            "/chat/dms",
            Some(json!({ "user_ids": [bot_user] })),
        )
        .await;
    if status == StatusCode::OK {
        let dm = fixture
            .chat(
                &guest,
                "POST",
                "/dms",
                Some(json!({ "user_ids": [bot_user] })),
            )
            .await["result"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        fixture.send(&guest, &dm, "hello").await;
        webhook_bots::dispatch(&fixture.bots).await.unwrap();
    }
    assert_eq!(fixture.server.received().len(), 1);
}

#[tokio::test]
async fn a_bot_gets_at_most_30_messages_a_minute() {
    let fixture = Fixture::new().await;
    let member = fixture.add_person("Mia", "member").await;
    let general = fixture.general().await;
    let (bot, _) = fixture.create_bot("Later", "/async", &[&general]).await;
    for _ in 0..32 {
        fixture
            .send(&member, &general, &format!("<@{}> ping", user_of(&bot)))
            .await;
    }
    let states: Vec<String> = fixture
        .deliveries()
        .await
        .into_iter()
        .map(|(status, _, _)| status)
        .collect();
    assert_eq!(
        states.iter().filter(|status| *status == "pending").count(),
        30
    );
    assert_eq!(
        states.iter().filter(|status| *status == "limited").count(),
        2
    );
    webhook_bots::dispatch(&fixture.bots).await.unwrap();
    assert_eq!(fixture.server.received().len(), 30);
    // One note for the minute, not one per message.
    let notes: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM chat_messages WHERE author_id = $1 AND body LIKE '%too many messages%'",
    )
    .bind(user_of(&bot))
    .fetch_one(fixture.database.pool())
    .await
    .unwrap();
    assert_eq!(notes, 1);

    // After 30 days finished deliveries go; the 30 that still wait for an answer stay.
    sqlx::query("UPDATE webhook_bot_deliveries SET created_at = 0")
        .execute(fixture.database.pool())
        .await
        .unwrap();
    assert_eq!(webhook_bots::forget_old(&fixture.bots).await.unwrap(), 2);
    assert_eq!(fixture.deliveries().await.len(), 30);
}

#[tokio::test]
async fn bots_reach_only_public_https_addresses() {
    let fixture = Fixture::new().await;
    let member = fixture.add_person("Mia", "member").await;
    let general = fixture.general().await;
    // Refused when saved: plain http, local and private IPs (IPv4, IPv6 and mapped forms), user info.
    for url in [
        "http://bot.example.com/",
        "https://127.0.0.2/",
        "https://10.0.0.5/",
        "https://192.168.1.10/",
        "https://169.254.169.254/latest/meta-data",
        "https://[::1]/",
        "https://[fd00::1]/",
        "https://[fe80::1]/",
        "https://[::ffff:127.0.0.1]/",
        "https://user:pass@bot.example.com/",
        "ftp://bot.example.com/",
        "not a url",
    ] {
        let (status, body) = fixture
            .api(
                &fixture.owner,
                "POST",
                "/bots",
                Some(json!({ "name": "Bad", "url": url })),
            )
            .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{url}: {body}");
    }

    // A host name is checked after DNS, at each call: `localhost` resolves to a loopback address.
    let port = fixture.host.rsplit(':').next().unwrap();
    let (status, created) = fixture
        .api(&fixture.owner, "POST", "/bots", Some(json!({ "name": "Rebind", "url": format!("https://localhost:{port}/sync"), "channel_ids": [general] })))
        .await;
    assert_eq!(status, StatusCode::OK, "{created}");
    let rebind = created["bot"].clone();
    // A redirect is not followed, also to the same host.
    let (redirect, _) = fixture.create_bot("Hop", "/redirect", &[&general]).await;

    let first = fixture
        .send(&member, &general, &format!("<@{}> hi", user_of(&rebind)))
        .await;
    let second = fixture
        .send(&member, &general, &format!("<@{}> hi", user_of(&redirect)))
        .await;
    webhook_bots::dispatch(&fixture.bots).await.unwrap();
    let paths: Vec<String> = fixture
        .server
        .received()
        .into_iter()
        .map(|got| got.path)
        .collect();
    assert_eq!(paths, vec!["/redirect".to_owned()]);
    let mut states = fixture.deliveries().await;
    states.sort();
    assert_eq!(
        states,
        vec![
            (
                "failed".to_owned(),
                None,
                Some("blocked_address".to_owned())
            ),
            (
                "failed".to_owned(),
                Some(307),
                Some("http_status".to_owned())
            ),
        ]
    );
    for (root, bot) in [(&first, &rebind), (&second, &redirect)] {
        assert_eq!(
            bodies_by(&fixture.thread(&member, root).await, user_of(bot)),
            vec!["_No answer: the bot could not be reached._".to_owned()]
        );
    }
}

#[tokio::test]
async fn a_deleted_bot_is_not_reached_and_its_open_deliveries_end() {
    let fixture = Fixture::new().await;
    let member = fixture.add_person("Mia", "member").await;
    let general = fixture.general().await;
    let (bot, _) = fixture.create_bot("Echo", "/sync", &[&general]).await;
    fixture
        .send(&member, &general, &format!("<@{}> hi", user_of(&bot)))
        .await;
    let (status, _) = fixture
        .api(
            &fixture.owner,
            "DELETE",
            &format!("/bots/{}", bot["id"].as_str().unwrap()),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    fixture
        .send(&member, &general, &format!("<@{}> hi", user_of(&bot)))
        .await;
    assert_eq!(webhook_bots::dispatch(&fixture.bots).await.unwrap(), 0);
    assert!(fixture.server.received().is_empty());
    assert_eq!(
        fixture.deliveries().await,
        vec![("failed".to_owned(), None, Some("bot_deleted".to_owned()))]
    );
}
