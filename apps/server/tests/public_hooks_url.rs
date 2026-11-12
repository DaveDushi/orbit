//! `ORBIT_PUBLIC_HOOKS_URL` is the origin of the incoming webhook URL and of the `response_url`
//! of a button click; without it, both use `ORBIT_PUBLIC_ORIGIN`.

use axum::Router;
use axum::routing::post;
use orbit_platform::{Config, EnvironmentMode, PasswordService, TimestampMillis};
use orbit_server::app::App;
use orbit_server::repositories::identity::{IdentityRepository, SetupRequest};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

const COOKIE: &str = "orbit_session_dev";

/// Makes a webhook, posts a button with it and clicks the button. Returns the webhook URL and
/// the `response_url` of the click.
async fn hook_urls(public_hooks_url: Option<&str>) -> (String, String) {
    let root = tempfile::tempdir().unwrap();
    let mut config = Config {
        environment: EnvironmentMode::Development,
        app_key: Some(orbit_platform::AppKey::new([1; 32])),
        ..Config::default()
    };
    config.http.public_hooks_url = public_hooks_url.map(str::to_owned);
    config.data.database = root.path().join("data/orbit.sqlite");
    config.data.attachments = root.path().join("data/attachments");
    config.data.backups = root.path().join("backups");
    let app = App::build(config).await.unwrap();
    let identity = IdentityRepository::new(app.database().clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let shutdown = CancellationToken::new();
    let token = shutdown.clone();
    let server = tokio::spawn(async move { app.serve(listener, token).await.unwrap() });

    // The integration: it keeps the payload of each click.
    let (clicks, mut received) = mpsc::unbounded_channel::<String>();
    let receiver = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let request_url = format!("http://{}/clicks", receiver.local_addr().unwrap());
    tokio::spawn(async move {
        let routes = Router::new().route(
            "/clicks",
            post(move |body: String| async move {
                clicks.send(body).unwrap();
            }),
        );
        axum::serve(receiver, routes).await.unwrap();
    });

    let now = TimestampMillis::now();
    identity
        .store_setup_token(
            "hooks",
            TimestampMillis::from_millis(now.as_millis() + 60_000),
        )
        .await
        .unwrap();
    let setup = identity
        .complete_setup(
            SetupRequest {
                token: "hooks".to_owned(),
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
    let http = reqwest::Client::new();
    let call = |method: &str, path: String, body: Option<Value>| {
        let mut request = http
            .request(method.parse().unwrap(), format!("{base}{path}"))
            .header("origin", &base)
            .header("cookie", format!("{COOKIE}={}", setup.session.token));
        if let Some(body) = body {
            request = request.json(&body);
        }
        async move {
            let response = request.send().await.unwrap();
            assert!(
                response.status().is_success(),
                "{path}: {}",
                response.status()
            );
            response.json::<Value>().await.unwrap_or(Value::Null)
        }
    };
    let workspace = format!("/api/v1/workspaces/{}", setup.workspace_id);
    let conversations = call("GET", format!("{workspace}/chat/conversations"), None).await;
    let general = conversations
        .as_array()
        .unwrap()
        .iter()
        .find(|conversation| conversation["name"] == "general")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let channel = format!("{workspace}/chat/conversations/{general}");
    let created = call(
        "POST",
        format!("{channel}/webhooks"),
        Some(json!({"name": "Deploys"})),
    )
    .await;
    let webhook_url = created["url"].as_str().unwrap().to_owned();
    let webhook_id = created["webhook"]["id"].as_str().unwrap();
    call(
        "PUT",
        format!("{channel}/webhooks/{webhook_id}/interactivity"),
        Some(json!({"request_url": request_url})),
    )
    .await;

    // The URL names the hooks origin; the post goes to this server.
    let path = &webhook_url[webhook_url.find("/api/v1/hooks/").unwrap()..];
    let posted = http
        .post(format!("{base}{path}"))
        .json(&json!({
            "text": "Deploy?",
            "blocks": [{
                "type": "actions",
                "block_id": "deploy",
                "elements": [{
                    "type": "button",
                    "action_id": "go",
                    "text": {"type": "plain_text", "text": "Go"},
                    "value": "go",
                }],
            }],
        }))
        .send()
        .await
        .unwrap();
    assert!(posted.status().is_success(), "{}", posted.status());
    let messages = call("GET", format!("{channel}/messages"), None).await;
    let message = messages["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|message| message["body"] == "Deploy?")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    call(
        "POST",
        format!("{workspace}/chat/messages/{message}/actions"),
        Some(json!({"block_id": "deploy", "action_id": "go"})),
    )
    .await;
    let body = received.recv().await.unwrap();
    let payload: Value = url::form_urlencoded::parse(body.as_bytes())
        .find(|(key, _)| key == "payload")
        .map(|(_, value)| serde_json::from_str(&value).unwrap())
        .unwrap();
    let response_url = payload["response_url"].as_str().unwrap().to_owned();

    shutdown.cancel();
    server.await.unwrap();
    (webhook_url, response_url)
}

#[tokio::test]
async fn the_hooks_url_names_the_webhook_and_the_response_url() {
    let (webhook, response) = hook_urls(Some("https://hooks.example/")).await;
    assert!(
        webhook.starts_with("https://hooks.example/api/v1/hooks/chat/"),
        "{webhook}"
    );
    assert!(
        response.starts_with("https://hooks.example/api/v1/hooks/chat/"),
        "{response}"
    );
    assert!(response.contains("/responses/"), "{response}");
}

#[tokio::test]
async fn without_the_hooks_url_both_use_the_public_origin() {
    let origin = Config::default().http.public_origin;
    let (webhook, response) = hook_urls(None).await;
    for url in [webhook, response] {
        assert!(
            url.starts_with(&format!("{origin}/api/v1/hooks/chat/")),
            "{url}"
        );
    }
}
