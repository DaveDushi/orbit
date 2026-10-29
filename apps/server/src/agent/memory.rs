//! The agent's long-term memory: per workspace a team memory, and per person a private one. Each is
//! a log of one-line memories (positions 0, 1, 2, … without gaps) and a binary tree of one-line
//! summaries over aligned blocks (`#0-1`, `#2-3`, `#0-3`, …) that is only a cache of the log. A run
//! reads the "wake view": recent memories word for word, older ones as coarser summaries. The
//! model writes the summaries when a note makes a block due. Design: `.ai/agent-design.md`,
//! "Memory" (after OptMem, reimplemented).

use std::collections::{HashMap, HashSet};

use orbit_domain::Actor;
use orbit_platform::{Database, Id, Row, SqlBuilder, TimestampMillis, generate_opaque_token};
use serde::Serialize;
use sqlx::AnyConnection;

use crate::repositories::membership;

/// The longest memory or summary, in UTF-8 bytes.
pub const MAX_BYTES: usize = 280;
/// Memories one scope can hold.
pub const MAX_MEMORIES: i64 = 20_000;
/// How long a run may take to write a summary it claimed.
const CLAIM_MS: i64 = 10 * 60 * 1_000;
/// The largest wake view a caller may ask for, in bytes (≈ 20k tokens).
pub const MAX_WAKE_BYTES: usize = 80_000;
/// The most lines a wake view can have.
const MAX_WAKE_LINES: u64 = 400;

/// Which memory a request is about. `Private` is always the caller's own.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Scope {
    Team,
    Private,
}

impl Scope {
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "team" => Some(Self::Team),
            "private" => Some(Self::Private),
            _ => None,
        }
    }

    /// The owner of the scope's rows: nobody for the team memory.
    const fn owner(self, user_id: Id) -> Option<Id> {
        match self {
            Self::Team => None,
            Self::Private => Some(user_id),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum MemoryError {
    #[error("not found")]
    NotFound,
    #[error("forbidden")]
    Forbidden,
    #[error("{0}")]
    Invalid(&'static str),
    #[error("the memory is full")]
    Full,
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

// ---------------------------------------------------------------------------------------------
// The tree, without storage.

/// Splits `[0, total)` into aligned power-of-two blocks `(lo, size)`, oldest first. A block stays
/// whole when it lies inside `[0, total)` and its size is at most `alpha` times its age
/// (`total - lo`), so blocks get bigger with age. A larger `alpha` gives fewer blocks.
fn tiles(total: u64, alpha: f64) -> Vec<(u64, u64)> {
    let mut found = Vec::new();
    let mut todo = vec![(0, total.next_power_of_two().max(1))];
    while let Some((lo, size)) = todo.pop() {
        if lo >= total {
            continue;
        }
        #[allow(clippy::cast_precision_loss)]
        let too_big = size as f64 > alpha * (total - lo) as f64;
        if size > 1 && (lo + size > total || too_big) {
            let half = size / 2;
            todo.push((lo + half, half));
            todo.push((lo, half));
        } else {
            found.push((lo, size));
        }
    }
    found
}

/// The blocks of a wake view with at most `lines` lines (at least one), oldest first: as fine as
/// the budget allows, finest at the newest memories.
#[must_use]
pub fn cover(total: u64, lines: u64) -> Vec<(u64, u64)> {
    let lines = lines.max(1);
    if total <= lines {
        return (0..total).map(|position| (position, 1)).collect();
    }
    // Fewer blocks as alpha grows, down to one block per set bit of `total`: find the smallest
    // alpha that fits, or the fewest blocks when none does.
    let (mut low, mut high) = (0.0_f64, 1.0_f64);
    let fewest = u64::from(total.count_ones());
    while tiles(total, high).len() as u64 > lines.max(fewest) {
        high *= 2.0;
    }
    for _ in 0..50 {
        let middle = f64::midpoint(low, high);
        if tiles(total, middle).len() as u64 > lines {
            low = middle;
        } else {
            high = middle;
        }
    }
    let mut blocks = tiles(total, high);
    // Sizes jump in powers of two: spend the lines left on the newest blocks.
    while (blocks.len() as u64) < lines {
        let Some(index) = blocks.iter().rposition(|(_, size)| *size > 1) else {
            break;
        };
        let (lo, size) = blocks[index];
        blocks.splice(index..=index, [(lo, size / 2), (lo + size / 2, size / 2)]);
    }
    blocks
}

/// What the store knows about one block's summary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Summary {
    /// Written; `empty` when every memory under it is deleted.
    Written { empty: bool },
    /// A run is writing it until this time (ms).
    Claimed { until: i64 },
}

/// A block whose summary can be written now: both halves exist.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DueBlock {
    pub lo: u64,
    pub size: u64,
    /// Every memory under it is deleted: its summary is empty, no model needed.
    pub empty: bool,
}

/// Blocks that are due, smallest first, then oldest. A block is due when it lies inside
/// `[0, total)`, has no summary and no live claim, and both halves exist (a memory, or a written
/// summary). `deleted` holds the deleted positions.
#[must_use]
pub fn due(
    total: u64,
    summaries: &HashMap<(u64, u64), Summary>,
    deleted: &HashSet<u64>,
    now: i64,
) -> Vec<DueBlock> {
    // `Some(empty)` when the half exists.
    let half = |lo: u64, size: u64| -> Option<bool> {
        if size == 1 {
            return Some(deleted.contains(&lo));
        }
        match summaries.get(&(lo, size)) {
            Some(Summary::Written { empty }) => Some(*empty),
            _ => None,
        }
    };
    let mut found = Vec::new();
    let mut size = 2;
    while size <= total {
        for index in 0..total / size {
            let lo = index * size;
            match summaries.get(&(lo, size)) {
                Some(Summary::Written { .. }) => continue,
                Some(Summary::Claimed { until }) if *until > now => continue,
                _ => {}
            }
            let (Some(first), Some(second)) = (half(lo, size / 2), half(lo + size / 2, size / 2))
            else {
                continue;
            };
            found.push(DueBlock {
                lo,
                size,
                empty: first && second,
            });
        }
        size *= 2;
    }
    found
}

/// `#lo-hi` (inclusive) parsed into `(lo, size)`; only real blocks: an aligned power of two of at
/// least two memories.
#[must_use]
pub fn parse_block(value: &str) -> Option<(u64, u64)> {
    let (lo, hi) = value.trim_start_matches('#').split_once('-')?;
    let lo: u64 = lo.parse().ok()?;
    let hi: u64 = hi.parse().ok()?;
    let size = hi.checked_sub(lo)?.checked_add(1)?;
    (size >= 2 && size.is_power_of_two() && lo.is_multiple_of(size)).then_some((lo, size))
}

/// One line of memory: a memory (`lo == hi`) or the summary of `#lo-hi`.
#[derive(Clone, Debug, Serialize)]
pub struct Line {
    pub lo: u64,
    pub hi: u64,
    pub body: String,
    /// For memories: who noted it and when.
    pub author_name: Option<String>,
    pub created_at: Option<TimestampMillis>,
}

impl Line {
    /// The line as the model reads it.
    #[must_use]
    pub fn render(&self) -> String {
        if self.lo == self.hi {
            let date = self
                .created_at
                .map(|at| at.to_string().chars().take(10).collect::<String>())
                .unwrap_or_default();
            let author = self.author_name.as_deref().unwrap_or("someone");
            format!("#{} ({date}, {author}) {}", self.lo, self.body)
        } else {
            format!("#{}-{} {}", self.lo, self.hi, self.body)
        }
    }
}

fn check_line(body: &str) -> Result<String, MemoryError> {
    let body = body.trim();
    if body.is_empty() {
        return Err(MemoryError::Invalid("A memory needs text."));
    }
    if body.contains(['\n', '\r']) {
        return Err(MemoryError::Invalid(
            "A memory is one line: join the lines, or note them one by one.",
        ));
    }
    if body.len() > MAX_BYTES {
        return Err(MemoryError::Invalid(
            "A memory has at most 280 bytes (accented letters count 2): make it shorter.",
        ));
    }
    Ok(body.to_owned())
}

fn to_i64(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn to_u64(value: i64) -> u64 {
    u64::try_from(value).unwrap_or_default()
}

// ---------------------------------------------------------------------------------------------
// Storage.

/// Where a memory came from: a chat thread (`root_message_id`) or a DM's main list.
#[derive(Clone, Debug, Default, Serialize, serde::Deserialize)]
pub struct Source {
    pub conversation_id: Option<Id>,
    pub root_message_id: Option<Id>,
}

/// A block the caller is asked to summarize. Only the holder of `claim` can save it.
#[derive(Debug, Serialize)]
pub struct Due {
    pub lo: u64,
    pub hi: u64,
    pub claim: String,
    pub halves: Vec<Line>,
    /// More blocks due after this one.
    pub remaining: usize,
}

#[derive(Debug, Serialize)]
pub struct Noted {
    pub position: u64,
    pub due: Option<Due>,
}

#[derive(Debug, Serialize)]
pub struct Summarized {
    pub saved: bool,
    pub due: Option<Due>,
}

#[derive(Debug, Serialize)]
pub struct Wake {
    /// Memories ever noted in the scope.
    pub total: u64,
    pub lines: Vec<Line>,
    pub text: String,
}

#[derive(Debug, Serialize)]
pub struct Memory {
    pub position: u64,
    pub body: String,
    pub author_id: Option<String>,
    pub author_name: Option<String>,
    pub created_at: TimestampMillis,
    pub source: Option<Source>,
}

#[derive(Debug, Serialize)]
pub struct MemoryPage {
    pub items: Vec<Memory>,
    /// Pass as `before` for older ones.
    pub next_before: Option<u64>,
    /// The caller may delete memories and summaries of this scope.
    pub can_delete: bool,
}

/// The scope's id and size, or `None` when nothing was noted in it yet.
async fn find_scope(
    conn: &mut AnyConnection,
    workspace_id: Id,
    owner: Option<Id>,
) -> Result<Option<(String, u64)>, sqlx::Error> {
    let row = sqlx::query(
        "SELECT id, size FROM agent_memory_scopes WHERE workspace_id = $1 \
         AND owner_user_id IS NOT DISTINCT FROM $2",
    )
    .bind(workspace_id.to_string())
    .bind(owner.map(|id| id.to_string()))
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row.map(|row| (row.get("id"), to_u64(row.get("size")))))
}

/// The caller as a member, and the scope's owner. Outsiders get `NotFound`.
async fn member(
    conn: &mut AnyConnection,
    workspace_id: Id,
    user_id: Id,
    scope: Scope,
) -> Result<(Actor, Option<Id>), MemoryError> {
    let actor = membership::actor(&mut *conn, workspace_id, user_id)
        .await?
        .ok_or(MemoryError::NotFound)?;
    Ok((actor, scope.owner(user_id)))
}

/// Every written or claimed summary of a scope, and its deleted positions.
async fn tree_state(
    conn: &mut AnyConnection,
    scope_id: &str,
) -> Result<(HashMap<(u64, u64), Summary>, HashSet<u64>), sqlx::Error> {
    let rows = sqlx::query(
        "SELECT lo, size, body, claimed_until FROM agent_memory_summaries WHERE scope_id = $1",
    )
    .bind(scope_id)
    .fetch_all(&mut *conn)
    .await?;
    let summaries = rows
        .iter()
        .map(|row| {
            let key = (to_u64(row.get("lo")), to_u64(row.get("size")));
            let state = match row.get::<Option<String>, _>("body") {
                Some(body) => Summary::Written {
                    empty: body.is_empty(),
                },
                None => Summary::Claimed {
                    until: row
                        .get::<Option<i64>, _>("claimed_until")
                        .unwrap_or_default(),
                },
            };
            (key, state)
        })
        .collect();
    let deleted = sqlx::query_scalar::<_, i64>(
        "SELECT position FROM agent_memories WHERE scope_id = $1 AND deleted_at IS NOT NULL",
    )
    .bind(scope_id)
    .fetch_all(&mut *conn)
    .await?
    .into_iter()
    .map(to_u64)
    .collect();
    Ok((summaries, deleted))
}

/// Claims the next due block for `user_id`, after giving fully deleted blocks their empty
/// summary. Call inside an immediate transaction.
async fn claim_next(
    conn: &mut AnyConnection,
    scope_id: &str,
    total: u64,
    user_id: Id,
) -> Result<Option<Due>, sqlx::Error> {
    let now = TimestampMillis::now().as_millis();
    loop {
        let (summaries, deleted) = tree_state(conn, scope_id).await?;
        let blocks = due(total, &summaries, &deleted, now);
        let empty: Vec<_> = blocks.iter().filter(|block| block.empty).collect();
        if !empty.is_empty() {
            for block in empty {
                upsert_summary(
                    conn,
                    scope_id,
                    block.lo,
                    block.size,
                    Some(""),
                    None,
                    None,
                    now,
                )
                .await?;
            }
            // Parents of these blocks may be due now.
            continue;
        }
        let Some(block) = blocks.first() else {
            return Ok(None);
        };
        let claim = generate_opaque_token();
        upsert_summary(
            conn,
            scope_id,
            block.lo,
            block.size,
            None,
            Some(&claim),
            Some(user_id),
            now,
        )
        .await?;
        let mut reader = Reader::new(scope_id);
        let half = block.size / 2;
        let halves = reader
            .halves(conn, &[(block.lo, half), (block.lo + half, half)])
            .await?;
        return Ok(Some(Due {
            lo: block.lo,
            hi: block.lo + block.size - 1,
            claim,
            halves,
            remaining: blocks.len() - 1,
        }));
    }
}

/// Writes a summary (`body`) or a claim on it.
#[allow(clippy::too_many_arguments)]
async fn upsert_summary(
    conn: &mut AnyConnection,
    scope_id: &str,
    lo: u64,
    size: u64,
    body: Option<&str>,
    claim: Option<&str>,
    author: Option<Id>,
    now: i64,
) -> Result<(), sqlx::Error> {
    let claimed_until = claim.map(|_| now + CLAIM_MS);
    sqlx::query(
        "INSERT INTO agent_memory_summaries (scope_id, lo, size, body, claim, claimed_until, author_id, created_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
         ON CONFLICT (scope_id, lo, size) DO UPDATE SET body = $4, claim = $5, claimed_until = $6, \
         author_id = $7, created_at = $8",
    )
    .bind(scope_id)
    .bind(to_i64(lo))
    .bind(to_i64(size))
    .bind(body)
    .bind(claim)
    .bind(claimed_until)
    .bind(author.map(|id| id.to_string()))
    .bind(now)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// Reads memories and summaries of one scope, with a cache so the wake budget search reads each
/// row once.
struct Reader<'a> {
    scope_id: &'a str,
    memories: HashMap<u64, Option<Line>>,
    /// `None`: no written summary.
    summaries: HashMap<(u64, u64), Option<String>>,
}

impl<'a> Reader<'a> {
    fn new(scope_id: &'a str) -> Self {
        Self {
            scope_id,
            memories: HashMap::new(),
            summaries: HashMap::new(),
        }
    }

    async fn load_summaries(
        &mut self,
        conn: &mut AnyConnection,
        blocks: &[(u64, u64)],
    ) -> Result<(), sqlx::Error> {
        let missing: Vec<_> = blocks
            .iter()
            .filter(|block| block.1 > 1 && !self.summaries.contains_key(block))
            .copied()
            .collect();
        for chunk in missing.chunks(100) {
            let mut query = SqlBuilder::new(
                "SELECT lo, size, body FROM agent_memory_summaries WHERE body IS NOT NULL AND scope_id = ",
            );
            query.push_bind(self.scope_id).push(" AND (");
            for (index, (lo, size)) in chunk.iter().enumerate() {
                query
                    .push(if index == 0 { "(lo = " } else { " OR (lo = " })
                    .push_bind(to_i64(*lo))
                    .push(" AND size = ")
                    .push_bind(to_i64(*size))
                    .push(")");
            }
            query.push(")");
            for block in chunk {
                self.summaries.insert(*block, None);
            }
            for row in query.build().fetch_all(&mut *conn).await? {
                self.summaries.insert(
                    (to_u64(row.get("lo")), to_u64(row.get("size"))),
                    row.get("body"),
                );
            }
        }
        Ok(())
    }

    async fn load_memories(
        &mut self,
        conn: &mut AnyConnection,
        positions: &[u64],
    ) -> Result<(), sqlx::Error> {
        let missing: Vec<_> = positions
            .iter()
            .filter(|position| !self.memories.contains_key(position))
            .copied()
            .collect();
        for chunk in missing.chunks(200) {
            let mut query = SqlBuilder::new(
                "SELECT m.position, m.body, m.deleted_at, m.created_at, u.display_name FROM agent_memories m \
                 LEFT JOIN users u ON u.id = m.author_id WHERE m.scope_id = ",
            );
            query.push_bind(self.scope_id).push(" AND m.position IN (");
            let mut list = query.separated(", ");
            for position in chunk {
                list.push_bind(to_i64(*position));
            }
            query.push(")");
            for position in chunk {
                self.memories.insert(*position, None);
            }
            for row in query.build().fetch_all(&mut *conn).await? {
                if row.get::<Option<i64>, _>("deleted_at").is_some() {
                    continue;
                }
                let position = to_u64(row.get("position"));
                self.memories.insert(
                    position,
                    Some(Line {
                        lo: position,
                        hi: position,
                        body: row.get("body"),
                        author_name: row.get("display_name"),
                        created_at: Some(TimestampMillis::from_millis(row.get("created_at"))),
                    }),
                );
            }
        }
        Ok(())
    }

    /// The lines for `blocks`, oldest first. A block without a summary shows as its halves, a
    /// deleted memory and an empty summary show as nothing.
    async fn lines(
        &mut self,
        conn: &mut AnyConnection,
        blocks: &[(u64, u64)],
    ) -> Result<Vec<Line>, sqlx::Error> {
        let mut blocks = blocks.to_vec();
        loop {
            self.load_summaries(conn, &blocks).await?;
            let mut split = false;
            let mut next = Vec::with_capacity(blocks.len());
            for (lo, size) in blocks {
                if size > 1 && self.summaries.get(&(lo, size)).is_some_and(Option::is_none) {
                    next.extend([(lo, size / 2), (lo + size / 2, size / 2)]);
                    split = true;
                } else {
                    next.push((lo, size));
                }
            }
            blocks = next;
            if !split {
                break;
            }
        }
        let positions: Vec<_> = blocks
            .iter()
            .filter(|block| block.1 == 1)
            .map(|block| block.0)
            .collect();
        self.load_memories(conn, &positions).await?;
        Ok(blocks
            .into_iter()
            .filter_map(|(lo, size)| {
                if size == 1 {
                    return self.memories.get(&lo).cloned().flatten();
                }
                let body = self.summaries.get(&(lo, size)).cloned().flatten()?;
                (!body.is_empty()).then(|| Line {
                    lo,
                    hi: lo + size - 1,
                    body,
                    author_name: None,
                    created_at: None,
                })
            })
            .collect())
    }

    /// One line per half (for a summary prompt or a zoom), with "(deleted)" for empty halves.
    async fn halves(
        &mut self,
        conn: &mut AnyConnection,
        halves: &[(u64, u64)],
    ) -> Result<Vec<Line>, sqlx::Error> {
        let mut found = Vec::new();
        for &(lo, size) in halves {
            let lines = self.lines(conn, &[(lo, size)]).await?;
            if lines.is_empty() {
                found.push(Line {
                    lo,
                    hi: lo + size - 1,
                    body: "(deleted)".to_owned(),
                    author_name: None,
                    created_at: None,
                });
            } else {
                found.extend(lines);
            }
        }
        Ok(found)
    }
}

/// Memory operations; every one checks the caller's membership first.
#[derive(Clone)]
pub struct MemoryStore {
    database: Database,
}

impl MemoryStore {
    #[must_use]
    pub const fn new(database: Database) -> Self {
        Self { database }
    }

    /// Appends one memory and claims the next due summary for the caller.
    pub async fn note(
        &self,
        workspace_id: Id,
        user_id: Id,
        scope: Scope,
        body: &str,
        source: &Source,
    ) -> Result<Noted, MemoryError> {
        let body = check_line(body)?;
        let now = TimestampMillis::now().as_millis();
        let mut tx = self.database.immediate_transaction().await?;
        let (_, owner) = member(&mut tx, workspace_id, user_id, scope).await?;
        if let Some(conversation) = source.conversation_id {
            let found = sqlx::query_scalar::<_, String>(
                "SELECT id FROM chat_conversations WHERE id = $1 AND workspace_id = $2",
            )
            .bind(conversation.to_string())
            .bind(workspace_id.to_string())
            .fetch_optional(&mut *tx)
            .await?;
            if found.is_none() {
                return Err(MemoryError::Invalid("The source conversation is unknown."));
            }
        }
        if let Some(root) = source.root_message_id {
            let found = sqlx::query_scalar::<_, String>(
                "SELECT id FROM chat_messages WHERE id = $1 AND conversation_id = $2",
            )
            .bind(root.to_string())
            .bind(source.conversation_id.map(|id| id.to_string()))
            .fetch_optional(&mut *tx)
            .await?;
            if found.is_none() {
                return Err(MemoryError::Invalid("The source thread is unknown."));
            }
        }
        let scope_id = match find_scope(&mut tx, workspace_id, owner).await? {
            Some((id, _)) => id,
            None => {
                let id = Id::new_v7().to_string();
                sqlx::query(
                    "INSERT INTO agent_memory_scopes (id, workspace_id, owner_user_id, size, created_at) \
                     VALUES ($1, $2, $3, 0, $4)",
                )
                .bind(&id)
                .bind(workspace_id.to_string())
                .bind(owner.map(|id| id.to_string()))
                .bind(now)
                .execute(&mut *tx)
                .await?;
                id
            }
        };
        let size: i64 = sqlx::query_scalar(
            "UPDATE agent_memory_scopes SET size = size + 1 WHERE id = $1 RETURNING size",
        )
        .bind(&scope_id)
        .fetch_one(&mut *tx)
        .await?;
        if size > MAX_MEMORIES {
            return Err(MemoryError::Full);
        }
        let position = size - 1;
        sqlx::query(
            "INSERT INTO agent_memories (scope_id, position, body, author_id, source_conversation_id, \
             source_root_message_id, created_at) VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(&scope_id)
        .bind(position)
        .bind(&body)
        .bind(user_id.to_string())
        .bind(source.conversation_id.map(|id| id.to_string()))
        .bind(source.root_message_id.map(|id| id.to_string()))
        .bind(now)
        .execute(&mut *tx)
        .await?;
        let due = claim_next(&mut tx, &scope_id, to_u64(size), user_id).await?;
        tx.commit().await?;
        Ok(Noted {
            position: to_u64(position),
            due,
        })
    }

    /// Saves the summary of `(lo, size)` when `claim` still holds it, then claims the next due one.
    pub async fn summarize(
        &self,
        workspace_id: Id,
        user_id: Id,
        scope: Scope,
        (lo, size): (u64, u64),
        claim: &str,
        body: &str,
    ) -> Result<Summarized, MemoryError> {
        let body = check_line(body)?;
        let now = TimestampMillis::now().as_millis();
        let mut tx = self.database.immediate_transaction().await?;
        let (_, owner) = member(&mut tx, workspace_id, user_id, scope).await?;
        let Some((scope_id, total)) = find_scope(&mut tx, workspace_id, owner).await? else {
            return Err(MemoryError::NotFound);
        };
        let saved = sqlx::query(
            "UPDATE agent_memory_summaries SET body = $1, claim = NULL, claimed_until = NULL, \
             author_id = $2, created_at = $3 \
             WHERE scope_id = $4 AND lo = $5 AND size = $6 AND claim = $7 AND body IS NULL",
        )
        .bind(&body)
        .bind(user_id.to_string())
        .bind(now)
        .bind(&scope_id)
        .bind(to_i64(lo))
        .bind(to_i64(size))
        .bind(claim)
        .execute(&mut *tx)
        .await?
        .rows_affected()
            == 1;
        let due = claim_next(&mut tx, &scope_id, total, user_id).await?;
        tx.commit().await?;
        Ok(Summarized { saved, due })
    }

    /// The wake view of a scope in at most `bytes` bytes of text.
    pub async fn wake(
        &self,
        workspace_id: Id,
        user_id: Id,
        scope: Scope,
        bytes: usize,
    ) -> Result<Wake, MemoryError> {
        let bytes = bytes.min(MAX_WAKE_BYTES);
        let mut conn = self.database.pool().acquire().await?;
        let (_, owner) = member(&mut conn, workspace_id, user_id, scope).await?;
        let Some((scope_id, total)) = find_scope(&mut conn, workspace_id, owner).await? else {
            return Ok(Wake {
                total: 0,
                lines: Vec::new(),
                text: String::new(),
            });
        };
        let mut reader = Reader::new(&scope_id);
        let size = |lines: &[Line]| {
            lines
                .iter()
                .map(|line| line.render().len() + 1)
                .sum::<usize>()
        };
        // The most lines that fit: more lines read more detail and more bytes.
        let (mut low, mut high) = (0_u64, total.min(MAX_WAKE_LINES));
        let mut best = Vec::new();
        while low < high {
            let middle = low + (high - low).div_ceil(2);
            let lines = reader.lines(&mut conn, &cover(total, middle)).await?;
            if size(&lines) <= bytes {
                best = lines;
                low = middle;
            } else {
                high = middle - 1;
            }
        }
        let text = best.iter().map(Line::render).collect::<Vec<_>>().join("\n");
        Ok(Wake {
            total,
            lines: best,
            text,
        })
    }

    /// The two halves of block `(lo, size)`.
    pub async fn zoom(
        &self,
        workspace_id: Id,
        user_id: Id,
        scope: Scope,
        (lo, size): (u64, u64),
    ) -> Result<Vec<Line>, MemoryError> {
        let mut conn = self.database.pool().acquire().await?;
        let (_, owner) = member(&mut conn, workspace_id, user_id, scope).await?;
        let Some((scope_id, total)) = find_scope(&mut conn, workspace_id, owner).await? else {
            return Err(MemoryError::NotFound);
        };
        if lo + size > total {
            return Err(MemoryError::NotFound);
        }
        let half = size / 2;
        Ok(Reader::new(&scope_id)
            .halves(&mut conn, &[(lo, half), (lo + half, half)])
            .await?)
    }

    /// Memories, newest first; with `query`, only those that contain every word of it.
    pub async fn list(
        &self,
        workspace_id: Id,
        user_id: Id,
        scope: Scope,
        query: Option<&str>,
        before: Option<u64>,
        limit: u64,
    ) -> Result<MemoryPage, MemoryError> {
        let mut conn = self.database.pool().acquire().await?;
        let (actor, owner) = member(&mut conn, workspace_id, user_id, scope).await?;
        let can_delete = actor.can_delete_agent_memory(owner);
        let Some((scope_id, _)) = find_scope(&mut conn, workspace_id, owner).await? else {
            return Ok(MemoryPage {
                items: Vec::new(),
                next_before: None,
                can_delete,
            });
        };
        let limit = limit.clamp(1, 100);
        let mut sql = SqlBuilder::new(
            "SELECT m.position, m.body, m.author_id, u.display_name, m.source_conversation_id, \
             m.source_root_message_id, m.created_at FROM agent_memories m \
             LEFT JOIN users u ON u.id = m.author_id WHERE m.deleted_at IS NULL AND m.scope_id = ",
        );
        sql.push_bind(scope_id);
        if let Some(before) = before {
            sql.push(" AND m.position < ").push_bind(to_i64(before));
        }
        for word in query.unwrap_or_default().split_whitespace().take(8) {
            let escaped = word
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_");
            sql.push(" AND lower(m.body) LIKE lower(")
                .push_bind(format!("%{escaped}%"))
                .push(") ESCAPE '\\'");
        }
        sql.push(" ORDER BY m.position DESC LIMIT ")
            .push_bind(to_i64(limit + 1));
        let rows = sql.build().fetch_all(&mut *conn).await?;
        let more = rows.len() as u64 > limit;
        let items: Vec<Memory> = rows
            .iter()
            .take(usize::try_from(limit).unwrap_or(usize::MAX))
            .map(|row| {
                let conversation = row
                    .get::<Option<String>, _>("source_conversation_id")
                    .and_then(|id| id.parse().ok());
                Memory {
                    position: to_u64(row.get("position")),
                    body: row.get("body"),
                    author_id: row.get("author_id"),
                    author_name: row.get("display_name"),
                    created_at: TimestampMillis::from_millis(row.get("created_at")),
                    source: conversation.map(|conversation_id| Source {
                        conversation_id: Some(conversation_id),
                        root_message_id: row
                            .get::<Option<String>, _>("source_root_message_id")
                            .and_then(|id| id.parse().ok()),
                    }),
                }
            })
            .collect();
        Ok(MemoryPage {
            next_before: if more {
                items.last().map(|memory| memory.position)
            } else {
                None
            },
            items,
            can_delete,
        })
    }

    /// Erases one memory and drops every summary built from it; they are written again later.
    pub async fn delete(
        &self,
        workspace_id: Id,
        user_id: Id,
        scope: Scope,
        position: u64,
    ) -> Result<(), MemoryError> {
        let now = TimestampMillis::now().as_millis();
        let mut tx = self.database.immediate_transaction().await?;
        let (actor, owner) = member(&mut tx, workspace_id, user_id, scope).await?;
        if !actor.can_delete_agent_memory(owner) {
            return Err(MemoryError::Forbidden);
        }
        let Some((scope_id, _)) = find_scope(&mut tx, workspace_id, owner).await? else {
            return Err(MemoryError::NotFound);
        };
        let erased = sqlx::query(
            "UPDATE agent_memories SET body = '', deleted_at = $1, deleted_by = $2 \
             WHERE scope_id = $3 AND position = $4 AND deleted_at IS NULL",
        )
        .bind(now)
        .bind(user_id.to_string())
        .bind(&scope_id)
        .bind(to_i64(position))
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if erased == 0 {
            return Err(MemoryError::NotFound);
        }
        sqlx::query(
            "DELETE FROM agent_memory_summaries WHERE scope_id = $1 AND lo <= $2 AND lo + size > $2",
        )
        .bind(&scope_id)
        .bind(to_i64(position))
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Drops the summary of `(lo, size)` and the summaries built from it (OptMem's `forget`).
    pub async fn forget(
        &self,
        workspace_id: Id,
        user_id: Id,
        scope: Scope,
        (lo, size): (u64, u64),
    ) -> Result<(), MemoryError> {
        let mut tx = self.database.immediate_transaction().await?;
        let (actor, owner) = member(&mut tx, workspace_id, user_id, scope).await?;
        if !actor.can_delete_agent_memory(owner) {
            return Err(MemoryError::Forbidden);
        }
        let Some((scope_id, _)) = find_scope(&mut tx, workspace_id, owner).await? else {
            return Err(MemoryError::NotFound);
        };
        let dropped = sqlx::query(
            "DELETE FROM agent_memory_summaries WHERE scope_id = $1 AND lo <= $2 AND lo + size >= $3",
        )
        .bind(&scope_id)
        .bind(to_i64(lo))
        .bind(to_i64(lo + size))
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if dropped == 0 {
            return Err(MemoryError::NotFound);
        }
        tx.commit().await?;
        Ok(())
    }
}

/// Deletes a person's private memory in a workspace they leave. Runs in the removal's transaction.
pub(crate) async fn drop_private(
    conn: &mut AnyConnection,
    workspace_id: Id,
    user_id: Id,
) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM agent_memory_scopes WHERE workspace_id = $1 AND owner_user_id = $2")
        .bind(workspace_id.to_string())
        .bind(user_id.to_string())
        .execute(&mut *conn)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use super::{DueBlock, Summary, cover, due, parse_block};

    fn written(blocks: &[(u64, u64)]) -> HashMap<(u64, u64), Summary> {
        blocks
            .iter()
            .map(|block| (*block, Summary::Written { empty: false }))
            .collect()
    }

    /// The blocks tile `[0, total)` in order, without gaps or overlaps, and are aligned.
    fn assert_tiling(total: u64, blocks: &[(u64, u64)]) {
        let mut next = 0;
        for &(lo, size) in blocks {
            assert_eq!(lo, next, "{blocks:?}");
            assert!(
                size.is_power_of_two() && lo.is_multiple_of(size),
                "{blocks:?}"
            );
            next = lo + size;
        }
        assert_eq!(next, total, "{blocks:?}");
    }

    #[test]
    fn a_small_memory_is_read_word_for_word() {
        assert_eq!(cover(3, 10), vec![(0, 1), (1, 1), (2, 1)]);
        assert!(cover(0, 10).is_empty());
    }

    #[test]
    fn the_wake_view_fits_its_budget_and_keeps_the_newest_in_detail() {
        for total in [5_u64, 17, 100, 1_000, 12_345] {
            for lines in [1_u64, 4, 12, 40, 96] {
                let blocks = cover(total, lines);
                assert_tiling(total, &blocks);
                let fewest = u64::from(total.count_ones());
                assert!(
                    blocks.len() as u64 <= lines.max(fewest),
                    "{total} {lines}: {}",
                    blocks.len()
                );
                // Sizes never grow towards the newest memories.
                assert!(
                    blocks.windows(2).all(|pair| pair[0].1 >= pair[1].1),
                    "{blocks:?}"
                );
                if lines >= 12 {
                    assert_eq!(blocks.last().unwrap().1, 1, "the newest memory is whole");
                    // The budget is used, not wasted.
                    assert!(
                        blocks.len() as u64 > lines.min(total) / 2,
                        "{total} {lines}: {blocks:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn blocks_are_due_bottom_up_once_their_halves_exist() {
        let none = HashSet::new();
        // 5 memories: #0-1 and #2-3 are due; #0-3 needs them first.
        let found = due(5, &HashMap::new(), &none, 0);
        assert_eq!(
            found,
            vec![
                DueBlock {
                    lo: 0,
                    size: 2,
                    empty: false
                },
                DueBlock {
                    lo: 2,
                    size: 2,
                    empty: false
                }
            ]
        );
        let found = due(5, &written(&[(0, 2), (2, 2)]), &none, 0);
        assert_eq!(
            found,
            vec![DueBlock {
                lo: 0,
                size: 4,
                empty: false
            }]
        );
        assert!(due(5, &written(&[(0, 2), (2, 2), (0, 4)]), &none, 0).is_empty());
    }

    #[test]
    fn a_live_claim_hides_a_block_and_an_old_one_does_not() {
        let mut summaries = written(&[]);
        summaries.insert((0, 2), Summary::Claimed { until: 1_000 });
        let none = HashSet::new();
        assert_eq!(
            due(4, &summaries, &none, 999),
            vec![DueBlock {
                lo: 2,
                size: 2,
                empty: false
            }]
        );
        assert_eq!(due(4, &summaries, &none, 1_000).len(), 2);
        // A claimed half is not written yet: its parent waits.
        summaries.insert((2, 2), Summary::Written { empty: false });
        assert!(
            !due(4, &summaries, &none, 0)
                .iter()
                .any(|block| block.size == 4)
        );
    }

    #[test]
    fn fully_deleted_blocks_need_no_model() {
        let deleted: HashSet<u64> = [0, 1, 3].into();
        let found = due(4, &HashMap::new(), &deleted, 0);
        assert_eq!(
            found,
            vec![
                DueBlock {
                    lo: 0,
                    size: 2,
                    empty: true
                },
                DueBlock {
                    lo: 2,
                    size: 2,
                    empty: false
                }
            ]
        );
        let mut summaries = HashMap::new();
        summaries.insert((0, 2), Summary::Written { empty: true });
        summaries.insert((2, 2), Summary::Written { empty: true });
        assert_eq!(
            due(4, &summaries, &deleted, 0),
            vec![DueBlock {
                lo: 0,
                size: 4,
                empty: true
            }]
        );
    }

    #[test]
    fn only_real_blocks_parse() {
        assert_eq!(parse_block("#0-1"), Some((0, 2)));
        assert_eq!(parse_block("16-31"), Some((16, 16)));
        for wrong in ["4-4", "5-6", "4-6", "8-15x", "a-b", "3-2", "-1"] {
            assert_eq!(parse_block(wrong), None, "{wrong}");
        }
    }
}
