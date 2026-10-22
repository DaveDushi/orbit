//! Orbit Cloud billing over real HTTP: the billing routes, the Stripe webhook → sync job → stored
//! subscription path against a local Stripe stand-in, and the read-only guard.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Form, Json, Router};
use orbit_platform::{
    AuthenticatedUser, BillingConfig, Config, Database, EnvironmentMode, Id, PasswordService, Plan,
    Secret, TimestampMillis,
};
use orbit_server::app::App;
use orbit_server::billing::stripe::{API_VERSION, sign};
use orbit_server::repositories::identity::{IdentityRepository, SetupRequest};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

const COOKIE: &str = "orbit_session_dev";
const WEBHOOK_SECRET: &str = "whsec_billing_test";
const SECRET_KEY: &str = "sk_test_billing";

/// A Stripe stand-in: records requests, serves subscriptions the test sets.
#[derive(Clone, Default)]
struct FakeStripe {
    requests: Arc<Mutex<Vec<StripeRequest>>>,
    subscriptions: Arc<Mutex<HashMap<String, Value>>>,
}

#[derive(Clone, Debug)]
struct StripeRequest {
    path: String,
    form: HashMap<String, String>,
    idempotency_key: Option<String>,
}

impl FakeStripe {
    fn record(&self, path: &str, headers: &HeaderMap, form: HashMap<String, String>) {
        assert_eq!(
            headers["authorization"].to_str().unwrap(),
            format!("Bearer {SECRET_KEY}")
        );
        assert_eq!(headers["stripe-version"].to_str().unwrap(), API_VERSION);
        self.requests.lock().unwrap().push(StripeRequest {
            path: path.to_owned(),
            form,
            idempotency_key: headers
                .get("idempotency-key")
                .map(|value| value.to_str().unwrap().to_owned()),
        });
    }

    fn requests(&self, path: &str) -> Vec<StripeRequest> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|request| request.path == path)
            .cloned()
            .collect()
    }

    fn set_subscription(&self, id: &str, workspace_id: &str, status: &str, price: &str) {
        self.subscriptions.lock().unwrap().insert(
            id.to_owned(),
            json!({
                "id": id,
                "object": "subscription",
                "customer": "cus_test",
                "status": status,
                "cancel_at_period_end": false,
                "cancel_at": null,
                "metadata": {"workspace_id": workspace_id},
                "items": {"object": "list", "data": [
                    {"id": "si_1", "price": {"id": price}, "current_period_end": 1_900_000_000}
                ]},
            }),
        );
    }

    async fn start(&self) -> String {
        let router = Router::new()
            .route(
                "/v1/customers",
                post(
                    |State(stripe): State<FakeStripe>,
                     headers: HeaderMap,
                     Form(form): Form<HashMap<String, String>>| async move {
                        stripe.record("/v1/customers", &headers, form);
                        Json(json!({"id": "cus_test", "object": "customer"}))
                    },
                ),
            )
            .route(
                "/v1/checkout/sessions",
                post(
                    |State(stripe): State<FakeStripe>,
                     headers: HeaderMap,
                     Form(form): Form<HashMap<String, String>>| async move {
                        stripe.record("/v1/checkout/sessions", &headers, form);
                        Json(
                            json!({"id": "cs_test", "url": "https://checkout.stripe.test/cs_test"}),
                        )
                    },
                ),
            )
            .route(
                "/v1/billing_portal/sessions",
                post(
                    |State(stripe): State<FakeStripe>,
                     headers: HeaderMap,
                     Form(form): Form<HashMap<String, String>>| async move {
                        stripe.record("/v1/billing_portal/sessions", &headers, form);
                        Json(json!({"id": "bps_test", "url": "https://billing.stripe.test/p"}))
                    },
                ),
            )
            .route(
                "/v1/subscriptions/{id}",
                get(
                    |State(stripe): State<FakeStripe>,
                     headers: HeaderMap,
                     Path(id): Path<String>| async move {
                        stripe.record("/v1/subscriptions", &headers, HashMap::new());
                        match stripe.subscriptions.lock().unwrap().get(&id) {
                            Some(subscription) => (StatusCode::OK, Json(subscription.clone())),
                            None => (
                                StatusCode::NOT_FOUND,
                                Json(json!({"error": {"message": "No such subscription"}})),
                            ),
                        }
                    },
                ),
            )
            .with_state(self.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        base
    }
}

struct Server {
    base: String,
    database: Database,
    identity: IdentityRepository,
    shutdown: CancellationToken,
    task: tokio::task::JoinHandle<()>,
    http: reqwest::Client,
    session: String,
    workspace_id: String,
    _root: TempDir,
}

impl Server {
    async fn start(stripe_base: Option<&str>) -> Self {
        let root = tempfile::tempdir().unwrap();
        let mut config = Config {
            environment: EnvironmentMode::Development,
            app_key: Some(orbit_platform::AppKey::new([1; 32])),
            ..Config::default()
        };
        config.data.database = root.path().join("data/orbit.sqlite");
        config.data.attachments = root.path().join("data/attachments");
        config.data.backups = root.path().join("backups");
        config.billing = stripe_base.map(|base| BillingConfig {
            api_base: base.parse().unwrap(),
            ..BillingConfig::new(
                Secret::new(SECRET_KEY),
                Secret::new(WEBHOOK_SECRET),
                vec![
                    Plan {
                        key: "team".to_owned(),
                        price_id: "price_team".to_owned(),
                    },
                    Plan {
                        key: "business".to_owned(),
                        price_id: "price_business".to_owned(),
                    },
                ],
            )
        });
        let app = App::build(config).await.unwrap();
        let database = app.database().clone();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let shutdown = CancellationToken::new();
        let token = shutdown.clone();
        let task = tokio::spawn(async move { app.serve(listener, token).await.unwrap() });
        let identity = IdentityRepository::new(database.clone());
        let now = TimestampMillis::now();
        identity
            .store_setup_token(
                "billing-setup",
                TimestampMillis::from_millis(now.as_millis() + 60_000),
            )
            .await
            .unwrap();
        let setup = identity
            .complete_setup(
                SetupRequest {
                    token: "billing-setup".to_owned(),
                    email: "owner@example.com".to_owned(),
                    display_name: "Owner".to_owned(),
                    password_hash: PasswordService::default()
                        .hash("correct horse battery")
                        .unwrap(),
                    workspace_name: "Acme".to_owned(),
                    project_name: "General".to_owned(),
                },
                now,
            )
            .await
            .unwrap();
        Self {
            base,
            database,
            identity,
            shutdown,
            task,
            http: reqwest::Client::new(),
            session: setup.session.token,
            workspace_id: setup.workspace_id.to_string(),
            _root: root,
        }
    }

    async fn stop(self) {
        self.shutdown.cancel();
        self.task.await.unwrap();
    }

    async fn call_as(
        &self,
        session: &str,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> (u16, Value) {
        let mut request = self
            .http
            .request(method.parse().unwrap(), format!("{}{path}", self.base))
            .header("origin", &self.base)
            .header("cookie", format!("{COOKIE}={session}"));
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request.send().await.unwrap();
        let status = response.status().as_u16();
        (status, response.json().await.unwrap_or(Value::Null))
    }

    async fn call(&self, method: &str, path: &str, body: Option<Value>) -> (u16, Value) {
        self.call_as(&self.session, method, path, body).await
    }

    fn workspace(&self, rest: &str) -> String {
        format!("/api/v1/workspaces/{}{rest}", self.workspace_id)
    }

    async fn create_task(&self, title: &str) -> (u16, Value) {
        let project: String = sqlx::query_scalar("SELECT id FROM projects WHERE workspace_id = $1")
            .bind(&self.workspace_id)
            .fetch_one(self.database.pool())
            .await
            .unwrap();
        let status: String = sqlx::query_scalar(
            "SELECT id FROM task_statuses WHERE project_id = $1 ORDER BY position LIMIT 1",
        )
        .bind(&project)
        .fetch_one(self.database.pool())
        .await
        .unwrap();
        self.call(
            "POST",
            &self.workspace("/tasks"),
            Some(json!({"project_id": project, "status_id": status, "title": title})),
        )
        .await
    }

    /// Posts a signed Stripe event; returns the status code.
    async fn webhook(&self, event: &Value) -> u16 {
        let body = event.to_string();
        let signature = sign(
            WEBHOOK_SECRET,
            body.as_bytes(),
            TimestampMillis::now().as_millis() / 1000,
        );
        self.http
            .post(format!("{}/api/v1/billing/stripe/webhook", self.base))
            .header("stripe-signature", signature)
            .header("content-type", "application/json")
            .body(body)
            .send()
            .await
            .unwrap()
            .status()
            .as_u16()
    }

    async fn sync_jobs(&self) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM jobs WHERE kind = 'billing.stripe_sync'")
            .fetch_one(self.database.pool())
            .await
            .unwrap()
    }

    /// Waits until every sync job finished, then returns the workspace's billing view.
    async fn settled_billing(&self) -> Value {
        for _ in 0..100 {
            let pending: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM jobs WHERE kind = 'billing.stripe_sync' AND state != 'succeeded'",
            )
            .fetch_one(self.database.pool())
            .await
            .unwrap();
            if pending == 0 {
                return self.call("GET", &self.workspace("/billing"), None).await.1;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("billing.stripe_sync jobs did not finish");
    }

    async fn add_member(&self) -> String {
        let id = Id::new_v7();
        let now = TimestampMillis::now();
        sqlx::query(
            "INSERT INTO users (id, email, normalized_email, display_name, password_hash, created_at, updated_at) \
             VALUES ($1, 'member@example.com', 'member@example.com', 'Member', 'unused', $2, $3)",
        )
        .bind(id.to_string())
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
        .bind(&self.workspace_id)
        .bind(id.to_string())
        .bind(now.as_millis())
        .bind(now.as_millis())
        .execute(self.database.pool())
        .await
        .unwrap();
        self.identity
            .create_session(
                &AuthenticatedUser {
                    id,
                    email: "member@example.com".to_owned(),
                    display_name: "Member".to_owned(),
                },
                now,
            )
            .await
            .unwrap()
            .token
    }
}

fn subscription_event(id: &str, kind: &str, subscription: &str, workspace_id: &str) -> Value {
    json!({
        "id": id,
        "object": "event",
        "type": kind,
        "data": {"object": {
            "id": subscription,
            "object": "subscription",
            "metadata": {"workspace_id": workspace_id},
        }},
    })
}

fn checkout_completed(id: &str, subscription: &str, workspace_id: &str) -> Value {
    json!({
        "id": id,
        "object": "event",
        "type": "checkout.session.completed",
        "data": {"object": {
            "id": "cs_test",
            "object": "checkout.session",
            "mode": "subscription",
            "subscription": subscription,
            "client_reference_id": workspace_id,
        }},
    })
}

#[tokio::test]
async fn a_workspace_without_a_subscription_is_read_only_until_it_subscribes() {
    let stripe = FakeStripe::default();
    let server = Server::start(Some(&stripe.start().await)).await;

    let (status, problem) = server.create_task("Blocked").await;
    assert_eq!(status, 402, "{problem}");
    assert_eq!(problem["code"], "workspace_read_only");
    let (status, _) = server.call("GET", &server.workspace("/tasks"), None).await;
    assert_eq!(status, 200);
    let (status, _) = server
        .call("POST", &server.workspace("/tasks/query"), Some(json!({})))
        .await;
    assert_ne!(status, 402);

    // A percent-encoded workspace id reaches the same handler, so the guard must decode it too.
    let encoded: String = server
        .workspace_id
        .bytes()
        .map(|byte| format!("%{byte:02X}"))
        .collect();
    let (status, problem) = server
        .call(
            "PATCH",
            &format!("/api/v1/workspaces/{encoded}"),
            Some(json!({"name": "Renamed"})),
        )
        .await;
    assert_eq!(status, 402, "{problem}");

    let (status, options) = server.call("GET", "/api/v1/auth/options", None).await;
    assert_eq!(status, 200);
    assert_eq!(options["billing_enabled"], true);
    let (status, billing) = server
        .call("GET", &server.workspace("/billing"), None)
        .await;
    assert_eq!(status, 200);
    assert_eq!(
        billing,
        json!({
            "plan": null,
            "status": null,
            "access": "read_only",
            "cancel_at_period_end": false,
            "current_period_end": null,
            "plans": ["team", "business"],
            "has_customer": false,
            "can_manage": true,
        })
    );

    let (status, problem) = server
        .call(
            "POST",
            &server.workspace("/billing/checkout"),
            Some(json!({"plan": "gold"})),
        )
        .await;
    assert_eq!((status, &problem["code"]), (422, &json!("unknown_plan")));
    let (status, problem) = server
        .call("POST", &server.workspace("/billing/portal"), None)
        .await;
    assert_eq!(
        (status, &problem["code"]),
        (409, &json!("billing_customer_missing"))
    );

    let (status, redirect) = server
        .call(
            "POST",
            &server.workspace("/billing/checkout"),
            Some(json!({"plan": "business"})),
        )
        .await;
    assert_eq!(status, 200, "{redirect}");
    assert_eq!(
        redirect,
        json!({"url": "https://checkout.stripe.test/cs_test"})
    );
    let customers = stripe.requests("/v1/customers");
    assert_eq!(customers.len(), 1);
    assert_eq!(
        customers[0].idempotency_key.as_deref(),
        Some(format!("orbit-workspace-{}-customer", server.workspace_id).as_str())
    );
    assert_eq!(
        customers[0].form["metadata[workspace_id]"],
        server.workspace_id
    );
    assert_eq!(customers[0].form["email"], "owner@example.com");
    assert_eq!(customers[0].form["name"], "Acme");
    let checkout = &stripe.requests("/v1/checkout/sessions")[0].form;
    let expected: HashMap<String, String> = [
        ("mode", "subscription".to_owned()),
        ("customer", "cus_test".to_owned()),
        ("line_items[0][price]", "price_business".to_owned()),
        ("line_items[0][quantity]", "1".to_owned()),
        ("client_reference_id", server.workspace_id.clone()),
        (
            "subscription_data[metadata][workspace_id]",
            server.workspace_id.clone(),
        ),
        (
            "success_url",
            "http://127.0.0.1:8080/settings/billing?checkout=success".to_owned(),
        ),
        (
            "cancel_url",
            "http://127.0.0.1:8080/settings/billing?checkout=cancel".to_owned(),
        ),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), value))
    .collect();
    assert_eq!(checkout, &expected);

    // A second checkout reuses the stored customer.
    let (status, _) = server
        .call(
            "POST",
            &server.workspace("/billing/checkout"),
            Some(json!({"plan": "team"})),
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(stripe.requests("/v1/customers").len(), 1);

    stripe.set_subscription("sub_1", &server.workspace_id, "active", "price_business");
    assert_eq!(
        server
            .webhook(&checkout_completed(
                "evt_checkout",
                "sub_1",
                &server.workspace_id
            ))
            .await,
        200
    );
    let billing = server.settled_billing().await;
    assert_eq!(billing["access"], "full", "{billing}");
    assert_eq!(billing["status"], "active");
    assert_eq!(billing["plan"], "business");
    assert_eq!(billing["has_customer"], true);
    assert_eq!(billing["current_period_end"], "2030-03-17T17:46:40.000Z");

    let (status, task) = server.create_task("Allowed").await;
    assert_eq!(status, 201, "{task}");
    let (status, problem) = server
        .call(
            "POST",
            &server.workspace("/billing/checkout"),
            Some(json!({"plan": "team"})),
        )
        .await;
    assert_eq!(
        (status, &problem["code"]),
        (409, &json!("subscription_active"))
    );
    let (status, redirect) = server
        .call("POST", &server.workspace("/billing/portal"), None)
        .await;
    assert_eq!(status, 200);
    assert_eq!(redirect["url"], "https://billing.stripe.test/p");
    let portal = &stripe.requests("/v1/billing_portal/sessions")[0].form;
    assert_eq!(portal["customer"], "cus_test");
    assert_eq!(
        portal["return_url"],
        "http://127.0.0.1:8080/settings/billing"
    );

    let member = server.add_member().await;
    let (status, billing) = server
        .call_as(&member, "GET", &server.workspace("/billing"), None)
        .await;
    assert_eq!(status, 200);
    assert_eq!(billing["can_manage"], false);
    let (status, problem) = server
        .call_as(&member, "POST", &server.workspace("/billing/portal"), None)
        .await;
    assert_eq!(
        (status, &problem["code"]),
        (403, &json!("billing_owner_required"))
    );

    // The subscription is cancelled: the workspace goes back to read-only, data stays readable.
    stripe.set_subscription("sub_1", &server.workspace_id, "canceled", "price_business");
    assert_eq!(
        server
            .webhook(&subscription_event(
                "evt_deleted",
                "customer.subscription.deleted",
                "sub_1",
                &server.workspace_id,
            ))
            .await,
        200
    );
    let billing = server.settled_billing().await;
    assert_eq!(billing["access"], "read_only");
    assert_eq!(billing["status"], "canceled");
    assert_eq!(server.create_task("Blocked again").await.0, 402);
    let (status, tasks) = server.call("GET", &server.workspace("/tasks"), None).await;
    assert_eq!(status, 200);
    assert!(tasks.to_string().contains("Allowed"));

    let audited: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM audit_events WHERE action = 'billing.subscription_status_changed'",
    )
    .fetch_one(server.database.pool())
    .await
    .unwrap();
    assert_eq!(audited, 2);
    server.stop().await;
}

#[tokio::test]
async fn redelivered_and_out_of_order_events_converge_on_stripe() {
    let stripe = FakeStripe::default();
    let server = Server::start(Some(&stripe.start().await)).await;
    let workspace = server.workspace_id.clone();

    // Stripe now says active; the older "created" event (incomplete at the time) arrives last.
    stripe.set_subscription("sub_2", &workspace, "active", "price_team");
    let updated = subscription_event(
        "evt_updated",
        "customer.subscription.updated",
        "sub_2",
        &workspace,
    );
    assert_eq!(server.webhook(&updated).await, 200);
    assert_eq!(server.webhook(&updated).await, 200);
    assert_eq!(server.sync_jobs().await, 1);
    assert_eq!(
        server
            .webhook(&subscription_event(
                "evt_created",
                "customer.subscription.created",
                "sub_2",
                &workspace,
            ))
            .await,
        200
    );
    assert_eq!(server.sync_jobs().await, 2);
    let billing = server.settled_billing().await;
    assert_eq!(billing["status"], "active");
    assert_eq!(billing["plan"], "team");

    // A late event about an old, cancelled subscription leaves the new one in place.
    stripe.set_subscription("sub_old", &workspace, "canceled", "price_team");
    assert_eq!(
        server
            .webhook(&subscription_event(
                "evt_old",
                "customer.subscription.deleted",
                "sub_old",
                &workspace,
            ))
            .await,
        200
    );
    assert_eq!(server.settled_billing().await["access"], "full");

    // Unknown prices are stored without a plan; other event types queue nothing.
    stripe.set_subscription("sub_2", &workspace, "trialing", "price_legacy");
    let ignored = json!({"id": "evt_invoice", "type": "invoice.paid", "data": {"object": {}}});
    assert_eq!(server.webhook(&ignored).await, 200);
    assert_eq!(
        server
            .webhook(&subscription_event(
                "evt_legacy",
                "customer.subscription.updated",
                "sub_2",
                &workspace,
            ))
            .await,
        200
    );
    assert_eq!(server.sync_jobs().await, 4);
    let billing = server.settled_billing().await;
    assert_eq!(billing["status"], "trialing");
    assert_eq!(billing["plan"], Value::Null);
    assert_eq!(billing["access"], "full");
    server.stop().await;
}

#[tokio::test]
async fn webhooks_without_a_valid_signature_are_rejected() {
    let stripe = FakeStripe::default();
    let server = Server::start(Some(&stripe.start().await)).await;
    let body = subscription_event(
        "evt_forged",
        "customer.subscription.updated",
        "sub_x",
        &server.workspace_id,
    )
    .to_string();
    let now = TimestampMillis::now().as_millis() / 1000;
    for signature in [
        None,
        Some(sign("whsec_wrong", body.as_bytes(), now)),
        Some(sign(WEBHOOK_SECRET, body.as_bytes(), now - 600)),
        Some("t=1,v1=00".to_owned()),
    ] {
        let mut request = server
            .http
            .post(format!("{}/api/v1/billing/stripe/webhook", server.base))
            .body(body.clone());
        if let Some(signature) = signature {
            request = request.header("stripe-signature", signature);
        }
        let response = request.send().await.unwrap();
        assert_eq!(response.status().as_u16(), 400);
        let problem: Value = response.json().await.unwrap();
        assert_eq!(problem["code"], "invalid_stripe_signature");
    }
    assert_eq!(server.sync_jobs().await, 0);
    server.stop().await;
}

#[tokio::test]
async fn self_hosted_servers_have_no_billing_and_no_guard() {
    let server = Server::start(None).await;

    let (status, task) = server.create_task("Self-hosted").await;
    assert_eq!(status, 201, "{task}");
    let (status, _) = server
        .call("GET", &server.workspace("/billing"), None)
        .await;
    assert_eq!(status, 404);
    let (status, _) = server
        .call(
            "POST",
            &server.workspace("/billing/checkout"),
            Some(json!({"plan": "team"})),
        )
        .await;
    assert_eq!(status, 404);
    let response = server
        .http
        .post(format!("{}/api/v1/billing/stripe/webhook", server.base))
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 404);
    let (_, options) = server.call("GET", "/api/v1/auth/options", None).await;
    assert_eq!(options["billing_enabled"], false);
    server.stop().await;
}
