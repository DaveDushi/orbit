//! A Drive image to Shopify for the team agent: `POST /api/v1/workspaces/{id}/agent-tools/shopify-image`,
//! for agent sessions of the driver of a running thread only (as `/agent-tools/call`). One call reads
//! the image from Drive as the driver, uploads it to the store's Files (`stagedUploadsCreate`, a
//! multipart POST to the staged target, `fileCreate`), waits until Shopify has processed it and
//! returns its URL. It can then set it as an article's image or add it at the end of the article's
//! body (`articleUpdate`). The bytes never reach the model.
//!
//! The integration is an HTTP API of the workspace whose base URL is a Shopify Admin API
//! (`…/admin/api/<version>`, the GraphQL endpoint is `/graphql.json` below it), with POST allowed.
//! The role rule and the agent's ticks of that integration apply, and every call needs the driver's
//! "yes" in the thread (`super::integrations::approve`), as any integration write. The store needs
//! the `write_files` scope, and `write_content` for articles.
//!
//! Shopify Admin GraphQL: <https://shopify.dev/docs/api/admin-graphql/latest/mutations/stagedUploadsCreate>,
//! <https://shopify.dev/docs/api/admin-graphql/latest/mutations/fileCreate>,
//! <https://shopify.dev/docs/api/admin-graphql/latest/mutations/articleUpdate>.

use std::time::Duration;

use axum::extract::{Extension, Path, State};
use axum::http::{HeaderMap, Uri};
use axum::routing::post;
use axum::{Json, Router};
use orbit_platform::{Id, RequestId};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use url::Url;

use super::AgentState;
use super::integrations::{
    Approval, Approves, Integration, api_send, approve, canonical, capitalized, driver_access,
    hash, load_one, record, require_running, unavailable,
};
use super::outbound::join_api_url;
use crate::audit::AuditOutcome;
use crate::mcp_drive::drive_failure;
use crate::repositories::drive::DriveNodeKind;
use crate::task_routes::ApiError;

type RequestIdExtension = Option<Extension<RequestId>>;

/// The tool's name, in approvals and in the agent's tool list.
pub(crate) const TOOL: &str = "drive_image_to_shopify";
/// The most alt text Orbit sends.
const MAX_ALT_CHARS: usize = 512;
/// How often Orbit asks whether Shopify has processed the file, one second apart.
const READY_CHECKS: u32 = 20;

pub fn router(state: AgentState) -> Router {
    Router::new()
        .route(
            "/api/v1/workspaces/{workspace_id}/agent-tools/shopify-image",
            post(call),
        )
        .with_state(state)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CallBody {
    #[serde(default)]
    arguments: Map<String, Value>,
    /// The thread of the run: approvals belong to it.
    conversation_id: String,
    root_message_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Args {
    /// The integration's slug.
    integration: String,
    node_id: String,
    alt: Option<String>,
    article_id: Option<String>,
    place: Option<Place>,
}

#[derive(Clone, Copy, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Place {
    /// The article's image (featured image).
    Image,
    /// At the end of the article's body.
    Body,
}

/// What the image goes to, checked.
struct Plan {
    alt: Option<String>,
    article: Option<(String, Place)>,
}

fn refused(detail: &str) -> Json<Value> {
    Json(json!({ "status": "refused", "detail": detail }))
}

fn failed(detail: &str) -> Json<Value> {
    Json(json!({ "status": "failed", "detail": detail }))
}

/// An article id as the model gives it (`123` or `gid://shopify/Article/123`) as a GraphQL id.
fn article_gid(raw: &str) -> Option<String> {
    let raw = raw.trim();
    let number = raw.strip_prefix("gid://shopify/Article/").unwrap_or(raw);
    (!number.is_empty() && number.len() <= 20 && number.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| format!("gid://shopify/Article/{number}"))
}

fn plan(args: &Args) -> Result<Plan, &'static str> {
    let alt = args
        .alt
        .as_deref()
        .map(|alt| {
            alt.chars()
                .map(|character| {
                    if character.is_control() {
                        ' '
                    } else {
                        character
                    }
                })
                .collect::<String>()
                .trim()
                .to_owned()
        })
        .filter(|alt| !alt.is_empty());
    if alt
        .as_ref()
        .is_some_and(|alt| alt.chars().count() > MAX_ALT_CHARS)
    {
        return Err("The alt text is longer than 512 characters.");
    }
    let article = match (&args.article_id, args.place) {
        (None, None) => None,
        (Some(id), Some(place)) => Some((
            article_gid(id).ok_or("article_id is a number or gid://shopify/Article/<number>.")?,
            place,
        )),
        _ => return Err("Give article_id and place together, or neither."),
    };
    Ok(Plan { alt, article })
}

/// The image type from the first bytes, never from the name: JPEG, PNG, GIF or WebP.
pub(crate) fn image_type(bytes: &[u8]) -> Option<(&'static str, &'static str)> {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(("image/jpeg", "jpg"))
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(("image/png", "png"))
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some(("image/gif", "gif"))
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some(("image/webp", "webp"))
    } else {
        None
    }
}

/// A file name Shopify gets: letters, digits, `.`, `-` and `_` of the Drive name, with the
/// extension of the real type.
fn upload_name(drive_name: &str, extension: &str) -> String {
    let stem = drive_name
        .rsplit_once('.')
        .map_or(drive_name, |(stem, _)| stem);
    let stem: String = stem
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '-'
            }
        })
        .take(100)
        .collect();
    let stem = stem.trim_matches('-');
    format!(
        "{}.{extension}",
        if stem.is_empty() { "image" } else { stem }
    )
}

/// The integration is a Shopify Admin API: its GraphQL URL.
fn graphql_url(integration: &Integration) -> Result<Url, &'static str> {
    const NOT_SHOPIFY: &str = "This integration is not a Shopify Admin API (its base URL must end with /admin/api/<version>).";
    if integration.kind != "http" {
        return Err(NOT_SHOPIFY);
    }
    let base = Url::parse(&integration.url).map_err(|_| NOT_SHOPIFY)?;
    let segments: Vec<&str> = base
        .path()
        .trim_end_matches('/')
        .rsplit('/')
        .take(3)
        .collect();
    if !matches!(segments.as_slice(), [version, "api", "admin"] if !version.is_empty()) {
        return Err(NOT_SHOPIFY);
    }
    if !integration
        .allowed_methods
        .iter()
        .any(|method| method == "POST")
    {
        return Err("This Shopify integration does not allow POST.");
    }
    join_api_url(&base, "/graphql.json", &[]).map_err(|_| NOT_SHOPIFY)
}

fn size_text(bytes: usize) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{} KiB", bytes.div_ceil(1024))
    }
}

/// The exact action, as the driver approves it.
fn describe(integration: &Integration, path: &str, mime: &str, size: usize, plan: &Plan) -> String {
    let then = match &plan.article {
        None => String::new(),
        Some((id, Place::Image)) => format!(", then set it as the image of article {id}"),
        Some((id, Place::Body)) => format!(", then add it at the end of the body of article {id}"),
    };
    let alt = plan
        .alt
        .as_ref()
        .map_or_else(String::new, |alt| format!(" (alt text \"{alt}\")"));
    format!(
        "{}: upload the Drive image \"{path}\" ({mime}, {}) to the store's Files{then}{alt}.",
        integration.name,
        size_text(size)
    )
}

// An axum handler: each extractor is one argument.
#[allow(clippy::too_many_arguments)]
async fn call(
    State(state): State<AgentState>,
    Path(workspace_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    request_id: RequestIdExtension,
    dry: Option<Extension<super::scope::DryRun>>,
    Json(body): Json<CallBody>,
) -> Result<Json<Value>, ApiError> {
    let (user, workspace_id, role, access) =
        driver_access(&state, &headers, &workspace_id, &uri, &request_id).await?;
    let database = state.database();
    let thread = (
        body.conversation_id.as_str(),
        body.root_message_id.as_deref(),
    );
    require_running(database, (workspace_id, user), thread, &uri, &request_id).await?;
    let raw = Value::Object(body.arguments);
    let Ok(args) = serde_json::from_value::<Args>(raw.clone()) else {
        return Ok(refused(
            "Give integration and node_id, and optionally alt, and article_id with place (image or body).",
        ));
    };
    let plan = match plan(&args) {
        Ok(plan) => plan,
        Err(detail) => return Ok(refused(detail)),
    };
    let integration = load_one(database, workspace_id, "slug", args.integration.trim())
        .await
        .map_err(unavailable(&uri, &request_id))?;
    let Some(integration) = integration else {
        return Ok(refused("There is no integration with this name."));
    };
    if !integration.allows(role) {
        return Ok(refused("The driver's role may not use this integration."));
    }
    if !access.allows(&integration.id) {
        return Ok(refused(
            "This agent may not use this integration: it is not ticked on the agent's card.",
        ));
    }
    let graphql = match graphql_url(&integration) {
        Ok(url) => url,
        Err(detail) => return Ok(refused(detail)),
    };
    let Some(drive) = &state.drive else {
        return Ok(refused("Drive is not available on this server."));
    };
    // As the driver: a file they cannot see is "not available", like in every Drive tool.
    let Ok(node_id) = args.node_id.trim().parse::<Id>() else {
        return Ok(failed(drive_failure(
            &crate::repositories::drive::DriveError::NotFound,
        )));
    };
    let detail = match drive
        .service
        .repository()
        .node(workspace_id, user, node_id)
        .await
    {
        Ok(detail) if detail.node.kind == DriveNodeKind::File => detail,
        Ok(_) => return Ok(refused("This is a folder: give an image file.")),
        Err(error) => return Ok(failed(drive_failure(&error))),
    };
    let (node, bytes) = match drive.service.file_bytes(workspace_id, user, node_id).await {
        Ok(file) => file,
        Err(error) => return Ok(failed(drive_failure(&error))),
    };
    let Some((mime, extension)) = image_type(&bytes) else {
        return Ok(refused(
            "Only JPEG, PNG, GIF and WebP images can go to Shopify, and this file is none of them.",
        ));
    };
    let path = detail
        .path
        .iter()
        .map(|crumb| crumb.name.as_str())
        .chain([node.name.as_str()])
        .collect::<Vec<_>>()
        .join("/");
    let action = describe(&integration, &path, mime, bytes.len(), &plan);
    // A test run reads, and only says which writes it would make.
    if dry.is_some() {
        return Ok(Json(json!({ "status": "dry_run", "action": action })));
    }
    let approval = approve(
        database,
        Approves::Integration(&integration.id),
        user,
        thread,
        TOOL,
        &hash(&canonical(&raw)),
    )
    .await
    .map_err(unavailable(&uri, &request_id))?;
    let approval_id = match approval {
        Approval::Needed { id, expires_at } => {
            return Ok(Json(json!({
                "status": "approval_required",
                "approval_id": id,
                "action": action,
                "expires_at": expires_at,
            })));
        }
        Approval::Granted { id } => id,
    };
    let shopify = Shopify {
        state: &state,
        integration: &integration,
        url: &graphql,
    };
    let outcome = shopify
        .upload(&upload_name(&node.name, extension), mime, bytes, &plan)
        .await;
    let mut metadata = json!({
        "integration": integration.slug,
        "tool": TOOL,
        "node_id": node_id.to_string(),
        "article_id": plan.article.as_ref().map(|(id, _)| id),
        "approval_id": approval_id,
        "approved": true,
    });
    match &outcome {
        Ok(result) => metadata["shopify_file_id"] = result["shopify_file_id"].clone(),
        Err(error) => {
            metadata["error"] = json!(error.chars().take(200).collect::<String>());
        }
    }
    record(
        database,
        workspace_id,
        user,
        "agent_integration.shopify_image",
        if outcome.is_ok() {
            AuditOutcome::Success
        } else {
            AuditOutcome::Failure
        },
        &integration.id,
        &request_id,
        metadata,
    )
    .await;
    Ok(Json(match outcome {
        Ok(mut result) => {
            result["file"] = json!({ "id": node.id, "name": node.name });
            json!({ "status": "ok", "label": format!("integration {}", integration.slug), "result": result })
        }
        Err(detail) => json!({ "status": "failed", "detail": detail }),
    }))
}

/// The messages of Shopify's `errors` or `userErrors`, short.
fn messages(errors: &Value) -> Option<String> {
    let list = errors.as_array()?;
    if list.is_empty() {
        return None;
    }
    let text = list
        .iter()
        .map(|error| {
            error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("unknown error")
        })
        .collect::<Vec<_>>()
        .join("; ");
    Some(text.chars().take(500).collect())
}

/// HTML for an image at the end of an article's body: the URL and the alt text escaped.
fn image_html(url: &str, alt: Option<&str>) -> String {
    let escape = |text: &str| {
        text.replace('&', "&amp;")
            .replace('"', "&quot;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    };
    format!(
        "<p><img src=\"{}\" alt=\"{}\"></p>",
        escape(url),
        escape(alt.unwrap_or_default())
    )
}

struct Shopify<'a> {
    state: &'a AgentState,
    integration: &'a Integration,
    url: &'a Url,
}

impl Shopify<'_> {
    /// One GraphQL request: its `data`, or Shopify's errors as text.
    async fn graphql(&self, query: &str, variables: Value) -> Result<Value, String> {
        let answer = api_send(
            self.state,
            self.integration,
            self.url,
            reqwest::Method::POST,
            Some(&json!({ "query": query, "variables": variables })),
        )
        .await
        .map_err(|error| capitalized(&error.to_string()))?;
        let errors = answer.body.get("errors").and_then(messages);
        if !(200..300).contains(&answer.status) {
            return Err(format!(
                "Shopify answered HTTP {}{}",
                answer.status,
                errors.map_or_else(|| ".".to_owned(), |text| format!(": {text}"))
            ));
        }
        if let Some(text) = errors {
            return Err(format!("Shopify: {text}"));
        }
        Ok(answer.body.get("data").cloned().unwrap_or(Value::Null))
    }

    /// `data.<field>`, or its `userErrors` as text.
    async fn mutation(&self, field: &str, query: &str, variables: Value) -> Result<Value, String> {
        let data = self.graphql(query, variables).await?;
        let result = data.get(field).cloned().unwrap_or(Value::Null);
        if let Some(text) = result.get("userErrors").and_then(messages) {
            return Err(format!("Shopify: {text}"));
        }
        Ok(result)
    }

    async fn upload(
        &self,
        file_name: &str,
        mime: &str,
        bytes: Vec<u8>,
        plan: &Plan,
    ) -> Result<Value, String> {
        let staged = self
            .mutation(
                "stagedUploadsCreate",
                "mutation ($input: [StagedUploadInput!]!) { stagedUploadsCreate(input: $input) { \
                 stagedTargets { url resourceUrl parameters { name value } } userErrors { field message } } }",
                json!({ "input": [{
                    "resource": "IMAGE",
                    "filename": file_name,
                    "mimeType": mime,
                    "httpMethod": "POST",
                    "fileSize": bytes.len().to_string(),
                }] }),
            )
            .await?;
        let target = &staged["stagedTargets"][0];
        let (Some(target_url), Some(resource_url)) = (
            target["url"].as_str().and_then(|url| Url::parse(url).ok()),
            target["resourceUrl"].as_str(),
        ) else {
            return Err("Shopify gave no upload target.".to_owned());
        };
        let fields: Vec<(String, String)> = target["parameters"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|parameter| {
                Some((
                    parameter["name"].as_str()?.to_owned(),
                    parameter["value"].as_str()?.to_owned(),
                ))
            })
            .collect();
        // The target is signed: no credential goes with it, and it is never shown or logged.
        let status = self
            .state
            .outbound
            .post_file(&target_url, &fields, (file_name, mime, bytes))
            .await
            .map_err(|error| capitalized(&format!("the upload to Shopify failed: {error}")))?;
        if !(200..300).contains(&status) {
            return Err(format!("The upload to Shopify failed with HTTP {status}."));
        }
        let created = self
            .mutation(
                "fileCreate",
                "mutation ($files: [FileCreateInput!]!) { fileCreate(files: $files) { \
                 files { id fileStatus ... on MediaImage { image { url } } } userErrors { field message } } }",
                json!({ "files": [{
                    "originalSource": resource_url,
                    "contentType": "IMAGE",
                    "alt": plan.alt.clone().unwrap_or_default(),
                }] }),
            )
            .await?;
        let file = &created["files"][0];
        let file_id = file["id"]
            .as_str()
            .ok_or("Shopify did not create the file.")?
            .to_owned();
        let url = self.ready_url(&file_id, file).await?;
        let mut result = json!({ "shopify_file_id": file_id, "url": url });
        if let Some((article_id, place)) = &plan.article {
            self.place(article_id, *place, &url, plan.alt.as_deref())
                .await
                .map_err(|error| {
                    format!("The image is in Shopify's Files at {url}, but the article was not changed: {error}")
                })?;
            result["article"] = json!({
                "id": article_id,
                "placed": if *place == Place::Image { "image" } else { "body" },
            });
        }
        Ok(result)
    }

    /// The image's URL once Shopify has processed the file (it does so after `fileCreate`).
    async fn ready_url(&self, file_id: &str, first: &Value) -> Result<String, String> {
        let mut file = first.clone();
        for check in 0..=READY_CHECKS {
            match file["fileStatus"].as_str() {
                Some("FAILED") => {
                    return Err(format!(
                        "Shopify could not process the image{}",
                        file.get("fileErrors")
                            .and_then(messages)
                            .map_or_else(|| ".".to_owned(), |text| format!(": {text}"))
                    ));
                }
                Some("READY") => {
                    if let Some(url) = file["image"]["url"].as_str() {
                        return Ok(url.to_owned());
                    }
                }
                _ => {}
            }
            if check == READY_CHECKS {
                break;
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
            let data = self
                .graphql(
                    "query ($id: ID!) { node(id: $id) { ... on MediaImage { fileStatus \
                     fileErrors { message } image { url } } } }",
                    json!({ "id": file_id }),
                )
                .await?;
            file = data["node"].clone();
        }
        Err(format!(
            "Shopify is still processing the image ({file_id}); look for it in Files later."
        ))
    }

    async fn place(
        &self,
        article_id: &str,
        place: Place,
        url: &str,
        alt: Option<&str>,
    ) -> Result<(), String> {
        let article = match place {
            Place::Image => {
                let mut image = json!({ "url": url });
                if let Some(alt) = alt {
                    image["altText"] = json!(alt);
                }
                json!({ "image": image })
            }
            Place::Body => {
                let data = self
                    .graphql(
                        "query ($id: ID!) { article(id: $id) { body } }",
                        json!({ "id": article_id }),
                    )
                    .await?;
                if data["article"].is_null() {
                    return Err("Shopify found no article with this id.".to_owned());
                }
                let body = data["article"]["body"].as_str().unwrap_or_default();
                json!({ "body": format!("{body}\n{}", image_html(url, alt)) })
            }
        };
        let updated = self
            .mutation(
                "articleUpdate",
                "mutation ($id: ID!, $article: ArticleUpdateInput!) { articleUpdate(id: $id, article: $article) { \
                 article { id } userErrors { field message } } }",
                json!({ "id": article_id, "article": article }),
            )
            .await?;
        if updated["article"].is_null() {
            return Err("Shopify found no article with this id.".to_owned());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{article_gid, image_html, image_type, upload_name};

    #[test]
    fn the_type_comes_from_the_bytes_never_the_name() {
        assert_eq!(
            image_type(b"\xFF\xD8\xFF\xE0rest"),
            Some(("image/jpeg", "jpg"))
        );
        assert_eq!(
            image_type(b"\x89PNG\r\n\x1a\nrest"),
            Some(("image/png", "png"))
        );
        assert_eq!(image_type(b"GIF89a.."), Some(("image/gif", "gif")));
        assert_eq!(
            image_type(b"RIFF\0\0\0\0WEBPVP8 "),
            Some(("image/webp", "webp"))
        );
        assert_eq!(
            image_type(b"<svg xmlns='http://www.w3.org/2000/svg'/>"),
            None
        );
        assert_eq!(image_type(b"%PDF-1.7"), None);
        assert_eq!(image_type(b""), None);
    }

    #[test]
    fn shopify_gets_a_plain_file_name_with_the_real_extension() {
        assert_eq!(
            upload_name("Hero shot (final).PNG", "jpg"),
            "Hero-shot--final.jpg"
        );
        assert_eq!(upload_name("\"><script>.png", "png"), "script.png");
        assert_eq!(upload_name("...", "gif"), "image.gif");
    }

    #[test]
    fn article_ids_are_numbers_or_article_gids() {
        assert_eq!(
            article_gid("635382366358").as_deref(),
            Some("gid://shopify/Article/635382366358")
        );
        assert_eq!(
            article_gid("gid://shopify/Article/7").as_deref(),
            Some("gid://shopify/Article/7")
        );
        for bad in [
            "",
            "gid://shopify/Product/7",
            "7 OR 1",
            "gid://shopify/Article/",
        ] {
            assert_eq!(article_gid(bad), None, "{bad}");
        }
    }

    #[test]
    fn the_body_image_escapes_the_url_and_the_alt_text() {
        assert_eq!(
            image_html("https://cdn.shopify.com/a.jpg?v=1&x=2", Some("\"><script>")),
            "<p><img src=\"https://cdn.shopify.com/a.jpg?v=1&amp;x=2\" alt=\"&quot;&gt;&lt;script&gt;\"></p>"
        );
    }
}
