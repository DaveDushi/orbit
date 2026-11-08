//! Checks the desktop's view of `/api/v1` against the server's contract
//! (`packages/api/src/generated/openapi.json`): every path and method it calls exists, every field
//! its types read is a property of the schema, and every enum value it knows is the server's.
//! Run it on each server change that touches chat routes.

use std::path::Path;

use orbit_client::{ConversationRecord, ConversationStateRecord, Fixture};
use serde_json::Value;

/// The calls the app makes, and the schema of each one's item.
const CALLS: &[(&str, &str, &str)] = &[
    ("get", "/api/v1/workspaces/{workspace_id}/chat/conversations", "ConversationRecord"),
    ("get", "/api/v1/workspaces/{workspace_id}/chat/states", "ConversationStateRecord"),
];

fn openapi() -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../packages/api/src/generated/openapi.json");
    serde_json::from_slice(&std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))).unwrap()
}

fn schema<'a>(doc: &'a Value, name: &str) -> &'a Value {
    &doc["components"]["schemas"][name]
}

fn fixture() -> Fixture {
    Fixture::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/demo")).unwrap()
}

/// The fields one of our types writes back are the fields it reads.
fn fields_of<T: serde::Serialize>(value: &T) -> Vec<String> {
    serde_json::to_value(value).unwrap().as_object().unwrap().keys().cloned().collect()
}

fn assert_subset(doc: &Value, name: &str, ours: &[String]) {
    let properties = schema(doc, name)["properties"].as_object().unwrap_or_else(|| panic!("no schema {name}"));
    for field in ours {
        assert!(properties.contains_key(field), "{name}.{field} is not in openapi.json");
    }
}

#[test]
fn the_calls_exist_with_the_item_schemas_we_read() {
    let doc = openapi();
    for (method, path, item) in CALLS {
        let response = &doc["paths"][path][method]["responses"]["200"]["content"]["application/json"]["schema"];
        assert_eq!(response["type"], "array", "{method} {path}");
        assert_eq!(response["items"]["$ref"], format!("#/components/schemas/{item}"), "{method} {path}");
    }
}

#[test]
fn the_fields_we_read_are_in_the_contract() {
    let doc = openapi();
    let fixture = fixture();
    let conversation: &ConversationRecord = &fixture.conversations[0];
    let state: &ConversationStateRecord = &fixture.states[0];
    assert_subset(&doc, "ConversationRecord", &fields_of(conversation));
    assert_subset(&doc, "ConversationStateRecord", &fields_of(state));
}

#[test]
fn the_enum_values_we_know_are_the_servers() {
    let doc = openapi();
    let values = |name: &str| -> Vec<String> {
        schema(&doc, name)["enum"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_owned()).collect()
    };
    assert_eq!(values("ConversationKind"), ["public", "private", "dm"]);
    assert_eq!(values("NotifyLevel"), ["all", "mentions", "muted"]);
}
