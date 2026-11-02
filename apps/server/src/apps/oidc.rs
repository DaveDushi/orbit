//! "Sign in with Orbit": OpenID Connect for the clients of workspace apps, on Orbit's OAuth server
//! (same issuer, `/oauth/authorize` and `/oauth/token` hand these clients over to this module).
//! Authorization code flow, confidential clients, RS256 ID tokens, no consent screen (an admin
//! approved the app), no refresh tokens. Access tokens live in `app_oidc_tokens` and only
//! `/oauth/userinfo` reads that table, so they grant nothing else; `/oauth/userinfo` reads nothing
//! else, so MCP tokens get nothing there.
//! Spec: <https://openid.net/specs/openid-connect-core-1_0.html>.

use axum::Router;
use axum::body::Bytes;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Json, Redirect, Response};
use axum::routing::get;
use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use orbit_domain::WorkspaceRole;
use orbit_platform::{Id, Row, TimestampMillis, generate_opaque_token};
use ring::rand::SystemRandom;
use ring::signature::{RSA_PKCS1_SHA256, RsaKeyPair};
use rsa::pkcs8::EncodePrivateKey;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use url::Url;

use super::{AppsState, COLUMNS, WorkspaceApp};
use crate::auth_routes::{avatar_url, request_session};
use crate::secret_box::{decrypt_secret, encrypt_secret};

const CODE_MILLIS: i64 = 60_000;
const TOKEN_SECONDS: i64 = 10 * 60;
const SUPPORTED_SCOPES: [&str; 3] = ["openid", "profile", "email"];

pub(super) fn router(state: AppsState) -> Router {
    Router::new()
        .route("/.well-known/openid-configuration", get(discovery))
        .route("/oauth/jwks", get(jwks))
        .route("/oauth/userinfo", get(userinfo).post(userinfo))
        .layer(middleware::from_fn(public_cors))
        .with_state(state)
}

// No cookies are read here, so any origin may call these.
async fn public_cors(request: Request, next: Next) -> Response {
    let mut response = if request.method() == axum::http::Method::OPTIONS {
        StatusCode::NO_CONTENT.into_response()
    } else {
        next.run(request).await
    };
    let headers = response.headers_mut();
    headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*".parse().unwrap());
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        "GET, POST, OPTIONS".parse().unwrap(),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        "Authorization".parse().unwrap(),
    );
    response
}

async fn discovery(State(state): State<AppsState>) -> Json<Value> {
    let origin = &state.origin;
    Json(json!({
        "issuer": origin,
        "authorization_endpoint": format!("{origin}/oauth/authorize"),
        "token_endpoint": format!("{origin}/oauth/token"),
        "userinfo_endpoint": format!("{origin}/oauth/userinfo"),
        "jwks_uri": format!("{origin}/oauth/jwks"),
        "scopes_supported": SUPPORTED_SCOPES,
        "response_types_supported": ["code"],
        "response_modes_supported": ["query"],
        "grant_types_supported": ["authorization_code"],
        "subject_types_supported": ["public"],
        "id_token_signing_alg_values_supported": ["RS256"],
        "token_endpoint_auth_methods_supported": ["client_secret_basic", "client_secret_post"],
        "code_challenge_methods_supported": ["S256"],
        "claims_supported": ["iss", "sub", "aud", "exp", "iat", "nonce", "at_hash", "name", "picture", "email", "email_verified", "workspace_id", "workspace_role"],
        "request_parameter_supported": false,
        "request_uri_parameter_supported": false,
        "authorization_response_iss_parameter_supported": true,
    }))
}

// ---------------------------------------------------------------------------------------------
// The signing key.

/// Makes the instance's key if there is none. Safe on several servers: the first insert wins and
/// every server then reads that row.
pub async fn ensure_signing_key(state: &AppsState) -> Result<(), sqlx::Error> {
    let Some(app_key) = state.app_key else {
        return Ok(());
    };
    let existing: Option<String> =
        sqlx::query_scalar("SELECT kid FROM oidc_signing_keys WHERE slot = 'primary'")
            .fetch_optional(state.database.pool())
            .await?;
    if existing.is_some() {
        return Ok(());
    }
    let generated = tokio::task::spawn_blocking(move || generate_key(&app_key))
        .await
        .ok()
        .flatten();
    let Some((kid, sealed, jwk)) = generated else {
        tracing::error!("the OpenID Connect signing key could not be made");
        return Err(sqlx::Error::PoolTimedOut);
    };
    sqlx::query(
        "INSERT INTO oidc_signing_keys (slot, kid, sealed_key, public_jwk, created_at) \
         VALUES ('primary', $1, $2, $3, $4) ON CONFLICT (slot) DO NOTHING",
    )
    .bind(kid)
    .bind(sealed)
    .bind(jwk)
    .bind(TimestampMillis::now().as_millis())
    .execute(state.database.pool())
    .await?;
    Ok(())
}

fn generate_key(app_key: &[u8; 32]) -> Option<(String, Vec<u8>, String)> {
    let mut rng = rand_core::OsRng;
    let key = rsa::RsaPrivateKey::new(&mut rng, 2048).ok()?;
    let pkcs8 = key.to_pkcs8_der().ok()?;
    let pair = RsaKeyPair::from_pkcs8(pkcs8.as_bytes()).ok()?;
    let public = ring::rsa::PublicKeyComponents::<Vec<u8>>::from(pair.public());
    let (modulus, exponent) = (public.n, public.e);
    let kid = URL_SAFE_NO_PAD.encode(&Sha256::digest(&modulus)[..16]);
    let jwk = json!({"kty": "RSA", "use": "sig", "alg": "RS256", "kid": kid,
        "n": URL_SAFE_NO_PAD.encode(modulus), "e": URL_SAFE_NO_PAD.encode(exponent)});
    let sealed = encrypt_secret(app_key, &STANDARD.encode(pkcs8.as_bytes())).ok()?;
    Some((kid, sealed, jwk.to_string()))
}

async fn signing_key(state: &AppsState) -> Option<(String, RsaKeyPair)> {
    let app_key = state.app_key?;
    if let Err(error) = ensure_signing_key(state).await {
        tracing::warn!(%error, "OpenID Connect signing key");
        return None;
    }
    let row = sqlx::query("SELECT kid, sealed_key FROM oidc_signing_keys WHERE slot = 'primary'")
        .fetch_optional(state.database.pool())
        .await
        .ok()??;
    let sealed: Vec<u8> = row.get("sealed_key");
    let pkcs8 = STANDARD
        .decode(decrypt_secret(&app_key, &sealed).ok()?)
        .ok()?;
    Some((row.get("kid"), RsaKeyPair::from_pkcs8(&pkcs8).ok()?))
}

async fn jwks(State(state): State<AppsState>) -> Response {
    let keys = sqlx::query_scalar::<_, String>("SELECT public_jwk FROM oidc_signing_keys")
        .fetch_all(state.database.pool())
        .await;
    match keys {
        Ok(keys) => {
            let keys: Vec<Value> = keys
                .iter()
                .filter_map(|key| serde_json::from_str(key).ok())
                .collect();
            Json(json!({ "keys": keys })).into_response()
        }
        Err(error) => {
            tracing::warn!(%error, "OpenID Connect keys");
            StatusCode::SERVICE_UNAVAILABLE.into_response()
        }
    }
}

fn sign_jwt(kid: &str, key: &RsaKeyPair, claims: &Value) -> Option<String> {
    let header = json!({"alg": "RS256", "typ": "JWT", "kid": kid});
    let message = format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(header.to_string()),
        URL_SAFE_NO_PAD.encode(claims.to_string())
    );
    let mut signature = vec![0; key.public().modulus_len()];
    key.sign(
        &RSA_PKCS1_SHA256,
        &SystemRandom::new(),
        message.as_bytes(),
        &mut signature,
    )
    .ok()?;
    Some(format!("{message}.{}", URL_SAFE_NO_PAD.encode(signature)))
}

// ---------------------------------------------------------------------------------------------
// Who is signing in.

struct Person {
    role: WorkspaceRole,
    email: String,
    display_name: String,
    email_verified: bool,
    avatar_at: Option<i64>,
}

/// The person as an active member of the app's live workspace with an allowed role, else `None`.
/// Checked on every authorize, token and userinfo call, so a change takes effect at once.
async fn person(
    state: &AppsState,
    app: &WorkspaceApp,
    user_id: &str,
) -> Result<Option<Person>, sqlx::Error> {
    let row = sqlx::query(
        "SELECT m.role, u.email, u.display_name, u.email_verified_at, \
         (SELECT a.updated_at FROM user_avatars a WHERE a.user_id = u.id) AS avatar_at \
         FROM memberships m JOIN users u ON u.id = m.user_id \
         JOIN workspaces w ON w.id = m.workspace_id \
         WHERE m.workspace_id = $1 AND m.user_id = $2 AND u.suspended_at IS NULL \
         AND w.deleted_at IS NULL",
    )
    .bind(&app.workspace_id)
    .bind(user_id)
    .fetch_optional(state.database.pool())
    .await?;
    Ok(row.and_then(|row| {
        let role = WorkspaceRole::parse(row.get::<String, _>("role").as_str())?;
        app.allows(role).then(|| Person {
            role,
            email: row.get("email"),
            display_name: row.get("display_name"),
            email_verified: row.get::<Option<i64>, _>("email_verified_at").is_some(),
            avatar_at: row.get("avatar_at"),
        })
    }))
}

/// The claims `scope` releases. The workspace claims come with every scope: they say in which
/// workspace the app knows this person.
fn person_claims(
    state: &AppsState,
    app: &WorkspaceApp,
    user_id: &str,
    person: &Person,
    scope: &str,
) -> Map<String, Value> {
    let mut claims = Map::new();
    claims.insert("sub".to_owned(), json!(user_id));
    claims.insert("workspace_id".to_owned(), json!(app.workspace_id));
    claims.insert("workspace_role".to_owned(), json!(person.role.as_str()));
    let has = |name: &str| scope.split(' ').any(|item| item == name);
    if has("email") {
        claims.insert("email".to_owned(), json!(person.email));
        claims.insert("email_verified".to_owned(), json!(person.email_verified));
    }
    if has("profile") {
        claims.insert("name".to_owned(), json!(person.display_name));
        if let Some(path) = user_id
            .parse::<Id>()
            .ok()
            .and_then(|id| avatar_url(id, person.avatar_at))
        {
            claims.insert(
                "picture".to_owned(),
                json!(format!("{}{path}", state.origin)),
            );
        }
    }
    claims
}

async fn app_by_client(
    state: &AppsState,
    client_id: &str,
) -> Result<Option<WorkspaceApp>, sqlx::Error> {
    let row = sqlx::query(sqlx::AssertSqlSafe(format!(
        "SELECT {COLUMNS} FROM workspace_apps WHERE oidc_client_id = $1"
    )))
    .bind(client_id)
    .fetch_optional(state.database.pool())
    .await?;
    Ok(row.as_ref().map(WorkspaceApp::from_row))
}

// ---------------------------------------------------------------------------------------------
// The authorization endpoint.

fn oauth_error(status: StatusCode, code: &str) -> Response {
    (
        status,
        [(header::CACHE_CONTROL, "no-store")],
        Json(json!({ "error": code })),
    )
        .into_response()
}

/// The app's registered redirect URI, with the parameters of the answer.
fn callback_url(
    state: &AppsState,
    redirect_uri: &str,
    client_state: Option<&str>,
    pairs: &[(&str, &str)],
) -> Option<Url> {
    let mut callback = Url::parse(redirect_uri).ok()?;
    {
        let mut query = callback.query_pairs_mut();
        for (name, value) in pairs {
            query.append_pair(name, value);
        }
        if let Some(client_state) = client_state {
            query.append_pair("state", client_state);
        }
        query.append_pair("iss", &state.origin);
    }
    Some(callback)
}

/// Back to the app's registered redirect URI, with the parameters of the answer.
fn back_to_client(
    state: &AppsState,
    redirect_uri: &str,
    client_state: Option<&str>,
    pairs: &[(&str, &str)],
) -> Response {
    match callback_url(state, redirect_uri, client_state, pairs) {
        Some(callback) => Redirect::to(callback.as_str()).into_response(),
        None => oauth_error(StatusCode::INTERNAL_SERVER_ERROR, "server_error"),
    }
}

/// Constant, so the page's CSP allows it by hash. Only to Orbit's own origin (the frame's
/// ancestors are all Orbit: `frame-ancestors 'self'`), with no state, code or address.
const LOGIN_REQUIRED_SCRIPT: &str = "top.postMessage({ type: 'orbit:app-login-required' }, location.origin)\n\
     location.replace(document.querySelector('meta[name=orbit-redirect]').content)";

/// `login_required` for an app in Orbit's frame: tells the Orbit page (which shows "Sign in to
/// <app>"), then goes to the same callback as the redirect would.
fn framed_login_required(callback: &Url) -> Response {
    let hash = STANDARD.encode(Sha256::digest(LOGIN_REQUIRED_SCRIPT.as_bytes()));
    let policy = format!(
        "default-src 'none'; script-src 'sha256-{hash}'; frame-ancestors 'self'; base-uri 'none'; form-action 'none'"
    );
    let target = super::escape_html(callback.as_str());
    let body = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>Sign in</title>\
         <meta name=\"orbit-redirect\" content=\"{target}\"><script>{LOGIN_REQUIRED_SCRIPT}</script></head></html>"
    );
    (
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8".to_owned()),
            (header::CONTENT_SECURITY_POLICY, policy),
            (header::CACHE_CONTROL, "no-store".to_owned()),
            (header::REFERRER_POLICY, "no-referrer".to_owned()),
        ],
        body,
    )
        .into_response()
}

pub(crate) async fn authorize(state: &AppsState, headers: &HeaderMap, raw_query: &str) -> Response {
    let param = |name: &str| {
        url::form_urlencoded::parse(raw_query.as_bytes())
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.into_owned())
    };
    // Nothing goes back to a redirect URI until the client and the exact URI are known.
    let app = match app_by_client(state, &param("client_id").unwrap_or_default()).await {
        Ok(Some(app)) => app,
        Ok(None) => return oauth_error(StatusCode::BAD_REQUEST, "invalid_client"),
        Err(error) => {
            tracing::warn!(%error, "OpenID Connect authorize");
            return oauth_error(StatusCode::SERVICE_UNAVAILABLE, "temporarily_unavailable");
        }
    };
    let redirect_uri = param("redirect_uri").unwrap_or_default();
    if !app.redirect_uris.contains(&redirect_uri) {
        return oauth_error(StatusCode::BAD_REQUEST, "invalid_redirect_uri");
    }
    let client_state = param("state");
    let fail = |code: &str| {
        back_to_client(
            state,
            &redirect_uri,
            client_state.as_deref(),
            &[("error", code)],
        )
    };
    // A frame cannot show Orbit's login (frame-ancestors 'none'), and a cross-site frame gets no
    // session cookie at all. The app must offer a top-level sign-in, and Orbit's page says so.
    let in_frame = headers
        .get("sec-fetch-dest")
        .is_some_and(|value| value == "iframe");
    let login_required = || {
        let callback = callback_url(
            state,
            &redirect_uri,
            client_state.as_deref(),
            &[("error", "login_required")],
        );
        match callback {
            Some(callback) if in_frame => framed_login_required(&callback),
            _ => fail("login_required"),
        }
    };
    if client_state
        .as_ref()
        .is_some_and(|value| value.len() > 2048)
        || param("nonce").is_some_and(|value| value.len() > 512)
    {
        return fail("invalid_request");
    }
    if param("response_type").as_deref() != Some("code") {
        return fail("unsupported_response_type");
    }
    if param("request").is_some() {
        return fail("request_not_supported");
    }
    if param("request_uri").is_some() {
        return fail("request_uri_not_supported");
    }
    let requested = param("scope").unwrap_or_default();
    if !requested.split(' ').any(|item| item == "openid") {
        return fail("invalid_scope");
    }
    // Unknown scopes are ignored, as the spec says.
    let scope = SUPPORTED_SCOPES
        .iter()
        .filter(|name| requested.split(' ').any(|item| item == **name))
        .copied()
        .collect::<Vec<_>>()
        .join(" ");
    let challenge = param("code_challenge");
    match (&challenge, param("code_challenge_method").as_deref()) {
        (Some(value), Some("S256"))
            if URL_SAFE_NO_PAD
                .decode(value)
                .is_ok_and(|bytes| bytes.len() == 32) => {}
        (None, None) => {}
        _ => return fail("invalid_request"),
    }
    let prompt = param("prompt").unwrap_or_default();
    let prompts: Vec<&str> = prompt.split(' ').collect();
    if prompts.contains(&"login") {
        // Orbit cannot make a signed-in person sign in again here: say so, do not pretend.
        return login_required();
    }
    let Some(session) = request_session(&state.identity, state.cookie_mode, headers).await else {
        if in_frame || prompts.contains(&"none") {
            return login_required();
        }
        let path_and_query = format!("/oauth/authorize?{raw_query}");
        let target: String =
            url::form_urlencoded::byte_serialize(path_and_query.as_bytes()).collect();
        return Redirect::to(&format!("{}/login?return_to={target}", state.origin)).into_response();
    };
    let user_id = session.user.id.to_string();
    match person(state, &app, &user_id).await {
        Ok(Some(_)) => {}
        Ok(None) => return fail("access_denied"),
        Err(error) => {
            tracing::warn!(%error, "OpenID Connect authorize");
            return fail("temporarily_unavailable");
        }
    }
    let code = generate_opaque_token();
    let now = TimestampMillis::now().as_millis();
    let stored = async {
        let mut tx = state.database.immediate_transaction().await?;
        sqlx::query("DELETE FROM app_oidc_codes WHERE expires_at <= $1")
            .bind(now)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "INSERT INTO app_oidc_codes (code_hash, app_id, user_id, redirect_uri, scope, nonce, \
             code_challenge, expires_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(hash(&code))
        .bind(&app.id)
        .bind(&user_id)
        .bind(&redirect_uri)
        .bind(&scope)
        .bind(param("nonce"))
        .bind(&challenge)
        .bind(now + CODE_MILLIS)
        .execute(&mut *tx)
        .await?;
        tx.commit().await
    }
    .await;
    if let Err(error) = stored {
        tracing::warn!(%error, "OpenID Connect authorize");
        return fail("temporarily_unavailable");
    }
    back_to_client(
        state,
        &redirect_uri,
        client_state.as_deref(),
        &[("code", &code)],
    )
}

// ---------------------------------------------------------------------------------------------
// The token endpoint.

pub(crate) fn uses_basic_auth(headers: &HeaderMap) -> bool {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.len() > 6 && value[..6].eq_ignore_ascii_case("basic "))
}

/// `client_secret_basic` (form-encoded id and secret) or `client_secret_post`, not both.
fn client_credentials(headers: &HeaderMap, form: &[(String, String)]) -> Option<(String, String)> {
    let field = |name: &str| {
        form.iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.clone())
    };
    if uses_basic_auth(headers) {
        if field("client_secret").is_some() {
            return None;
        }
        let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
        let decoded = String::from_utf8(STANDARD.decode(value[6..].trim()).ok()?).ok()?;
        let (id, secret) = decoded.split_once(':')?;
        let decode = |text: &str| {
            url::form_urlencoded::parse(format!("v={text}").as_bytes())
                .next()
                .map(|(_, value)| value.into_owned())
        };
        return Some((decode(id)?, decode(secret)?));
    }
    Some((field("client_id")?, field("client_secret")?))
}

fn invalid_client() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        [
            (header::CACHE_CONTROL, "no-store"),
            (header::WWW_AUTHENTICATE, "Basic realm=\"orbit\""),
        ],
        Json(json!({"error": "invalid_client"})),
    )
        .into_response()
}

fn hash(secret: &str) -> Vec<u8> {
    Sha256::digest(secret.as_bytes()).to_vec()
}

fn valid_verifier(verifier: &str) -> bool {
    (43..=128).contains(&verifier.len())
        && verifier
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-._~".contains(&byte))
}

pub(crate) async fn token(state: &AppsState, headers: &HeaderMap, body: &Bytes) -> Response {
    match issue(state, headers, body).await {
        Ok(response) => response,
        Err(error) => {
            tracing::warn!(%error, "OpenID Connect token");
            oauth_error(StatusCode::SERVICE_UNAVAILABLE, "temporarily_unavailable")
        }
    }
}

async fn issue(
    state: &AppsState,
    headers: &HeaderMap,
    body: &Bytes,
) -> Result<Response, sqlx::Error> {
    let form: Vec<(String, String)> = url::form_urlencoded::parse(body)
        .map(|(name, value)| (name.into_owned(), value.into_owned()))
        .collect();
    let field = |name: &str| {
        form.iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    };
    let Some((client_id, secret)) = client_credentials(headers, &form) else {
        return Ok(invalid_client());
    };
    let Some(app) = app_by_client(state, &client_id).await? else {
        return Ok(invalid_client());
    };
    if app.secret_hash.as_deref() != Some(hash(&secret).as_slice()) {
        return Ok(invalid_client());
    }
    let bad_grant = || oauth_error(StatusCode::BAD_REQUEST, "invalid_grant");
    if field("grant_type") != Some("authorization_code") {
        return Ok(oauth_error(
            StatusCode::BAD_REQUEST,
            "unsupported_grant_type",
        ));
    }
    let (Some(code), Some(redirect_uri)) = (field("code"), field("redirect_uri")) else {
        return Ok(oauth_error(StatusCode::BAD_REQUEST, "invalid_request"));
    };
    let code_hash = hash(code);
    let now = TimestampMillis::now().as_millis();

    // Getting the key may make it, which is slow: that happens before the writers' lock is taken.
    let Some((kid, key)) = signing_key(state).await else {
        return Ok(oauth_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "temporarily_unavailable",
        ));
    };

    let mut tx = state.database.immediate_transaction().await?;
    let row = sqlx::query(
        "SELECT user_id, redirect_uri, scope, nonce, code_challenge, used_at, expires_at \
         FROM app_oidc_codes WHERE code_hash = $1 AND app_id = $2",
    )
    .bind(&code_hash)
    .bind(&app.id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(row) = row else {
        return Ok(bad_grant());
    };
    if row.get::<Option<i64>, _>("used_at").is_some() {
        // A second use means the code leaked: what the first use gave ends too.
        sqlx::query("DELETE FROM app_oidc_tokens WHERE code_hash = $1")
            .bind(&code_hash)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        return Ok(bad_grant());
    }
    if row.get::<i64, _>("expires_at") <= now
        || row.get::<String, _>("redirect_uri") != redirect_uri
    {
        return Ok(bad_grant());
    }
    let verifier = field("code_verifier");
    match (row.get::<Option<String>, _>("code_challenge"), verifier) {
        (None, None) => {}
        (Some(challenge), Some(verifier))
            if valid_verifier(verifier)
                && URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())) == challenge => {}
        _ => return Ok(bad_grant()),
    }
    let user_id: String = row.get("user_id");
    let scope: String = row.get("scope");
    let nonce: Option<String> = row.get("nonce");
    sqlx::query("UPDATE app_oidc_codes SET used_at = $1 WHERE code_hash = $2")
        .bind(now)
        .bind(&code_hash)
        .execute(&mut *tx)
        .await?;
    let Some(person) = person(state, &app, &user_id).await? else {
        tx.commit().await?;
        return Ok(bad_grant());
    };

    let access = format!("orbit-app-a-{}", generate_opaque_token());
    let digest = Sha256::digest(access.as_bytes());
    let mut claims = person_claims(state, &app, &user_id, &person, &scope);
    claims.insert("iss".to_owned(), json!(state.origin));
    claims.insert("aud".to_owned(), json!(client_id));
    claims.insert("iat".to_owned(), json!(now / 1000));
    claims.insert("exp".to_owned(), json!(now / 1000 + TOKEN_SECONDS));
    claims.insert(
        "at_hash".to_owned(),
        json!(URL_SAFE_NO_PAD.encode(&digest[..16])),
    );
    if let Some(nonce) = nonce {
        claims.insert("nonce".to_owned(), json!(nonce));
    }
    let Some(id_token) = sign_jwt(&kid, &key, &Value::Object(claims)) else {
        return Ok(oauth_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "server_error",
        ));
    };
    sqlx::query("DELETE FROM app_oidc_tokens WHERE expires_at <= $1")
        .bind(now)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT INTO app_oidc_tokens (access_hash, app_id, user_id, code_hash, scope, expires_at) \
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(digest.to_vec())
    .bind(&app.id)
    .bind(&user_id)
    .bind(&code_hash)
    .bind(&scope)
    .bind(now + TOKEN_SECONDS * 1000)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok((
        [
            (header::CACHE_CONTROL, "no-store"),
            (header::PRAGMA, "no-cache"),
        ],
        Json(json!({"access_token": access, "token_type": "Bearer",
            "expires_in": TOKEN_SECONDS, "id_token": id_token, "scope": scope})),
    )
        .into_response())
}

// ---------------------------------------------------------------------------------------------
// Userinfo: the only place an app's access token works.

async fn userinfo(State(state): State<AppsState>, headers: HeaderMap) -> Response {
    let refuse = || {
        (
            StatusCode::UNAUTHORIZED,
            [
                (header::CACHE_CONTROL, "no-store"),
                (header::WWW_AUTHENTICATE, "Bearer error=\"invalid_token\""),
            ],
        )
            .into_response()
    };
    let Some(token) = orbit_platform::bearer_token(&headers) else {
        return refuse();
    };
    let found = async {
        let row = sqlx::query(
            "SELECT app_id, user_id, scope FROM app_oidc_tokens \
             WHERE access_hash = $1 AND expires_at > $2",
        )
        .bind(hash(token))
        .bind(TimestampMillis::now().as_millis())
        .fetch_optional(state.database.pool())
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let app = sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT {COLUMNS} FROM workspace_apps WHERE id = $1"
        )))
        .bind(row.get::<String, _>("app_id"))
        .fetch_one(state.database.pool())
        .await?;
        let app = WorkspaceApp::from_row(&app);
        let user_id: String = row.get("user_id");
        let scope: String = row.get("scope");
        let member = person(&state, &app, &user_id).await?;
        Ok::<_, sqlx::Error>(
            member.map(|member| person_claims(&state, &app, &user_id, &member, &scope)),
        )
    }
    .await;
    match found {
        Ok(Some(claims)) => (
            [(header::CACHE_CONTROL, "no-store")],
            Json(Value::Object(claims)),
        )
            .into_response(),
        Ok(None) => refuse(),
        Err(error) => {
            tracing::warn!(%error, "OpenID Connect userinfo");
            StatusCode::SERVICE_UNAVAILABLE.into_response()
        }
    }
}
