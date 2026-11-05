//! Public Drive links: only the token's hash is kept, a link opens only its file or folder and what is below it,
//! expiry, revoke and trash end it at once, a password gives a pass and wrong ones are throttled, and downloads
//! are redirects to the bucket. Against the in-memory S3 server.

use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use orbit_platform::{
    AuthenticatedUser, Id, ObjectStorage, ObjectStorageState, PasswordService, S3Bucket,
    TestDatabase, TimestampMillis, start_fake_s3,
};
use orbit_server::drive::DriveService;
use orbit_server::drive_routes::{DriveState, drive_router};
use orbit_server::drive_share_routes::{DriveShareState, drive_share_router};
use orbit_server::repositories::identity::{IdentityRepository, SetupRequest};
use serde_json::{Value, json};
use tower::ServiceExt;

const PASSWORD: &str = "correct horse battery staple";

struct Fixture {
    database: TestDatabase,
    identity: Arc<IdentityRepository>,
    app: axum::Router,
    workspace_id: Id,
    owner: String,
}

struct Answer {
    status: StatusCode,
    body: Value,
    location: Option<String>,
}

async fn fixture() -> Fixture {
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
                email: "root@example.com".to_owned(),
                display_name: "Root".to_owned(),
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
    let storage = ObjectStorage::default();
    storage.configure(ObjectStorageState {
        bucket: Some(S3Bucket::new(start_fake_s3().await).unwrap()),
        ..Default::default()
    });
    let drive = DriveService::new((*database).clone(), storage);
    let cookie_mode = orbit_server::auth_routes::CookieMode::secure();
    let app = drive_router(DriveState::new(
        Arc::clone(&identity),
        drive.clone(),
        cookie_mode,
    ))
    .merge(drive_share_router(DriveShareState::new(
        Arc::clone(&identity),
        drive,
        &[7; 32],
        "https://orbit.test".to_owned(),
        cookie_mode,
    )));
    Fixture {
        database,
        identity,
        app,
        workspace_id: setup.workspace_id,
        owner: format!("__Host-orbit_session={}", setup.session.token),
    }
}

impl Fixture {
    async fn call(
        &self,
        method: &str,
        path: &str,
        cookie: Option<&str>,
        pass: Option<&str>,
        body: Option<Value>,
    ) -> Answer {
        let mut request = Request::builder().method(method).uri(path);
        if let Some(cookie) = cookie {
            request = request.header(header::COOKIE, cookie);
        }
        if let Some(pass) = pass {
            request = request.header("x-orbit-share-pass", pass);
        }
        let request = match body {
            Some(body) => request
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string())),
            None => request.body(Body::empty()),
        }
        .unwrap();
        let response = self.app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let location = response
            .headers()
            .get(header::LOCATION)
            .map(|value| value.to_str().unwrap().to_owned());
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        Answer {
            status,
            body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
            location,
        }
    }

    fn drive(&self, path: &str) -> String {
        format!("/api/v1/workspaces/{}/drive{path}", self.workspace_id)
    }

    /// A call by a member that must answer `expected`.
    async fn member_call(
        &self,
        cookie: &str,
        method: &str,
        path: &str,
        body: Option<Value>,
        expected: StatusCode,
    ) -> Value {
        let answer = self
            .call(method, &self.drive(path), Some(cookie), None, body)
            .await;
        assert_eq!(answer.status, expected, "{method} {path}: {}", answer.body);
        answer.body
    }

    async fn folder(&self, parent: Option<&str>, name: &str) -> String {
        self.member_call(
            &self.owner,
            "POST",
            "/folders",
            Some(json!({"parent_id": parent, "name": name})),
            StatusCode::CREATED,
        )
        .await["id"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    async fn file(&self, parent: Option<&str>, name: &str, bytes: &[u8]) -> String {
        let started = self
            .member_call(
                &self.owner,
                "POST",
                "/uploads",
                Some(json!({"parent_id": parent, "name": name, "size": bytes.len(), "mime": "text/plain"})),
                StatusCode::CREATED,
            )
            .await;
        reqwest::Client::new()
            .put(started["url"].as_str().unwrap())
            .body(bytes.to_vec())
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
        let upload = started["upload"]["id"].as_str().unwrap();
        self.member_call(
            &self.owner,
            "POST",
            &format!("/uploads/{upload}/complete"),
            None,
            StatusCode::CREATED,
        )
        .await["id"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    /// Makes a link as `cookie` and returns its id and token.
    async fn share(&self, cookie: &str, node: &str, body: Value) -> (String, String) {
        let created = self
            .member_call(
                cookie,
                "POST",
                &format!("/nodes/{node}/shares"),
                Some(body),
                StatusCode::CREATED,
            )
            .await;
        let url = created["url"].as_str().unwrap();
        let token = url
            .strip_prefix("https://orbit.test/s/")
            .unwrap()
            .to_owned();
        (created["share"]["id"].as_str().unwrap().to_owned(), token)
    }

    async fn public(&self, token: &str, path: &str, pass: Option<&str>) -> Answer {
        self.call(
            "GET",
            &format!("/api/v1/public/drive/shares/{token}{path}"),
            None,
            pass,
            None,
        )
        .await
    }

    async fn member(&self, email: &str, role: &str) -> String {
        let id = Id::new_v7();
        let now = TimestampMillis::now();
        sqlx::query(
            "INSERT INTO users (id, email, normalized_email, display_name, password_hash, created_at, updated_at) \
             VALUES ($1, $2, $2, 'User', 'unused', $3, $3)",
        )
        .bind(id.to_string())
        .bind(email)
        .bind(now.as_millis())
        .execute(self.database.pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO memberships (id, workspace_id, user_id, role, version, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, 0, $5, $5)",
        )
        .bind(Id::new_v7().to_string())
        .bind(self.workspace_id.to_string())
        .bind(id.to_string())
        .bind(role)
        .bind(now.as_millis())
        .execute(self.database.pool())
        .await
        .unwrap();
        let session = self
            .identity
            .create_session(
                &AuthenticatedUser {
                    id,
                    email: email.to_owned(),
                    display_name: "User".to_owned(),
                },
                now,
            )
            .await
            .unwrap();
        format!("__Host-orbit_session={}", session.token)
    }
}

/// A time `offset` milliseconds from now, as the API writes times.
fn at(offset: i64) -> Value {
    serde_json::to_value(TimestampMillis::from_millis(
        TimestampMillis::now().as_millis() + offset,
    ))
    .unwrap()
}

fn not_found(answer: &Answer) -> bool {
    answer.status == StatusCode::NOT_FOUND && answer.body["code"] == "drive_share_not_found"
}

#[tokio::test]
async fn a_file_link_keeps_only_the_token_hash_and_redirects_downloads_to_the_bucket() {
    let fixture = fixture().await;
    let file = fixture.file(None, "Bericht.txt", b"quarterly").await;
    let (share_id, token) = fixture.share(&fixture.owner, &file, json!({})).await;
    assert_eq!(token.len(), 43);
    let stored: String = sqlx::query_scalar("SELECT token_hash FROM drive_shares WHERE id = $1")
        .bind(&share_id)
        .fetch_one(fixture.database.pool())
        .await
        .unwrap();
    assert_ne!(stored, token);
    assert!(!stored.contains(&token));

    let opened = fixture.public(&token, "", None).await;
    assert_eq!(opened.status, StatusCode::OK);
    assert_eq!(opened.body["locked"], false);
    assert_eq!(opened.body["node"]["name"], "Bericht.txt");
    // Nothing about the members or the folders above.
    for private in ["created_by", "updated_by", "parent_id"] {
        assert!(opened.body["node"].get(private).is_none(), "{private}");
    }
    let download = fixture.public(&token, "/download", None).await;
    assert_eq!(download.status, StatusCode::FOUND);
    let response = reqwest::get(download.location.unwrap()).await.unwrap();
    assert!(
        response.headers()["content-disposition"]
            .to_str()
            .unwrap()
            .contains("Bericht.txt")
    );
    assert_eq!(response.bytes().await.unwrap(), b"quarterly".as_slice());
    let listed = fixture
        .member_call(
            &fixture.owner,
            "GET",
            &format!("/nodes/{file}/shares"),
            None,
            StatusCode::OK,
        )
        .await;
    assert_eq!(listed["items"][0]["download_count"], 1);
    assert_eq!(listed["items"][0]["has_password"], false);

    // Wrong, cut or made-up tokens all look the same.
    let mut wrong = token.clone();
    wrong.replace_range(0..1, if token.starts_with('A') { "B" } else { "A" });
    for token in [wrong.as_str(), &token[..42], "not-a-token"] {
        assert!(not_found(&fixture.public(token, "", None).await), "{token}");
        assert!(
            not_found(&fixture.public(token, "/download", None).await),
            "{token}"
        );
    }
    // A file link has no folder.
    assert!(not_found(&fixture.public(&token, "/folder", None).await));
}

#[tokio::test]
async fn a_folder_link_reaches_only_its_folder_and_what_is_below() {
    let fixture = fixture().await;
    let top = fixture.folder(None, "Top").await;
    let shared = fixture.folder(Some(&top), "Shared").await;
    let inner = fixture.folder(Some(&shared), "Inner").await;
    let inside = fixture.file(Some(&inner), "inside.txt", b"in").await;
    let sibling = fixture.file(Some(&top), "secret.txt", b"secret").await;
    let other = fixture.folder(Some(&top), "Other").await;
    let trashed = fixture.file(Some(&shared), "gone.txt", b"gone").await;
    fixture
        .member_call(
            &fixture.owner,
            "POST",
            &format!("/nodes/{trashed}/trash"),
            None,
            StatusCode::NO_CONTENT,
        )
        .await;
    let (_, token) = fixture.share(&fixture.owner, &shared, json!({})).await;

    let root = fixture.public(&token, "/folder", None).await;
    assert_eq!(root.status, StatusCode::OK);
    assert_eq!(root.body["path"], json!([{"id": shared, "name": "Shared"}]));
    let names: Vec<&str> = root.body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["Inner"]);
    let below = fixture
        .public(&token, &format!("/folder?folder_id={inner}"), None)
        .await;
    assert_eq!(
        below.body["path"],
        json!([{"id": shared, "name": "Shared"}, {"id": inner, "name": "Inner"}])
    );
    assert_eq!(below.body["items"][0]["name"], "inside.txt");
    let download = fixture
        .public(&token, &format!("/download?node_id={inside}"), None)
        .await;
    assert_eq!(download.status, StatusCode::FOUND);

    // Nothing above or beside the shared folder, nothing trashed, and no folder as a download.
    for folder in [&top, &other] {
        assert!(not_found(
            &fixture
                .public(&token, &format!("/folder?folder_id={folder}"), None)
                .await
        ));
    }
    for node in [&sibling, &trashed, &inner, &shared] {
        assert!(
            not_found(
                &fixture
                    .public(&token, &format!("/download?node_id={node}"), None)
                    .await
            ),
            "{node}"
        );
    }
    assert!(not_found(
        &fixture
            .public(&token, &format!("/folder?folder_id={inside}"), None)
            .await
    ));
}

#[tokio::test]
async fn expiry_revoke_and_trash_end_a_link_at_once() {
    let fixture = fixture().await;
    let file = fixture.file(None, "a.txt", b"a").await;
    let past = at(-60_000);
    let problem = fixture
        .member_call(
            &fixture.owner,
            "POST",
            &format!("/nodes/{file}/shares"),
            Some(json!({"expires_at": past})),
            StatusCode::UNPROCESSABLE_ENTITY,
        )
        .await;
    assert_eq!(problem["code"], "drive_share_expired");

    let soon = at(3_600_000);
    let (expiring_id, expiring) = fixture
        .share(&fixture.owner, &file, json!({"expires_at": soon}))
        .await;
    assert_eq!(
        fixture.public(&expiring, "", None).await.status,
        StatusCode::OK
    );
    sqlx::query("UPDATE drive_shares SET expires_at = $1 WHERE id = $2")
        .bind(TimestampMillis::now().as_millis() - 1)
        .bind(&expiring_id)
        .execute(fixture.database.pool())
        .await
        .unwrap();
    assert!(not_found(&fixture.public(&expiring, "", None).await));
    assert!(not_found(
        &fixture.public(&expiring, "/download", None).await
    ));

    // Only the creator or a manager turns a link off.
    let member = fixture.member("member@example.com", "member").await;
    let other = fixture.member("other@example.com", "member").await;
    let admin = fixture.member("admin@example.com", "admin").await;
    let (mine, my_token) = fixture.share(&member, &file, json!({})).await;
    let (owners, owners_token) = fixture.share(&fixture.owner, &file, json!({})).await;
    let listed = fixture
        .member_call(
            &other,
            "GET",
            &format!("/nodes/{file}/shares"),
            None,
            StatusCode::OK,
        )
        .await;
    assert!(
        listed["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|share| share["can_revoke"] == false)
    );
    let problem = fixture
        .member_call(
            &other,
            "POST",
            &format!("/shares/{mine}/revoke"),
            None,
            StatusCode::FORBIDDEN,
        )
        .await;
    assert_eq!(problem["code"], "drive_forbidden");
    assert_eq!(
        fixture.public(&my_token, "", None).await.status,
        StatusCode::OK
    );
    let revoked = fixture
        .member_call(
            &member,
            "POST",
            &format!("/shares/{mine}/revoke"),
            None,
            StatusCode::OK,
        )
        .await;
    assert!(revoked["revoked_at"].is_string());
    assert!(not_found(&fixture.public(&my_token, "", None).await));
    fixture
        .member_call(
            &admin,
            "POST",
            &format!("/shares/{owners}/revoke"),
            None,
            StatusCode::OK,
        )
        .await;
    assert!(not_found(
        &fixture.public(&owners_token, "/download", None).await
    ));

    // A link to a trashed file stops, and comes back with the file.
    let (_, live) = fixture.share(&fixture.owner, &file, json!({})).await;
    fixture
        .member_call(
            &fixture.owner,
            "POST",
            &format!("/nodes/{file}/trash"),
            None,
            StatusCode::NO_CONTENT,
        )
        .await;
    assert!(not_found(&fixture.public(&live, "", None).await));
    fixture
        .member_call(
            &fixture.owner,
            "POST",
            &format!("/nodes/{file}/restore"),
            None,
            StatusCode::OK,
        )
        .await;
    assert_eq!(fixture.public(&live, "", None).await.status, StatusCode::OK);

    // Outsiders cannot make, see or turn off links.
    let outsider_cookie = {
        let id = Id::new_v7();
        let now = TimestampMillis::now();
        sqlx::query(
            "INSERT INTO users (id, email, normalized_email, display_name, password_hash, created_at, updated_at) \
             VALUES ($1, 'out@example.com', 'out@example.com', 'Out', 'unused', $2, $2)",
        )
        .bind(id.to_string())
        .bind(now.as_millis())
        .execute(fixture.database.pool())
        .await
        .unwrap();
        let session = fixture
            .identity
            .create_session(
                &AuthenticatedUser {
                    id,
                    email: "out@example.com".to_owned(),
                    display_name: "Out".to_owned(),
                },
                now,
            )
            .await
            .unwrap();
        format!("__Host-orbit_session={}", session.token)
    };
    for (method, path) in [
        ("POST", format!("/nodes/{file}/shares")),
        ("GET", format!("/nodes/{file}/shares")),
        ("POST", format!("/shares/{expiring_id}/revoke")),
    ] {
        let answer = fixture
            .call(
                method,
                &fixture.drive(&path),
                Some(&outsider_cookie),
                None,
                Some(json!({})),
            )
            .await;
        assert_eq!(answer.status, StatusCode::NOT_FOUND, "{method} {path}");
        assert!(!answer.body.to_string().contains("/s/"));
    }
}

#[tokio::test]
async fn a_password_link_opens_only_with_a_pass_and_throttles_wrong_passwords() {
    let fixture = fixture().await;
    let folder = fixture.folder(None, "Private").await;
    let file = fixture.file(Some(&folder), "plan.txt", b"plan").await;
    let problem = fixture
        .member_call(
            &fixture.owner,
            "POST",
            &format!("/nodes/{folder}/shares"),
            Some(json!({"password": "short"})),
            StatusCode::UNPROCESSABLE_ENTITY,
        )
        .await;
    assert_eq!(problem["code"], "drive_share_invalid_password");
    let (share_id, token) = fixture
        .share(&fixture.owner, &folder, json!({"password": PASSWORD}))
        .await;
    let (_, other_token) = fixture
        .share(&fixture.owner, &folder, json!({"password": PASSWORD}))
        .await;
    let stored: String = sqlx::query_scalar("SELECT password_hash FROM drive_shares WHERE id = $1")
        .bind(&share_id)
        .fetch_one(fixture.database.pool())
        .await
        .unwrap();
    assert!(stored.starts_with("$argon2"));

    let locked = fixture.public(&token, "", None).await;
    assert_eq!(
        locked.body,
        json!({"locked": true, "node": null, "expires_at": null})
    );
    for path in ["/folder".to_owned(), format!("/download?node_id={file}")] {
        let answer = fixture.public(&token, &path, None).await;
        assert_eq!(answer.status, StatusCode::UNAUTHORIZED, "{path}");
        assert_eq!(answer.body["code"], "drive_share_locked");
    }

    let unlock = |token: String, password: &'static str| {
        let fixture = &fixture;
        async move {
            fixture
                .call(
                    "POST",
                    &format!("/api/v1/public/drive/shares/{token}/unlock"),
                    None,
                    None,
                    Some(json!({"password": password})),
                )
                .await
        }
    };
    let wrong = unlock(token.clone(), "not the password at all").await;
    assert_eq!(wrong.status, StatusCode::FORBIDDEN);
    assert_eq!(wrong.body["code"], "drive_share_wrong_password");
    let pass = unlock(token.clone(), PASSWORD).await;
    assert_eq!(pass.status, StatusCode::OK);
    let pass = pass.body["pass"].as_str().unwrap().to_owned();

    let opened = fixture.public(&token, "", Some(&pass)).await;
    assert_eq!(opened.body["node"]["name"], "Private");
    assert_eq!(
        fixture.public(&token, "/folder", Some(&pass)).await.status,
        StatusCode::OK
    );
    let download = fixture
        .public(
            &token,
            &format!("/download?node_id={file}&pass={pass}"),
            None,
        )
        .await;
    assert_eq!(download.status, StatusCode::FOUND);
    // A pass opens only its own link.
    assert_eq!(
        fixture
            .public(&other_token, "/folder", Some(&pass))
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    // A revoked link takes its passes with it.
    fixture
        .member_call(
            &fixture.owner,
            "POST",
            &format!("/shares/{share_id}/revoke"),
            None,
            StatusCode::OK,
        )
        .await;
    assert!(not_found(
        &fixture.public(&token, "/folder", Some(&pass)).await
    ));

    // Wrong passwords on one link are throttled like sign-in.
    let mut throttled = None;
    for attempt in 1..=10 {
        let answer = unlock(other_token.clone(), "not the password at all").await;
        if answer.status == StatusCode::TOO_MANY_REQUESTS {
            throttled = Some(attempt);
            assert_eq!(answer.body["code"], "drive_share_throttled");
            break;
        }
        assert_eq!(answer.status, StatusCode::FORBIDDEN);
    }
    assert!(
        throttled.is_some(),
        "no throttling after 10 wrong passwords"
    );
    // While throttled, even the right password waits.
    assert_eq!(
        unlock(other_token, PASSWORD).await.status,
        StatusCode::TOO_MANY_REQUESTS
    );
}
