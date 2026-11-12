//! Followers of a task: the members who get an Inbox notification for its new notes
//! (`task_commented`) and status changes (`task_status_changed`). A member follows a task they
//! created, are assigned to, wrote a note on or were @mentioned in, and can follow or unfollow it
//! by hand. A follower who is no longer an active member (removed, suspended, now a guest) stays
//! in the table but is not listed or notified.

use orbit_platform::{Id, Row, TimestampMillis};
use serde::Serialize;
use sqlx::{Any, Transaction};
use utoipa::ToSchema;

use super::tasks::{NotificationActor, NotificationRequest, TaskError, notify_users, parse_id};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, ToSchema)]
pub struct TaskFollowers {
    /// The active members who follow the task, in the order they started.
    #[schema(value_type = Vec<String>)]
    pub user_ids: Vec<Id>,
    /// Whether the caller follows the task.
    pub following: bool,
}

/// `users` follow the task; nothing changes for one who already does.
pub(super) async fn follow_in_tx(
    tx: &mut Transaction<'_, Any>,
    task_id: Id,
    users: &[Id],
    now: TimestampMillis,
) -> Result<(), TaskError> {
    for user_id in users {
        sqlx::query(
            "INSERT INTO task_followers (task_id, user_id, created_at) VALUES ($1, $2, $3) \
             ON CONFLICT DO NOTHING",
        )
        .bind(task_id.to_string())
        .bind(user_id.to_string())
        .bind(now.as_millis())
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

pub(super) async fn unfollow_in_tx(
    tx: &mut Transaction<'_, Any>,
    task_id: Id,
    user_id: Id,
) -> Result<(), TaskError> {
    sqlx::query("DELETE FROM task_followers WHERE task_id = $1 AND user_id = $2")
        .bind(task_id.to_string())
        .bind(user_id.to_string())
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// The task's followers who are active members of the workspace and not guests.
pub(super) async fn followers_in_tx(
    tx: &mut Transaction<'_, Any>,
    workspace_id: Id,
    task_id: Id,
) -> Result<Vec<Id>, TaskError> {
    sqlx::query(
        "SELECT task_followers.user_id FROM task_followers \
         JOIN memberships ON memberships.user_id = task_followers.user_id \
         AND memberships.workspace_id = $1 AND memberships.role <> 'guest' \
         JOIN users ON users.id = task_followers.user_id AND users.suspended_at IS NULL \
         WHERE task_followers.task_id = $2 \
         ORDER BY task_followers.created_at, task_followers.user_id",
    )
    .bind(workspace_id.to_string())
    .bind(task_id.to_string())
    .fetch_all(&mut **tx)
    .await?
    .into_iter()
    .map(|row| parse_id(row.get::<String, _>("user_id")))
    .collect()
}

/// What a followed task's change is, for the followers' notifications.
pub(super) struct FollowerNotice<'a> {
    pub kind: &'a str,
    pub task_id: Id,
    pub comment_id: Option<Id>,
    /// Members who get another notification of the same change (an @mention, an assignment).
    pub skip: &'a [Id],
}

/// Notifies the task's followers of a change by `actor`; never the actor.
pub(super) async fn notify_followers_in_tx<'a>(
    tx: &mut Transaction<'_, Any>,
    workspace_id: Id,
    actor: impl Into<NotificationActor<'a>>,
    notice: FollowerNotice<'_>,
    now: TimestampMillis,
) -> Result<(), TaskError> {
    let actor = actor.into();
    let recipients: Vec<Id> = followers_in_tx(tx, workspace_id, notice.task_id)
        .await?
        .into_iter()
        .filter(|id| actor != NotificationActor::User(*id) && !notice.skip.contains(id))
        .collect();
    notify_users(
        tx,
        workspace_id,
        actor,
        NotificationRequest {
            kind: notice.kind,
            task_id: notice.task_id,
            comment_id: notice.comment_id,
            recipients: &recipients,
        },
        now,
    )
    .await
}
