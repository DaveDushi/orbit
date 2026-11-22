//! The GitHub issues and pull requests linked to a task.
use orbit_platform::{Id, Row};
use serde::Serialize;
use utoipa::ToSchema;

use super::{TaskError, TaskRepository};

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct GithubLink {
    /// `issue` or `pull_request`.
    pub kind: String,
    pub title: String,
    pub url: String,
    pub state: String,
    pub source: bool,
    /// `owner/name` of the issue or pull request the task syncs from (`source` only).
    pub repository: Option<String>,
    /// Its number (`source` only).
    pub number: Option<i64>,
    /// How many repositories the task's project syncs from, its connected one included
    /// (`source` only): the clients name the repository when there is more than one.
    pub project_repositories: Option<i64>,
}

impl TaskRepository {
    /// The links of `task_id`, a task of `project_id`. The caller checks first that the actor sees
    /// the task.
    pub async fn github_links(
        &self,
        workspace_id: Id,
        task_id: Id,
        project_id: Id,
    ) -> Result<Vec<GithubLink>, TaskError> {
        let rows = sqlx::query("SELECT kind, repository || '#' || issue_number AS title, 'https://github.com/' || repository || CASE kind WHEN 'pull_request' THEN '/pull/' ELSE '/issues/' END || issue_number AS url, CASE WHEN sync_paused = 1 THEN 'paused' WHEN kind = 'pull_request' THEN pull_state ELSE 'active' END AS state, 1 AS source, repository, issue_number FROM github_issue_links WHERE workspace_id = $1 AND task_id = $2 UNION ALL SELECT 'pull_request' AS kind, title, url, state, 0 AS source, NULL AS repository, NULL AS issue_number FROM github_pull_links WHERE workspace_id = $3 AND task_id = $4 ORDER BY kind, title")
            .bind(workspace_id.to_string())
            .bind(task_id.to_string())
            .bind(workspace_id.to_string())
            .bind(task_id.to_string())
            .fetch_all(self.database().pool())
            .await?;
        let project_repositories: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM (SELECT repository FROM github_project_connections WHERE workspace_id = $1 AND project_id = $2 \
             UNION SELECT github_issue_links.repository FROM github_issue_links JOIN tasks ON tasks.id = github_issue_links.task_id \
             WHERE github_issue_links.workspace_id = $3 AND tasks.project_id = $4) AS repositories",
        )
        .bind(workspace_id.to_string())
        .bind(project_id.to_string())
        .bind(workspace_id.to_string())
        .bind(project_id.to_string())
        .fetch_one(self.database().pool())
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| {
                let source = row.get::<i64, _>("source") != 0;
                GithubLink {
                    kind: row.get("kind"),
                    title: row.get("title"),
                    url: row.get("url"),
                    state: row.get("state"),
                    source,
                    repository: row.get("repository"),
                    number: row.get("issue_number"),
                    project_repositories: source.then_some(project_repositories),
                }
            })
            .collect())
    }
}
