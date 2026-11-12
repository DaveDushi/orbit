//! GitHub comment sync, the GitHub side: `issue_comment` webhooks become notes, and the
//! `github.comment` job posts, edits and deletes the GitHub comment of a note written in Orbit
//! with the App's installation token. The database side is `repositories::github_comments`.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use orbit_platform::{
    Database, Id, JobError, JobKindRegistrationError, Row, TimestampMillis, Worker,
};
use ring::rand::SystemRandom;
use ring::signature::{RSA_PKCS1_SHA256, RsaKeyPair};
use serde_json::{Value, json};

use crate::export::markdown::{KeepLinks, blocks_to_markdown};
use crate::repositories::github_comments::{
    self, GithubComment, GithubCommentAction, GithubThread, job_kind,
};
use crate::repositories::tasks::TaskError;
use crate::secret_box::decrypt_secret;

/// GitHub refuses longer comment bodies.
const MAX_GITHUB_BODY_CHARS: usize = 65_536;
/// Longer comments from GitHub are not copied.
const MAX_NOTE_CHARS: usize = 100_000;

/// Applies an `issue_comment` delivery of App `app_id` to workspace `workspace_id`. Returns
/// whether a note changed. Comments written by the App itself (the notes Orbit posted) are
/// ignored, so a note never comes back.
pub async fn webhook(
    database: &Database,
    workspace_id: Id,
    app_id: i64,
    payload: &Value,
    request_id: &str,
) -> Result<bool, TaskError> {
    let action = match payload.get("action").and_then(Value::as_str) {
        Some("created") => GithubCommentAction::Created,
        Some("edited") => GithubCommentAction::Edited,
        Some("deleted") => GithubCommentAction::Deleted,
        _ => return Ok(false),
    };
    let slug: Option<String> =
        sqlx::query_scalar("SELECT slug FROM github_apps WHERE workspace_id = $1 AND app_id = $2")
            .bind(workspace_id.to_string())
            .bind(app_id)
            .fetch_optional(database.pool())
            .await?;
    let (Some(comment), Some(issue)) = (payload.get("comment"), payload.get("issue")) else {
        return Ok(false);
    };
    if written_by_app(comment, app_id, slug.as_deref()) {
        return Ok(false);
    }
    let Some(comment) = parse_comment(comment) else {
        return Ok(false);
    };
    let Some(number) = issue
        .get("number")
        .and_then(Value::as_i64)
        .filter(|n| *n > 0)
    else {
        return Ok(false);
    };
    // Pull request conversation comments are issue comments of the pull request.
    let kind = if issue.get("pull_request").is_some() {
        "pull_request"
    } else {
        "issue"
    };
    let thread = GithubThread {
        repository: payload
            .pointer("/repository/full_name")
            .and_then(Value::as_str)
            .unwrap_or(""),
        number,
        kind,
        installation_id: payload
            .pointer("/installation/id")
            .and_then(Value::as_i64)
            .unwrap_or(0),
    };
    // The same member the issue sync acts as.
    let actor: String = sqlx::query_scalar(
        "SELECT user_id FROM memberships WHERE workspace_id = $1 AND role = 'owner' LIMIT 1",
    )
    .bind(workspace_id.to_string())
    .fetch_one(database.pool())
    .await?;
    let actor_id: Id = actor.parse().map_err(|_| TaskError::Conflict)?;
    github_comments::apply(
        database,
        workspace_id,
        actor_id,
        &thread,
        action,
        &comment,
        request_id,
        TimestampMillis::now(),
    )
    .await
}

/// A comment the App wrote: its author is the App's bot user, or GitHub names the App.
fn written_by_app(comment: &Value, app_id: i64, slug: Option<&str>) -> bool {
    let via_app = comment
        .pointer("/performed_via_github_app/id")
        .and_then(Value::as_i64)
        == Some(app_id);
    let login = comment.pointer("/user/login").and_then(Value::as_str);
    let bot = slug.is_some_and(|slug| login == Some(format!("{slug}[bot]").as_str()));
    via_app || bot
}

fn parse_comment(comment: &Value) -> Option<GithubComment> {
    let id = comment
        .get("id")
        .and_then(Value::as_i64)
        .filter(|id| *id > 0)?;
    let body = comment
        .get("body")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    if body.chars().count() > MAX_NOTE_CHARS {
        tracing::warn!(id, "GitHub comment too long for a note; skipped");
        return None;
    }
    // Shown as text and in URLs: only GitHub's login characters (and a bot's `[bot]`).
    let login = comment
        .pointer("/user/login")
        .and_then(Value::as_str)
        .filter(|login| {
            (1..=100).contains(&login.len())
                && login
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '[' | ']'))
        })
        .unwrap_or("ghost")
        .to_owned();
    let avatar_url = comment
        .pointer("/user/avatar_url")
        .and_then(Value::as_str)
        .filter(|url| url.starts_with("https://avatars.githubusercontent.com/") && url.len() <= 500)
        .map(str::to_owned);
    Some(GithubComment {
        id,
        body,
        login,
        avatar_url,
    })
}

/// The GitHub REST API (`https://api.github.com`; a local server in tests).
#[derive(Clone)]
pub struct GithubApi {
    client: reqwest::Client,
    base: String,
}

impl Default for GithubApi {
    fn default() -> Self {
        Self::new("https://api.github.com")
    }
}

/// A failed GitHub call: `retry` for network errors, rate limits and server errors.
#[derive(Debug)]
struct ApiFailure {
    status: Option<u16>,
    retry: bool,
}

impl GithubApi {
    #[must_use]
    pub fn new(base: &str) -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
            base: base.trim_end_matches('/').to_owned(),
        }
    }

    async fn call(
        &self,
        method: reqwest::Method,
        path: &str,
        token: &str,
        body: Option<Value>,
    ) -> Result<Value, ApiFailure> {
        let mut request = self
            .client
            .request(method, format!("{}{path}", self.base))
            .header("User-Agent", "Orbit")
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .bearer_auth(token);
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request.send().await.map_err(|_| ApiFailure {
            status: None,
            retry: true,
        })?;
        let status = response.status();
        if !status.is_success() {
            return Err(ApiFailure {
                status: Some(status.as_u16()),
                retry: status.is_server_error() || matches!(status.as_u16(), 403 | 429),
            });
        }
        Ok(response.json().await.unwrap_or(Value::Null))
    }

    async fn installation_token(
        &self,
        app_id: i64,
        private_key: &str,
        installation_id: i64,
    ) -> Result<String, JobError> {
        let jwt = app_jwt(app_id, private_key)
            .ok_or_else(|| JobError::Permanent("the GitHub App key is invalid".to_owned()))?;
        let token = self
            .call(
                reqwest::Method::POST,
                &format!("/app/installations/{installation_id}/access_tokens"),
                &jwt,
                None,
            )
            .await
            .map_err(|failure| job_error("installation token", &failure))?;
        token
            .get("token")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| JobError::Retryable("GitHub sent no installation token".to_owned()))
    }
}

fn job_error(what: &str, failure: &ApiFailure) -> JobError {
    let message = format!("GitHub {what} failed (status {:?})", failure.status);
    tracing::warn!(status = ?failure.status, "{message}");
    if failure.retry {
        JobError::Retryable(message)
    } else {
        JobError::Permanent(message)
    }
}

/// The App's RS256 JWT (10 minutes at most, issued a minute early for clock drift).
fn app_jwt(app_id: i64, private_key: &str) -> Option<String> {
    let der = STANDARD
        .decode(
            private_key
                .lines()
                .filter(|line| !line.starts_with("-----"))
                .collect::<String>()
                .trim(),
        )
        .ok()?;
    // GitHub hands out PKCS#1 keys ("RSA PRIVATE KEY"); accept PKCS#8 too.
    let key = RsaKeyPair::from_der(&der)
        .or_else(|_| RsaKeyPair::from_pkcs8(&der))
        .ok()?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
    let header = json!({"alg": "RS256", "typ": "JWT"});
    let claims = json!({"iat": now - 60, "exp": now + 540, "iss": app_id.to_string()});
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

pub fn register_job(
    worker: Worker,
    database: Database,
    app_key: Option<[u8; 32]>,
) -> Result<Worker, JobKindRegistrationError> {
    // Debug builds can use a local stand-in for GitHub (manual tests); release builds cannot.
    let api = match std::env::var("ORBIT_DEV_GITHUB_API_URL") {
        Ok(base) if cfg!(debug_assertions) => GithubApi::new(&base),
        _ => GithubApi::default(),
    };
    worker.with_handler(job_kind(), move |context| {
        let database = database.clone();
        let api = api.clone();
        async move { sync_note(&database, app_key, &api, &context.job.payload).await }
    })
}

/// Runs one `github.comment` job: makes the GitHub comment of a note match the note. The note is
/// read again, so a job queued before an edit sends the latest text.
pub async fn sync_note(
    database: &Database,
    app_key: Option<[u8; 32]>,
    api: &GithubApi,
    payload: &Value,
) -> Result<(), JobError> {
    let field = |name: &str| payload.get(name).and_then(Value::as_str).unwrap_or("");
    let (workspace_id, task_id, comment_id) =
        (field("workspace_id"), field("task_id"), field("comment_id"));
    let unavailable = |error: sqlx::Error| JobError::Retryable(error.to_string());
    // The link and the App of the task's project, as now: a paused sync sends nothing.
    let Some(link) = sqlx::query(
        "SELECT github_issue_links.repository, github_issue_links.issue_number, github_issue_links.sync_paused, \
         github_project_connections.installation_id, github_apps.app_id, github_apps.private_key_encrypted \
         FROM github_issue_links JOIN tasks ON tasks.id = github_issue_links.task_id \
         JOIN github_project_connections ON github_project_connections.project_id = tasks.project_id \
         AND github_project_connections.workspace_id = github_issue_links.workspace_id \
         AND github_project_connections.repository = github_issue_links.repository \
         JOIN github_apps ON github_apps.workspace_id = github_issue_links.workspace_id \
         WHERE github_issue_links.task_id = $1 AND github_issue_links.workspace_id = $2",
    )
    .bind(task_id)
    .bind(workspace_id)
    .fetch_optional(database.pool())
    .await
    .map_err(unavailable)?
    else {
        return Ok(());
    };
    if link.get::<bool, _>("sync_paused") {
        return Ok(());
    }
    let note = sqlx::query(
        "SELECT task_comments.body, task_comments.body_json, task_comments.github_comment_id, \
         task_comments.github_author_login, users.display_name, service_accounts.name AS service_account_name \
         FROM task_comments JOIN users ON users.id = task_comments.author_id \
         LEFT JOIN service_accounts ON service_accounts.id = task_comments.author_service_account_id \
         WHERE task_comments.id = $1 AND task_comments.workspace_id = $2",
    )
    .bind(comment_id)
    .bind(workspace_id)
    .fetch_optional(database.pool())
    .await
    .map_err(unavailable)?;
    if note.as_ref().is_some_and(|note| {
        note.get::<Option<String>, _>("github_author_login")
            .is_some()
    }) {
        return Ok(());
    }
    let queued_github_id = payload.get("github_comment_id").and_then(Value::as_i64);
    if note.is_none() && queued_github_id.is_none() {
        // Deleted before it was posted.
        return Ok(());
    }
    let key = app_key.ok_or_else(|| JobError::Permanent("Orbit has no app key".to_owned()))?;
    let private_key = decrypt_secret(&key, &link.get::<Vec<u8>, _>("private_key_encrypted"))
        .map_err(|_| JobError::Permanent("the GitHub App key cannot be decrypted".to_owned()))?;
    let token = api
        .installation_token(
            link.get("app_id"),
            &private_key,
            link.get("installation_id"),
        )
        .await?;
    let repository: String = link.get("repository");
    let comment_path = |github_id: i64| format!("/repos/{repository}/issues/comments/{github_id}");
    let delete = |github_id: i64| {
        let path = comment_path(github_id);
        let token = token.clone();
        async move {
            match api.call(reqwest::Method::DELETE, &path, &token, None).await {
                Err(ApiFailure {
                    status: Some(404), ..
                })
                | Ok(_) => Ok(()),
                Err(failure) => Err(job_error("comment delete", &failure)),
            }
        }
    };
    let Some(note) = note else {
        return delete(queued_github_id.unwrap_or_default()).await;
    };
    let body = github_body(&note);
    match note.get::<Option<i64>, _>("github_comment_id") {
        Some(github_id) => {
            match api
                .call(
                    reqwest::Method::PATCH,
                    &comment_path(github_id),
                    &token,
                    Some(json!({ "body": body })),
                )
                .await
            {
                // The GitHub copy was deleted on GitHub: the note stays as it is.
                Err(ApiFailure {
                    status: Some(404), ..
                })
                | Ok(_) => Ok(()),
                Err(failure) => Err(job_error("comment edit", &failure)),
            }
        }
        None => {
            let number: i64 = link.get("issue_number");
            let created = api
                .call(
                    reqwest::Method::POST,
                    &format!("/repos/{repository}/issues/{number}/comments"),
                    &token,
                    Some(json!({ "body": body })),
                )
                .await
                .map_err(|failure| job_error("comment create", &failure))?;
            let Some(github_id) = created.get("id").and_then(Value::as_i64) else {
                return Err(JobError::Permanent("GitHub sent no comment id".to_owned()));
            };
            let stored = sqlx::query(
                "UPDATE task_comments SET github_comment_id = $1 WHERE id = $2 AND github_comment_id IS NULL",
            )
            .bind(github_id)
            .bind(comment_id)
            .execute(database.pool())
            .await
            .map_err(unavailable)?;
            if stored.rows_affected() == 0 {
                // The note was deleted while it was posted.
                return delete(github_id).await;
            }
            Ok(())
        }
    }
}

/// The note's markdown for GitHub, signed with its Orbit author. Mentions of members become code
/// spans, so `@Name` does not notify a GitHub user of that name.
fn github_body(note: &sqlx::any::AnyRow) -> String {
    let blocks: Option<Vec<Value>> = note
        .get::<Option<String>, _>("body_json")
        .and_then(|json| serde_json::from_str(&json).ok());
    let text = match blocks {
        Some(mut blocks) => {
            blocks.iter_mut().for_each(quote_mentions);
            blocks_to_markdown(&blocks, &KeepLinks)
        }
        None => note.get("body"),
    };
    let author = note
        .get::<Option<String>, _>("service_account_name")
        .unwrap_or_else(|| note.get("display_name"));
    let signature = format!("\n\n— {author} via Orbit");
    let room = MAX_GITHUB_BODY_CHARS - signature.chars().count();
    let mut body: String = text.trim().chars().take(room).collect();
    body.push_str(&signature);
    body
}

fn quote_mentions(value: &mut Value) {
    match value {
        Value::Array(items) => items.iter_mut().for_each(quote_mentions),
        Value::Object(map) => {
            if map.get("type").and_then(Value::as_str) == Some("mention") {
                let name = map
                    .get("props")
                    .and_then(|props| props.get("name"))
                    .and_then(Value::as_str)
                    .unwrap_or("member");
                *value =
                    json!({"type": "text", "text": format!("@{name}"), "styles": {"code": true}});
                return;
            }
            map.values_mut().for_each(quote_mentions);
        }
        _ => {}
    }
}
