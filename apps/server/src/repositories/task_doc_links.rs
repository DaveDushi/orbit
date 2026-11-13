//! Docs pages linked to tasks ("Add doc" in the task detail, "Linked tasks" on a page). A member
//! links a page they see to a task they see; tasks are open to every member who sees their
//! project, so seeing a task is being able to change it. Each side lists only what its reader
//! sees: a task lists the live pages the reader sees, a page lists the live tasks of projects the
//! reader sees. Guests reach neither (`require_access`).
//!
//! Links and unlinks are audited on the task with the page id only, never its title, so the
//! audit log and the activity of a task never show a private page's title. The task activity
//! adds the title for readers who see the page ([`TaskRepository::add_doc_titles`]).

use orbit_platform::{Id, Row, TimestampMillis};
use serde::Serialize;
use serde_json::{Value, json};
use sqlx::{Any, Transaction};
use utoipa::ToSchema;

use super::pages::visible;
use super::project_access::ProjectScope;
use super::task_relations::{RelatedTask, visible_task_title_in_tx};
use super::tasks::{TaskError, TaskRepository, parse_id, require_access, require_access_tx};
use crate::audit::{self, AuditEvent, AuditOutcome};

pub const DOC_LINKED: &str = "task.doc_linked";
pub const DOC_UNLINKED: &str = "task.doc_unlinked";

/// A Docs page linked to a task, as its reader sees it.
#[derive(Clone, Debug, Serialize, ToSchema)]
#[schema(as = LinkedDoc)]
pub struct LinkedDoc {
    /// The page's id.
    #[schema(value_type = String)]
    pub id: Id,
    pub title: String,
    #[schema(required = true)]
    pub icon: Option<String>,
    /// True when the page is in the reader's private space.
    pub private: bool,
    #[schema(value_type = String, format = DateTime)]
    pub linked_at: TimestampMillis,
}

/// A task linked to a Docs page, as its reader sees it.
#[derive(Clone, Debug, Serialize, ToSchema)]
#[schema(as = LinkedTask)]
pub struct LinkedTask {
    pub task: RelatedTask,
    #[schema(value_type = String, format = DateTime)]
    pub linked_at: TimestampMillis,
}

/// Linked docs of task `$1` in workspace `$2` that the user bound at `$3` sees.
fn linked_docs_sql() -> String {
    format!(
        "SELECT pages.id, pages.title, pages.icon, pages.teamspace_id, task_doc_links.created_at \
         FROM task_doc_links JOIN pages ON pages.id = task_doc_links.page_id \
         WHERE task_doc_links.task_id = $1 AND task_doc_links.workspace_id = $2 \
         AND pages.workspace_id = $2 AND pages.deleted_at IS NULL AND {}",
        visible(3)
    )
}

fn linked_doc_from_row(row: &sqlx::any::AnyRow) -> Result<LinkedDoc, TaskError> {
    Ok(LinkedDoc {
        id: parse_id(row.get("id"))?,
        title: row.get("title"),
        icon: row.get("icon"),
        private: row.get::<Option<String>, _>("teamspace_id").is_none(),
        linked_at: TimestampMillis::from_millis(row.get("created_at")),
    })
}

/// Whether `page_id` is a live page of the workspace that `actor_id` sees.
async fn page_visible<'e>(
    executor: impl sqlx::Executor<'e, Database = Any>,
    workspace_id: Id,
    page_id: Id,
    actor_id: Id,
) -> Result<bool, TaskError> {
    Ok(sqlx::query(sqlx::AssertSqlSafe(format!(
        "SELECT 1 FROM pages WHERE id = $1 AND workspace_id = $2 AND deleted_at IS NULL AND {}",
        visible(3)
    )))
    .bind(page_id.to_string())
    .bind(workspace_id.to_string())
    .bind(actor_id.to_string())
    .fetch_optional(executor)
    .await?
    .is_some())
}

/// The live task `task_id` in a project the actor sees, or `NotFound`.
async fn require_task_in_tx(
    tx: &mut Transaction<'_, Any>,
    scope: ProjectScope,
    workspace_id: Id,
    task_id: Id,
) -> Result<(), TaskError> {
    visible_task_title_in_tx(tx, scope, workspace_id, task_id)
        .await?
        .map(|_| ())
        .ok_or(TaskError::NotFound)
}

#[allow(clippy::too_many_arguments)]
async fn record_link_audit(
    tx: &mut Transaction<'_, Any>,
    workspace_id: Id,
    actor_id: Id,
    action: &str,
    task_id: Id,
    page_id: Id,
    request_id: &str,
    now: TimestampMillis,
) -> Result<(), TaskError> {
    audit::record(
        tx,
        workspace_id,
        Some(actor_id),
        action,
        AuditOutcome::Success,
        "task",
        Some(task_id),
        request_id,
        json!({ "page_id": page_id }),
        now,
    )
    .await?;
    Ok(())
}

impl TaskRepository {
    /// The docs linked to a task that `actor_id` sees, oldest link first.
    pub async fn linked_docs(
        &self,
        workspace_id: Id,
        task_id: Id,
        actor_id: Id,
    ) -> Result<Vec<LinkedDoc>, TaskError> {
        self.get_task(workspace_id, task_id, actor_id).await?;
        let sql = format!(
            "{} ORDER BY task_doc_links.created_at, task_doc_links.id",
            linked_docs_sql()
        );
        sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(task_id.to_string())
            .bind(workspace_id.to_string())
            .bind(actor_id.to_string())
            .fetch_all(self.database().pool())
            .await?
            .iter()
            .map(linked_doc_from_row)
            .collect()
    }

    /// The tasks linked to a page that `actor_id` sees (live tasks of projects they see), oldest
    /// link first. `NotFound` unless the actor sees the page.
    pub async fn linked_tasks(
        &self,
        workspace_id: Id,
        page_id: Id,
        actor_id: Id,
    ) -> Result<Vec<LinkedTask>, TaskError> {
        let pool = self.database().pool();
        let scope = ProjectScope::of(require_access(pool, workspace_id, actor_id).await?);
        if !page_visible(pool, workspace_id, page_id, actor_id).await? {
            return Err(TaskError::NotFound);
        }
        let sql = format!(
            "SELECT tasks.id, tasks.project_id, tasks.number, tasks.title, tasks.status_id, tasks.version, \
             task_doc_links.created_at FROM task_doc_links \
             JOIN tasks ON tasks.id = task_doc_links.task_id \
             JOIN projects ON projects.id = tasks.project_id \
             WHERE task_doc_links.page_id = $1 AND task_doc_links.workspace_id = $2 \
             AND tasks.deleted_at IS NULL AND projects.deleted_at IS NULL AND {} \
             ORDER BY task_doc_links.created_at, task_doc_links.id",
            scope.project("tasks.project_id")
        );
        sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(page_id.to_string())
            .bind(workspace_id.to_string())
            .fetch_all(pool)
            .await?
            .iter()
            .map(|row| {
                Ok(LinkedTask {
                    task: RelatedTask {
                        id: parse_id(row.get("id"))?,
                        project_id: parse_id(row.get("project_id"))?,
                        number: row.get::<Option<i64>, _>("number").unwrap_or_default(),
                        title: row.get("title"),
                        status_id: parse_id(row.get("status_id"))?,
                        version: u64::try_from(row.get::<i64, _>("version"))
                            .map_err(|_| TaskError::Conflict)?,
                    },
                    linked_at: TimestampMillis::from_millis(row.get("created_at")),
                })
            })
            .collect()
    }

    /// Links a page to a task; linking it again changes nothing. The task must be one the actor
    /// sees (`NotFound`), the page one they see (`Invalid { field: "page_id" }`).
    pub async fn link_doc(
        &self,
        workspace_id: Id,
        task_id: Id,
        actor_id: Id,
        page_id: Id,
        request_id: &str,
        now: TimestampMillis,
    ) -> Result<LinkedDoc, TaskError> {
        let mut tx = self.database().immediate_transaction().await?;
        let scope = ProjectScope::of(require_access_tx(&mut tx, workspace_id, actor_id).await?);
        require_task_in_tx(&mut tx, scope, workspace_id, task_id).await?;
        if !page_visible(&mut *tx, workspace_id, page_id, actor_id).await? {
            return Err(TaskError::Invalid { field: "page_id" });
        }
        let inserted = sqlx::query(
            "INSERT INTO task_doc_links (id, workspace_id, task_id, page_id, created_by, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6) ON CONFLICT (task_id, page_id) DO NOTHING",
        )
        .bind(Id::new_v7().to_string())
        .bind(workspace_id.to_string())
        .bind(task_id.to_string())
        .bind(page_id.to_string())
        .bind(actor_id.to_string())
        .bind(now.as_millis())
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if inserted == 1 {
            record_link_audit(
                &mut tx,
                workspace_id,
                actor_id,
                DOC_LINKED,
                task_id,
                page_id,
                request_id,
                now,
            )
            .await?;
        }
        let row = sqlx::query(sqlx::AssertSqlSafe(format!(
            "{} AND pages.id = $4",
            linked_docs_sql()
        )))
        .bind(task_id.to_string())
        .bind(workspace_id.to_string())
        .bind(actor_id.to_string())
        .bind(page_id.to_string())
        .fetch_one(&mut *tx)
        .await?;
        let doc = linked_doc_from_row(&row)?;
        tx.commit().await?;
        Ok(doc)
    }

    /// Removes a page's link from a task. `NotFound` unless the actor sees both and they are linked.
    pub async fn unlink_doc(
        &self,
        workspace_id: Id,
        task_id: Id,
        actor_id: Id,
        page_id: Id,
        request_id: &str,
        now: TimestampMillis,
    ) -> Result<(), TaskError> {
        let mut tx = self.database().immediate_transaction().await?;
        let scope = ProjectScope::of(require_access_tx(&mut tx, workspace_id, actor_id).await?);
        require_task_in_tx(&mut tx, scope, workspace_id, task_id).await?;
        if !page_visible(&mut *tx, workspace_id, page_id, actor_id).await? {
            return Err(TaskError::NotFound);
        }
        let deleted = sqlx::query(
            "DELETE FROM task_doc_links WHERE task_id = $1 AND page_id = $2 AND workspace_id = $3",
        )
        .bind(task_id.to_string())
        .bind(page_id.to_string())
        .bind(workspace_id.to_string())
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if deleted == 0 {
            return Err(TaskError::NotFound);
        }
        record_link_audit(
            &mut tx,
            workspace_id,
            actor_id,
            DOC_UNLINKED,
            task_id,
            page_id,
            request_id,
            now,
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Adds `page_title` to the doc link events whose page `actor_id` sees (a live page); the
    /// others keep only the page id.
    pub(super) async fn add_doc_titles(
        &self,
        workspace_id: Id,
        actor_id: Id,
        events: &mut [AuditEvent],
    ) -> Result<(), TaskError> {
        for event in events
            .iter_mut()
            .filter(|event| event.action == DOC_LINKED || event.action == DOC_UNLINKED)
        {
            let Some(page_id) = event.metadata.get("page_id").and_then(Value::as_str) else {
                continue;
            };
            let title: Option<String> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                "SELECT title FROM pages WHERE id = $1 AND workspace_id = $2 AND deleted_at IS NULL AND {}",
                visible(3)
            )))
            .bind(page_id.to_owned())
            .bind(workspace_id.to_string())
            .bind(actor_id.to_string())
            .fetch_optional(self.database().pool())
            .await?;
            if let Some(title) = title {
                event.metadata["page_title"] = json!(title);
            }
        }
        Ok(())
    }
}
