//! Blocks and related links between tasks of the token's approved projects. Duplicates are
//! marked and unmarked in the app only.
use orbit_platform::{Id, TimestampMillis};

use super::{Authorized, Result, TokenApiError};
use crate::repositories::api_tokens::ApiTokenScope;
use crate::repositories::task_relations::{
    NewTaskRelationType, TaskRelationRecord, TaskRelationType,
};
use crate::repositories::tasks::TaskError;

impl Authorized<'_> {
    /// The relations of a task whose other task is in an approved project.
    pub async fn relations(&self, task_ref: &str) -> Result<Vec<TaskRelationRecord>> {
        self.need(ApiTokenScope::TasksRead)?;
        self.visible_relations(task_ref).await
    }

    async fn visible_relations(&self, task_ref: &str) -> Result<Vec<TaskRelationRecord>> {
        let task = self.task(task_ref).await?;
        let p = self.principal;
        let relations = self
            .api
            .tasks
            .task_relations(p.workspace_id, task.id, p.actor_id)
            .await?;
        Ok(relations
            .into_iter()
            .filter(|relation| self.approved(relation.task.project_id))
            .collect())
    }

    /// Links two tasks of approved projects; an existing link between them is replaced.
    pub async fn add_relation(
        &self,
        task_ref: &str,
        kind: NewTaskRelationType,
        other_ref: &str,
    ) -> Result<TaskRelationRecord> {
        self.need(ApiTokenScope::TasksWrite)?;
        let task = self.task(task_ref).await?;
        let other = match self.task(other_ref).await {
            Ok(other) => other,
            Err(TokenApiError::Task(TaskError::Unavailable(error))) => {
                return Err(TokenApiError::Internal(error.to_string()));
            }
            Err(_) => return Err(TaskError::Invalid { field: "task_id" }.into()),
        };
        let p = self.principal;
        Ok(self
            .granted()
            .add_task_relation(
                p.workspace_id,
                task.id,
                p.actor_id,
                kind,
                other.id,
                self.request_id,
                TimestampMillis::now(),
            )
            .await?)
    }

    /// Removes a blocks or related relation.
    pub async fn remove_relation(&self, task_ref: &str, relation_id: Id) -> Result<()> {
        self.need(ApiTokenScope::TasksWrite)?;
        let relations = self.visible_relations(task_ref).await?;
        let relation = relations
            .iter()
            .find(|relation| relation.id == relation_id)
            .ok_or(TokenApiError::NotAvailable)?;
        // Unmarking a duplicate moves the task to its old status and runs the sub-issue automation.
        if relation.relation_type == TaskRelationType::Duplicate {
            return Err(TokenApiError::Conflict(
                "A duplicate relation is removed in Orbit: unmark the task there",
            ));
        }
        let task_id = self.task_id(task_ref).await?;
        let p = self.principal;
        Ok(self
            .granted()
            .remove_task_relation(
                p.workspace_id,
                task_id,
                relation.id,
                p.actor_id,
                self.request_id,
                TimestampMillis::now(),
            )
            .await?)
    }
}
