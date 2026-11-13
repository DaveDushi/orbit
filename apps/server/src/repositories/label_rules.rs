//! Label rules: "if a task has label X, assign person Y", per project. A rule only adds
//! assignees: it never removes one, and a person already assigned gets nothing. With the
//! project's owner switch on, a label `owner:<name>` also assigns the member whose handle (the
//! part of the email before `@`) or name is `<name>`, without regard to case. A rule of an
//! `owner:` label is the name map entry of `<name>` (Rules card → owner names): it wins over that
//! match, so an `owner:` label with a rule assigns only the rule's people.
//!
//! Every label that a task gets goes through [`insert_task_label`], whatever the path (web, API
//! tokens, MCP, GitHub sync, Discord), so the rules run in one place. A backup restore does not
//! use it: a restored task keeps the assignees it had. Only owners and admins
//! ([`Permission::LabelRulesManage`]) see and change the rules.

use orbit_domain::Permission;
use orbit_platform::{Database, Flag, Id, Row, TimestampMillis};
use serde::Serialize;
use serde_json::json;
use sqlx::{Any, Transaction};
use utoipa::ToSchema;

use super::membership;
use super::project_access::{user_sees_project_sql, user_sees_task_sql};
use super::task_followers;
use super::tasks::{NotificationRequest, TaskError, notify_users};
use crate::audit::{self, AuditOutcome};

/// The label prefix of the owner switch.
const OWNER_PREFIX: &str = "owner:";

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct LabelRuleRecord {
    #[schema(value_type = String)]
    pub id: Id,
    #[schema(value_type = String)]
    pub project_id: Id,
    #[schema(value_type = String)]
    pub label_id: Id,
    pub label_name: String,
    pub label_color: String,
    #[schema(value_type = String)]
    pub user_id: Id,
    pub user_name: String,
    pub enabled: bool,
    #[schema(value_type = String, format = DateTime)]
    pub created_at: TimestampMillis,
    #[schema(value_type = String, format = DateTime)]
    pub updated_at: TimestampMillis,
}

/// A project's rules and its owner switch.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct LabelRuleSettings {
    pub rules: Vec<LabelRuleRecord>,
    /// `owner:<name>` labels assign the member `<name>`.
    pub owner_labels: bool,
    /// The `<name>` of each `owner:` label of the project that has no rule (name map entry) and
    /// names no member (or more than one).
    pub unmatched_owner_names: Vec<String>,
}

/// A change to a rule; `None` keeps the field.
#[derive(Clone, Debug, Default)]
pub struct LabelRulePatch {
    pub label_id: Option<Id>,
    pub user_id: Option<Id>,
    pub enabled: Option<bool>,
}

/// Who adds a label, for the rule's notification and history.
pub(super) struct LabelWrite<'a> {
    pub workspace_id: Id,
    /// The person behind the write; the assignment notification comes from them.
    pub actor_id: Id,
    pub request_id: &'a str,
    pub now: TimestampMillis,
}

/// Gives the task the label, then assigns the people that the project's rules name for it.
pub(super) async fn insert_task_label(
    tx: &mut Transaction<'_, Any>,
    write: &LabelWrite<'_>,
    task_id: Id,
    label_id: &str,
) -> Result<(), TaskError> {
    sqlx::query("INSERT INTO task_labels (task_id, label_id) VALUES ($1, $2)")
        .bind(task_id.to_string())
        .bind(label_id)
        .execute(&mut **tx)
        .await?;
    let Some(label) = sqlx::query("SELECT name, project_id FROM labels WHERE id = $1")
        .bind(label_id)
        .fetch_optional(&mut **tx)
        .await?
    else {
        return Ok(());
    };
    let label_name: String = label.get("name");
    let project_id: String = label.get("project_id");
    let rules = sqlx::query(
        "SELECT user_id, enabled FROM label_rules WHERE label_id = $1 ORDER BY created_at, id",
    )
    .bind(label_id)
    .fetch_all(&mut **tx)
    .await?;
    let mut people: Vec<String> = rules
        .iter()
        .filter(|rule| rule.get::<bool, _>("enabled"))
        .map(|rule| rule.get("user_id"))
        .collect();
    // A rule of an `owner:` label is its name map entry: it wins over the handle match, even
    // when it is off (then the label assigns nobody).
    if rules.is_empty()
        && let Some(name) = owner_name(&label_name)
        && owner_labels_in_tx(tx, &project_id).await?
        && let Some(user_id) = owner_member(tx, write.workspace_id, name).await?
    {
        people.push(user_id);
    }
    for user_id in people {
        assign_by_rule(tx, write, task_id, &label_name, &user_id).await?;
    }
    Ok(())
}

/// Assigns one person by a rule, unless they are assigned already or may not be assigned (not
/// a member any more, a guest, suspended, or not on the member list of a private project): then
/// nothing happens and the task write goes on.
async fn assign_by_rule(
    tx: &mut Transaction<'_, Any>,
    write: &LabelWrite<'_>,
    task_id: Id,
    label_name: &str,
    user_id: &str,
) -> Result<(), TaskError> {
    let Some(member) = sqlx::query(sqlx::AssertSqlSafe(format!(
        "SELECT memberships.id, users.display_name FROM memberships JOIN users ON users.id = memberships.user_id \
         WHERE memberships.workspace_id = $1 AND memberships.user_id = $2 AND memberships.role <> 'guest' \
         AND users.suspended_at IS NULL AND users.deleted_at IS NULL AND {}",
        user_sees_task_sql("$3", "users.id")
    )))
    .bind(write.workspace_id.to_string())
    .bind(user_id)
    .bind(task_id.to_string())
    .fetch_optional(&mut **tx)
    .await?
    else {
        return Ok(());
    };
    let added = sqlx::query(
        "INSERT INTO task_assignees (task_id, membership_id, user_id) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
    )
    .bind(task_id.to_string())
    .bind(member.get::<String, _>("id"))
    .bind(user_id)
    .execute(&mut **tx)
    .await?;
    if added.rows_affected() == 0 {
        return Ok(());
    }
    let user: Id = user_id
        .parse()
        .map_err(|_| TaskError::Invalid { field: "user_id" })?;
    task_followers::follow_in_tx(tx, task_id, &[user], write.now).await?;
    notify_users(
        tx,
        write.workspace_id,
        write.actor_id,
        NotificationRequest {
            kind: "task_assigned",
            task_id,
            comment_id: None,
            recipients: &[user],
        },
        write.now,
    )
    .await?;
    // The rule is the actor (no person): the history reads "Assigned by rule: label → person".
    audit::record(
        tx,
        write.workspace_id,
        None,
        "task.assigned_by_rule",
        AuditOutcome::Success,
        "task",
        Some(task_id),
        write.request_id,
        json!({
            "label_name": label_name,
            "assignee_id": user_id,
            "assignee_name": member.get::<String, _>("display_name"),
        }),
        write.now,
    )
    .await?;
    Ok(())
}

/// `<name>` of a label `owner:<name>` (any case), trimmed; `None` for other labels.
fn owner_name(label: &str) -> Option<&str> {
    let prefix = label.get(..OWNER_PREFIX.len())?;
    if !prefix.eq_ignore_ascii_case(OWNER_PREFIX) {
        return None;
    }
    Some(label[OWNER_PREFIX.len()..].trim()).filter(|name| !name.is_empty())
}

/// Whether `name` is the member's handle (email before `@`), name, or name with dashes for
/// spaces ("David S" → `david-s`), without regard to case.
fn names_member(name: &str, email: &str, display_name: &str) -> bool {
    let name = name.to_lowercase();
    let handle = email.split('@').next().unwrap_or_default().to_lowercase();
    let display = display_name.trim().to_lowercase();
    let dashed = display.split_whitespace().collect::<Vec<_>>().join("-");
    name == handle || name == display || name == dashed
}

/// The one member that `name` names, or `None` when none or several do.
async fn owner_member(
    tx: &mut Transaction<'_, Any>,
    workspace_id: Id,
    name: &str,
) -> Result<Option<String>, TaskError> {
    let rows = sqlx::query(
        "SELECT users.id, users.email, users.display_name FROM memberships JOIN users ON users.id = memberships.user_id \
         WHERE memberships.workspace_id = $1 AND memberships.role <> 'guest' \
         AND users.suspended_at IS NULL AND users.deleted_at IS NULL",
    )
    .bind(workspace_id.to_string())
    .fetch_all(&mut **tx)
    .await?;
    let mut matches = rows.iter().filter(|row| {
        names_member(
            name,
            &row.get::<String, _>("email"),
            &row.get::<String, _>("display_name"),
        )
    });
    let first = matches.next().map(|row| row.get::<String, _>("id"));
    Ok(if matches.next().is_some() {
        None
    } else {
        first
    })
}

async fn owner_labels_in_tx(
    tx: &mut Transaction<'_, Any>,
    project_id: &str,
) -> Result<bool, TaskError> {
    let Flag(on) = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM project_label_rule_settings WHERE project_id = $1 AND owner_labels = 1)",
    )
    .bind(project_id)
    .fetch_one(&mut **tx)
    .await?;
    Ok(on)
}

/// The rules' admin side (Project settings → Rules, and the agent's tools).
pub struct LabelRuleRepository {
    database: Database,
}

impl LabelRuleRepository {
    #[must_use]
    pub fn new(database: Database) -> Self {
        Self { database }
    }

    pub async fn settings(
        &self,
        workspace_id: Id,
        project_id: Id,
        actor_id: Id,
    ) -> Result<LabelRuleSettings, TaskError> {
        let mut tx = self.database.transaction().await?;
        require_manager(&mut tx, workspace_id, project_id, actor_id).await?;
        let settings = settings_in_tx(&mut tx, workspace_id, project_id).await?;
        tx.commit().await?;
        Ok(settings)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn create_rule(
        &self,
        workspace_id: Id,
        project_id: Id,
        actor_id: Id,
        label_id: Id,
        user_id: Id,
        enabled: bool,
        request_id: &str,
        now: TimestampMillis,
    ) -> Result<LabelRuleRecord, TaskError> {
        let mut tx = self.database.immediate_transaction().await?;
        require_manager(&mut tx, workspace_id, project_id, actor_id).await?;
        check_rule(&mut tx, workspace_id, project_id, label_id, user_id, None).await?;
        let id = Id::new_v7();
        sqlx::query(
            "INSERT INTO label_rules (id, workspace_id, project_id, label_id, user_id, enabled, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(id.to_string())
        .bind(workspace_id.to_string())
        .bind(project_id.to_string())
        .bind(label_id.to_string())
        .bind(user_id.to_string())
        .bind(i64::from(enabled))
        .bind(now.as_millis())
        .bind(now.as_millis())
        .execute(&mut *tx)
        .await?;
        record(
            &mut tx,
            workspace_id,
            actor_id,
            "label_rule.created",
            project_id,
            id,
            request_id,
            now,
        )
        .await?;
        let rule = rule_in_tx(&mut tx, project_id, id).await?;
        tx.commit().await?;
        Ok(rule)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn update_rule(
        &self,
        workspace_id: Id,
        project_id: Id,
        rule_id: Id,
        actor_id: Id,
        patch: LabelRulePatch,
        request_id: &str,
        now: TimestampMillis,
    ) -> Result<LabelRuleRecord, TaskError> {
        let mut tx = self.database.immediate_transaction().await?;
        require_manager(&mut tx, workspace_id, project_id, actor_id).await?;
        let current = rule_in_tx(&mut tx, project_id, rule_id).await?;
        let label_id = patch.label_id.unwrap_or(current.label_id);
        let user_id = patch.user_id.unwrap_or(current.user_id);
        if label_id != current.label_id || user_id != current.user_id {
            check_rule(
                &mut tx,
                workspace_id,
                project_id,
                label_id,
                user_id,
                Some(rule_id),
            )
            .await?;
        }
        sqlx::query(
            "UPDATE label_rules SET label_id = $1, user_id = $2, enabled = $3, updated_at = $4 WHERE id = $5",
        )
        .bind(label_id.to_string())
        .bind(user_id.to_string())
        .bind(i64::from(patch.enabled.unwrap_or(current.enabled)))
        .bind(now.as_millis())
        .bind(rule_id.to_string())
        .execute(&mut *tx)
        .await?;
        record(
            &mut tx,
            workspace_id,
            actor_id,
            "label_rule.updated",
            project_id,
            rule_id,
            request_id,
            now,
        )
        .await?;
        let rule = rule_in_tx(&mut tx, project_id, rule_id).await?;
        tx.commit().await?;
        Ok(rule)
    }

    pub async fn delete_rule(
        &self,
        workspace_id: Id,
        project_id: Id,
        rule_id: Id,
        actor_id: Id,
        request_id: &str,
        now: TimestampMillis,
    ) -> Result<(), TaskError> {
        let mut tx = self.database.immediate_transaction().await?;
        require_manager(&mut tx, workspace_id, project_id, actor_id).await?;
        rule_in_tx(&mut tx, project_id, rule_id).await?;
        sqlx::query("DELETE FROM label_rules WHERE id = $1")
            .bind(rule_id.to_string())
            .execute(&mut *tx)
            .await?;
        record(
            &mut tx,
            workspace_id,
            actor_id,
            "label_rule.deleted",
            project_id,
            rule_id,
            request_id,
            now,
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn set_owner_labels(
        &self,
        workspace_id: Id,
        project_id: Id,
        actor_id: Id,
        enabled: bool,
        request_id: &str,
        now: TimestampMillis,
    ) -> Result<LabelRuleSettings, TaskError> {
        let mut tx = self.database.immediate_transaction().await?;
        require_manager(&mut tx, workspace_id, project_id, actor_id).await?;
        sqlx::query(
            "INSERT INTO project_label_rule_settings (project_id, owner_labels, updated_at) VALUES ($1, $2, $3) \
             ON CONFLICT (project_id) DO UPDATE SET owner_labels = excluded.owner_labels, updated_at = excluded.updated_at",
        )
        .bind(project_id.to_string())
        .bind(i64::from(enabled))
        .bind(now.as_millis())
        .execute(&mut *tx)
        .await?;
        audit::record(
            &mut tx,
            workspace_id,
            Some(actor_id),
            "label_rules.owner_labels.updated",
            AuditOutcome::Success,
            "project",
            Some(project_id),
            request_id,
            json!({ "enabled": enabled }),
            now,
        )
        .await?;
        let settings = settings_in_tx(&mut tx, workspace_id, project_id).await?;
        tx.commit().await?;
        Ok(settings)
    }
}

/// Only owners and admins manage rules, of a live project of their workspace.
async fn require_manager(
    tx: &mut Transaction<'_, Any>,
    workspace_id: Id,
    project_id: Id,
    actor_id: Id,
) -> Result<(), TaskError> {
    let actor = membership::actor(&mut **tx, workspace_id, actor_id)
        .await?
        .ok_or(TaskError::NotFound)?;
    let Flag(live) = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM projects WHERE id = $1 AND workspace_id = $2 AND deleted_at IS NULL)",
    )
    .bind(project_id.to_string())
    .bind(workspace_id.to_string())
    .fetch_one(&mut **tx)
    .await?;
    if !live {
        return Err(TaskError::NotFound);
    }
    if !actor.can(Permission::LabelRulesManage) {
        return Err(TaskError::Forbidden);
    }
    Ok(())
}

/// The label is the project's, the person can be assigned, and no other rule has both.
async fn check_rule(
    tx: &mut Transaction<'_, Any>,
    workspace_id: Id,
    project_id: Id,
    label_id: Id,
    user_id: Id,
    rule_id: Option<Id>,
) -> Result<(), TaskError> {
    let Flag(label) = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM labels WHERE id = $1 AND workspace_id = $2 AND project_id = $3)",
    )
    .bind(label_id.to_string())
    .bind(workspace_id.to_string())
    .bind(project_id.to_string())
    .fetch_one(&mut **tx)
    .await?;
    if !label {
        return Err(TaskError::Invalid { field: "label_id" });
    }
    // The person must see the project: a private project's rule names one of its members.
    let Flag(member) = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT EXISTS(SELECT 1 FROM memberships JOIN users ON users.id = memberships.user_id \
         WHERE memberships.workspace_id = $1 AND memberships.user_id = $2 AND memberships.role <> 'guest' \
         AND users.suspended_at IS NULL AND users.deleted_at IS NULL AND {})",
        user_sees_project_sql("$3", "users.id")
    )))
    .bind(workspace_id.to_string())
    .bind(user_id.to_string())
    .bind(project_id.to_string())
    .fetch_one(&mut **tx)
    .await?;
    if !member {
        return Err(TaskError::Invalid { field: "user_id" });
    }
    let Flag(taken) = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM label_rules WHERE label_id = $1 AND user_id = $2 AND id IS DISTINCT FROM $3)",
    )
    .bind(label_id.to_string())
    .bind(user_id.to_string())
    .bind(rule_id.map(|id| id.to_string()))
    .fetch_one(&mut **tx)
    .await?;
    if taken {
        return Err(TaskError::Conflict);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn record(
    tx: &mut Transaction<'_, Any>,
    workspace_id: Id,
    actor_id: Id,
    action: &str,
    project_id: Id,
    rule_id: Id,
    request_id: &str,
    now: TimestampMillis,
) -> Result<(), TaskError> {
    audit::record(
        tx,
        workspace_id,
        Some(actor_id),
        action,
        AuditOutcome::Success,
        "label_rule",
        Some(rule_id),
        request_id,
        json!({ "project_id": project_id }),
        now,
    )
    .await?;
    Ok(())
}

const RULE_COLUMNS: &str = "label_rules.id, label_rules.project_id, label_rules.label_id, labels.name AS label_name, \
     labels.color AS label_color, label_rules.user_id, users.display_name AS user_name, label_rules.enabled, \
     label_rules.created_at, label_rules.updated_at \
     FROM label_rules JOIN labels ON labels.id = label_rules.label_id JOIN users ON users.id = label_rules.user_id";

async fn rule_in_tx(
    tx: &mut Transaction<'_, Any>,
    project_id: Id,
    rule_id: Id,
) -> Result<LabelRuleRecord, TaskError> {
    let row = sqlx::query(sqlx::AssertSqlSafe(format!(
        "SELECT {RULE_COLUMNS} WHERE label_rules.id = $1 AND label_rules.project_id = $2"
    )))
    .bind(rule_id.to_string())
    .bind(project_id.to_string())
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(TaskError::NotFound)?;
    rule_from_row(&row)
}

async fn settings_in_tx(
    tx: &mut Transaction<'_, Any>,
    workspace_id: Id,
    project_id: Id,
) -> Result<LabelRuleSettings, TaskError> {
    let rows = sqlx::query(sqlx::AssertSqlSafe(format!(
        "SELECT {RULE_COLUMNS} WHERE label_rules.project_id = $1 ORDER BY lower(labels.name), lower(users.display_name), label_rules.id"
    )))
    .bind(project_id.to_string())
    .fetch_all(&mut **tx)
    .await?;
    let rules = rows
        .iter()
        .map(rule_from_row)
        .collect::<Result<Vec<_>, _>>()?;
    let owner_labels = owner_labels_in_tx(tx, &project_id.to_string()).await?;
    // A name with a map entry (a rule of its label) is not unmatched.
    let labels: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM labels WHERE project_id = $1 AND lower(name) LIKE 'owner:%' \
         AND NOT EXISTS (SELECT 1 FROM label_rules WHERE label_rules.label_id = labels.id) ORDER BY lower(name)",
    )
    .bind(project_id.to_string())
    .fetch_all(&mut **tx)
    .await?;
    let mut unmatched_owner_names = Vec::new();
    for label in &labels {
        if let Some(name) = owner_name(label)
            && owner_member(tx, workspace_id, name).await?.is_none()
        {
            unmatched_owner_names.push(name.to_owned());
        }
    }
    Ok(LabelRuleSettings {
        rules,
        owner_labels,
        unmatched_owner_names,
    })
}

fn rule_from_row(row: &sqlx::any::AnyRow) -> Result<LabelRuleRecord, TaskError> {
    let id = |column: &str| {
        row.get::<String, _>(column)
            .parse::<Id>()
            .map_err(|_| TaskError::Invalid { field: "id" })
    };
    Ok(LabelRuleRecord {
        id: id("id")?,
        project_id: id("project_id")?,
        label_id: id("label_id")?,
        label_name: row.get("label_name"),
        label_color: row.get("label_color"),
        user_id: id("user_id")?,
        user_name: row.get("user_name"),
        enabled: row.get("enabled"),
        created_at: TimestampMillis::from_millis(row.get("created_at")),
        updated_at: TimestampMillis::from_millis(row.get("updated_at")),
    })
}

#[cfg(test)]
mod tests {
    use super::{names_member, owner_name};

    #[test]
    fn owner_labels_name_a_member_by_handle_or_name_in_any_case() {
        assert_eq!(owner_name("owner:raymond"), Some("raymond"));
        assert_eq!(owner_name("Owner: David-S "), Some("David-S"));
        assert_eq!(owner_name("owner:"), None);
        assert_eq!(owner_name("bug"), None);
        assert_eq!(owner_name("own"), None);
        assert!(names_member("RAYMOND", "raymond@example.com", "Ray Mond"));
        assert!(names_member("david-s", "ds@example.com", "David S"));
        assert!(names_member("erol", "e.k@example.com", "Erol"));
        assert!(!names_member("david", "ds@example.com", "David S"));
    }
}
