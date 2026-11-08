//! A card that the team agent posts with its answer: a json-render spec (a flat map of elements
//! and the `root` one) built from the catalog in `apps/agent/src/ui-catalog.ts`. The sidecar and
//! the web app check the spec against that catalog; this is the gate that counts, so it checks
//! the shape and the limits again and never trusts the model.
//!
//! The elements must form one tree from `root`: each element is used once, so a spec cannot loop
//! or make the renderer repeat a branch. Props are plain JSON: no `$…` keys, which json-render
//! would read as state bindings or expressions.

use std::collections::HashSet;

use serde_json::{Map, Value};

use super::blocks::{BlocksError, MessageBlock, check_message_blocks};

/// The version of our catalog (not of the library) that a stored card was made for.
pub const UI_VERSION: u32 = 1;
const MAX_ELEMENTS: usize = 200;
const MAX_DEPTH: usize = 8;
const MAX_STRING_CHARS: usize = 2000;
const MAX_ARRAY_ITEMS: usize = 200;
const MAX_PROPS_DEPTH: usize = 5;
const MAX_FORM_FIELDS: usize = 10;
const MAX_PARAMS_BYTES: usize = 2048;
const ELEMENT_ID_MAX_CHARS: usize = 64;
const TITLE_MAX_CHARS: usize = 150;

/// The component types of catalog version 1, the props each may have, and whether it has
/// children.
const COMPONENTS: [(&str, &[&str], bool); 8] = [
    ("Card", &["title", "description"], true),
    ("Stack", &["direction"], true),
    ("Text", &["text", "tone"], false),
    ("Table", &["columns", "rows"], false),
    (
        "Chart",
        &["kind", "title", "x_label", "y_label", "series"],
        false,
    ),
    ("TaskList", &["tasks"], false),
    ("Form", &["fields", "submit_label"], false),
    ("Button", &["label", "style", "action", "params"], false),
];

/// What a button does: `reply` sends the click to the agent; `open_task` only opens a task in
/// the web app and never reaches the server.
const BUTTON_ACTIONS: [&str; 2] = ["reply", "open_task"];

fn fail(at: impl std::fmt::Display, problem: &str) -> BlocksError {
    BlocksError(format!("{at}: {problem}"))
}

/// Checks a card's `title` and `spec`. `at` names the block in the error (`blocks[0]`).
pub fn check_ui(version: u32, title: &str, spec: &Value, at: &str) -> Result<(), BlocksError> {
    if version != UI_VERSION {
        return Err(fail(format!("{at}.version"), "must be 1"));
    }
    if title.trim().is_empty() || title.chars().count() > TITLE_MAX_CHARS {
        return Err(fail(format!("{at}.title"), "must be 1 to 150 characters"));
    }
    let at = format!("{at}.spec");
    let Value::Object(spec) = spec else {
        return Err(fail(&at, "must be an object"));
    };
    if let Some(key) = spec
        .keys()
        .find(|key| !matches!(key.as_str(), "root" | "elements"))
    {
        return Err(fail(&at, &format!("unknown key `{key}`")));
    }
    let Some(root) = spec.get("root").and_then(Value::as_str) else {
        return Err(fail(format!("{at}.root"), "must be an element id"));
    };
    let Some(Value::Object(elements)) = spec.get("elements") else {
        return Err(fail(format!("{at}.elements"), "must be an object"));
    };
    if elements.is_empty() || elements.len() > MAX_ELEMENTS {
        return Err(fail(
            format!("{at}.elements"),
            "must have 1 to 200 elements",
        ));
    }
    for (id, element) in elements {
        check_element(id, element, &format!("{at}.elements.{id}"))?;
    }

    // One tree from the root: every element is reached once, at most 8 levels deep.
    let mut seen = HashSet::new();
    let mut stack = vec![(root, 1_usize)];
    while let Some((id, depth)) = stack.pop() {
        let Some(element) = elements.get(id) else {
            return Err(fail(&at, &format!("element `{id}` does not exist")));
        };
        if !seen.insert(id) {
            return Err(fail(&at, &format!("element `{id}` is used more than once")));
        }
        if depth > MAX_DEPTH {
            return Err(fail(&at, "elements can be nested at most 8 levels deep"));
        }
        for child in children(element) {
            stack.push((child, depth + 1));
        }
    }
    if let Some(id) = elements.keys().find(|id| !seen.contains(id.as_str())) {
        return Err(fail(&at, &format!("element `{id}` is not in the tree")));
    }
    Ok(())
}

fn children(element: &Value) -> impl Iterator<Item = &str> {
    element
        .get("children")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
}

fn check_element(id: &str, element: &Value, at: &str) -> Result<(), BlocksError> {
    if id.is_empty() || id.chars().count() > ELEMENT_ID_MAX_CHARS {
        return Err(fail(at, "the element id must be 1 to 64 characters"));
    }
    let Value::Object(element) = element else {
        return Err(fail(at, "must be an object"));
    };
    if let Some(key) = element
        .keys()
        .find(|key| !matches!(key.as_str(), "type" | "props" | "children"))
    {
        return Err(fail(at, &format!("unknown key `{key}`")));
    }
    let kind = element.get("type").and_then(Value::as_str).unwrap_or("");
    let Some((_, allowed, has_children)) = COMPONENTS.iter().find(|(name, ..)| *name == kind)
    else {
        return Err(fail(format!("{at}.type"), "is not a known component"));
    };
    match element.get("children") {
        None => {}
        Some(Value::Array(list)) if list.is_empty() => {}
        Some(Value::Array(list)) if *has_children => {
            if list.iter().any(|child| !child.is_string()) {
                return Err(fail(format!("{at}.children"), "must be element ids"));
            }
        }
        Some(_) => {
            return Err(fail(
                format!("{at}.children"),
                &format!("{kind} cannot have children"),
            ));
        }
    }
    let empty = Map::new();
    let props = match element.get("props") {
        None => &empty,
        Some(Value::Object(props)) => props,
        Some(_) => return Err(fail(format!("{at}.props"), "must be an object")),
    };
    for (key, value) in props {
        if !allowed.contains(&key.as_str()) {
            return Err(fail(
                format!("{at}.props"),
                &format!("{kind} has no prop `{key}`"),
            ));
        }
        check_value(value, 1, &format!("{at}.props.{key}"))?;
    }
    match kind {
        "Button" => {
            let action = props.get("action").and_then(Value::as_str).unwrap_or("");
            if !BUTTON_ACTIONS.contains(&action) {
                return Err(fail(
                    format!("{at}.props.action"),
                    "must be reply or open_task",
                ));
            }
            match props.get("params") {
                None | Some(Value::Null) => {}
                Some(params @ Value::Object(_)) if params.to_string().len() <= MAX_PARAMS_BYTES => {
                }
                Some(_) => {
                    return Err(fail(
                        format!("{at}.props.params"),
                        "must be an object of at most 2 KiB",
                    ));
                }
            }
        }
        "Form" => {
            let fields = props
                .get("fields")
                .and_then(Value::as_array)
                .ok_or_else(|| fail(format!("{at}.props.fields"), "must be a list"))?;
            if fields.is_empty() || fields.len() > MAX_FORM_FIELDS {
                return Err(fail(
                    format!("{at}.props.fields"),
                    "must have 1 to 10 fields",
                ));
            }
            let mut names = HashSet::new();
            for field in fields {
                let name = field.get("name").and_then(Value::as_str).unwrap_or("");
                if name.is_empty() || !names.insert(name) {
                    return Err(fail(
                        format!("{at}.props.fields"),
                        "each field needs its own name",
                    ));
                }
            }
        }
        _ => {}
    }
    Ok(())
}

/// Plain JSON within the limits: no `$` keys, short strings and lists, little nesting.
fn check_value(value: &Value, depth: usize, at: &str) -> Result<(), BlocksError> {
    if depth > MAX_PROPS_DEPTH {
        return Err(fail(at, "is nested too deep"));
    }
    match value {
        Value::String(text) if text.chars().count() > MAX_STRING_CHARS => {
            Err(fail(at, "must be at most 2000 characters"))
        }
        Value::Array(items) => {
            if items.len() > MAX_ARRAY_ITEMS {
                return Err(fail(at, "can have at most 200 items"));
            }
            items
                .iter()
                .try_for_each(|item| check_value(item, depth + 1, at))
        }
        Value::Object(map) => {
            if map.keys().any(|key| key.starts_with('$')) {
                return Err(fail(at, "keys must not start with $"));
            }
            map.iter()
                .try_for_each(|(key, item)| check_value(item, depth + 1, &format!("{at}.{key}")))
        }
        _ => Ok(()),
    }
}

/// The blocks the team agent sends with an answer: one `ui` block and nothing else. Webhooks
/// cannot post `ui` (`parse_blocks` drops it); only the sidecar's route comes here.
pub fn parse_agent_blocks(value: &Value) -> Result<Vec<MessageBlock>, BlocksError> {
    let Value::Array(items) = value else {
        return Err(BlocksError("blocks: must be an array".to_owned()));
    };
    if items.len() > 1 {
        return Err(BlocksError(
            "blocks: an answer can have one card".to_owned(),
        ));
    }
    let mut blocks = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let at = format!("blocks[{index}]");
        if item.get("type").and_then(Value::as_str) != Some("ui") {
            return Err(fail(&at, "must be a ui block"));
        }
        let mut block = serde_json::from_value::<MessageBlock>(item.clone())
            .map_err(|error| fail(&at, &error.to_string()))?;
        let MessageBlock::Ui {
            version,
            title,
            spec,
            block_id,
            ..
        } = &mut block
        else {
            return Err(fail(&at, "must be a ui block"));
        };
        check_ui(*version, title, spec, &at)?;
        if block_id
            .as_deref()
            .is_none_or(|id| id.trim().is_empty() || id.chars().count() > ELEMENT_ID_MAX_CHARS)
        {
            *block_id = Some(format!("ui{index}"));
        }
        blocks.push(block);
    }
    check_message_blocks(&blocks)?;
    Ok(blocks)
}

/// A card of a stored message.
pub struct FoundUi<'a> {
    pub title: &'a str,
    pub spec: &'a Value,
    pub live: bool,
}

/// The card of a stored message that a click names.
pub fn find_ui<'a>(blocks: &'a [MessageBlock], block_id: &str) -> Option<FoundUi<'a>> {
    blocks.iter().find_map(|block| match block {
        MessageBlock::Ui {
            title,
            spec,
            block_id: Some(id),
            live,
            ..
        } if id == block_id => Some(FoundUi {
            title: title.as_str(),
            spec,
            live: *live,
        }),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::{check_ui, parse_agent_blocks};
    use crate::repositories::chat::parse_blocks;

    fn spec(elements: Value) -> Value {
        json!({ "root": "card", "elements": elements })
    }

    fn good() -> Value {
        spec(json!({
            "card": { "type": "Card", "props": { "title": "Open tasks" }, "children": ["table", "go"] },
            "table": { "type": "Table", "props": {
                "columns": [{ "key": "id", "label": "ID" }], "rows": [{ "id": "GEN-1" }] } },
            "go": { "type": "Button", "props": { "label": "Plan", "action": "reply", "params": { "step": 2 } } },
        }))
    }

    fn error(spec: &Value) -> String {
        check_ui(1, "Tasks", spec, "blocks[0]").unwrap_err().0
    }

    #[test]
    fn a_card_within_the_rules_passes() {
        check_ui(1, "Tasks", &good(), "blocks[0]").unwrap();
    }

    #[test]
    fn refuses_unknown_types_props_keys_and_versions() {
        assert!(check_ui(2, "Tasks", &good(), "b").is_err());
        assert!(
            error(&spec(json!({ "card": { "type": "Iframe", "props": {} } })))
                .contains("known component")
        );
        assert!(
            error(&spec(
                json!({ "card": { "type": "Text", "props": { "text": "x", "className": "x" } } })
            ))
            .contains("no prop")
        );
        assert!(
            error(&spec(
                json!({ "card": { "type": "Text", "props": { "text": "x" }, "visible": true } })
            ))
            .contains("unknown key")
        );
        assert!(
            error(&spec(
                json!({ "card": { "type": "Text", "props": { "text": { "$state": "/x" } } } })
            ))
            .contains("$")
        );
        assert!(error(&json!({ "root": "card", "elements": { "card": { "type": "Text", "props": { "text": "x" } } }, "state": {} })).contains("unknown key"));
        assert!(error(&spec(json!({ "card": { "type": "Button", "props": { "label": "x", "action": "delete_all" } } }))).contains("reply"));
        assert!(error(&spec(json!({ "card": { "type": "Text", "props": { "text": "x" }, "children": ["y"] }, "y": { "type": "Text", "props": { "text": "y" } } }))).contains("cannot have children"));
    }

    #[test]
    fn refuses_a_spec_that_is_not_one_shallow_tree() {
        // A missing child, a missing root, a loop, a shared child, an unused element.
        assert!(
            error(&spec(
                json!({ "card": { "type": "Card", "props": {}, "children": ["nope"] } })
            ))
            .contains("does not exist")
        );
        assert!(
            error(&json!({ "root": "x", "elements": { "card": { "type": "Card", "props": {} } } }))
                .contains("does not exist")
        );
        assert!(
            error(&spec(json!({
                "card": { "type": "Card", "props": {}, "children": ["a"] },
                "a": { "type": "Stack", "props": {}, "children": ["card"] },
            })))
            .contains("more than once")
        );
        assert!(
            error(&spec(json!({
                "card": { "type": "Card", "props": {}, "children": ["a", "a"] },
                "a": { "type": "Text", "props": { "text": "x" } },
            })))
            .contains("more than once")
        );
        assert!(
            error(&spec(json!({
                "card": { "type": "Card", "props": {} },
                "lost": { "type": "Text", "props": { "text": "x" } },
            })))
            .contains("not in the tree")
        );
        // Nine levels.
        let mut elements = serde_json::Map::new();
        for level in 0..9 {
            let id = if level == 0 {
                "card".to_owned()
            } else {
                format!("s{level}")
            };
            let children = if level < 8 {
                json!([format!("s{}", level + 1)])
            } else {
                json!([])
            };
            elements.insert(
                id,
                json!({ "type": "Stack", "props": {}, "children": children }),
            );
        }
        assert!(error(&spec(Value::Object(elements))).contains("8 levels"));
    }

    #[test]
    fn refuses_specs_over_the_limits() {
        let mut elements = serde_json::Map::new();
        let ids: Vec<String> = (0..201).map(|n| format!("t{n}")).collect();
        elements.insert(
            "card".to_owned(),
            json!({ "type": "Card", "props": {}, "children": ids }),
        );
        for id in &ids {
            elements.insert(
                id.clone(),
                json!({ "type": "Text", "props": { "text": "x" } }),
            );
        }
        assert!(error(&spec(Value::Object(elements))).contains("200 elements"));
        let rows: Vec<Value> = (0..201).map(|n| json!({ "n": n })).collect();
        assert!(
            error(&spec(
                json!({ "card": { "type": "Table", "props": { "columns": [], "rows": rows } } })
            ))
            .contains("200 items")
        );
        assert!(
            error(&spec(
                json!({ "card": { "type": "Text", "props": { "text": "x".repeat(2001) } } })
            ))
            .contains("2000")
        );
        let fields: Vec<Value> = (0..11)
            .map(|n| json!({ "name": format!("f{n}"), "label": "F", "kind": "text" }))
            .collect();
        assert!(error(&spec(json!({ "card": { "type": "Form", "props": { "fields": fields, "submit_label": "Send" } } }))).contains("1 to 10"));
        assert!(error(&spec(json!({ "card": { "type": "Button", "props": { "label": "x", "action": "reply", "params": { "a": "y".repeat(1990), "b": "y".repeat(100) } } } }))).contains("2 KiB"));
        // The whole message: 48 KiB.
        let mut elements = serde_json::Map::new();
        let ids: Vec<String> = (0..30).map(|n| format!("t{n}")).collect();
        elements.insert(
            "card".to_owned(),
            json!({ "type": "Card", "props": {}, "children": ids }),
        );
        for id in &ids {
            elements.insert(
                id.clone(),
                json!({ "type": "Text", "props": { "text": "x".repeat(2000) } }),
            );
        }
        let block = json!([{ "type": "ui", "version": 1, "title": "Big", "spec": spec(Value::Object(elements)) }]);
        assert!(parse_agent_blocks(&block).unwrap_err().0.contains("48 KiB"));
    }

    #[test]
    fn only_the_agent_route_takes_a_card() {
        let block = json!([{ "type": "ui", "version": 1, "title": "Tasks", "spec": good() }]);
        let blocks = parse_agent_blocks(&block).unwrap();
        assert_eq!(blocks.len(), 1);
        // A webhook payload with the same block posts nothing of it.
        assert!(parse_blocks(&block).unwrap().is_empty());
        // The agent route takes nothing but one card.
        assert!(parse_agent_blocks(&json!([{ "type": "divider" }])).is_err());
        assert!(parse_agent_blocks(&json!([block[0].clone(), block[0].clone()])).is_err());
    }
}
