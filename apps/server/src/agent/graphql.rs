//! Whether a GraphQL request over POST only reads, for an integration that lets such reads run
//! without the driver's approval. It fails closed: anything not clearly one query needs approval.
//! The agent's claims count for nothing: only the parsed document that Orbit sends is checked.

use apollo_parser::Parser;
use apollo_parser::cst::{self, CstNode};
use serde_json::Value;

/// Below these limits for any real read; beyond them the parser reports an error.
const RECURSION_LIMIT: usize = 128;
const TOKEN_LIMIT: usize = 32_768;
/// The root types of writes. A fragment on one of them is never part of a plain read.
const WRITE_ROOT_TYPES: [&str; 2] = ["Mutation", "Subscription"];

/// `true` only for a JSON object with a `query` string that holds exactly one query operation (and
/// fragments), and only the keys `variables` and `operationName` beside it. Other keys, such as a
/// persisted query id, could change what runs.
pub(crate) fn is_read_only_request(body: &Value) -> bool {
    let Some(request) = body.as_object() else {
        return false;
    };
    let Some(query) = request.get("query").and_then(Value::as_str) else {
        return false;
    };
    if request
        .keys()
        .any(|key| !matches!(key.as_str(), "query" | "variables" | "operationName"))
        || !request
            .get("variables")
            .is_none_or(|variables| variables.is_null() || variables.is_object())
    {
        return false;
    }
    let operation_name = match request.get("operationName") {
        None | Some(Value::Null) => None,
        Some(Value::String(name)) => Some(name.as_str()),
        Some(_) => return false,
    };
    let tree = Parser::new(query)
        .recursion_limit(RECURSION_LIMIT)
        .token_limit(TOKEN_LIMIT)
        .parse();
    if tree.errors().len() > 0 {
        return false;
    }
    let document = tree.document();
    let mut operations = 0;
    for definition in document.definitions() {
        match definition {
            cst::Definition::OperationDefinition(operation) => {
                operations += 1;
                let query_type = operation
                    .operation_type()
                    .is_none_or(|kind| kind.query_token().is_some());
                let name = operation.name().map(|name| name.text().to_string());
                if !query_type
                    || operation.directives().is_some()
                    || operation_name.is_some_and(|wanted| name.as_deref() != Some(wanted))
                {
                    return false;
                }
            }
            cst::Definition::FragmentDefinition(_) => {}
            // Schema definitions do not belong in a request.
            _ => return false,
        }
    }
    let write_fragment = document
        .syntax()
        .descendants()
        .filter_map(cst::TypeCondition::cast)
        .filter_map(|condition| condition.named_type()?.name())
        .any(|name| WRITE_ROOT_TYPES.contains(&name.text().as_ref()));
    operations == 1 && !write_fragment
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::is_read_only_request;

    fn query(text: &str) -> Value {
        json!({ "query": text })
    }

    #[test]
    fn a_single_query_is_a_read() {
        for text in [
            "{ shop { name } }",
            "query { shop { name } }",
            "query Shop($first: Int) { products(first: $first) { edges { node { id } } } }",
            // Comments and strings are not code.
            "# mutation { deleteAll }\nquery { shop { name } }",
            "query { shop { name } } # mutation { deleteAll }",
            "query { products(query: \"mutation { deleteAll }\") { edges { cursor } } }",
            // Aliases and names that look like keywords.
            "{ mine: shop { name } }",
            "{ mutation: shop { name } }",
            "query mutation { shop { name } }",
            "{ mutation }",
            // Fragments.
            "query { shop { ...Fields } } fragment Fields on Shop { name }",
            "query { node(id: 1) { ... on Product { title } } }",
            "fragment Fields on Shop { name } query { shop { ...Fields } }",
        ] {
            assert!(is_read_only_request(&query(text)), "{text}");
        }
        for body in [
            json!({ "query": "query { shop { name } }", "variables": { "a": 1 } }),
            json!({ "query": "query { shop { name } }", "variables": null, "operationName": null }),
            json!({ "query": "query Shop { shop { name } }", "operationName": "Shop" }),
        ] {
            assert!(is_read_only_request(&body), "{body}");
        }
    }

    #[test]
    fn anything_else_needs_approval() {
        for text in [
            "",
            "# only a comment",
            "query {",
            "query { shop { name }",
            "not graphql",
            "mutation { deleteAll }",
            "mutation Delete { deleteAll }",
            "subscription { orders { id } }",
            // A write beside, after or before a query.
            "query { shop { name } } mutation { deleteAll }",
            "mutation { deleteAll } query { shop { name } }",
            "query A { shop { name } } query B { shop { id } }",
            "{ shop { name } } { shop { id } }",
            "query { shop { name } } } mutation { deleteAll }",
            // A write in a fragment.
            "query { ...Hidden } fragment Hidden on Mutation { deleteAll }",
            "query { ... on Mutation { deleteAll } }",
            "fragment Hidden on Subscription { orders { id } } query { shop { name } }",
            // Only a fragment; no operation.
            "fragment Fields on Shop { name }",
            // Schema definitions and directives on the operation.
            "type Shop { name: String } query { shop { name } }",
            "query @custom { shop { name } }",
        ] {
            assert!(!is_read_only_request(&query(text)), "{text}");
        }
        for body in [
            Value::Null,
            json!("query { shop { name } }"),
            json!([{ "query": "query { shop { name } }" }]),
            json!({}),
            json!({ "query": null }),
            json!({ "query": 1 }),
            json!({ "query": ["{ shop { name } }"] }),
            json!({ "variables": {} }),
            json!({ "query": "{ shop { name } }", "variables": "{}" }),
            json!({ "query": "{ shop { name } }", "operationName": 1 }),
            // A name that is not the operation's, also with several operations.
            json!({ "query": "query A { shop { name } }", "operationName": "B" }),
            json!({ "query": "{ shop { name } }", "operationName": "A" }),
            json!({ "query": "query A { a } mutation B { b }", "operationName": "A" }),
            json!({ "query": "query A { a } query B { b }", "operationName": "A" }),
            // Fields that can change what runs.
            json!({ "query": "{ shop { name } }", "id": "persisted" }),
            json!({ "query": "{ shop { name } }", "documentId": "persisted" }),
            json!({ "query": "{ shop { name } }", "extensions": { "persistedQuery": {} } }),
        ] {
            assert!(!is_read_only_request(&body), "{body}");
        }
    }

    #[test]
    fn very_deep_or_long_documents_need_approval_and_do_not_crash() {
        let deep = format!("{{ a {} }}", "{ a ".repeat(30_000) + &"}".repeat(30_000));
        assert!(!is_read_only_request(&query(&deep)));
        let long = format!("{{ {} }}", "a ".repeat(40_000));
        assert!(!is_read_only_request(&query(&long)));
    }
}
