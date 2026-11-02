//! Workspace apps: who may manage, list and open them, the wrapper document, and "Sign in with
//! Orbit" (OpenID Connect), including that its tokens and MCP tokens never cross over.
//! Runs on SQLite, and on Postgres with `ORBIT_TEST_POSTGRES_URL`.

use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::http::{Request, Response, StatusCode, header};
use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use orbit_platform::{
    AuthenticatedUser, HttpPlatformLayer, Id, OriginPolicy, PasswordService, Row, TestDatabase,
    TimestampMillis,
};
use orbit_server::apps::{AppsState, oidc};
use orbit_server::auth_routes::{AuthState, CookieMode, auth_router};
use orbit_server::mcp::{McpState, router as mcp_router};
use orbit_server::oauth::OAuthState;
use orbit_server::repositories::api_tokens::ApiTokenRepository;
use orbit_server::repositories::identity::{IdentityRepository, SetupRequest};
use orbit_server::repositories::tasks::TaskRepository;
use orbit_server::repositories::workspaces::WorkspaceRepository;
use ring::signature::{RSA_PKCS1_2048_8192_SHA256, RsaPublicKeyComponents};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::OnceCell;
use tower::ServiceExt;

const ORIGIN: &str = "https://orbit.test";
const APP_URL: &str = "https://app.example.com/home";
const CALLBACK: &str = "https://app.example.com/callback";
const VERIFIER: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-._~";
const APP_KEY: [u8; 32] = [3; 32];

/// Making an RSA key takes long in a debug build, so all tests share one.
static KEY: OnceCell<(String, Vec<u8>, String)> = OnceCell::const_new();

struct User {
    id: Id,
    cookie: String,
}

struct Fixture {
    database: TestDatabase,
    identity: Arc<IdentityRepository>,
    app: axum::Router,
    workspace: String,
    owner: User,
    project: Id,
}

struct Client {
    id: String,
    secret: String,
    app_id: String,
    slug: String,
}

struct Answer {
    status: StatusCode,
    headers: axum::http::HeaderMap,
    text: String,
}

impl Answer {
    async fn of(response: Response<Body>) -> Self {
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        Self {
            status,
            headers,
            text: String::from_utf8_lossy(&bytes).into_owned(),
        }
    }
    fn json(&self) -> Value {
        serde_json::from_str(&self.text).unwrap_or(Value::Null)
    }
    fn location(&self) -> url::Url {
        url::Url::parse(self.headers[header::LOCATION].to_str().unwrap()).unwrap()
    }
    fn param(&self, name: &str) -> Option<String> {
        self.location()
            .query_pairs()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.into_owned())
    }
}

impl Fixture {
    async fn new() -> Self {
        let database = TestDatabase::new().await.unwrap();
        let identity = Arc::new(IdentityRepository::new((*database).clone()));
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
                    password_hash: PasswordService::default()
                        .hash("correct horse battery")
                        .unwrap(),
                    workspace_name: "Orbit".into(),
                    project_name: "General".into(),
                },
                now,
            )
            .await
            .unwrap();
        let apps = AppsState::new(
            Arc::clone(&identity),
            CookieMode::secure(),
            ORIGIN,
            Some(APP_KEY),
            false,
        );
        let oauth = OAuthState::new(Arc::clone(&identity), CookieMode::secure(), ORIGIN)
            .with_apps(apps.clone());
        let key = KEY
            .get_or_init(|| async {
                oidc::ensure_signing_key(&apps).await.unwrap();
                let row = sqlx::query("SELECT kid, sealed_key, public_jwk FROM oidc_signing_keys")
                    .fetch_one(database.pool())
                    .await
                    .unwrap();
                (row.get("kid"), row.get("sealed_key"), row.get("public_jwk"))
            })
            .await;
        sqlx::query(
            "INSERT INTO oidc_signing_keys (slot, kid, sealed_key, public_jwk, created_at) \
             VALUES ('primary', $1, $2, $3, 0) ON CONFLICT (slot) DO NOTHING",
        )
        .bind(&key.0)
        .bind(&key.1)
        .bind(&key.2)
        .execute(database.pool())
        .await
        .unwrap();
        let mcp = mcp_router(
            McpState::new(
                Arc::new(ApiTokenRepository::new((*database).clone())),
                Arc::new(TaskRepository::new((*database).clone())),
            )
            .with_oauth(oauth.clone()),
            ORIGIN,
        );
        let app = orbit_server::apps::router(apps)
            .merge(orbit_server::oauth::router(oauth))
            .merge(mcp)
            .merge(auth_router(AuthState::new(
                Arc::clone(&identity),
                CookieMode::secure(),
            )))
            .layer(axum::middleware::from_fn_with_state(
                (*database).clone(),
                orbit_server::agent::scope::layer,
            ))
            .layer(HttpPlatformLayer::new(OriginPolicy::new(ORIGIN)));
        Self {
            database,
            identity,
            app,
            workspace: setup.workspace_id.to_string(),
            owner: User {
                id: setup.user_id,
                cookie: format!("__Host-orbit_session={}", setup.session.token),
            },
            project: setup.project_id,
        }
    }

    /// A user with `role` in the workspace, or with no membership when `role` is `None`.
    async fn user(&self, name: &str, role: Option<&str>) -> User {
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
        if let Some(role) = role {
            sqlx::query(
                "INSERT INTO memberships (id, workspace_id, user_id, role, version, created_at, updated_at) \
                 VALUES ($1, $2, $3, $4, 0, $5, $5)",
            )
            .bind(Id::new_v7().to_string())
            .bind(&self.workspace)
            .bind(id.to_string())
            .bind(role)
            .bind(now)
            .execute(self.database.pool())
            .await
            .unwrap();
        }
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
        User {
            id,
            cookie: format!("__Host-orbit_session={}", session.token),
        }
    }

    async fn send(&self, request: Request<Body>) -> Answer {
        Answer::of(self.app.clone().oneshot(request).await.unwrap()).await
    }

    async fn api(&self, method: &str, path: &str, user: &User, body: Option<Value>) -> Answer {
        let builder = Request::builder()
            .method(method)
            .uri(format!("/api/v1/workspaces/{}{path}", self.workspace))
            .header(header::HOST, "orbit.test")
            .header(header::ORIGIN, ORIGIN)
            .header(header::COOKIE, &user.cookie);
        self.send(match body {
            Some(body) => builder
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
            None => builder.body(Body::empty()).unwrap(),
        })
        .await
    }

    async fn add_app(&self, name: &str, roles: &[&str]) -> Client {
        let answer = self
            .api(
                "POST",
                "/workspace-apps",
                &self.owner,
                Some(json!({"name": name, "url": APP_URL, "allowed_roles": roles, "redirect_uris": [CALLBACK]})),
            )
            .await;
        assert_eq!(answer.status, StatusCode::CREATED, "{}", answer.text);
        let body = answer.json();
        Client {
            id: body["sign_in"]["client_id"].as_str().unwrap().to_owned(),
            secret: body["client_secret"].as_str().unwrap().to_owned(),
            app_id: body["id"].as_str().unwrap().to_owned(),
            slug: body["slug"].as_str().unwrap().to_owned(),
        }
    }

    async fn authorize_with(&self, user: Option<&User>, pairs: &[(&str, &str)]) -> Answer {
        let query = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(pairs.iter().copied())
            .finish();
        let mut builder = Request::get(format!("/oauth/authorize?{query}"));
        if let Some(user) = user {
            builder = builder.header(header::COOKIE, &user.cookie);
        }
        self.send(builder.body(Body::empty()).unwrap()).await
    }

    /// The usual request of a signed-in person, with PKCE and a nonce.
    async fn authorize(&self, user: &User, client: &Client) -> Answer {
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(VERIFIER));
        self.authorize_with(
            Some(user),
            &[
                ("client_id", client.id.as_str()),
                ("response_type", "code"),
                ("redirect_uri", CALLBACK),
                ("scope", "openid profile email"),
                ("state", "the-state"),
                ("nonce", "the-nonce"),
                ("code_challenge", challenge.as_str()),
                ("code_challenge_method", "S256"),
            ],
        )
        .await
    }

    async fn token(&self, fields: &[(&str, &str)]) -> Answer {
        let body = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(fields.iter().copied())
            .finish();
        self.send(
            Request::post("/oauth/token")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
    }

    async fn redeem(&self, client: &Client, code: &str) -> Answer {
        self.token(&[
            ("grant_type", "authorization_code"),
            ("client_id", &client.id),
            ("client_secret", &client.secret),
            ("code", code),
            ("redirect_uri", CALLBACK),
            ("code_verifier", VERIFIER),
        ])
        .await
    }

    /// A full sign-in; the answer of the token endpoint.
    async fn sign_in(&self, user: &User, client: &Client) -> Value {
        let answer = self.authorize(user, client).await;
        assert_eq!(answer.status, StatusCode::SEE_OTHER, "{}", answer.text);
        let code = answer.param("code").expect("a code");
        let tokens = self.redeem(client, &code).await;
        assert_eq!(tokens.status, StatusCode::OK, "{}", tokens.text);
        tokens.json()
    }

    async fn userinfo(&self, token: &str) -> Answer {
        self.send(
            Request::get("/oauth/userinfo")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
    }

    async fn jwk(&self) -> Value {
        let answer = self
            .send(Request::get("/oauth/jwks").body(Body::empty()).unwrap())
            .await;
        answer.json()["keys"][0].clone()
    }

    async fn count(&self, table: &str, app_id: &str) -> i64 {
        sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT COUNT(*) FROM {table} WHERE app_id = $1"
        )))
        .bind(app_id)
        .fetch_one(self.database.pool())
        .await
        .unwrap()
    }
}

/// Checks the JWT's signature against the published key and returns its claims.
fn verify(jwt: &str, jwk: &Value) -> Value {
    let parts: Vec<&str> = jwt.split('.').collect();
    assert_eq!(parts.len(), 3);
    let header: Value = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(parts[0]).unwrap()).unwrap();
    assert_eq!(header["alg"], "RS256");
    assert_eq!(header["kid"], jwk["kid"]);
    let key = RsaPublicKeyComponents {
        n: URL_SAFE_NO_PAD.decode(jwk["n"].as_str().unwrap()).unwrap(),
        e: URL_SAFE_NO_PAD.decode(jwk["e"].as_str().unwrap()).unwrap(),
    };
    key.verify(
        &RSA_PKCS1_2048_8192_SHA256,
        format!("{}.{}", parts[0], parts[1]).as_bytes(),
        &URL_SAFE_NO_PAD.decode(parts[2]).unwrap(),
    )
    .expect("the signature matches the published key");
    serde_json::from_slice(&URL_SAFE_NO_PAD.decode(parts[1]).unwrap()).unwrap()
}

#[tokio::test]
async fn only_owners_and_admins_manage_and_the_secret_shows_once() {
    let fixture = Fixture::new().await;
    let admin = fixture.user("admin", Some("admin")).await;
    let member = fixture.user("member", Some("member")).await;
    let outsider = fixture.user("outsider", None).await;
    let body = json!({"name": "Report", "url": APP_URL});

    for (user, status) in [
        (&member, StatusCode::FORBIDDEN),
        (&outsider, StatusCode::NOT_FOUND),
    ] {
        let answer = fixture
            .api("POST", "/workspace-apps", user, Some(body.clone()))
            .await;
        assert_eq!(answer.status, status);
        assert_eq!(
            fixture
                .api("GET", "/workspace-apps", user, None)
                .await
                .status,
            status
        );
    }
    let created = fixture
        .api("POST", "/workspace-apps", &admin, Some(body.clone()))
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.text);
    assert_eq!(created.json()["slug"], "report");
    assert!(created.json().get("client_secret").is_none());
    // The same address again is a conflict.
    let again = fixture
        .api("POST", "/workspace-apps", &admin, Some(body))
        .await;
    assert_eq!(again.status, StatusCode::CONFLICT);

    let client = fixture.add_app("Signed", &["member"]).await;
    let stored: Vec<u8> =
        sqlx::query_scalar("SELECT oidc_secret_hash FROM workspace_apps WHERE id = $1")
            .bind(&client.app_id)
            .fetch_one(fixture.database.pool())
            .await
            .unwrap();
    assert_eq!(stored, Sha256::digest(&client.secret).to_vec());
    let listed = fixture.api("GET", "/workspace-apps", &admin, None).await;
    assert!(!listed.text.contains(&client.secret));
    assert!(!listed.text.contains("client_secret"));
    assert!(!listed.text.contains("secret_hash"));
    let patched = fixture
        .api(
            "PATCH",
            &format!("/workspace-apps/{}", client.app_id),
            &admin,
            Some(json!({"name": "Signed 2"})),
        )
        .await;
    assert_eq!(patched.status, StatusCode::OK);
    assert!(patched.json().get("client_secret").is_none());
    let seen: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM audit_events WHERE workspace_id = $1 AND action LIKE 'app.%'",
    )
    .bind(&fixture.workspace)
    .fetch_one(fixture.database.pool())
    .await
    .unwrap();
    assert_eq!(seen, 3);
}

#[tokio::test]
async fn bad_urls_and_orbits_own_origin_are_refused() {
    let fixture = Fixture::new().await;
    for url in [
        "https://orbit.test/x",
        "http://app.example.com",
        "https://user:pw@app.example.com",
        "https://a;script-src*.example.com",
        "javascript:alert(1)",
    ] {
        let answer = fixture
            .api(
                "POST",
                "/workspace-apps",
                &fixture.owner,
                Some(json!({"name": "Bad", "url": url})),
            )
            .await;
        assert_eq!(answer.status, StatusCode::UNPROCESSABLE_ENTITY, "{url}");
    }
    let answer = fixture
        .api(
            "POST",
            "/workspace-apps",
            &fixture.owner,
            Some(
                json!({"name": "Bad", "url": APP_URL, "redirect_uris": ["https://orbit.test/cb"]}),
            ),
        )
        .await;
    assert_eq!(answer.status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn the_list_and_the_frame_follow_roles_and_the_frame_has_its_own_csp() {
    let fixture = Fixture::new().await;
    let member = fixture.user("member", Some("member")).await;
    let outsider = fixture.user("outsider", None).await;
    fixture
        .api(
            "POST",
            "/workspace-apps",
            &fixture.owner,
            Some(json!({"name": "Secret \"<b>", "slug": "admins", "url": APP_URL, "allowed_roles": ["owner", "admin"]})),
        )
        .await;
    fixture
        .api(
            "POST",
            "/workspace-apps",
            &fixture.owner,
            Some(json!({"name": "Everyone", "url": "https://other.example.com/"})),
        )
        .await;

    let items = fixture.api("GET", "/apps", &member, None).await.json();
    assert_eq!(items["items"].as_array().unwrap().len(), 1);
    assert_eq!(items["items"][0]["slug"], "everyone");
    assert_eq!(
        fixture
            .api("GET", "/apps/admins/frame", &member, None)
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    for path in ["/apps", "/apps/everyone/frame"] {
        assert_eq!(
            fixture.api("GET", path, &outsider, None).await.status,
            StatusCode::NOT_FOUND,
            "{path}"
        );
    }
    // Not signed in.
    let anonymous = fixture
        .send(
            Request::get(format!(
                "/api/v1/workspaces/{}/apps/everyone/frame",
                fixture.workspace
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await;
    assert_eq!(anonymous.status, StatusCode::UNAUTHORIZED);

    let frame = fixture
        .api("GET", "/apps/admins/frame", &fixture.owner, None)
        .await;
    assert_eq!(frame.status, StatusCode::OK);
    let policy = frame.headers[header::CONTENT_SECURITY_POLICY]
        .to_str()
        .unwrap();
    assert!(
        policy.contains("frame-src https://app.example.com;"),
        "{policy}"
    );
    assert!(policy.contains("frame-ancestors 'self'"));
    assert!(policy.contains("default-src 'none'"));
    assert!(!policy.contains("script-src"));
    assert!(!frame.text.contains("<script"));
    assert!(!frame.text.contains("<b>"));
    // `data-src`: the Orbit page sets `src` once it listens to the frame.
    assert!(
        frame
            .text
            .contains("<iframe data-src=\"https://app.example.com/home\"")
    );

    // The global policy never names an app: leaked host names, and a stale cache of them.
    let other = fixture
        .send(Request::get("/oauth/jwks").body(Body::empty()).unwrap())
        .await;
    let global = other.headers[header::CONTENT_SECURITY_POLICY]
        .to_str()
        .unwrap();
    assert!(global.contains("frame-ancestors 'none'"));
    assert!(!global.contains("example.com"));

    // A new_tab app has no frame.
    let opened = fixture
        .api(
            "POST",
            "/workspace-apps",
            &fixture.owner,
            Some(json!({"name": "Tab", "url": APP_URL, "open_mode": "new_tab"})),
        )
        .await;
    assert_eq!(opened.status, StatusCode::CREATED);
    assert_eq!(
        fixture
            .api("GET", "/apps/tab/frame", &fixture.owner, None)
            .await
            .status,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn agent_sessions_reach_no_app_route() {
    let fixture = Fixture::new().await;
    let client = fixture.add_app("Agentless", &["owner"]).await;
    let token = "orbit-agent.test-token";
    let hash: [u8; 32] = Sha256::digest(token.as_bytes()).into();
    let session = fixture
        .identity
        .create_session(
            &AuthenticatedUser {
                id: fixture.owner.id,
                email: "owner@example.com".to_owned(),
                display_name: "Owner".to_owned(),
            },
            TimestampMillis::now(),
        )
        .await
        .unwrap();
    sqlx::query("UPDATE sessions SET token_hash = $1 WHERE token_hash = $2")
        .bind(hash.to_vec())
        .bind(Sha256::digest(session.token.as_bytes()).to_vec())
        .execute(fixture.database.pool())
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO agent_sessions (session_id, user_id, workspace_id, expires_at) \
         SELECT id, user_id, $1, $2 FROM sessions WHERE token_hash = $3",
    )
    .bind(&fixture.workspace)
    .bind(TimestampMillis::now().as_millis() + 60_000)
    .bind(hash.to_vec())
    .execute(fixture.database.pool())
    .await
    .unwrap();
    let exists: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_sessions")
        .fetch_one(fixture.database.pool())
        .await
        .unwrap();
    assert_eq!(exists, 1, "the test must make a real agent session");
    for path in [
        format!("/api/v1/workspaces/{}/apps", fixture.workspace),
        format!("/api/v1/workspaces/{}/workspace-apps", fixture.workspace),
        format!(
            "/api/v1/workspaces/{}/apps/{}/frame",
            fixture.workspace, client.slug
        ),
        format!(
            "/oauth/authorize?client_id={}&response_type=code&scope=openid&redirect_uri={CALLBACK}",
            client.id
        ),
    ] {
        let answer = fixture
            .send(
                Request::get(&path)
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert_eq!(answer.status, StatusCode::FORBIDDEN, "{path}");
    }
}

#[tokio::test]
async fn sign_in_gives_a_signed_id_token_and_userinfo() {
    let fixture = Fixture::new().await;
    let member = fixture.user("member", Some("member")).await;
    let client = fixture.add_app("Report", &["member"]).await;

    let discovery = fixture
        .send(
            Request::get("/.well-known/openid-configuration")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .json();
    assert_eq!(discovery["issuer"], ORIGIN);
    assert_eq!(discovery["jwks_uri"], "https://orbit.test/oauth/jwks");

    let answer = fixture.authorize(&member, &client).await;
    assert_eq!(answer.status, StatusCode::SEE_OTHER);
    assert_eq!(answer.param("state").unwrap(), "the-state");
    assert_eq!(answer.param("iss").unwrap(), ORIGIN);
    assert!(answer.location().as_str().starts_with(CALLBACK));
    let code = answer.param("code").unwrap();

    let tokens = fixture.redeem(&client, &code).await;
    assert_eq!(tokens.status, StatusCode::OK, "{}", tokens.text);
    assert_eq!(tokens.headers[header::CACHE_CONTROL], "no-store");
    let tokens = tokens.json();
    assert!(tokens.get("refresh_token").is_none());
    assert_eq!(tokens["token_type"], "Bearer");

    let jwk = fixture.jwk().await;
    // Public values only: a private RSA key would also have d, p, q, dp, dq and qi.
    let mut fields: Vec<&str> = jwk
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    fields.sort_unstable();
    assert_eq!(fields, ["alg", "e", "kid", "kty", "n", "use"]);
    let claims = verify(tokens["id_token"].as_str().unwrap(), &jwk);
    assert_eq!(claims["iss"], ORIGIN);
    assert_eq!(claims["aud"], client.id.as_str());
    assert_eq!(claims["sub"], member.id.to_string());
    assert_eq!(claims["nonce"], "the-nonce");
    assert_eq!(claims["name"], "member");
    assert_eq!(claims["email"], "member@example.com");
    assert_eq!(claims["email_verified"], false);
    assert_eq!(claims["workspace_id"], fixture.workspace.as_str());
    assert_eq!(claims["workspace_role"], "member");
    assert!(claims["exp"].as_i64().unwrap() > claims["iat"].as_i64().unwrap());
    let access = tokens["access_token"].as_str().unwrap();
    assert_eq!(
        claims["at_hash"],
        URL_SAFE_NO_PAD.encode(&Sha256::digest(access)[..16])
    );

    let info = fixture.userinfo(access).await;
    assert_eq!(info.status, StatusCode::OK, "{}", info.text);
    assert_eq!(info.json()["sub"], member.id.to_string());
    assert_eq!(info.json()["workspace_role"], "member");

    // HTTP Basic works as well, with the scope narrowed to what was asked.
    let narrow = fixture
        .authorize_with(
            Some(&member),
            &[
                ("client_id", client.id.as_str()),
                ("response_type", "code"),
                ("redirect_uri", CALLBACK),
                ("scope", "openid unknown"),
            ],
        )
        .await;
    let basic = STANDARD.encode(format!("{}:{}", client.id, client.secret));
    let body = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs([
            ("grant_type", "authorization_code"),
            ("code", narrow.param("code").unwrap().as_str()),
            ("redirect_uri", CALLBACK),
        ])
        .finish();
    let answer = fixture
        .send(
            Request::post("/oauth/token")
                .header(header::AUTHORIZATION, format!("Basic {basic}"))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(body))
                .unwrap(),
        )
        .await;
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.text);
    let narrow_info = fixture
        .userinfo(answer.json()["access_token"].as_str().unwrap())
        .await
        .json();
    assert!(narrow_info.get("email").is_none());
    assert!(narrow_info.get("name").is_none());
    assert_eq!(narrow_info["sub"], member.id.to_string());
}

#[tokio::test]
async fn a_bad_client_redirect_or_person_never_gets_a_code() {
    let fixture = Fixture::new().await;
    let client = fixture.add_app("Report", &["owner"]).await;
    let member = fixture.user("member", Some("member")).await;
    let outsider = fixture.user("outsider", None).await;
    let base = |redirect: &'static str, client_id: String| {
        vec![
            ("client_id", client_id),
            ("response_type", "code".to_owned()),
            ("redirect_uri", redirect.to_owned()),
            ("scope", "openid".to_owned()),
        ]
    };
    let call = |pairs: Vec<(&'static str, String)>| {
        let fixture = &fixture;
        async move {
            let refs: Vec<(&str, &str)> = pairs.iter().map(|(a, b)| (*a, b.as_str())).collect();
            fixture.authorize_with(Some(&fixture.owner), &refs).await
        }
    };

    // Unknown client, near-miss redirect URIs: an error here, never a redirect.
    for redirect in [
        "https://app.example.com/callback/",
        "https://app.example.com/callback?x=1",
        "https://APP.example.com/callback",
        "https://evil.example.com/callback",
    ] {
        let answer = call(base(redirect, client.id.clone())).await;
        assert_eq!(answer.status, StatusCode::BAD_REQUEST, "{redirect}");
        assert!(!answer.headers.contains_key(header::LOCATION));
    }
    let answer = call(base(CALLBACK, "orbit-app-unknown".to_owned())).await;
    assert_eq!(answer.status, StatusCode::BAD_REQUEST);

    // A person the app does not allow, and one outside the workspace.
    for user in [&member, &outsider] {
        let answer = fixture.authorize(user, &client).await;
        assert_eq!(answer.status, StatusCode::SEE_OTHER);
        assert_eq!(answer.param("error").unwrap(), "access_denied");
        assert!(answer.param("code").is_none());
    }
    // No openid scope.
    let mut pairs = base(CALLBACK, client.id.clone());
    pairs[3].1 = "profile".to_owned();
    assert_eq!(call(pairs).await.param("error").unwrap(), "invalid_scope");
    // PKCE plain is refused.
    let mut pairs = base(CALLBACK, client.id.clone());
    pairs.push(("code_challenge", "x".repeat(43)));
    pairs.push(("code_challenge_method", "plain".to_owned()));
    assert_eq!(call(pairs).await.param("error").unwrap(), "invalid_request");

    // Not signed in: to the login page and back, or an error to the app inside a frame.
    let request = |frame: bool| {
        let mut builder = Request::get(format!(
            "/oauth/authorize?client_id={}&response_type=code&scope=openid&redirect_uri={CALLBACK}",
            client.id
        ));
        if frame {
            builder = builder.header("sec-fetch-dest", "iframe");
        }
        builder.body(Body::empty()).unwrap()
    };
    let top = fixture.send(request(false)).await;
    assert_eq!(top.status, StatusCode::SEE_OTHER);
    assert_eq!(top.location().path(), "/login");
    assert!(
        top.param("return_to")
            .unwrap()
            .starts_with("/oauth/authorize?client_id=")
    );
    let framed = fixture.send(request(true)).await;
    assert_eq!(framed.status, StatusCode::OK);
    assert!(!framed.headers.contains_key(header::LOCATION));
    assert!(framed.text.contains(&format!(
        "content=\"{CALLBACK}?error=login_required&amp;iss="
    )));

    // A suspended person.
    let suspended = fixture.user("suspended", Some("member")).await;
    let open = fixture.add_app("Open", &["owner", "member"]).await;
    let code = fixture
        .authorize(&suspended, &open)
        .await
        .param("code")
        .unwrap();
    sqlx::query("UPDATE users SET suspended_at = 1 WHERE id = $1")
        .bind(suspended.id.to_string())
        .execute(fixture.database.pool())
        .await
        .unwrap();
    assert_eq!(
        fixture.redeem(&open, &code).await.status,
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn a_code_works_once_with_its_verifier_secret_and_redirect() {
    let fixture = Fixture::new().await;
    let client = fixture.add_app("Report", &["owner"]).await;
    let other = fixture.add_app("Other", &["owner"]).await;
    let owner = &fixture.owner;
    let code = fixture
        .authorize(owner, &client)
        .await
        .param("code")
        .unwrap();
    let grant = |code: &str, secret: &str, verifier: &str, redirect: &str, id: &str| {
        vec![
            ("grant_type".to_owned(), "authorization_code".to_owned()),
            ("client_id".to_owned(), id.to_owned()),
            ("client_secret".to_owned(), secret.to_owned()),
            ("code".to_owned(), code.to_owned()),
            ("redirect_uri".to_owned(), redirect.to_owned()),
            ("code_verifier".to_owned(), verifier.to_owned()),
        ]
    };
    let send = |fields: Vec<(String, String)>| {
        let fixture = &fixture;
        async move {
            let refs: Vec<(&str, &str)> = fields
                .iter()
                .map(|(a, b)| (a.as_str(), b.as_str()))
                .collect();
            fixture.token(&refs).await
        }
    };
    let wrong_verifier = "z".repeat(50);
    // Wrong secret, wrong client, wrong verifier, wrong redirect: nothing is used up.
    let answer = send(grant(&code, "wrong", VERIFIER, CALLBACK, &client.id)).await;
    assert_eq!(answer.status, StatusCode::UNAUTHORIZED);
    assert_eq!(answer.json()["error"], "invalid_client");
    let answer = send(grant(&code, &other.secret, VERIFIER, CALLBACK, &other.id)).await;
    assert_eq!(answer.json()["error"], "invalid_grant");
    let answer = send(grant(
        &code,
        &client.secret,
        &wrong_verifier,
        CALLBACK,
        &client.id,
    ))
    .await;
    assert_eq!(answer.json()["error"], "invalid_grant");
    let answer = send(grant(
        &code,
        &client.secret,
        VERIFIER,
        "https://app.example.com/other",
        &client.id,
    ))
    .await;
    assert_eq!(answer.json()["error"], "invalid_grant");

    let first = fixture.redeem(&client, &code).await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.text);
    let access = first.json()["access_token"].as_str().unwrap().to_owned();
    assert_eq!(fixture.userinfo(&access).await.status, StatusCode::OK);
    // The second use fails and ends the token of the first.
    assert_eq!(
        fixture.redeem(&client, &code).await.status,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        fixture.userinfo(&access).await.status,
        StatusCode::UNAUTHORIZED
    );

    // A code made with PKCE needs the verifier; one made without it refuses a verifier.
    let code = fixture
        .authorize(owner, &client)
        .await
        .param("code")
        .unwrap();
    let answer = fixture
        .token(&[
            ("grant_type", "authorization_code"),
            ("client_id", &client.id),
            ("client_secret", &client.secret),
            ("code", &code),
            ("redirect_uri", CALLBACK),
        ])
        .await;
    assert_eq!(answer.json()["error"], "invalid_grant");

    // A rotated secret: the old one is refused, tokens end.
    let tokens = fixture.sign_in(owner, &client).await;
    let rotated = fixture
        .api(
            "POST",
            &format!("/workspace-apps/{}/rotate-secret", client.app_id),
            owner,
            None,
        )
        .await;
    assert_eq!(rotated.status, StatusCode::OK, "{}", rotated.text);
    assert!(rotated.json()["client_secret"].as_str().unwrap() != client.secret);
    assert_eq!(
        fixture
            .userinfo(tokens["access_token"].as_str().unwrap())
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    let code = fixture
        .authorize(owner, &client)
        .await
        .param("code")
        .unwrap();
    assert_eq!(
        fixture.redeem(&client, &code).await.status,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn app_tokens_and_mcp_tokens_never_cross_over() {
    let fixture = Fixture::new().await;
    let client = fixture.add_app("Report", &["owner"]).await;
    let tokens = fixture.sign_in(&fixture.owner, &client).await;
    let access = tokens["access_token"].as_str().unwrap();

    // Refused at the API and at /mcp.
    let me = fixture
        .send(
            Request::get("/api/v1/auth/me")
                .header(header::AUTHORIZATION, format!("Bearer {access}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(me.status, StatusCode::UNAUTHORIZED);
    let listed = fixture
        .send(
            Request::get(format!("/api/v1/workspaces/{}/apps", fixture.workspace))
                .header(header::AUTHORIZATION, format!("Bearer {access}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(listed.status, StatusCode::UNAUTHORIZED);
    let mcp_call = |token: String| {
        Request::post("/mcp")
            .header(header::HOST, "orbit.test")
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::ACCEPT, "application/json, text/event-stream")
            .header("mcp-protocol-version", "2025-11-25")
            .body(Body::from(
                json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"list_projects","arguments":{}}})
                    .to_string(),
            ))
            .unwrap()
    };
    assert_eq!(
        fixture.send(mcp_call(access.to_owned())).await.status,
        StatusCode::UNAUTHORIZED
    );

    // An MCP client through the existing flow: its token works at /mcp, and not at userinfo.
    let registered = fixture
        .send(
            Request::post("/oauth/register")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({"client_name":"Tool","redirect_uris":["http://127.0.0.1:6274/cb"],"token_endpoint_auth_method":"none"}).to_string(),
                ))
                .unwrap(),
        )
        .await;
    assert_eq!(registered.status, StatusCode::CREATED);
    let mcp_client = registered.json()["client_id"].as_str().unwrap().to_owned();
    assert!(!mcp_client.starts_with("orbit-app-"));
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(VERIFIER));
    let pending = fixture
        .authorize_with(
            None,
            &[
                ("client_id", mcp_client.as_str()),
                ("response_type", "code"),
                ("redirect_uri", "http://127.0.0.1:6274/cb"),
                ("code_challenge", challenge.as_str()),
                ("code_challenge_method", "S256"),
            ],
        )
        .await;
    assert_eq!(pending.status, StatusCode::SEE_OTHER, "{}", pending.text);
    let request_id = pending.param("request").unwrap();
    let consent_path = format!("/api/v1/oauth/consent/{request_id}");
    let consent = fixture
        .send(
            Request::get(&consent_path)
                .header(header::COOKIE, &fixture.owner.cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .json();
    let approved = fixture
        .send(
            Request::post(&consent_path)
                .header(header::HOST, "orbit.test")
                .header(header::ORIGIN, ORIGIN)
                .header(header::COOKIE, &fixture.owner.cookie)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({"approved":true,"csrf_token":consent["csrf_token"],"workspace_id":fixture.workspace,"project_ids":[fixture.project]}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .json();
    let callback = url::Url::parse(approved["redirect_uri"].as_str().unwrap()).unwrap();
    let code = callback
        .query_pairs()
        .find(|(key, _)| key == "code")
        .unwrap()
        .1
        .into_owned();
    let issued = fixture
        .token(&[
            ("grant_type", "authorization_code"),
            ("client_id", &mcp_client),
            ("code", &code),
            ("redirect_uri", "http://127.0.0.1:6274/cb"),
            ("code_verifier", VERIFIER),
        ])
        .await;
    assert_eq!(issued.status, StatusCode::OK, "{}", issued.text);
    let mcp_token = issued.json()["access_token"].as_str().unwrap().to_owned();
    assert_eq!(
        fixture.send(mcp_call(mcp_token.clone())).await.status,
        StatusCode::OK
    );
    assert_eq!(
        fixture.userinfo(&mcp_token).await.status,
        StatusCode::UNAUTHORIZED
    );
    // A session token is not an app token either.
    let session = fixture.owner.cookie.split('=').nth(1).unwrap();
    assert_eq!(
        fixture.userinfo(session).await.status,
        StatusCode::UNAUTHORIZED
    );
    // /oauth/register cannot make a client with sign-in rights: it gives no secret.
    assert!(registered.json().get("client_secret").is_none());
}

#[tokio::test]
async fn removal_deletion_and_a_role_change_end_access() {
    let fixture = Fixture::new().await;
    let client = fixture.add_app("Report", &["owner", "member"]).await;
    let member = fixture.user("member", Some("member")).await;

    // A role that is no longer allowed: at once, with the token still stored.
    let tokens = fixture.sign_in(&member, &client).await;
    let access = tokens["access_token"].as_str().unwrap();
    let changed = fixture
        .api(
            "PATCH",
            &format!("/workspace-apps/{}", client.app_id),
            &fixture.owner,
            Some(json!({"allowed_roles": ["owner"]})),
        )
        .await;
    assert_eq!(changed.status, StatusCode::OK);
    assert_eq!(
        fixture.userinfo(access).await.status,
        StatusCode::UNAUTHORIZED
    );
    fixture
        .api(
            "PATCH",
            &format!("/workspace-apps/{}", client.app_id),
            &fixture.owner,
            Some(json!({"allowed_roles": ["owner", "member"]})),
        )
        .await;
    assert_eq!(fixture.userinfo(access).await.status, StatusCode::OK);

    // A removed member: tokens and pending codes are deleted, not only refused.
    let pending = fixture
        .authorize(&member, &client)
        .await
        .param("code")
        .unwrap();
    let row = sqlx::query("SELECT id, version FROM memberships WHERE user_id = $1")
        .bind(member.id.to_string())
        .fetch_one(fixture.database.pool())
        .await
        .unwrap();
    let membership: String = row.get("id");
    let version: i64 = row.get("version");
    WorkspaceRepository::new((*fixture.database).clone())
        .remove_member(
            fixture.workspace.parse().unwrap(),
            membership.parse().unwrap(),
            fixture.owner.id,
            version as u64,
            "test",
            TimestampMillis::now(),
        )
        .await
        .unwrap();
    assert_eq!(fixture.count("app_oidc_tokens", &client.app_id).await, 0);
    assert_eq!(fixture.count("app_oidc_codes", &client.app_id).await, 0);
    assert_eq!(
        fixture.userinfo(access).await.status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        fixture.redeem(&client, &pending).await.status,
        StatusCode::BAD_REQUEST
    );

    // Turning sign-in off removes the client and what it gave out.
    let off = fixture.add_app("Off", &["owner"]).await;
    fixture.sign_in(&fixture.owner, &off).await;
    assert_eq!(fixture.count("app_oidc_tokens", &off.app_id).await, 1);
    let answer = fixture
        .api(
            "PATCH",
            &format!("/workspace-apps/{}", off.app_id),
            &fixture.owner,
            Some(json!({"redirect_uris": []})),
        )
        .await;
    assert!(answer.json()["sign_in"].is_null());
    assert_eq!(fixture.count("app_oidc_tokens", &off.app_id).await, 0);

    // A deleted app: client, codes and tokens go.
    let tokens = fixture.sign_in(&fixture.owner, &client).await;
    let pending = fixture
        .authorize(&fixture.owner, &client)
        .await
        .param("code")
        .unwrap();
    let deleted = fixture
        .api(
            "DELETE",
            &format!("/workspace-apps/{}", client.app_id),
            &fixture.owner,
            None,
        )
        .await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);
    assert_eq!(fixture.count("app_oidc_tokens", &client.app_id).await, 0);
    assert_eq!(fixture.count("app_oidc_codes", &client.app_id).await, 0);
    assert_eq!(
        fixture
            .userinfo(tokens["access_token"].as_str().unwrap())
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        fixture.redeem(&client, &pending).await.status,
        StatusCode::UNAUTHORIZED
    );
    let reauthorize = fixture.authorize(&fixture.owner, &client).await;
    assert_eq!(reauthorize.status, StatusCode::BAD_REQUEST);
}

/// Cloudflare Access as a client: it knows only the Auth, Token and Certificate URLs, a client id
/// and a secret, may or may not use PKCE, and reads the person from the ID token. The app behind it
/// has no code for this, so its origin and the callback's origin must both be allowed in the frame.
#[tokio::test]
async fn cloudflare_access_signs_in_as_a_generic_oidc_client() {
    const ACCESS_CALLBACK: &str = "https://edusi.cloudflareaccess.com/cdn-cgi/access/callback";
    let fixture = Fixture::new().await;
    let member = fixture.user("member", Some("member")).await;
    let created = fixture
        .api(
            "POST",
            "/workspace-apps",
            &fixture.owner,
            Some(
                json!({"name": "Products", "url": "https://products.wellrox.com",
                "allowed_roles": ["member"], "redirect_uris": [ACCESS_CALLBACK]}),
            ),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.text);
    let created = created.json();
    let client_id = created["sign_in"]["client_id"].as_str().unwrap();
    let secret = created["client_secret"].as_str().unwrap();

    // Both ways Access can be set up: without and with PKCE.
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(VERIFIER));
    for pkce in [false, true] {
        let mut request = vec![
            ("client_id", client_id),
            ("response_type", "code"),
            ("redirect_uri", ACCESS_CALLBACK),
            ("scope", "openid email profile"),
            ("state", "access-state"),
        ];
        if pkce {
            request.push(("code_challenge", challenge.as_str()));
            request.push(("code_challenge_method", "S256"));
        }
        let answer = fixture.authorize_with(Some(&member), &request).await;
        assert_eq!(answer.status, StatusCode::SEE_OTHER, "{}", answer.text);
        assert!(answer.location().as_str().starts_with(ACCESS_CALLBACK));
        assert_eq!(answer.param("state").unwrap(), "access-state");
        let code = answer.param("code").unwrap();

        // A server-to-server call: no cookie and no Origin header.
        let mut fields = vec![
            ("grant_type", "authorization_code"),
            ("client_id", client_id),
            ("client_secret", secret),
            ("code", code.as_str()),
            ("redirect_uri", ACCESS_CALLBACK),
        ];
        if pkce {
            fields.push(("code_verifier", VERIFIER));
        }
        let tokens = fixture.token(&fields).await;
        assert_eq!(tokens.status, StatusCode::OK, "{}", tokens.text);
        let claims = verify(
            tokens.json()["id_token"].as_str().unwrap(),
            &fixture.jwk().await,
        );
        assert_eq!(claims["iss"], ORIGIN);
        assert_eq!(claims["aud"], client_id);
        assert_eq!(claims["email"], "member@example.com");
        assert_eq!(claims["sub"], member.id.to_string());
    }

    // A role the app does not allow gets an error at Access's callback, never a code.
    let refused = fixture
        .authorize_with(
            Some(&fixture.owner),
            &[
                ("client_id", client_id),
                ("response_type", "code"),
                ("redirect_uri", ACCESS_CALLBACK),
                ("scope", "openid email profile"),
            ],
        )
        .await;
    assert_eq!(refused.param("error").unwrap(), "access_denied");
    assert!(refused.param("code").is_none());

    // The frame may pass through the app, Access and Orbit, and nothing else.
    let frame = fixture
        .api("GET", "/apps/products/frame", &member, None)
        .await;
    assert_eq!(frame.status, StatusCode::OK);
    let policy = frame.headers[header::CONTENT_SECURITY_POLICY]
        .to_str()
        .unwrap();
    assert!(
        policy.contains(
            "frame-src https://products.wellrox.com 'self' https://edusi.cloudflareaccess.com;"
        ),
        "{policy}"
    );
}

/// In Orbit's frame, `login_required` is a page that tells the Orbit page, then goes to the same
/// callback. It has one constant script (allowed by its hash), and the app's `state` only as text.
#[tokio::test]
async fn a_framed_login_required_tells_orbit_and_goes_back_to_the_app() {
    let fixture = Fixture::new().await;
    let client = fixture.add_app("Report", &["owner"]).await;
    let state = "s\"><script>alert(1)</script>&x='y";
    let request = |frame: bool, redirect: &str, prompt: &str| {
        let query: String = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("client_id", &client.id)
            .append_pair("response_type", "code")
            .append_pair("scope", "openid")
            .append_pair("redirect_uri", redirect)
            .append_pair("state", state)
            .append_pair("prompt", prompt)
            .finish();
        let mut builder = Request::get(format!("/oauth/authorize?{query}"));
        if frame {
            builder = builder.header("sec-fetch-dest", "iframe");
        }
        builder.body(Body::empty()).unwrap()
    };

    let page = fixture.send(request(true, CALLBACK, "")).await;
    assert_eq!(page.status, StatusCode::OK);
    assert_eq!(
        page.headers[header::CONTENT_TYPE],
        "text/html; charset=utf-8"
    );
    assert_eq!(page.headers[header::CACHE_CONTROL], "no-store");
    assert_eq!(page.headers[header::REFERRER_POLICY], "no-referrer");
    assert!(!page.headers.contains_key(header::X_FRAME_OPTIONS));
    let policy = page.headers[header::CONTENT_SECURITY_POLICY]
        .to_str()
        .unwrap();
    let script = page
        .text
        .split_once("<script>")
        .and_then(|(_, rest)| rest.split_once("</script>"))
        .map(|(script, _)| script)
        .unwrap();
    let hash = STANDARD.encode(Sha256::digest(script.as_bytes()));
    assert_eq!(
        policy,
        format!(
            "default-src 'none'; script-src 'sha256-{hash}'; frame-ancestors 'self'; base-uri 'none'; form-action 'none'"
        )
    );
    // The message goes only to Orbit's origin and carries nothing of the sign-in.
    assert!(
        script.contains("top.postMessage({ type: 'orbit:app-login-required' }, location.origin)")
    );
    assert!(!script.contains("state") && !script.contains("code") && !script.contains("http"));
    // The state is only text in an attribute: one script tag, no raw quote or tag of it.
    assert_eq!(page.text.matches("<script").count(), 1);
    assert!(!page.text.contains("alert(1)</script>"));
    let content = page
        .text
        .split_once("<meta name=\"orbit-redirect\" content=\"")
        .and_then(|(_, rest)| rest.split_once('"'))
        .map(|(content, _)| content)
        .unwrap();
    assert!(!content.contains(['<', '>', '\'']));
    let target = url::Url::parse(
        &content
            .replace("&#39;", "'")
            .replace("&quot;", "\"")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&amp;", "&"),
    )
    .unwrap();
    assert_eq!(target.as_str().split('?').next().unwrap(), CALLBACK);
    let param = |name: &str| {
        target
            .query_pairs()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.into_owned())
    };
    assert_eq!(param("error").unwrap(), "login_required");
    assert_eq!(param("state").unwrap(), state);
    assert_eq!(param("iss").unwrap(), ORIGIN);
    assert!(param("code").is_none());

    // prompt=login in a frame is the same answer.
    let again = fixture.send(request(true, CALLBACK, "login")).await;
    assert_eq!(again.status, StatusCode::OK);
    assert!(again.text.contains("orbit-redirect"));

    // A top-level request keeps the redirect (prompt=none: no login page either).
    let top = fixture.send(request(false, CALLBACK, "none")).await;
    assert_eq!(top.status, StatusCode::SEE_OTHER);
    assert_eq!(top.param("error").unwrap(), "login_required");
    assert_eq!(top.param("state").unwrap(), state);

    // A redirect URI that is not registered gets neither the page nor a redirect.
    let bad = fixture
        .send(request(true, "https://evil.example.com/callback", ""))
        .await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST);
    assert!(!bad.headers.contains_key(header::LOCATION));
    assert!(!bad.text.contains("orbit-redirect"));
}
