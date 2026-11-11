//! Saved scripts: JavaScript that an agent saved for a person, which the agent's code mode runs in a
//! sandbox (no network, no files) and which calls only that agent's own tools. A routine can run one on
//! each run. Its writes need the creator's yes to that exact code: the routine keeps the approved
//! `code_hash`, and while the script's hash differs, its runs write nothing (`super::runs`).
//!
//! Scripts belong to the person they were saved for. They see and change their own; owners and admins
//! (`routines.manage`) see every script of the workspace and may delete it, as for routines.

use orbit_domain::Permission;
use orbit_platform::{Id, Row, TimestampMillis};
use serde::{Deserialize, Serialize};
// The SELECT prefix is a constant: no input reaches the SQL text.
use sqlx::{AnyConnection, AssertSqlSafe};

use super::store::{ScheduledError, Store, clean_text, member};

pub const SCRIPT_MAX_CHARS: usize = 20_000;
pub const DESCRIPTION_MAX_CHARS: usize = 500;
/// Scripts one person may have in one workspace.
pub const MAX_SCRIPTS_PER_USER: i64 = 50;

/// The SHA-256 of the code, in hex: what a routine's write approval is bound to.
#[must_use]
pub fn code_hash(code: &str) -> String {
    crate::agent::integrations::hash(code)
}

#[derive(Clone, Debug, Serialize)]
pub struct ScriptRecord {
    pub id: String,
    pub workspace_id: String,
    pub owner_id: String,
    pub owner_name: String,
    pub name: String,
    pub description: String,
    pub code: String,
    pub code_hash: String,
    pub created_at: i64,
    pub updated_at: i64,
    /// The routines that run it, and whether their creator approved this exact code.
    pub routines: Vec<ScriptUse>,
    /// The caller may change it (only its owner).
    pub can_edit: bool,
    /// The caller may delete it (its owner, owners and admins).
    pub can_delete: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct ScriptUse {
    pub id: String,
    pub name: String,
    /// Its writes are approved for the script's current code.
    pub approved: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveScript {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub code: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScriptChange {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub code: Option<String>,
}

const SCRIPT_SELECT: &str = "SELECT s.id, s.workspace_id, s.owner_id, u.display_name AS owner_name, s.name, \
     s.description, s.code, s.code_hash, s.created_at, s.updated_at \
     FROM agent_scripts s JOIN users u ON u.id = s.owner_id";

fn clean_name(name: &str) -> Result<String, ScheduledError> {
    let line: String = name
        .chars()
        .map(|char| if char.is_control() { ' ' } else { char })
        .collect();
    clean_text(
        &line,
        80,
        "Give the script a name.",
        "The name can have at most 80 characters.",
    )
}

fn clean_description(description: &str) -> Result<String, ScheduledError> {
    let description = description.trim();
    if description.chars().count() > DESCRIPTION_MAX_CHARS {
        return Err(ScheduledError::Invalid(
            "The description can have at most 500 characters.",
        ));
    }
    Ok(description.to_owned())
}

fn clean_code(code: &str) -> Result<String, ScheduledError> {
    // Kept as written (its hash is what the creator approves), only without trailing blank space.
    let code = code.trim_end();
    if code.trim().is_empty() {
        return Err(ScheduledError::Invalid("The script has no code."));
    }
    if code.chars().count() > SCRIPT_MAX_CHARS {
        return Err(ScheduledError::Invalid(
            "A script can have at most 20,000 characters.",
        ));
    }
    Ok(code.to_owned())
}

async fn script_of(
    conn: &mut AnyConnection,
    row: &sqlx::any::AnyRow,
    actor: orbit_domain::Actor,
) -> Result<ScriptRecord, ScheduledError> {
    let owner: String = row.get("owner_id");
    let owner_id: Id = owner.parse().map_err(|_| ScheduledError::NotFound)?;
    let id: String = row.get("id");
    let code_hash: String = row.get("code_hash");
    let routines = sqlx::query(
        "SELECT id, name, script_approved_hash FROM agent_routines WHERE script_id = $1 ORDER BY created_at, id",
    )
    .bind(&id)
    .fetch_all(&mut *conn)
    .await?
    .iter()
    .map(|routine| ScriptUse {
        id: routine.get("id"),
        name: routine.get("name"),
        approved: routine
            .get::<Option<String>, _>("script_approved_hash")
            .as_deref()
            == Some(code_hash.as_str()),
    })
    .collect();
    Ok(ScriptRecord {
        id,
        workspace_id: row.get("workspace_id"),
        owner_id: owner,
        owner_name: row.get("owner_name"),
        name: row.get("name"),
        description: row.get("description"),
        code: row.get("code"),
        code_hash,
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
        routines,
        can_edit: actor.can_edit_routine(owner_id),
        can_delete: actor.can_stop_routine(owner_id),
    })
}

/// A script the caller may see: their own, or any for those who manage routines.
pub(super) async fn load_script(
    conn: &mut AnyConnection,
    workspace_id: Id,
    script_id: Id,
    actor: orbit_domain::Actor,
) -> Result<ScriptRecord, ScheduledError> {
    let row = sqlx::query(AssertSqlSafe(format!(
        "{SCRIPT_SELECT} WHERE s.id = $1 AND s.workspace_id = $2"
    )))
    .bind(script_id.to_string())
    .bind(workspace_id.to_string())
    .fetch_optional(&mut *conn)
    .await?
    .ok_or(ScheduledError::NotFound)?;
    let script = script_of(conn, &row, actor).await?;
    if !script.can_delete {
        return Err(ScheduledError::NotFound);
    }
    Ok(script)
}

/// The caller's own script (the only ones a routine of theirs may run): its name, code and hash.
pub(super) async fn own_script(
    conn: &mut AnyConnection,
    workspace_id: Id,
    user_id: Id,
    script_id: Id,
) -> Result<(String, String, String), ScheduledError> {
    let row = sqlx::query(
        "SELECT name, code, code_hash FROM agent_scripts WHERE id = $1 AND workspace_id = $2 AND owner_id = $3",
    )
    .bind(script_id.to_string())
    .bind(workspace_id.to_string())
    .bind(user_id.to_string())
    .fetch_optional(&mut *conn)
    .await?
    .ok_or(ScheduledError::Invalid("Pick one of your saved scripts."))?;
    Ok((row.get("name"), row.get("code"), row.get("code_hash")))
}

/// Whether the owner has another script with this name (names are unique per person, any case).
async fn name_taken(
    conn: &mut AnyConnection,
    workspace_id: Id,
    owner: Id,
    name: &str,
    except: Option<Id>,
) -> Result<bool, sqlx::Error> {
    let taken: Option<String> = sqlx::query_scalar(
        "SELECT id FROM agent_scripts WHERE workspace_id = $1 AND owner_id = $2 AND lower(name) = lower($3) \
         AND id <> $4",
    )
    .bind(workspace_id.to_string())
    .bind(owner.to_string())
    .bind(name)
    .bind(except.map(|id| id.to_string()).unwrap_or_default())
    .fetch_optional(&mut *conn)
    .await?;
    Ok(taken.is_some())
}

impl Store {
    /// Own scripts; every script of the workspace for those who manage routines.
    pub async fn list_scripts(
        &self,
        workspace_id: Id,
        user_id: Id,
    ) -> Result<Vec<ScriptRecord>, ScheduledError> {
        let mut conn = self.database.pool().acquire().await?;
        let actor = member(&mut conn, workspace_id, user_id).await?;
        let all = actor.can(Permission::RoutinesManage);
        let rows = sqlx::query(AssertSqlSafe(format!(
            "{SCRIPT_SELECT} WHERE s.workspace_id = $1 AND ($2 = 1 OR s.owner_id = $3) ORDER BY lower(s.name), s.id"
        )))
        .bind(workspace_id.to_string())
        .bind(i64::from(all))
        .bind(user_id.to_string())
        .fetch_all(&mut *conn)
        .await?;
        let mut scripts = Vec::with_capacity(rows.len());
        for row in &rows {
            scripts.push(script_of(&mut conn, row, actor).await?);
        }
        Ok(scripts)
    }

    /// Saves the caller's script under its name: a new one, or new code for the one of that name.
    /// New code changes the hash, so routines that run it write nothing until their creator approves
    /// it again (`ScriptRecord::routines`).
    pub async fn save_script(
        &self,
        workspace_id: Id,
        user_id: Id,
        input: SaveScript,
        now: TimestampMillis,
    ) -> Result<ScriptRecord, ScheduledError> {
        let name = clean_name(&input.name)?;
        let description = clean_description(input.description.as_deref().unwrap_or_default())?;
        let code = clean_code(&input.code)?;
        let hash = code_hash(&code);
        let mut tx = self.database.immediate_transaction().await?;
        let actor = member(&mut tx, workspace_id, user_id).await?;
        let existing: Option<String> = sqlx::query_scalar(
            "SELECT id FROM agent_scripts WHERE workspace_id = $1 AND owner_id = $2 AND lower(name) = lower($3)",
        )
        .bind(workspace_id.to_string())
        .bind(user_id.to_string())
        .bind(&name)
        .fetch_optional(&mut *tx)
        .await?;
        let id = match existing {
            Some(id) => {
                // An empty description keeps the old one.
                sqlx::query(
                    "UPDATE agent_scripts SET name = $1, description = CASE WHEN $2 = '' THEN description ELSE $2 END, \
                     code = $3, code_hash = $4, updated_at = $5 WHERE id = $6",
                )
                .bind(&name)
                .bind(&description)
                .bind(&code)
                .bind(&hash)
                .bind(now.as_millis())
                .bind(&id)
                .execute(&mut *tx)
                .await?;
                id.parse().map_err(|_| ScheduledError::NotFound)?
            }
            None => {
                let count: i64 = sqlx::query_scalar(
                    "SELECT COUNT(*) FROM agent_scripts WHERE workspace_id = $1 AND owner_id = $2",
                )
                .bind(workspace_id.to_string())
                .bind(user_id.to_string())
                .fetch_one(&mut *tx)
                .await?;
                if count >= MAX_SCRIPTS_PER_USER {
                    return Err(ScheduledError::Limit(
                        "You have 50 scripts in this workspace. Delete one first.",
                    ));
                }
                let id = Id::new_v7();
                sqlx::query(
                    "INSERT INTO agent_scripts (id, workspace_id, owner_id, name, description, code, code_hash, \
                     created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $8)",
                )
                .bind(id.to_string())
                .bind(workspace_id.to_string())
                .bind(user_id.to_string())
                .bind(&name)
                .bind(&description)
                .bind(&code)
                .bind(&hash)
                .bind(now.as_millis())
                .execute(&mut *tx)
                .await?;
                id
            }
        };
        let script = load_script(&mut tx, workspace_id, id, actor).await?;
        tx.commit().await?;
        Ok(script)
    }

    /// A new name, description or code, by the script's owner.
    pub async fn update_script(
        &self,
        workspace_id: Id,
        user_id: Id,
        script_id: Id,
        change: ScriptChange,
        now: TimestampMillis,
    ) -> Result<ScriptRecord, ScheduledError> {
        let mut tx = self.database.immediate_transaction().await?;
        let actor = member(&mut tx, workspace_id, user_id).await?;
        let current = load_script(&mut tx, workspace_id, script_id, actor).await?;
        if !current.can_edit {
            return Err(ScheduledError::Forbidden(
                "Only the person whose script it is can change it.",
            ));
        }
        let name = change
            .name
            .as_deref()
            .map(clean_name)
            .transpose()?
            .unwrap_or(current.name);
        if name_taken(&mut tx, workspace_id, user_id, &name, Some(script_id)).await? {
            return Err(ScheduledError::Invalid(
                "You have another script with this name.",
            ));
        }
        let description = change
            .description
            .as_deref()
            .map(clean_description)
            .transpose()?
            .unwrap_or(current.description);
        let code = change
            .code
            .as_deref()
            .map(clean_code)
            .transpose()?
            .unwrap_or(current.code);
        sqlx::query(
            "UPDATE agent_scripts SET name = $1, description = $2, code = $3, code_hash = $4, updated_at = $5 \
             WHERE id = $6",
        )
        .bind(&name)
        .bind(&description)
        .bind(&code)
        .bind(code_hash(&code))
        .bind(now.as_millis())
        .bind(script_id.to_string())
        .execute(&mut *tx)
        .await?;
        let script = load_script(&mut tx, workspace_id, script_id, actor).await?;
        tx.commit().await?;
        Ok(script)
    }

    /// Routines that ran it go back to their instruction (`ON DELETE SET NULL`).
    pub async fn delete_script(
        &self,
        workspace_id: Id,
        user_id: Id,
        script_id: Id,
    ) -> Result<(), ScheduledError> {
        let mut tx = self.database.immediate_transaction().await?;
        let actor = member(&mut tx, workspace_id, user_id).await?;
        load_script(&mut tx, workspace_id, script_id, actor).await?;
        sqlx::query("UPDATE agent_routines SET script_approved_hash = NULL WHERE script_id = $1")
            .bind(script_id.to_string())
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM agent_scripts WHERE id = $1")
            .bind(script_id.to_string())
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }
}
