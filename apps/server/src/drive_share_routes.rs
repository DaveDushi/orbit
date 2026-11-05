//! Public Drive links. Members make them under `/api/v1/workspaces/{id}/drive/…/shares`; anyone with the link
//! opens `/s/<token>` (a page of the web app), which uses `/api/v1/public/drive/shares/{token}/…`.
//!
//! The token is 32 random bytes; only its SHA-256 is stored. A link with a password answers only `locked` until
//! the password is given; then the browser gets a pass (an HMAC of the link id and an expiry, keyed by a subkey of
//! the app key) for one hour. Password attempts are throttled per link and IP address like sign-in. The bytes go
//! from the bucket to the browser: a download is a redirect to a 5-minute presigned URL. Every failure answers the
//! same `404 drive_share_not_found`, so a link that is wrong, revoked or expired, or names something outside its
//! folder, tells nothing.

use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;

use axum::extract::{Extension, Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use orbit_platform::{
    ClientIp, Id, PasswordError, PasswordExecutor, PasswordService, RequestId, ThrottleDecision,
    TimestampMillis,
};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use utoipa::{IntoParams, ToSchema};

use crate::auth_routes::CookieMode;
use crate::auth_routes::throttle::SharedThrottler;
use crate::drive::DriveService;
use crate::repositories::drive::{
    DriveCrumb, DriveError, DriveMedia, DriveNode, DriveNodeKind, Paging,
};
use crate::repositories::drive_shares::{DriveShare, DriveShareRepository, ShareAccess};
use crate::repositories::identity::IdentityRepository;
use crate::task_routes::{ApiError, ApiJson, ApiQuery, authenticate_session, request_id_value};

type RequestIdExtension = Option<Extension<RequestId>>;

/// How long the pass from a link's password works.
const PASS_LIFETIME_MS: i64 = 60 * 60 * 1000;
/// The header that carries the pass on JSON requests (downloads carry it in `pass=`).
const PASS_HEADER: &str = "x-orbit-share-pass";

#[derive(Clone)]
pub struct DriveShareState {
    identity: Arc<IdentityRepository>,
    drive: DriveService,
    shares: DriveShareRepository,
    passwords: PasswordExecutor,
    throttler: Arc<SharedThrottler>,
    passes: SharePasses,
    cookie_mode: CookieMode,
    public_origin: String,
}

impl DriveShareState {
    /// `app_key` keys the passes of links with a password.
    pub fn new(
        identity: Arc<IdentityRepository>,
        drive: DriveService,
        app_key: &[u8; 32],
        public_origin: String,
        cookie_mode: CookieMode,
    ) -> Self {
        let database = identity.database().clone();
        Self {
            shares: DriveShareRepository::new(database.clone()),
            passwords: PasswordExecutor::new(PasswordService::default(), 2)
                .expect("two parallel password checks are a valid limit"),
            throttler: Arc::new(SharedThrottler::new("drive_share", database)),
            passes: SharePasses::new(app_key),
            identity,
            drive,
            cookie_mode,
            public_origin: public_origin.trim_end_matches('/').to_owned(),
        }
    }
}

pub fn drive_share_router(state: DriveShareState) -> Router {
    Router::new()
        .route(
            "/api/v1/workspaces/{workspace_id}/drive/nodes/{node_id}/shares",
            get(list_drive_shares).post(create_drive_share),
        )
        .route(
            "/api/v1/workspaces/{workspace_id}/drive/shares/{share_id}/revoke",
            post(revoke_drive_share),
        )
        .route(
            "/api/v1/public/drive/shares/{token}",
            get(open_public_share),
        )
        .route(
            "/api/v1/public/drive/shares/{token}/unlock",
            post(unlock_public_share),
        )
        .route(
            "/api/v1/public/drive/shares/{token}/folder",
            get(public_share_folder),
        )
        .route(
            "/api/v1/public/drive/shares/{token}/download",
            get(download_public_share),
        )
        .with_state(state)
}

/// Passes of links with a password: base64url of `share id (36 ASCII bytes) || expiry (i64 ms, big-endian) ||
/// HMAC-SHA256`.
#[derive(Clone)]
struct SharePasses {
    key: [u8; 32],
}

impl SharePasses {
    fn new(app_key: &[u8; 32]) -> Self {
        let mut key = [0_u8; 32];
        Hkdf::<Sha256>::new(None, app_key)
            .expand(b"orbit drive share pass v1", &mut key)
            .expect("32 bytes is a valid HKDF-SHA256 output length");
        Self { key }
    }

    fn issue(&self, share_id: Id, expires_at: TimestampMillis) -> String {
        let mut pass = share_id.to_string().into_bytes();
        pass.extend_from_slice(&expires_at.as_millis().to_be_bytes());
        let signature = self.mac(&pass).finalize().into_bytes();
        pass.extend_from_slice(&signature);
        URL_SAFE_NO_PAD.encode(pass)
    }

    fn verify(&self, pass: &str, share_id: Id, now: TimestampMillis) -> bool {
        let Ok(bytes) = URL_SAFE_NO_PAD.decode(pass) else {
            return false;
        };
        if bytes.len() != 36 + 8 + 32 {
            return false;
        }
        let (payload, signature) = bytes.split_at(44);
        if self.mac(payload).verify_slice(signature).is_err() {
            return false;
        }
        let (id, expires_at) = payload.split_at(36);
        let expires_at = i64::from_be_bytes(expires_at.try_into().unwrap_or_default());
        id == share_id.to_string().as_bytes() && now.as_millis() < expires_at
    }

    fn mac(&self, payload: &[u8]) -> Hmac<Sha256> {
        let mut mac =
            <Hmac<Sha256> as Mac>::new_from_slice(&self.key).expect("HMAC accepts any key length");
        mac.update(payload);
        mac
    }
}

/// A new link token and the hash that is stored.
fn new_token() -> (String, String) {
    let mut bytes = [0_u8; 32];
    OsRng.fill_bytes(&mut bytes);
    let token = URL_SAFE_NO_PAD.encode(bytes);
    let hash = token_hash(&token);
    (token, hash)
}

fn token_hash(token: &str) -> String {
    format!("{:x}", Sha256::digest(token.as_bytes()))
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateDriveShare {
    /// The link stops working at this time; absent: it works until it is turned off.
    #[schema(value_type = Option<String>, format = DateTime)]
    expires_at: Option<TimestampMillis>,
    /// 12 to 128 characters; absent: anyone with the link may open it.
    password: Option<String>,
}

/// A new link. `url` holds the token: it is shown only now.
#[derive(Serialize, ToSchema)]
pub struct DriveShareCreated {
    share: DriveShare,
    url: String,
}

#[derive(Serialize, ToSchema)]
pub struct DriveShareList {
    items: Vec<DriveShare>,
}

/// What a link shows of a file or folder: no member, owner or folder above the shared one.
#[derive(Serialize, ToSchema)]
pub struct PublicDriveNode {
    #[schema(value_type = String)]
    id: Id,
    kind: DriveNodeKind,
    name: String,
    size: Option<i64>,
    mime: Option<String>,
    #[schema(value_type = String, format = DateTime)]
    updated_at: TimestampMillis,
    media: Option<DriveMedia>,
    #[serde(skip_serializing_if = "Option::is_none")]
    thumbnail_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    poster_url: Option<String>,
}

impl From<DriveNode> for PublicDriveNode {
    fn from(node: DriveNode) -> Self {
        Self {
            id: node.id,
            kind: node.kind,
            name: node.name,
            size: node.size,
            mime: node.mime,
            updated_at: node.updated_at,
            media: node.media,
            thumbnail_url: node.thumbnail_url,
            poster_url: node.poster_url,
        }
    }
}

/// A link that works. While a password is needed, only `locked` is set.
#[derive(Serialize, ToSchema)]
pub struct PublicDriveShare {
    locked: bool,
    node: Option<PublicDriveNode>,
    #[schema(value_type = Option<String>, format = DateTime)]
    expires_at: Option<TimestampMillis>,
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UnlockDriveShare {
    password: String,
}

/// Send `pass` in the `X-Orbit-Share-Pass` header (and as `pass=` on downloads) until `expires_at`.
#[derive(Serialize, ToSchema)]
pub struct DriveSharePass {
    pass: String,
    #[schema(value_type = String, format = DateTime)]
    expires_at: TimestampMillis,
}

#[derive(Deserialize, IntoParams, ToSchema)]
#[into_params(parameter_in = Query)]
#[serde(deny_unknown_fields)]
pub struct PublicFolderQuery {
    /// A folder below the shared folder; absent: the shared folder.
    folder_id: Option<String>,
    offset: Option<i64>,
    limit: Option<i64>,
}

#[derive(Serialize, ToSchema)]
pub struct PublicDriveFolder {
    /// The folders from the shared one down to this one.
    path: Vec<DriveCrumb>,
    items: Vec<PublicDriveNode>,
    next_offset: Option<i64>,
}

#[derive(Deserialize, IntoParams, ToSchema)]
#[into_params(parameter_in = Query)]
#[serde(deny_unknown_fields)]
pub struct PublicDownloadQuery {
    /// A file below a shared folder; absent: the shared file.
    node_id: Option<String>,
    /// Let the browser show the file instead of saving it.
    #[serde(default)]
    inline: bool,
    /// The pass of a link with a password.
    pass: Option<String>,
}

/// Makes a public link to a file or folder. Every member may; the URL with its token is shown only in this answer.
#[utoipa::path(post, path = "/api/v1/workspaces/{workspace_id}/drive/nodes/{node_id}/shares", params(("workspace_id" = String, Path), ("node_id" = String, Path)), request_body = CreateDriveShare, responses((status = 201, body = DriveShareCreated)))]
async fn create_drive_share(
    State(state): State<DriveShareState>,
    Path((workspace, node)): Path<(String, String)>,
    headers: HeaderMap,
    request_id: RequestIdExtension,
    ApiJson(body): ApiJson<CreateDriveShare>,
) -> Result<(StatusCode, Json<DriveShareCreated>), ApiError> {
    let instance = format!("/api/v1/workspaces/{workspace}/drive/nodes/{node}/shares");
    let request_id = request_id.as_ref();
    let (user_id, workspace_id, node_id) =
        member(&state, &headers, &workspace, &node, &instance, request_id).await?;
    let now = TimestampMillis::now();
    if body
        .expires_at
        .is_some_and(|expires_at| expires_at.as_millis() <= now.as_millis())
    {
        return Err(ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "drive_share_expired",
            "Expiry in the past",
            "The link must expire in the future.",
            &instance,
            request_id,
        ));
    }
    let password_hash = match body.password.filter(|password| !password.is_empty()) {
        Some(password) => Some(state.passwords.hash(password).await.map_err(|error| {
            let detail = match error {
                PasswordError::CommonPassword => "The password is too common. Pick another one.",
                _ => "A link password has 12 to 128 characters.",
            };
            ApiError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "drive_share_invalid_password",
                "Invalid password",
                detail,
                &instance,
                request_id,
            )
        })?),
        None => None,
    };
    let (token, hash) = new_token();
    let share = state
        .shares
        .create(
            workspace_id,
            user_id,
            node_id,
            &hash,
            body.expires_at,
            password_hash,
            request_id_value(request_id),
            now,
        )
        .await
        .map_err(|error| member_problem(error, &instance, request_id))?;
    Ok((
        StatusCode::CREATED,
        Json(DriveShareCreated {
            share,
            url: format!("{}/s/{token}", state.public_origin),
        }),
    ))
}

/// The links of a file or folder, newest first, also those that are off or expired.
#[utoipa::path(get, path = "/api/v1/workspaces/{workspace_id}/drive/nodes/{node_id}/shares", params(("workspace_id" = String, Path), ("node_id" = String, Path)), responses((status = 200, body = DriveShareList)))]
async fn list_drive_shares(
    State(state): State<DriveShareState>,
    Path((workspace, node)): Path<(String, String)>,
    headers: HeaderMap,
    request_id: RequestIdExtension,
) -> Result<Json<DriveShareList>, ApiError> {
    let instance = format!("/api/v1/workspaces/{workspace}/drive/nodes/{node}/shares");
    let request_id = request_id.as_ref();
    let (user_id, workspace_id, node_id) =
        member(&state, &headers, &workspace, &node, &instance, request_id).await?;
    let items = state
        .shares
        .list(workspace_id, user_id, node_id)
        .await
        .map_err(|error| member_problem(error, &instance, request_id))?;
    Ok(Json(DriveShareList { items }))
}

/// Turns a link off at once: its creator, or a member with `drive.shares.manage`.
#[utoipa::path(post, path = "/api/v1/workspaces/{workspace_id}/drive/shares/{share_id}/revoke", params(("workspace_id" = String, Path), ("share_id" = String, Path)), responses((status = 200, body = DriveShare)))]
async fn revoke_drive_share(
    State(state): State<DriveShareState>,
    Path((workspace, share)): Path<(String, String)>,
    headers: HeaderMap,
    request_id: RequestIdExtension,
) -> Result<Json<DriveShare>, ApiError> {
    let instance = format!("/api/v1/workspaces/{workspace}/drive/shares/{share}/revoke");
    let request_id = request_id.as_ref();
    let (user_id, workspace_id, share_id) =
        member(&state, &headers, &workspace, &share, &instance, request_id).await?;
    state
        .shares
        .revoke(
            workspace_id,
            user_id,
            share_id,
            request_id_value(request_id),
            TimestampMillis::now(),
        )
        .await
        .map(Json)
        .map_err(|error| member_problem(error, &instance, request_id))
}

/// Opens a link: the shared file or folder, or only `locked` while a password is needed.
#[utoipa::path(get, path = "/api/v1/public/drive/shares/{token}", params(("token" = String, Path)), responses((status = 200, body = PublicDriveShare)))]
async fn open_public_share(
    State(state): State<DriveShareState>,
    Path(token): Path<String>,
    headers: HeaderMap,
    request_id: RequestIdExtension,
) -> Result<Json<PublicDriveShare>, ApiError> {
    let request_id = request_id.as_ref();
    let access = resolve(&state, &token, request_id).await?;
    let now = TimestampMillis::now();
    if !unlocked(&state, &access, header_pass(&headers), now) {
        return Ok(Json(PublicDriveShare {
            locked: true,
            node: None,
            expires_at: None,
        }));
    }
    let mut node = state
        .shares
        .shared_node(&access)
        .await
        .map_err(|error| public_problem(&error, request_id))?;
    state
        .drive
        .sign_media(access.workspace_id, std::slice::from_mut(&mut node));
    Ok(Json(PublicDriveShare {
        locked: false,
        node: Some(node.into()),
        expires_at: access.expires_at,
    }))
}

/// Checks a link's password and gives a pass for one hour. Attempts are throttled per link and IP address.
#[utoipa::path(post, path = "/api/v1/public/drive/shares/{token}/unlock", params(("token" = String, Path)), request_body = UnlockDriveShare, responses((status = 200, body = DriveSharePass)))]
async fn unlock_public_share(
    State(state): State<DriveShareState>,
    Path(token): Path<String>,
    client_ip: Option<Extension<ClientIp>>,
    request_id: RequestIdExtension,
    ApiJson(body): ApiJson<UnlockDriveShare>,
) -> Result<Json<DriveSharePass>, Response> {
    let request_id = request_id.as_ref();
    let access = resolve(&state, &token, request_id)
        .await
        .map_err(IntoResponse::into_response)?;
    let Some(hash) = access.password_hash.clone() else {
        return Ok(Json(pass(&state, &access)));
    };
    let ip = client_ip.map_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED), |Extension(ip)| ip.0);
    let now = TimestampMillis::now();
    let reservation = match state
        .throttler
        .reserve(&access.share_id.to_string(), ip, now)
        .await
    {
        Ok(reservation) => reservation,
        Err(ThrottleDecision::RetryAfter(delay)) => {
            let mut response = ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "drive_share_throttled",
                "Too many attempts",
                "Too many wrong passwords. Try again later.",
                "/api/v1/public/drive/shares",
                request_id,
            )
            .into_response();
            if let Ok(value) = HeaderValue::from_str(&delay.as_secs().max(1).to_string()) {
                response.headers_mut().insert(header::RETRY_AFTER, value);
            }
            return Err(response);
        }
        Err(ThrottleDecision::Allowed) => unreachable!("allowed admission returns a reservation"),
    };
    let valid = state
        .passwords
        .verify(body.password, hash)
        .await
        .is_ok_and(|verification| verification.valid);
    if !valid {
        state.throttler.finish_failure(reservation, now).await;
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "drive_share_wrong_password",
            "Wrong password",
            "The password is not right.",
            "/api/v1/public/drive/shares",
            request_id,
        )
        .into_response());
    }
    state.throttler.finish_success(reservation).await;
    Ok(Json(pass(&state, &access)))
}

/// A folder of a folder link: the shared folder or one below it, never one above.
#[utoipa::path(get, path = "/api/v1/public/drive/shares/{token}/folder", params(("token" = String, Path), PublicFolderQuery), responses((status = 200, body = PublicDriveFolder)))]
async fn public_share_folder(
    State(state): State<DriveShareState>,
    Path(token): Path<String>,
    ApiQuery(query): ApiQuery<PublicFolderQuery>,
    headers: HeaderMap,
    request_id: RequestIdExtension,
) -> Result<Json<PublicDriveFolder>, ApiError> {
    let request_id = request_id.as_ref();
    let access = unlocked_access(&state, &token, header_pass(&headers), request_id).await?;
    let folder_id = query
        .folder_id
        .map(|id| id.parse::<Id>())
        .transpose()
        .map_err(|_| public_problem(&DriveError::NotFound, request_id))?;
    let (path, mut page) = state
        .shares
        .folder(&access, folder_id, Paging::new(query.offset, query.limit))
        .await
        .map_err(|error| public_problem(&error, request_id))?;
    state.drive.sign_media(access.workspace_id, &mut page.items);
    Ok(Json(PublicDriveFolder {
        path,
        items: page.items.into_iter().map(Into::into).collect(),
        next_offset: page.next_offset,
    }))
}

/// Sends the browser to a 5-minute URL of the file's bytes in the bucket (`302`), and counts the download.
#[utoipa::path(get, path = "/api/v1/public/drive/shares/{token}/download", params(("token" = String, Path), PublicDownloadQuery), responses((status = 302, description = "Redirect to the file in the bucket")))]
async fn download_public_share(
    State(state): State<DriveShareState>,
    Path(token): Path<String>,
    ApiQuery(query): ApiQuery<PublicDownloadQuery>,
    request_id: RequestIdExtension,
) -> Result<Response, ApiError> {
    let request_id = request_id.as_ref();
    let access = unlocked_access(&state, &token, query.pass.as_deref(), request_id).await?;
    let node_id = match query.node_id {
        Some(id) => id
            .parse::<Id>()
            .map_err(|_| public_problem(&DriveError::NotFound, request_id))?,
        None => access.node_id,
    };
    let object = state
        .shares
        .object(&access, node_id)
        .await
        .map_err(|error| public_problem(&error, request_id))?;
    let url = state
        .drive
        .object_url(
            access.workspace_id,
            &object.r2_key,
            (!query.inline).then_some(object.name.as_str()),
        )
        .map_err(|error| public_problem(&error, request_id))?;
    if !query.inline
        && let Err(error) = state.shares.count_download(access.share_id).await
    {
        tracing::warn!(%error, "counting a Drive link download");
    }
    let location = HeaderValue::from_str(&url)
        .map_err(|_| public_problem(&DriveError::NotFound, request_id))?;
    Ok((
        StatusCode::FOUND,
        [
            (header::LOCATION, location),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
            (
                header::REFERRER_POLICY,
                HeaderValue::from_static("no-referrer"),
            ),
        ],
    )
        .into_response())
}

fn pass(state: &DriveShareState, access: &ShareAccess) -> DriveSharePass {
    let now = TimestampMillis::now().as_millis();
    // Never past the link's own expiry.
    let expires_at =
        TimestampMillis::from_millis(access.expires_at.map_or(now + PASS_LIFETIME_MS, |expiry| {
            expiry.as_millis().min(now + PASS_LIFETIME_MS)
        }));
    DriveSharePass {
        pass: state.passes.issue(access.share_id, expires_at),
        expires_at,
    }
}

fn header_pass(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(PASS_HEADER)
        .and_then(|value| value.to_str().ok())
}

fn unlocked(
    state: &DriveShareState,
    access: &ShareAccess,
    pass: Option<&str>,
    now: TimestampMillis,
) -> bool {
    access.password_hash.is_none()
        || pass.is_some_and(|pass| state.passes.verify(pass, access.share_id, now))
}

async fn resolve(
    state: &DriveShareState,
    token: &str,
    request_id: Option<&Extension<RequestId>>,
) -> Result<ShareAccess, ApiError> {
    // A real token is 43 base64url characters; anything else is not looked up.
    if token.len() != 43
        || !token
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(public_problem(&DriveError::NotFound, request_id));
    }
    state
        .shares
        .resolve(&token_hash(token), TimestampMillis::now())
        .await
        .map_err(|error| public_problem(&error, request_id))?
        .ok_or_else(|| public_problem(&DriveError::NotFound, request_id))
}

async fn unlocked_access(
    state: &DriveShareState,
    token: &str,
    pass: Option<&str>,
    request_id: Option<&Extension<RequestId>>,
) -> Result<ShareAccess, ApiError> {
    let access = resolve(state, token, request_id).await?;
    if unlocked(state, &access, pass, TimestampMillis::now()) {
        Ok(access)
    } else {
        Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "drive_share_locked",
            "Password needed",
            "This link needs its password.",
            "/api/v1/public/drive/shares",
            request_id,
        ))
    }
}

/// The session's user, the workspace and the id in the path.
async fn member(
    state: &DriveShareState,
    headers: &HeaderMap,
    workspace: &str,
    id: &str,
    instance: &str,
    request_id: Option<&Extension<RequestId>>,
) -> Result<(Id, Id, Id), ApiError> {
    let session = authenticate_session(
        &state.identity,
        state.cookie_mode,
        headers,
        instance,
        request_id,
    )
    .await?;
    let not_found = || member_problem(DriveError::NotFound, instance, request_id);
    Ok((
        session.user.id,
        workspace.parse().map_err(|_| not_found())?,
        id.parse().map_err(|_| not_found())?,
    ))
}

fn member_problem(
    error: DriveError,
    instance: &str,
    request_id: Option<&Extension<RequestId>>,
) -> ApiError {
    match error {
        DriveError::Forbidden => ApiError::new(
            StatusCode::FORBIDDEN,
            "drive_forbidden",
            "Not allowed",
            "Only the member who made the link, or workspace owners and admins, may turn it off.",
            instance,
            request_id,
        ),
        error => crate::drive_routes::problem(error, instance, request_id),
    }
}

/// The one answer for every link that does not work, whatever the cause.
fn public_problem(error: &DriveError, request_id: Option<&Extension<RequestId>>) -> ApiError {
    if matches!(error, DriveError::Database(_) | DriveError::Storage(_)) {
        tracing::error!(%error, "public Drive link failed");
    }
    ApiError::new(
        StatusCode::NOT_FOUND,
        "drive_share_not_found",
        "Link not available",
        "This link does not exist, was turned off or has expired.",
        "/api/v1/public/drive/shares",
        request_id,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passes_work_for_their_link_until_they_expire() {
        let passes = SharePasses::new(&[7; 32]);
        let share = Id::new_v7();
        let now = TimestampMillis::from_millis(1_000_000);
        let pass = passes.issue(share, TimestampMillis::from_millis(2_000_000));
        assert!(passes.verify(&pass, share, now));
        assert!(!passes.verify(&pass, Id::new_v7(), now), "another link");
        assert!(
            !passes.verify(&pass, share, TimestampMillis::from_millis(2_000_000)),
            "expired"
        );
        assert!(
            !SharePasses::new(&[8; 32]).verify(&pass, share, now),
            "another key"
        );
        let mut forged = URL_SAFE_NO_PAD.decode(&pass).unwrap();
        forged[40] ^= 1;
        assert!(
            !passes.verify(&URL_SAFE_NO_PAD.encode(forged), share, now),
            "changed expiry"
        );
    }

    #[test]
    fn tokens_are_random_and_only_their_hash_is_kept() {
        let (first, hash) = new_token();
        let (second, _) = new_token();
        assert_eq!(first.len(), 43);
        assert_ne!(first, second);
        assert_eq!(hash, token_hash(&first));
        assert_eq!(hash.len(), 64);
        assert!(!hash.contains(&first));
    }
}
