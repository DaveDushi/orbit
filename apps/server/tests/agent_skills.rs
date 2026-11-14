//! Agent Skills: only owners and admins manage them (members and guests get 403, agent sessions
//! never reach them); each agent has its own list; an agent's session lists only the name and
//! description of its own skills and loads only those; zips and GitHub downloads are checked
//! (paths, links, sizes, local addresses, redirects). GitHub is a local mock: no real network.
//! Runs on SQLite, and on Postgres with `ORBIT_TEST_POSTGRES_URL`.

use std::io::{Cursor, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use axum::response::IntoResponse;
use orbit_platform::{
    AttachmentMutationCoordinator, AuthenticatedUser, Id, LocalBlobStore, PasswordService,
    TestDatabase, TimestampMillis, UploadLimits, UploadService,
};
use orbit_server::agent::{AgentConfig, AgentState};
use orbit_server::auth_routes::CookieMode;
use orbit_server::chat_routes::{ChatState, chat_router};
use orbit_server::repositories::identity::{IdentityRepository, SetupRequest};
use serde_json::{Value, json};
use tower::ServiceExt;
use zip::write::SimpleFileOptions;

const SECRET: &str = "test-agent-secret-0123456789abcdef0123456789";

struct Fixture {
    _root: tempfile::TempDir,
    database: TestDatabase,
    identity: Arc<IdentityRepository>,
    app: axum::Router,
    workspace_id: String,
    owner: Member,
    nonce: AtomicU64,
}

#[derive(Clone)]
struct Member {
    id: Id,
    cookie: String,
}

impl Fixture {
    /// `github`: the mock GitHub's base URL, and whether 127.0.0.1 is a test host (else the
    /// guard must refuse it). Tests never use the real GitHub.
    async fn new(github: Option<(&str, bool)>) -> Self {
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
        let mut agent = AgentState::new(
            Some(AgentConfig {
                url: "http://127.0.0.1:9".to_owned(),
                secret: SECRET.to_owned(),
            }),
            chat.clone(),
            Arc::clone(&identity),
            CookieMode::secure(),
            Some([3; 32]),
        );
        if let Some((github, test_host)) = github {
            if test_host {
                agent = agent.with_integration_test_hosts(&["127.0.0.1"]);
            }
            agent = agent.with_skill_sources(&format!("{github}/api"), &format!("{github}/raw"));
        }
        let app = chat_router(chat)
            .merge(orbit_server::agent::router(agent))
            .layer(axum::middleware::from_fn_with_state(
                (*database).clone(),
                orbit_server::agent::scope::layer,
            ));
        Self {
            _root: root,
            database,
            identity,
            app,
            workspace_id: setup.workspace_id.to_string(),
            owner: Member {
                id: setup.user_id,
                cookie: format!("__Host-orbit_session={}", setup.session.token),
            },
            nonce: AtomicU64::new(0),
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

    async fn send_request(&self, request: Request<Body>) -> (StatusCode, Value) {
        let response = self.app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    async fn request(
        &self,
        method: &str,
        uri: &str,
        auth: (&str, &str),
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let builder = Request::builder()
            .method(method)
            .uri(uri)
            .header(auth.0, auth.1);
        let request = match body {
            Some(value) => builder
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(value.to_string())),
            None => builder.body(Body::empty()),
        }
        .unwrap();
        self.send_request(request).await
    }

    async fn api(
        &self,
        member: &Member,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let uri = format!("/api/v1/workspaces/{}{path}", self.workspace_id);
        self.request(method, &uri, ("cookie", &member.cookie), body)
            .await
    }

    async fn upload(&self, member: &Member, zip: &[u8]) -> (StatusCode, Value) {
        let boundary = "skill-boundary-1234";
        let mut body = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"skill.zip\"\r\nContent-Type: application/zip\r\n\r\n"
        )
        .into_bytes();
        body.extend_from_slice(zip);
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        let request = Request::builder()
            .method("POST")
            .uri(format!(
                "/api/v1/workspaces/{}/agent-skills/upload",
                self.workspace_id
            ))
            .header("cookie", &member.cookie)
            .header(
                header::CONTENT_TYPE,
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(Body::from(body))
            .unwrap();
        self.send_request(request).await
    }

    async fn internal(&self, method: &str, path: &str, body: Option<Value>) -> (StatusCode, Value) {
        self.request(
            method,
            &format!("/api/internal/agent{path}"),
            ("authorization", &format!("Bearer {SECRET}")),
            body,
        )
        .await
    }

    async fn general(&self) -> String {
        let (status, conversations) = self
            .api(&self.owner, "GET", "/chat/conversations", None)
            .await;
        assert_eq!(status, StatusCode::OK);
        conversations
            .as_array()
            .unwrap()
            .iter()
            .find(|conversation| conversation["is_default"] == true)
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    /// The default agent's bot user (made by the first read of the inputs).
    async fn default_bot(&self) -> String {
        let (status, value) = self.internal("GET", "/inputs", None).await;
        assert_eq!(status, StatusCode::OK, "{value}");
        value["bot_user_id"].as_str().unwrap().to_owned()
    }

    /// An agent session of `member` for `bot` (`None`: the default agent), in a running thread.
    async fn agent_token(&self, member: &Member, bot: &str, custom: bool) -> String {
        let general = self.general().await;
        let nonce = format!("n{}", self.nonce.fetch_add(1, Ordering::Relaxed));
        let (status, sent) = self
            .api(
                member,
                "POST",
                &format!("/chat/conversations/{general}/messages"),
                Some(json!({ "body": format!("<@{bot}> hello"), "nonce": nonce })),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{sent}");
        let (_, value) = self.internal("GET", "/inputs", None).await;
        let input = value["inputs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|input| input["message_id"] == sent["result"]["id"])
            .unwrap()
            .clone();
        let (status, _) = self
            .internal(
                "PUT",
                &format!("/threads/{}", input["thread_key"].as_str().unwrap()),
                Some(json!({
                    "workspace_id": self.workspace_id,
                    "conversation_id": input["conversation_id"],
                    "root_message_id": input["root_message_id"],
                    "status": "running",
                    "driver_user_id": member.id.to_string(),
                    "waiting_for_user_id": null,
                    "agent_user_id": if custom { Some(bot) } else { None },
                })),
            )
            .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (status, session) = self
            .internal(
                "POST",
                "/sessions",
                Some(json!({
                    "user_id": member.id.to_string(),
                    "workspace_id": self.workspace_id,
                    "agent_user_id": if custom { Some(bot) } else { None },
                })),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{session}");
        format!("Bearer {}", session["token"].as_str().unwrap())
    }

    async fn as_agent(&self, token: &str, path: &str) -> (StatusCode, Value) {
        let uri = format!("/api/v1/workspaces/{}{path}", self.workspace_id);
        self.request("GET", &uri, ("authorization", token), None)
            .await
    }

    async fn write_skill(&self, name: &str, description: &str, body: &str) -> String {
        let (status, skill) = self
            .api(
                &self.owner,
                "POST",
                "/agent-skills",
                Some(json!({ "name": name, "description": description, "body": body })),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{skill}");
        skill["id"].as_str().unwrap().to_owned()
    }

    async fn turn_on(&self, skill: &str, agents: &[&str]) {
        let (status, value) = self
            .api(
                &self.owner,
                "PATCH",
                &format!("/agent-skills/{skill}"),
                Some(json!({ "agents": agents })),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{value}");
    }
}

fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, content) in entries {
        writer
            .start_file(*name, SimpleFileOptions::default())
            .unwrap();
        writer.write_all(content).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

const PDF_SKILL: &[u8] = b"---\nname: pdf-forms\ndescription: Fill PDF forms. Use when someone sends a form.\nlicense: MIT\n---\n# PDF forms\n\n1. Read references/fields.md.\n2. Run scripts/fill.sh.\n";

#[tokio::test]
async fn only_owners_and_admins_manage_skills() {
    let fixture = Fixture::new(None).await;
    let member = fixture.add_person("Mia", "member").await;
    let guest = fixture.add_person("Gus", "guest").await;
    let admin = fixture.add_person("Ada", "admin").await;
    let id = fixture
        .write_skill("weekly-report", "Write the weekly report.", "Steps")
        .await;
    let zip = zip_of(&[("SKILL.md", PDF_SKILL)]);
    for person in [&member, &guest] {
        for (method, path, body) in [
            ("GET", "/agent-skills".to_owned(), None),
            ("GET", format!("/agent-skills/{id}"), None),
            (
                "POST",
                "/agent-skills".to_owned(),
                Some(json!({ "name": "x", "description": "x", "body": "x" })),
            ),
            (
                "POST",
                "/agent-skills/install".to_owned(),
                Some(json!({ "source": "acme/skills@pdf" })),
            ),
            (
                "PATCH",
                format!("/agent-skills/{id}"),
                Some(json!({ "agents": ["default"] })),
            ),
            ("DELETE", format!("/agent-skills/{id}"), None),
        ] {
            let (status, problem) = fixture.api(person, method, &path, body).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{method} {path}: {problem}");
        }
        let (status, _) = fixture.upload(person, &zip).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }
    let (status, list) = fixture.api(&admin, "GET", "/agent-skills", None).await;
    assert_eq!(status, StatusCode::OK, "{list}");
    assert_eq!(list["skills"][0]["name"], "weekly-report");
    assert_eq!(list["agents"][0]["key"], "default");
    // The sidecar is offline here: nobody knows whether scripts can run.
    assert_eq!(list["sandbox"], Value::Null);
    let (status, _) = fixture.upload(&admin, &zip).await;
    assert_eq!(status, StatusCode::OK);

    // Names follow the spec and are unique in the workspace.
    let (status, problem) = fixture
        .api(
            &admin,
            "POST",
            "/agent-skills",
            Some(json!({ "name": "Weekly Report", "description": "x", "body": "x" })),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
    let (status, problem) = fixture
        .api(
            &admin,
            "POST",
            "/agent-skills",
            Some(json!({ "name": "weekly-report", "description": "x", "body": "x" })),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(problem["code"], "skill_exists");

    // An agent session never reaches the management routes, whatever its driver may do.
    let bot = fixture.default_bot().await;
    let token = fixture.agent_token(&fixture.owner, &bot, false).await;
    let (status, problem) = fixture.as_agent(&token, "/agent-skills").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(problem["code"], "agent_scope");

    let (status, _) = fixture
        .api(&admin, "DELETE", &format!("/agent-skills/{id}"), None)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let audits: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM audit_events WHERE action LIKE 'agent_skill.%'")
            .fetch_one(fixture.database.pool())
            .await
            .unwrap();
    assert_eq!(audits, 3);
}

#[tokio::test]
async fn each_agent_lists_and_loads_only_its_own_skills() {
    let fixture = Fixture::new(None).await;
    let member = fixture.add_person("Mia", "member").await;
    let bot = fixture.default_bot().await;
    let (status, writer) = fixture
        .api(
            &fixture.owner,
            "POST",
            "/agents",
            Some(json!({ "name": "Writer", "tools": ["read_page"], "all_channels": true })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{writer}");
    let writer_bot = writer["user_id"].as_str().unwrap().to_owned();

    let (status, uploaded) = fixture
        .upload(
            &fixture.owner,
            &zip_of(&[
                ("pdf-forms/SKILL.md", PDF_SKILL),
                ("pdf-forms/references/fields.md", b"Name, Date"),
                ("pdf-forms/scripts/fill.sh", b"#!/bin/sh\necho filled"),
                ("pdf-forms/assets/logo.bin", &[0, 159, 146, 150]),
            ]),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{uploaded}");
    assert_eq!(uploaded["has_scripts"], true);
    let pdf = uploaded["id"].as_str().unwrap().to_owned();
    let voice = fixture
        .write_skill(
            "brand-voice",
            "Write like us.",
            "Short sentences. SECRET-BODY",
        )
        .await;
    let unused = fixture
        .write_skill("unused", "Nobody has it.", "Never shown")
        .await;
    fixture.turn_on(&pdf, &["default"]).await;
    fixture.turn_on(&voice, &[&writer_bot]).await;
    // An agent of another workspace or a made-up one is refused.
    let (status, _) = fixture
        .api(
            &fixture.owner,
            "PATCH",
            &format!("/agent-skills/{unused}"),
            Some(json!({ "agents": [Id::new_v7().to_string()] })),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    // The default agent: its list has names and descriptions only, never the body.
    let token = fixture.agent_token(&member, &bot, false).await;
    let (status, list) = fixture.as_agent(&token, "/agent-tools/skills").await;
    assert_eq!(status, StatusCode::OK, "{list}");
    assert_eq!(
        list,
        json!({ "skills": [{ "name": "pdf-forms", "description": "Fill PDF forms. Use when someone sends a form.", "has_scripts": true }] })
    );
    let (status, loaded) = fixture
        .as_agent(&token, "/agent-tools/skills/pdf-forms")
        .await;
    assert_eq!(status, StatusCode::OK, "{loaded}");
    assert!(loaded["body"].as_str().unwrap().starts_with("# PDF forms"));
    assert_eq!(loaded["files"].as_array().unwrap().len(), 3);
    let (status, file) = fixture
        .as_agent(
            &token,
            "/agent-tools/skills/pdf-forms/files/references/fields.md",
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(file["text"], "Name, Date");
    let (_, binary) = fixture
        .as_agent(
            &token,
            "/agent-tools/skills/pdf-forms/files/assets/logo.bin",
        )
        .await;
    assert_eq!(binary["base64"], "AJ+Slg==");
    for path in [
        "/agent-tools/skills/brand-voice",
        "/agent-tools/skills/unused",
        "/agent-tools/skills/brand-voice/files/SKILL.md",
    ] {
        let (status, problem) = fixture.as_agent(&token, path).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}: {problem}");
    }

    // The custom agent has only its own skill.
    let writer_token = fixture.agent_token(&member, &writer_bot, true).await;
    let (_, list) = fixture.as_agent(&writer_token, "/agent-tools/skills").await;
    let names: Vec<&str> = list["skills"]
        .as_array()
        .unwrap()
        .iter()
        .map(|skill| skill["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["brand-voice"]);
    assert!(!list.to_string().contains("SECRET-BODY"));
    let (status, _) = fixture
        .as_agent(&writer_token, "/agent-tools/skills/pdf-forms")
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, loaded) = fixture
        .as_agent(&writer_token, "/agent-tools/skills/brand-voice")
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(loaded["body"], "Short sentences. SECRET-BODY");

    // A person's own session does not read skills this way.
    let (status, problem) = fixture
        .api(&member, "GET", "/agent-tools/skills/pdf-forms", None)
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{problem}");

    // Turned off: gone from the list at once. Deleted: the files and the agent rows go too.
    fixture.turn_on(&pdf, &[]).await;
    let (_, list) = fixture.as_agent(&token, "/agent-tools/skills").await;
    assert_eq!(list["skills"], json!([]));
    let (status, _) = fixture
        .api(
            &fixture.owner,
            "DELETE",
            &format!("/agent-skills/{voice}"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let left: i64 = sqlx::query_scalar(
        "SELECT (SELECT COUNT(*) FROM agent_skill_agents) + (SELECT COUNT(*) FROM agent_skill_files WHERE skill_id = $1)",
    )
    .bind(&voice)
    .fetch_one(fixture.database.pool())
    .await
    .unwrap();
    assert_eq!(left, 0);

    // A deleted agent reads no skills.
    fixture.turn_on(&unused, &[&writer_bot]).await;
    let (status, _) = fixture
        .api(
            &fixture.owner,
            "DELETE",
            &format!("/agents/{}", writer["id"].as_str().unwrap()),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = fixture.as_agent(&writer_token, "/agent-tools/skills").await;
    assert_ne!(status, StatusCode::OK);
}

#[tokio::test]
async fn bad_zips_are_refused_and_nothing_is_stored() {
    let fixture = Fixture::new(None).await;
    let mut symlink = zip::ZipWriter::new(Cursor::new(Vec::new()));
    symlink
        .start_file("SKILL.md", SimpleFileOptions::default())
        .unwrap();
    symlink.write_all(PDF_SKILL).unwrap();
    symlink
        .add_symlink("scripts/key", "/etc/shadow", SimpleFileOptions::default())
        .unwrap();
    let symlink = symlink.finish().unwrap().into_inner();
    let big = vec![b'a'; 600 * 1024];
    for (zip, expected) in [
        (
            zip_of(&[("SKILL.md", PDF_SKILL), ("../../evil.sh", b"x")]),
            "not allowed",
        ),
        (
            zip_of(&[("SKILL.md", PDF_SKILL), ("/etc/cron.d/x", b"x")]),
            "not allowed",
        ),
        (symlink, "symbolic link"),
        (
            zip_of(&[("SKILL.md", PDF_SKILL), ("big.bin", &big)]),
            "too large",
        ),
        (zip_of(&[("README.md", b"no skill")]), "no SKILL.md"),
        (
            zip_of(&[("SKILL.md", b"---\nname: Bad Name\ndescription: x\n---\nx")]),
            "not valid",
        ),
        (b"not a zip at all".to_vec(), "not a valid .zip"),
    ] {
        let (status, problem) = fixture.upload(&fixture.owner, &zip).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
        assert!(
            problem["detail"].as_str().unwrap().contains(expected),
            "{expected}: {problem}"
        );
    }
    let stored: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_skills")
        .fetch_one(fixture.database.pool())
        .await
        .unwrap();
    assert_eq!(stored, 0);
}

/// A tiny GitHub: the tree API and raw files of `acme/skills`, a moved repository and a huge one.
async fn mock_github() -> String {
    use axum::extract::Path;
    use axum::routing::get;
    let tree = json!({
        "tree": [
            { "path": "README.md", "type": "blob", "mode": "100644", "size": 5 },
            { "path": "skills", "type": "tree", "mode": "040000" },
            { "path": "skills/pdf-forms", "type": "tree", "mode": "040000" },
            { "path": "skills/pdf-forms/SKILL.md", "type": "blob", "mode": "100644", "size": PDF_SKILL.len() },
            { "path": "skills/pdf-forms/scripts/fill.sh", "type": "blob", "mode": "100755", "size": 7 },
            { "path": "skills/linked/SKILL.md", "type": "blob", "mode": "100644", "size": 10 },
            { "path": "skills/linked/data", "type": "blob", "mode": "120000", "size": 11 },
            { "path": "skills/huge/SKILL.md", "type": "blob", "mode": "100644", "size": 10 },
            { "path": "skills/huge/blob.bin", "type": "blob", "mode": "100644", "size": 900_000 },
        ],
        "truncated": false,
    });
    let app = axum::Router::new()
        .route(
            "/api/repos/{owner}/{repo}/git/trees/{reference}",
            get(
                move |Path((owner, repo, reference)): Path<(String, String, String)>| {
                    let tree = tree.clone();
                    async move {
                        match (owner.as_str(), repo.as_str(), reference.as_str()) {
                            ("acme", "skills", "HEAD") => axum::Json(tree).into_response(),
                            ("acme", "moved", _) => (
                                StatusCode::MOVED_PERMANENTLY,
                                [(header::LOCATION, "http://169.254.169.254/latest/meta-data")],
                            )
                                .into_response(),
                            _ => StatusCode::NOT_FOUND.into_response(),
                        }
                    }
                },
            ),
        )
        .route(
            "/raw/{*path}",
            get(|Path(path): Path<String>| async move {
                match path.as_str() {
                    "acme/skills/HEAD/skills/pdf-forms/SKILL.md" => {
                        PDF_SKILL.to_vec().into_response()
                    }
                    "acme/skills/HEAD/skills/pdf-forms/scripts/fill.sh" => {
                        b"echo hi".to_vec().into_response()
                    }
                    _ => StatusCode::NOT_FOUND.into_response(),
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{address}")
}

#[tokio::test]
async fn installs_from_github_and_skills_sh_through_the_guarded_client() {
    let github = mock_github().await;
    let fixture = Fixture::new(Some((&github, true))).await;
    let (status, skill) = fixture
        .api(
            &fixture.owner,
            "POST",
            "/agent-skills/install",
            Some(json!({ "source": "https://skills.sh/acme/skills/pdf-forms" })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{skill}");
    assert_eq!(skill["name"], "pdf-forms");
    assert_eq!(skill["source"], "skills_sh");
    assert_eq!(
        skill["files"],
        json!([{ "path": "scripts/fill.sh", "size": 7 }])
    );
    assert_eq!(skill["agents"], json!([]));

    for (source, status, expected) in [
        // Only github.com and skills.sh, over https.
        (
            "https://evil.example/acme/skills",
            StatusCode::UNPROCESSABLE_ENTITY,
            "github.com",
        ),
        (
            "http://github.com/acme/skills",
            StatusCode::UNPROCESSABLE_ENTITY,
            "GitHub folder",
        ),
        (
            "acme/skills@missing",
            StatusCode::BAD_GATEWAY,
            "No folder named missing",
        ),
        (
            "https://github.com/acme/skills/tree/HEAD/skills/linked",
            StatusCode::BAD_GATEWAY,
            "symbolic link",
        ),
        (
            "https://github.com/acme/skills/tree/HEAD/skills/huge",
            StatusCode::BAD_GATEWAY,
            "too large",
        ),
        // A redirect is not followed, wherever it points.
        (
            "https://github.com/acme/moved",
            StatusCode::BAD_GATEWAY,
            "redirect",
        ),
        (
            "https://github.com/acme/private-one",
            StatusCode::BAD_GATEWAY,
            "public repositories",
        ),
    ] {
        let (code, problem) = fixture
            .api(
                &fixture.owner,
                "POST",
                "/agent-skills/install",
                Some(json!({ "source": source })),
            )
            .await;
        assert_eq!(code, status, "{source}: {problem}");
        assert!(
            problem["detail"].as_str().unwrap().contains(expected),
            "{source}: {problem}"
        );
    }

    // Without the test host, the same local server is a private address: refused before any
    // connection, over https too.
    let guarded = Fixture::new(Some((&github.replace("http://", "https://"), false))).await;
    let (status, problem) = guarded
        .api(
            &guarded.owner,
            "POST",
            "/agent-skills/install",
            Some(json!({ "source": "acme/skills@pdf-forms" })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_GATEWAY, "{problem}");
    assert!(
        problem["detail"]
            .as_str()
            .unwrap()
            .contains("local or private"),
        "{problem}"
    );
}
