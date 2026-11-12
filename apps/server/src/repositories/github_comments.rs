//! GitHub comment sync, the database side. A note of a task linked to a GitHub issue or pull
//! request (sync not paused) is a GitHub comment too: GitHub comments arrive through the webhook
//! (`apply`), and a note written in Orbit queues a `github.comment` job (`queue_in`) that
//! `crate::github_comments` runs against GitHub. Notes from GitHub are never sent back.

use orbit_platform::{Database, Flag, Id, Job, JobKind, JobStore, Row, TimestampMillis};
use serde_json::{Value, json};
use sqlx::{Any, Transaction};

use super::page_mentions::mentioned_users;
use super::task_followers::{self, FollowerNotice};
use super::tasks::{
    CommentContent, NotificationActor, NotificationRequest, TaskError, comment_content,
    github_service_account_in_tx, mentionable, notify_users, parse_id, record_principal_mutation,
    require_access_tx,
};

pub const JOB_KIND: &str = "github.comment";

/// One job at a time on all servers: two jobs of one new note must not both post it.
#[must_use]
pub fn job_kind() -> JobKind {
    JobKind::new(JOB_KIND).with_concurrency_limit(1)
}

/// Queues the GitHub side of a change of note `comment_id` (call it before a delete: the job
/// needs the note's GitHub comment id). Nothing when the task has no active GitHub link or the
/// note came from GitHub.
pub(super) async fn queue_in(
    tx: &mut Transaction<'_, Any>,
    workspace_id: Id,
    task_id: Id,
    comment_id: Id,
    now: TimestampMillis,
) -> Result<(), TaskError> {
    let row = sqlx::query(
        "SELECT task_comments.github_comment_id, task_comments.github_author_login FROM task_comments \
         JOIN github_issue_links ON github_issue_links.task_id = task_comments.task_id \
         AND github_issue_links.workspace_id = task_comments.workspace_id \
         WHERE task_comments.id = $1 AND task_comments.workspace_id = $2 AND task_comments.task_id = $3 \
         AND github_issue_links.sync_paused = 0",
    )
    .bind(comment_id.to_string())
    .bind(workspace_id.to_string())
    .bind(task_id.to_string())
    .fetch_optional(&mut **tx)
    .await?;
    let Some(row) = row else {
        return Ok(());
    };
    if row
        .get::<Option<String>, _>("github_author_login")
        .is_some()
    {
        return Ok(());
    }
    let payload = json!({
        "workspace_id": workspace_id.to_string(),
        "task_id": task_id.to_string(),
        "comment_id": comment_id.to_string(),
        "github_comment_id": row.get::<Option<i64>, _>("github_comment_id"),
    });
    let job = Job::new(job_kind(), payload, now).with_workspace(workspace_id.to_string());
    JobStore::enqueue_in(tx, &job)
        .await
        .map_err(|error| TaskError::Unavailable(sqlx::Error::Protocol(error.to_string())))?;
    Ok(())
}

/// The GitHub issue or pull request a webhook comment belongs to.
pub struct GithubThread<'a> {
    pub repository: &'a str,
    pub number: i64,
    /// `issue` or `pull_request`.
    pub kind: &'a str,
    pub installation_id: i64,
}

/// A comment from GitHub. Its text is untrusted: it becomes a note through the markdown import
/// and block sanitizer like any other note.
pub struct GithubComment {
    pub id: i64,
    pub body: String,
    pub login: String,
    pub avatar_url: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GithubCommentAction {
    Created,
    Edited,
    Deleted,
}

/// Applies a GitHub comment event to the note of the linked task, as `actor_id` (the sync actor)
/// on behalf of the "GitHub" service account; its notifications are from the GitHub author.
/// Returns whether a note changed; `false` for a task that is not linked, a paused sync, a
/// comment Orbit already has, or an unknown comment.
#[allow(clippy::too_many_arguments)]
pub async fn apply(
    database: &Database,
    workspace_id: Id,
    actor_id: Id,
    thread: &GithubThread<'_>,
    action: GithubCommentAction,
    comment: &GithubComment,
    request_id: &str,
    now: TimestampMillis,
) -> Result<bool, TaskError> {
    let mut tx = database.immediate_transaction().await?;
    let task: Option<String> = sqlx::query_scalar(
        "SELECT github_issue_links.task_id FROM github_issue_links \
         JOIN tasks ON tasks.id = github_issue_links.task_id \
         JOIN github_project_connections ON github_project_connections.project_id = tasks.project_id \
         AND github_project_connections.workspace_id = github_issue_links.workspace_id \
         AND github_project_connections.repository = github_issue_links.repository \
         WHERE github_issue_links.workspace_id = $1 AND github_issue_links.repository = $2 \
         AND github_issue_links.issue_number = $3 AND github_issue_links.kind = $4 \
         AND github_issue_links.sync_paused = 0 AND tasks.deleted_at IS NULL \
         AND github_project_connections.installation_id = $5",
    )
    .bind(workspace_id.to_string())
    .bind(thread.repository)
    .bind(thread.number)
    .bind(thread.kind)
    .bind(thread.installation_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(task_id) = task.map(parse_id).transpose()? else {
        return Ok(false);
    };
    require_access_tx(&mut tx, workspace_id, actor_id).await?;
    let changed = match action {
        GithubCommentAction::Created => {
            create(
                &mut tx,
                workspace_id,
                task_id,
                actor_id,
                comment,
                request_id,
                now,
            )
            .await?
        }
        GithubCommentAction::Edited => {
            edit(
                &mut tx,
                workspace_id,
                task_id,
                actor_id,
                comment,
                request_id,
                now,
            )
            .await?
        }
        GithubCommentAction::Deleted => {
            delete(
                &mut tx,
                workspace_id,
                task_id,
                actor_id,
                comment,
                request_id,
                now,
            )
            .await?
        }
    };
    if changed {
        tx.commit().await?;
    }
    Ok(changed)
}

async fn create(
    tx: &mut Transaction<'_, Any>,
    workspace_id: Id,
    task_id: Id,
    actor_id: Id,
    comment: &GithubComment,
    request_id: &str,
    now: TimestampMillis,
) -> Result<bool, TaskError> {
    // A redelivery, or the echo of a note Orbit posted.
    let Flag(known) = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM task_comments WHERE workspace_id = $1 AND github_comment_id = $2)",
    )
    .bind(workspace_id.to_string())
    .bind(comment.id)
    .fetch_one(&mut **tx)
    .await?;
    if known || comment.body.trim().is_empty() {
        return Ok(false);
    }
    let (service_account_id, service_account_name) =
        github_service_account_in_tx(tx, workspace_id, actor_id, now).await?;
    let (body, blocks) = markdown(tx, workspace_id, task_id, &comment.body).await?;
    // The GitHub author is not a member: nobody is skipped as the author of their own note.
    let author = NotificationActor::Github(&comment.login);
    let mentioned = mentionable(
        tx,
        workspace_id,
        mentioned_users(&blocks).into_keys().collect(),
        author,
    )
    .await?;
    let id = Id::new_v7();
    sqlx::query(
        "INSERT INTO task_comments (id, workspace_id, task_id, author_id, parent_id, body, body_json, version, \
         created_at, updated_at, author_service_account_id, github_comment_id, github_author_login, github_author_avatar_url) \
         VALUES ($1, $2, $3, $4, NULL, $5, $6, 0, $7, $7, $8, $9, $10, $11)",
    )
    .bind(id.to_string())
    .bind(workspace_id.to_string())
    .bind(task_id.to_string())
    .bind(actor_id.to_string())
    .bind(&body)
    .bind(Value::Array(blocks).to_string())
    .bind(now.as_millis())
    .bind(service_account_id.to_string())
    .bind(comment.id)
    .bind(&comment.login)
    .bind(comment.avatar_url.as_deref())
    .execute(&mut **tx)
    .await?;
    // As for a note written in Orbit, from the GitHub author.
    notify_users(
        tx,
        workspace_id,
        author,
        NotificationRequest {
            kind: "comment_mentioned",
            task_id,
            comment_id: Some(id),
            recipients: &mentioned,
        },
        now,
    )
    .await?;
    task_followers::notify_followers_in_tx(
        tx,
        workspace_id,
        author,
        FollowerNotice {
            kind: "task_commented",
            task_id,
            comment_id: Some(id),
            skip: &mentioned,
        },
        now,
    )
    .await?;
    task_followers::follow_in_tx(tx, task_id, &mentioned, now).await?;
    record_principal_mutation(
        tx,
        workspace_id,
        actor_id,
        Some((service_account_id, &service_account_name)),
        "comment.created",
        "comment",
        id,
        request_id,
        now,
    )
    .await?;
    Ok(true)
}

async fn edit(
    tx: &mut Transaction<'_, Any>,
    workspace_id: Id,
    task_id: Id,
    actor_id: Id,
    comment: &GithubComment,
    request_id: &str,
    now: TimestampMillis,
) -> Result<bool, TaskError> {
    let Some(row) = sqlx::query(
        "SELECT id, body, body_json FROM task_comments WHERE workspace_id = $1 AND task_id = $2 \
         AND github_comment_id = $3 AND github_author_login IS NOT NULL",
    )
    .bind(workspace_id.to_string())
    .bind(task_id.to_string())
    .bind(comment.id)
    .fetch_optional(&mut **tx)
    .await?
    else {
        return Ok(false);
    };
    let id = parse_id(row.get("id"))?;
    if row.get::<String, _>("body") == comment.body.trim() || comment.body.trim().is_empty() {
        return Ok(false);
    }
    let before: Vec<Value> = row
        .get::<Option<String>, _>("body_json")
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default();
    let (body, blocks) = markdown(tx, workspace_id, task_id, &comment.body).await?;
    let known = mentioned_users(&before);
    let added = mentioned_users(&blocks)
        .into_keys()
        .filter(|id| !known.contains_key(id))
        .collect();
    let author = NotificationActor::Github(&comment.login);
    let added = mentionable(tx, workspace_id, added, author).await?;
    sqlx::query(
        "UPDATE task_comments SET body = $1, body_json = $2, version = version + 1, updated_at = $3, \
         github_author_login = $4, github_author_avatar_url = $5 WHERE id = $6",
    )
    .bind(&body)
    .bind(Value::Array(blocks).to_string())
    .bind(now.as_millis())
    .bind(&comment.login)
    .bind(comment.avatar_url.as_deref())
    .bind(id.to_string())
    .execute(&mut **tx)
    .await?;
    notify_users(
        tx,
        workspace_id,
        author,
        NotificationRequest {
            kind: "comment_mentioned",
            task_id,
            comment_id: Some(id),
            recipients: &added,
        },
        now,
    )
    .await?;
    task_followers::follow_in_tx(tx, task_id, &added, now).await?;
    let (service_account_id, service_account_name) =
        github_service_account_in_tx(tx, workspace_id, actor_id, now).await?;
    record_principal_mutation(
        tx,
        workspace_id,
        actor_id,
        Some((service_account_id, &service_account_name)),
        "comment.updated",
        "comment",
        id,
        request_id,
        now,
    )
    .await?;
    Ok(true)
}

/// Only notes from GitHub go: a note written in Orbit stays when its GitHub copy is deleted.
async fn delete(
    tx: &mut Transaction<'_, Any>,
    workspace_id: Id,
    task_id: Id,
    actor_id: Id,
    comment: &GithubComment,
    request_id: &str,
    now: TimestampMillis,
) -> Result<bool, TaskError> {
    let Some(id) = sqlx::query_scalar::<_, String>(
        "SELECT id FROM task_comments WHERE workspace_id = $1 AND task_id = $2 \
         AND github_comment_id = $3 AND github_author_login IS NOT NULL",
    )
    .bind(workspace_id.to_string())
    .bind(task_id.to_string())
    .bind(comment.id)
    .fetch_optional(&mut **tx)
    .await?
    else {
        return Ok(false);
    };
    let id = parse_id(id)?;
    sqlx::query("DELETE FROM task_comments WHERE id = $1")
        .bind(id.to_string())
        .execute(&mut **tx)
        .await?;
    let (service_account_id, service_account_name) =
        github_service_account_in_tx(tx, workspace_id, actor_id, now).await?;
    record_principal_mutation(
        tx,
        workspace_id,
        actor_id,
        Some((service_account_id, &service_account_name)),
        "comment.deleted",
        "comment",
        id,
        request_id,
        now,
    )
    .await?;
    Ok(true)
}

async fn markdown(
    tx: &mut Transaction<'_, Any>,
    workspace_id: Id,
    task_id: Id,
    body: &str,
) -> Result<(String, Vec<Value>), TaskError> {
    comment_content(
        tx,
        workspace_id,
        task_id,
        &CommentContent::Markdown {
            body: body.to_owned(),
            mentioned_user_ids: Vec::new(),
        },
    )
    .await
}
