//! One-time repair (`orbit pages repair-markdown`) of Docs pages that the agent wrote before its
//! Markdown converter knew GFM tables and links: such a table is one plain paragraph per row
//! (`| a | b |`, then `|---|---|`), and a link is its Markdown text (`[label](https://…)`).
//!
//! Only blocks in the exact shape the agent's old converter wrote change: plain text (one unstyled
//! text item) without children. A run of such paragraphs that is a GFM table becomes one table
//! block, and such a block whose text holds an http(s) or mailto Markdown link gets its text
//! re-read as inline Markdown; both go through [`markdown_to_blocks`], the converter of Markdown
//! imports. A repaired block no longer has that shape, so a second run changes nothing.

use orbit_platform::{Database, Id, Row, TimestampMillis};
use serde_json::Value;

use crate::collab::markdown_import::markdown_to_blocks;
use crate::repositories::pages::{PageChanges, PageError, PageRepository};

/// The request id of the repair's page writes (audit log).
const REQUEST_ID: &str = "repair-page-markdown";

/// Block types whose plain text can hold a Markdown link.
const TEXT_BLOCKS: [&str; 6] = [
    "paragraph",
    "heading",
    "bulletListItem",
    "numberedListItem",
    "checkListItem",
    "quote",
];

/// The text of a block in the old converter's shape: one unstyled text item, no children.
fn plain_text(block: &Value) -> Option<&str> {
    let [item] = block["content"].as_array()?.as_slice() else {
        return None;
    };
    let unstyled = item["styles"]
        .as_object()
        .is_none_or(|styles| styles.is_empty());
    let childless = block["children"].as_array().is_none_or(Vec::is_empty);
    (item["type"] == "text" && unstyled && childless)
        .then(|| item["text"].as_str())
        .flatten()
}

fn table_row(block: &Value) -> Option<&str> {
    let text = plain_text(block)
        .filter(|_| block["type"] == "paragraph")?
        .trim();
    (text.len() > 1 && text.starts_with('|') && text.ends_with('|')).then_some(text)
}

/// A GFM delimiter row such as `|---|:--:|`.
fn delimiter_row(text: &str) -> bool {
    text.trim_matches('|').split('|').all(|cell| {
        let cell = cell.trim().trim_start_matches(':').trim_end_matches(':');
        !cell.is_empty() && cell.chars().all(|character| character == '-')
    })
}

/// Link schemes the editor keeps.
const SAFE_LINKS: [&str; 3] = ["http://", "https://", "mailto:"];

/// Whether plain text holds a Markdown link with a scheme the editor keeps.
fn has_markdown_link(text: &str) -> bool {
    SAFE_LINKS.iter().any(|scheme| {
        text.find(&format!("]({scheme}"))
            .is_some_and(|at| text[..at].contains('['))
    })
}

fn safe_link(item: &Value) -> bool {
    item["type"] == "link"
        && item["href"].as_str().is_some_and(|href| {
            SAFE_LINKS
                .iter()
                .any(|scheme| href.to_ascii_lowercase().starts_with(scheme))
        })
}

/// The repaired content, or `None` when no block is in a shape this repair changes.
#[must_use]
pub fn repair_blocks(blocks: &[Value]) -> Option<Vec<Value>> {
    let mut out = Vec::with_capacity(blocks.len());
    let mut changed = false;
    let mut index = 0;
    while index < blocks.len() {
        let rows: Vec<&str> = blocks[index..].iter().map_while(table_row).collect();
        if rows.len() >= 2 && delimiter_row(rows[1]) {
            if let [table] = markdown_to_blocks(&rows.join("\n"), &[]).as_slice()
                && table["type"] == "table"
            {
                out.push(table.clone());
                changed = true;
                index += rows.len();
                continue;
            }
        }
        let block = &blocks[index];
        index += 1;
        let relinked = plain_text(block)
            .filter(|text| {
                TEXT_BLOCKS.contains(&block["type"].as_str().unwrap_or(""))
                    && has_markdown_link(text)
            })
            .and_then(|text| match markdown_to_blocks(text, &[]).as_slice() {
                [paragraph]
                    if paragraph["type"] == "paragraph"
                        && paragraph["content"]
                            .as_array()
                            .is_some_and(|items| items.iter().any(safe_link)) =>
                {
                    Some(paragraph["content"].clone())
                }
                _ => None,
            });
        match relinked {
            Some(content) => {
                let mut block = block.clone();
                block["content"] = content;
                out.push(block);
                changed = true;
            }
            None => out.push(block.clone()),
        }
    }
    changed.then_some(out)
}

/// One page the repair changed, would change, or could not change.
#[derive(Debug)]
pub struct RepairedPage {
    pub id: Id,
    pub title: String,
    /// Why the page was left as it is (it is locked, or changed meanwhile).
    pub skipped: Option<String>,
}

/// Repairs every live page (a dry run only finds them). Pages are written as their last editor,
/// with a version check, like an edit: page history keeps the state before when its newest
/// version is older than the snapshot interval.
pub async fn repair_pages(
    database: &Database,
    dry_run: bool,
) -> Result<Vec<RepairedPage>, PageError> {
    let rows = sqlx::query(
        "SELECT id, workspace_id, title, updated_by, version, content_json FROM pages \
         WHERE deleted_at IS NULL ORDER BY workspace_id, id",
    )
    .fetch_all(database.pool())
    .await?;
    let pages = PageRepository::new(database.clone());
    let mut report = Vec::new();
    for row in rows {
        let parse = |column: &str| {
            row.get::<String, _>(column)
                .parse::<Id>()
                .map_err(|_| PageError::Corrupt)
        };
        let content: Vec<Value> = serde_json::from_str(&row.get::<String, _>("content_json"))
            .map_err(|_| PageError::Corrupt)?;
        let Some(repaired) = repair_blocks(&content) else {
            continue;
        };
        let (id, title) = (parse("id")?, row.get::<String, _>("title"));
        let mut skipped = None;
        if !dry_run {
            let written = pages
                .update_page(
                    parse("workspace_id")?,
                    id,
                    parse("updated_by")?,
                    row.get::<i64, _>("version") as u64,
                    PageChanges {
                        content: Some(repaired),
                        ..PageChanges::default()
                    },
                    REQUEST_ID,
                    TimestampMillis::now(),
                )
                .await;
            skipped = match written {
                Ok(_) => None,
                Err(PageError::Locked) => Some("locked".to_owned()),
                Err(PageError::VersionConflict { .. }) => Some("changed meanwhile".to_owned()),
                Err(PageError::NotFound) => Some("its last editor cannot see it".to_owned()),
                Err(error) => return Err(error),
            };
        }
        report.push(RepairedPage { id, title, skipped });
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn plain(kind: &str, text: &str) -> Value {
        json!({"type": kind, "content": [{"type": "text", "text": text, "styles": {}}]})
    }

    #[test]
    fn table_rows_and_markdown_links_are_repaired_once() {
        let content = vec![
            plain("heading", "Logistics"),
            plain("paragraph", "| Check | Fall |"),
            plain("paragraph", "|---|:---:|"),
            plain(
                "paragraph",
                "| Count | [GEN-24](https://orbit.example/tasks/1) |",
            ),
            plain(
                "bulletListItem",
                "[Logistics](https://orbit.example/docs/1)",
            ),
            plain("paragraph", "| not | a table |"),
            plain(
                "paragraph",
                "[run](javascript:alert(1)) and `[x](https://in.code)` text",
            ),
        ];
        let repaired = repair_blocks(&content).unwrap();
        let types: Vec<&str> = repaired
            .iter()
            .map(|block| block["type"].as_str().unwrap())
            .collect();
        assert_eq!(
            types,
            [
                "heading",
                "table",
                "bulletListItem",
                "paragraph",
                "paragraph"
            ]
        );
        let rows = repaired[1]["content"]["rows"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1]["cells"][1]["content"][0]["type"], "link");
        assert_eq!(
            repaired[2]["content"][0]["href"],
            "https://orbit.example/docs/1"
        );
        // Not a table (no delimiter row), an unsafe scheme, and a link in code stay as they were.
        assert_eq!(repaired[3], content[5]);
        assert_eq!(repaired[4], content[6]);
        assert_eq!(repair_blocks(&repaired), None);
    }

    #[test]
    fn styled_or_nested_blocks_are_not_the_agents_and_stay() {
        let styled = json!({"type": "paragraph", "content": [
            {"type": "text", "text": "[a](https://a.example)", "styles": {"bold": true}}
        ]});
        let mut nested = plain("paragraph", "[a](https://a.example)");
        nested["children"] = json!([plain("paragraph", "child")]);
        assert_eq!(repair_blocks(&[styled, nested]), None);
    }
}
