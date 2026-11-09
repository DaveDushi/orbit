//! Checks the desktop's view of `/api/v1` against the server's contract
//! (`packages/api/src/generated/openapi.json`): every path and method it calls exists, every field
//! its types read is a property of the schema, and every enum value it knows is the server's.
//! Run it on each server change that touches chat routes.

use std::path::Path;

use orbit_client::{AuthUser, ConversationRecord, ConversationStateRecord, Fixture, MemberRecord, MessageRecord, SetupStatus, WorkspaceRecord};
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

/// The properties of a schema, also when it is an `allOf` of objects.
fn properties(doc: &Value, name: &str) -> Vec<String> {
    let schema = schema(doc, name);
    let parts: Vec<&Value> = match schema["allOf"].as_array() {
        Some(parts) => parts.iter().collect(),
        None => vec![schema],
    };
    let mut names = Vec::new();
    for part in parts {
        let part = match part["$ref"].as_str() {
            Some(r) => self::schema(doc, r.trim_start_matches("#/components/schemas/")),
            None => part,
        };
        names.extend(part["properties"].as_object().into_iter().flat_map(|p| p.keys().cloned()));
    }
    assert!(!names.is_empty(), "no schema {name}");
    names
}

fn assert_subset(doc: &Value, name: &str, ours: &[String]) {
    let properties = properties(doc, name);
    for field in ours {
        assert!(properties.contains(field), "{name}.{field} is not in openapi.json");
    }
}

/// The sign-in, session and probe calls, and the schema of each status the client reads.
const AUTH_CALLS: &[(&str, &str, &str, &str)] = &[
    ("get", "/api/v1/setup/status", "200", "SetupStatus"),
    ("post", "/api/v1/auth/login", "200", "LoginResponse"),
    ("post", "/api/v1/auth/login", "202", "TwoFactorChallenge"),
    ("post", "/api/v1/auth/login/two-factor", "200", "LoginResponse"),
    ("get", "/api/v1/auth/me", "200", "AuthUserResponse"),
];

#[test]
fn the_auth_calls_exist_with_the_schemas_we_read() {
    let doc = openapi();
    for (method, path, status, name) in AUTH_CALLS {
        let response = &doc["paths"][path][method]["responses"][status]["content"]["application/json"]["schema"];
        assert_eq!(response["$ref"], format!("#/components/schemas/{name}"), "{method} {path} {status}");
    }
    assert!(doc["paths"]["/api/v1/auth/logout"]["post"]["responses"]["204"].is_object());
    let workspaces = &doc["paths"]["/api/v1/workspaces"]["get"]["responses"]["200"]["content"]["application/json"]["schema"];
    assert_eq!(workspaces["items"]["$ref"], "#/components/schemas/WorkspaceRecord");

    let user = AuthUser { id: "u".into(), email: "e".into(), display_name: "d".into() };
    assert_subset(&doc, "AuthUserResponse", &fields_of(&user));
    assert_subset(&doc, "WorkspaceRecord", &fields_of(&WorkspaceRecord { id: "w".into(), name: "n".into() }));
    assert_subset(&doc, "SetupStatus", &fields_of(&SetupStatus { complete: true }));
    let strings = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert_subset(&doc, "LoginResponse", &strings(&["user", "session_id", "session_token"]));
    assert_subset(&doc, "TwoFactorChallenge", &strings(&["two_factor_token"]));
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
    let message: &MessageRecord = &fixture.messages.values().next().unwrap()[0];
    assert_subset(&doc, "MessageRecord", &fields_of(message));
    let member: &MemberRecord = &fixture.members[0];
    let member_item = &doc["components"]["schemas"]["Page_MemberRecord"]["properties"]["items"]["items"]["properties"];
    for field in fields_of(member) {
        assert!(member_item.get(&field).is_some(), "MemberRecord.{field} is not in openapi.json");
    }
}

/// The chat calls of the message list, the thread panel and the composer: path, method, and the
/// schema of the 200 body (and of the request body for writes).
const CHAT_CALLS: &[(&str, &str, &str, Option<&str>)] = &[
    ("get", "/api/v1/workspaces/{workspace_id}/members", "Page_MemberRecord", None),
    ("get", "/api/v1/workspaces/{workspace_id}/chat/conversations/{conversation_id}/messages", "MessagePage", None),
    ("get", "/api/v1/workspaces/{workspace_id}/chat/threads/{root_id}", "ThreadPage", None),
    ("post", "/api/v1/workspaces/{workspace_id}/chat/conversations/{conversation_id}/messages", "ChatWrite_MessageRecord", Some("ChatSendBody")),
    ("patch", "/api/v1/workspaces/{workspace_id}/chat/messages/{message_id}", "ChatWrite_MessageRecord", Some("ChatEditBody")),
];

#[test]
fn the_chat_calls_exist_with_the_schemas_we_read_and_write() {
    let doc = openapi();
    for (method, path, response, request) in CHAT_CALLS {
        let operation = &doc["paths"][path][method];
        let schema = &operation["responses"]["200"]["content"]["application/json"]["schema"];
        assert_eq!(schema["$ref"], format!("#/components/schemas/{response}"), "{method} {path}");
        if let Some(request) = request {
            let body = &operation["requestBody"]["content"]["application/json"]["schema"];
            assert_eq!(body["$ref"], format!("#/components/schemas/{request}"), "{method} {path} body");
        }
    }
    let strings = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert_subset(&doc, "ChatSendBody", &strings(&["body", "nonce", "thread_root_id"]));
    assert_subset(&doc, "ChatEditBody", &strings(&["body"]));
    assert_subset(&doc, "ChatWrite_MessageRecord", &strings(&["result"]));
    assert_subset(&doc, "MessagePage", &strings(&["items", "before", "after"]));
    assert_subset(&doc, "ThreadPage", &strings(&["root", "items", "before", "after"]));
}

#[test]
fn the_enum_values_we_know_are_the_servers() {
    let doc = openapi();
    let values = |name: &str| -> Vec<String> {
        schema(&doc, name)["enum"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_owned()).collect()
    };
    assert_eq!(values("ConversationKind"), ["public", "private", "dm"]);
    assert_eq!(values("NotifyLevel"), ["all", "mentions", "muted"]);
    assert_eq!(values("MessageKind"), ["message", "pin", "join", "leave"]);
}
