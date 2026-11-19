//! The @handle of an agent's bot user. People see the part of a member's email before `@` as
//! their handle (web, iPhone, desktop), so an agent's bot user has the email
//! `<handle>@agents.orbit.invalid`, with the handle made from the agent's name: `Research Bot`
//! is `@research-bot`. A handle is unique in the agent's workspace (`-2`, `-3` … on a clash with
//! another member) and across all agents (the email is unique). Mentions, DMs and MCP use the
//! user id, never the handle, so a new handle breaks nothing.

use orbit_platform::{Database, Id, Row, TimestampMillis};

/// The domain of agent bot emails. `.invalid` never receives mail (RFC 2606).
const DOMAIN: &str = "agents.orbit.invalid";
const MAX_LEN: usize = 32;

/// The handle that `name` gives before a clash: lowercase ASCII letters and digits, other
/// characters as one `-`. `agent` when nothing is left.
pub fn slug(name: &str) -> String {
    let mut slug = String::new();
    for character in name.chars().flat_map(char::to_lowercase) {
        if character.is_ascii_alphanumeric() {
            slug.push(character);
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    slug.truncate(MAX_LEN);
    let slug = slug.trim_end_matches('-');
    if slug.is_empty() {
        "agent".to_owned()
    } else {
        slug.to_owned()
    }
}

/// The email of the bot user `user_id` with a handle from `name` that no other member of
/// `workspace_id` and no other agent has.
pub(crate) async fn email(
    conn: &mut sqlx::AnyConnection,
    workspace_id: Id,
    user_id: Id,
    name: &str,
) -> Result<String, sqlx::Error> {
    let base = slug(name);
    for number in 1_u32.. {
        let handle = if number == 1 {
            base.clone()
        } else {
            format!("{base}-{number}")
        };
        let email = format!("{handle}@{DOMAIN}");
        let taken: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM users u WHERE u.id <> $1 AND (u.normalized_email = $2 \
             OR (lower(u.email) LIKE $3 AND EXISTS (SELECT 1 FROM memberships m \
             WHERE m.user_id = u.id AND m.workspace_id = $4)))",
        )
        .bind(user_id.to_string())
        .bind(&email)
        .bind(format!("{handle}@%"))
        .bind(workspace_id.to_string())
        .fetch_one(&mut *conn)
        .await?;
        if taken == 0 {
            return Ok(email);
        }
    }
    unreachable!("a free handle is found before u32 ends")
}

/// Gives the bot user `user_id` the handle of `name` (after a create or a rename). Writes
/// nothing when its email already is the one it gets.
pub(crate) async fn sync(
    conn: &mut sqlx::AnyConnection,
    workspace_id: Id,
    user_id: Id,
    name: &str,
    now: TimestampMillis,
) -> Result<(), sqlx::Error> {
    let email = email(&mut *conn, workspace_id, user_id, name).await?;
    sqlx::query(
        "UPDATE users SET email = $1, normalized_email = $1, updated_at = $2, version = version + 1 \
         WHERE id = $3 AND normalized_email <> $1",
    )
    .bind(&email)
    .bind(now.as_millis())
    .bind(user_id.to_string())
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// Gives a handle to every agent bot user that still has an email of the form
/// `agent-<user id>@orbit.invalid` (agents made before handles). Runs at each start; after the
/// first run no such email is left, so it changes nothing. Returns the number of changed users.
pub async fn backfill(database: &Database) -> Result<u64, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT p.workspace_id, p.user_id, u.display_name FROM agent_profiles p \
         JOIN users u ON u.id = p.user_id WHERE u.normalized_email LIKE 'agent-%@orbit.invalid' \
         ORDER BY CASE WHEN p.deleted_at IS NULL THEN 0 ELSE 1 END, p.created_at, p.id",
    )
    .fetch_all(database.pool())
    .await?;
    // live agents first: they get the handle of their name before a deleted agent of that name
    let mut changed = 0;
    for row in rows {
        let workspace_id: String = row.get("workspace_id");
        let user_id: String = row.get("user_id");
        let (Ok(workspace_id), Ok(user_id)) = (workspace_id.parse::<Id>(), user_id.parse::<Id>())
        else {
            continue;
        };
        // a deleted agent keeps its name with " (deleted)"; its handle is the name it had
        let name: String = row.get("display_name");
        let name = name.strip_suffix(" (deleted)").unwrap_or(&name).to_owned();
        let mut tx = database.immediate_transaction().await?;
        sync(
            &mut tx,
            workspace_id,
            user_id,
            &name,
            TimestampMillis::now(),
        )
        .await?;
        tx.commit().await?;
        changed += 1;
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::slug;

    #[test]
    fn slug_is_lowercase_ascii_with_single_hyphens() {
        assert_eq!(slug("Research Bot"), "research-bot");
        assert_eq!(slug("  Donna!! (v2) "), "donna-v2");
        assert_eq!(slug("Zoë's helper"), "zo-s-helper");
        assert_eq!(slug("日本"), "agent");
        assert_eq!(slug(&"a".repeat(50)), "a".repeat(32));
        assert_eq!(
            slug("abcdefghijklmnopqrstuvwxyz abcd x"),
            "abcdefghijklmnopqrstuvwxyz-abcd"
        );
    }
}
