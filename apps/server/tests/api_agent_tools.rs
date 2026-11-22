//! The wider token tools (MCP and the public REST API): what each one needs, and that a credential
//! stays inside its scopes, its projects, its workspace and its person's access. SQLite by default,
//! Postgres when `ORBIT_TEST_POSTGRES_URL` is set.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::http::{HeaderMap, Request, StatusCode, header};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use orbit_platform::{
    AttachmentMutationCoordinator, HttpLimits, HttpPlatformLayer, Id, LocalBlobStore, OriginPolicy,
    PasswordService, TestDatabase, TimestampMillis, UploadLimits, UploadService,
};
use orbit_server::mcp::McpState;
use orbit_server::openapi::openapi_json;
use orbit_server::public_api::PublicApiState;
use orbit_server::repositories::api_tokens::{
    ApiTokenError, ApiTokenRepository, ApiTokenScope, TokenProjects,
};
use orbit_server::repositories::chat::ChatRepository;
use orbit_server::repositories::identity::{IdentityRepository, SetupRequest};
use orbit_server::repositories::pages::{CreatePage, PageRepository, SpaceRequest, revision};
use orbit_server::repositories::tasks::{CreateTask, TaskRepository};
use orbit_server::repositories::workspaces::WorkspaceRepository;
use serde_json::{Value, json};
use tower::ServiceExt;

const ORIGIN: &str = "https://orbit.test";
use ApiTokenScope::{
    ChatRead, DocsManage, DocsRead, DocsWrite, TasksManage, TasksRead, TasksWrite,
};

/// Every scope a person's token can have here (Drive has its own tests).
const ALL: [ApiTokenScope; 7] = [
    TasksRead,
    TasksWrite,
    TasksManage,
    DocsRead,
    DocsWrite,
    DocsManage,
    ChatRead,
];

struct Fixture {
    _root: tempfile::TempDir,
    database: TestDatabase,
    mcp: axum::Router,
    rest: axum::Router,
    tokens: ApiTokenRepository,
    tasks: TaskRepository,
    pages: Arc<PageRepository>,
    owner: Id,
    workspace: Id,
    /// On the limited token.
    project: Id,
    /// Not on the limited token, and private.
    secret: Id,
    task: Id,
    secret_task: Id,
}

impl Fixture {
    async fn new() -> Self {
        let database = TestDatabase::new().await.unwrap();
        let identity = IdentityRepository::new((*database).clone());
        let now = TimestampMillis::now();
        identity
            .store_setup_token(
                "setup",
                TimestampMillis::from_millis(now.as_millis() + 60_000),
            )
            .await
            .unwrap();
        let setup = identity
            .complete_setup(
                SetupRequest {
                    token: "setup".into(),
                    email: "owner@example.com".into(),
                    display_name: "Owner".into(),
                    password_hash: PasswordService::default()
                        .hash("correct horse battery")
                        .unwrap(),
                    workspace_name: "Orbit".into(),
                    project_name: "General".into(),
                },
                now,
            )
            .await
            .unwrap();
        let tasks = TaskRepository::new((*database).clone());
        let secret = tasks
            .create_project(
                setup.workspace_id,
                setup.user_id,
                "Hush project".into(),
                "HSH".into(),
                "#ff0000".into(),
                true,
                "test",
                now,
            )
            .await
            .unwrap()
            .id;
        let root = tempfile::tempdir().unwrap();
        let uploads = UploadService::new(
            (*database).clone(),
            Arc::new(LocalBlobStore::new(root.path().join("attachments"))),
            AttachmentMutationCoordinator::default(),
            UploadLimits::default(),
        );
        let tokens = ApiTokenRepository::new((*database).clone());
        let pages = Arc::new(PageRepository::new((*database).clone()));
        let chat = Arc::new(ChatRepository::new((*database).clone(), uploads.clone()));
        // The limits of the server: uploads up to 100 MiB, other bodies up to 2 MiB.
        let layer = || {
            HttpPlatformLayer::new(OriginPolicy::new(ORIGIN)).with_limits(HttpLimits {
                max_body_bytes: 100 * 1024 * 1024,
                ..HttpLimits::default()
            })
        };
        let mcp = orbit_server::mcp::router(
            McpState::new(Arc::new(tokens.clone()), Arc::new(tasks.clone()))
                .with_content(Arc::clone(&pages), Arc::clone(&chat))
                .with_uploads(uploads.clone()),
            ORIGIN,
        )
        .layer(layer());
        let rest = orbit_server::public_api::router(
            PublicApiState::new(Arc::new(tokens.clone()), Arc::new(tasks.clone()), ORIGIN)
                .with_pages(Arc::clone(&pages))
                .with_chat(chat)
                .with_uploads(uploads),
        )
        .layer(layer());
        let mut fixture = Self {
            _root: root,
            database,
            mcp,
            rest,
            tokens,
            tasks,
            pages,
            owner: setup.user_id,
            workspace: setup.workspace_id,
            project: setup.project_id,
            secret,
            task: setup.project_id,
            secret_task: secret,
        };
        fixture.task = fixture.new_task(fixture.project, "Open task", None).await;
        fixture.secret_task = fixture.new_task(secret, "Hush task", None).await;
        fixture
    }

    async fn new_task(&self, project: Id, title: &str, parent: Option<Id>) -> Id {
        let status = self.status(project).await;
        self.tasks
            .create_task(
                self.workspace,
                self.owner,
                CreateTask {
                    project_id: project,
                    status_id: status,
                    title: title.into(),
                    description: String::new(),
                    description_json: None,
                    source_url: None,
                    priority: "none".into(),
                    position: None,
                    assignee_ids: vec![],
                    label_ids: vec![],
                    due_start_at: None,
                    due_at: None,
                    parent_task_id: parent,
                    milestone_id: None,
                    cycle_id: None,
                    estimate: None,
                },
                "test",
                TimestampMillis::now(),
            )
            .await
            .unwrap()
            .task
            .id
    }

    async fn status(&self, project: Id) -> Id {
        sqlx::query_scalar::<_, String>(
            "SELECT id FROM task_statuses WHERE project_id = $1 ORDER BY position LIMIT 1",
        )
        .bind(project.to_string())
        .fetch_one(self.database.pool())
        .await
        .unwrap()
        .parse()
        .unwrap()
    }

    /// A personal token of `creator`.
    async fn token_by(
        &self,
        creator: Id,
        projects: impl Into<TokenProjects>,
        scopes: &[ApiTokenScope],
    ) -> String {
        self.tokens
            .create(
                self.workspace,
                creator,
                format!("t-{}", Id::new_v7()),
                projects,
                scopes,
                false,
                None,
                "test",
                TimestampMillis::now(),
            )
            .await
            .unwrap()
            .token
    }

    /// The owner's token for `project` only.
    async fn limited(&self, scopes: &[ApiTokenScope]) -> String {
        self.token_by(self.owner, vec![self.project], scopes).await
    }

    async fn everything(&self) -> String {
        self.token_by(self.owner, TokenProjects::All, &ALL).await
    }

    async fn service_account(&self, scopes: &[ApiTokenScope]) -> Result<String, ApiTokenError> {
        self.tokens
            .create(
                self.workspace,
                self.owner,
                format!("sa-{}", Id::new_v7()),
                vec![self.project],
                scopes,
                true,
                None,
                "test",
                TimestampMillis::now(),
            )
            .await
            .map(|issued| issued.token)
    }

    async fn person(&self, name: &str, role: &str) -> Id {
        let user = Id::new_v7();
        sqlx::query("INSERT INTO users (id, email, normalized_email, display_name, password_hash, created_at, updated_at) VALUES ($1, $2, $2, $3, 'x', 1, 1)")
            .bind(user.to_string())
            .bind(format!("{name}@example.com"))
            .bind(name)
            .execute(self.database.pool())
            .await
            .unwrap();
        sqlx::query("INSERT INTO memberships (id, workspace_id, user_id, role, created_at, updated_at) VALUES ($1, $2, $3, $4, 1, 1)")
            .bind(Id::new_v7().to_string())
            .bind(self.workspace.to_string())
            .bind(user.to_string())
            .bind(role)
            .execute(self.database.pool())
            .await
            .unwrap();
        user
    }

    async fn set_role(&self, user: Id, role: &str) {
        sqlx::query("UPDATE memberships SET role = $1 WHERE user_id = $2")
            .bind(role)
            .bind(user.to_string())
            .execute(self.database.pool())
            .await
            .unwrap();
    }

    async fn doc(&self, actor: Id, title: &str, parent: Option<Id>, private: bool) -> Id {
        self.pages
            .create_page_with_content(
                self.workspace,
                actor,
                CreatePage {
                    parent_id: parent,
                    space: SpaceRequest {
                        teamspace_id: None,
                        private: private.then_some(true),
                    },
                    title: title.into(),
                    icon: None,
                    position: None,
                },
                &[json!({"type": "paragraph", "content": [{"type": "text", "text": "Body text", "styles": {}}]})],
                "test",
                TimestampMillis::now(),
            )
            .await
            .unwrap()
            .id
    }

    async fn doc_version(&self, page: Id) -> u64 {
        let page = self
            .pages
            .get_page(self.workspace, page, self.owner)
            .await
            .unwrap();
        revision(page.version, page.updated_at)
    }

    async fn rpc(&self, token: &str, method: &str, params: Value) -> (StatusCode, Value) {
        let body = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params});
        let body = body.to_string();
        let request = Request::builder()
            .method("POST")
            .uri("/mcp")
            .header(header::HOST, "orbit.test")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::CONTENT_LENGTH, body.len())
            .header(header::ACCEPT, "application/json, text/event-stream")
            .header("mcp-protocol-version", "2025-11-25")
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::from(body))
            .unwrap();
        let response = self.mcp.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 4 << 20).await.unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    /// A tool call: its JSON-RPC answer.
    async fn raw(&self, token: &str, name: &str, arguments: Value) -> Value {
        let (status, body) = self
            .rpc(
                token,
                "tools/call",
                json!({"name": name, "arguments": arguments}),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{name}: {body}");
        body
    }

    /// A tool call that must succeed: its structured result.
    async fn tool(&self, token: &str, name: &str, arguments: Value) -> Value {
        let body = self.raw(token, name, arguments).await;
        assert!(
            body["error"].is_null() && body["result"]["isError"] != true,
            "{name}: {body}"
        );
        body["result"]["structuredContent"].clone()
    }

    /// A tool call that must be refused: the whole answer as text.
    async fn refused(&self, token: &str, name: &str, arguments: Value) -> String {
        let body = self.raw(token, name, arguments).await;
        assert!(
            !body["error"].is_null() || body["result"]["isError"] == true,
            "{name} was not refused: {body}"
        );
        // Not for a mistake of this test.
        assert!(
            !body.to_string().contains("Invalid tool arguments"),
            "{name}: {body}"
        );
        body.to_string()
    }

    async fn tool_list(&self, token: &str) -> Vec<Value> {
        let (_, body) = self.rpc(token, "tools/list", json!({})).await;
        body["result"]["tools"].as_array().unwrap().clone()
    }

    async fn tool_names(&self, token: &str) -> BTreeSet<String> {
        self.tool_list(token)
            .await
            .iter()
            .map(|tool| tool["name"].as_str().unwrap().to_owned())
            .collect()
    }

    async fn rest_full(
        &self,
        token: &str,
        method: &str,
        path: &str,
        body: Option<(&str, Vec<u8>)>,
        headers: &[(&str, &str)],
    ) -> (StatusCode, Vec<u8>, HeaderMap) {
        let mut request = Request::builder()
            .method(method.to_ascii_uppercase().as_str())
            .uri(format!(
                "/api/public/v1/workspaces/{}{path}",
                self.workspace
            ))
            .header(header::AUTHORIZATION, format!("Bearer {token}"));
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        let request = match body {
            Some((content_type, body)) => request
                .header(header::CONTENT_TYPE, content_type)
                .body(Body::from(body)),
            None => request.body(Body::empty()),
        }
        .unwrap();
        let response = self.rest.clone().oneshot(request).await.unwrap();
        let (status, headers) = (response.status(), response.headers().clone());
        let bytes = to_bytes(response.into_body(), 4 << 20).await.unwrap();
        (status, bytes.to_vec(), headers)
    }

    async fn rest(
        &self,
        token: &str,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let body = body.map(|body| ("application/json", body.to_string().into_bytes()));
        let (status, bytes, _) = self.rest_full(token, method, path, body, &[]).await;
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    async fn count(&self, sql: &str, bind: &str) -> i64 {
        sqlx::query_scalar(sqlx::AssertSqlSafe(sql.to_owned()))
            .bind(bind)
            .fetch_one(self.database.pool())
            .await
            .unwrap()
    }

    /// Nothing about the secret project shows: no title, key or id.
    fn assert_hushed(&self, what: &str, text: &str) {
        for leak in [
            "Hush".to_owned(),
            "HSH".to_owned(),
            self.secret.to_string(),
            self.secret_task.to_string(),
        ] {
            // A refusal may repeat an id of its own request, never a name.
            if leak.contains('-') && what.starts_with("refused") {
                continue;
            }
            assert!(!text.contains(&leak), "{what} shows {leak}: {text}");
        }
    }
}

/// Every MCP tool with its scope, and whether it only reads or may destroy. A new tool fails this
/// test until someone decides.
fn tool_matrix() -> Vec<(&'static str, Option<ApiTokenScope>, &'static str)> {
    let (read, write, destroy) = ("read", "write", "destroy");
    let mut all = vec![("get_me", None, read)];
    for name in [
        "list_projects",
        "list_tasks",
        "get_task",
        "list_task_statuses",
        "list_labels",
        "list_milestones",
        "list_cycles",
        "list_members",
        "list_comments",
        "list_task_relations",
        "list_task_trash",
        "get_trashed_task",
        "list_task_activity",
        "list_task_github_links",
        "list_project_trash",
        "get_trashed_project",
        "list_notifications",
        "list_views",
        "get_view",
        "list_task_attachments",
        "download_task_attachment",
    ] {
        all.push((name, Some(TasksRead), read));
    }
    for (name, kind) in [
        ("create_task", write),
        ("update_task", destroy),
        ("delete_task", destroy),
        ("create_comment", write),
        ("create_project", write),
        ("create_label", write),
        ("update_label", destroy),
        ("create_task_relation", destroy),
        ("delete_task_relation", destroy),
        ("restore_task", write),
        ("create_milestone", write),
        ("update_milestone", destroy),
        ("create_cycle", write),
        ("update_cycle", destroy),
        ("update_comment", destroy),
        ("bulk_update_tasks", destroy),
        ("reorder_tasks", write),
        ("reorder_milestones", write),
        ("move_unfinished_cycle_tasks", write),
        ("mark_notification_read", write),
        ("mark_all_notifications_read", write),
        ("create_view", write),
        ("update_view", destroy),
        ("set_view_favorite", write),
        ("reorder_view_favorites", write),
        ("upload_task_attachment", write),
    ] {
        all.push((name, Some(TasksWrite), kind));
    }
    for (name, kind) in [
        ("update_project", destroy),
        ("create_task_status", write),
        ("update_task_status", destroy),
        ("reorder_task_statuses", write),
        ("delete_task_status", destroy),
        ("delete_label", destroy),
        ("delete_milestone", destroy),
        ("delete_cycle", destroy),
        ("delete_comment", destroy),
        ("delete_task_attachment", destroy),
        ("delete_project", destroy),
        ("restore_project", write),
        ("delete_view", destroy),
    ] {
        all.push((name, Some(TasksManage), kind));
    }
    for name in [
        "list_docs",
        "get_doc",
        "list_doc_versions",
        "get_doc_version",
        "list_doc_comments",
        "list_teamspaces",
        "list_doc_trash",
        "search_docs",
        "list_recent_docs",
        "list_favorite_docs",
        "download_doc_file",
    ] {
        all.push((name, Some(DocsRead), read));
    }
    for (name, kind) in [
        ("create_doc", write),
        ("update_doc", destroy),
        ("restore_doc", write),
        ("create_doc_comment", write),
        ("update_doc_comment", destroy),
        ("resolve_doc_thread", write),
        ("reopen_doc_thread", write),
        ("add_favorite_doc", write),
        ("remove_favorite_doc", write),
        ("move_favorite_doc", write),
        ("duplicate_doc", write),
        ("restore_doc_version", destroy),
        ("upload_doc_file", write),
    ] {
        all.push((name, Some(DocsWrite), kind));
    }
    for (name, kind) in [
        ("delete_doc", destroy),
        ("move_doc", destroy),
        ("set_doc_lock", destroy),
        ("delete_doc_comment", destroy),
        ("delete_doc_thread", destroy),
    ] {
        all.push((name, Some(DocsManage), kind));
    }
    for name in [
        "list_conversations",
        "list_messages",
        "get_thread",
        "search_messages",
    ] {
        all.push((name, Some(ChatRead), read));
    }
    all
}

/// Every operation of the public REST API with the scope it needs.
fn rest_matrix() -> BTreeMap<&'static str, Option<ApiTokenScope>> {
    let mut all = BTreeMap::from([("public_get_me", None)]);
    for (scope, operations) in [
        (
            TasksRead,
            vec![
                "public_list_projects",
                "public_list_statuses",
                "public_list_labels",
                "public_list_milestones",
                "public_list_cycles",
                "public_list_members",
                "public_list_tasks",
                "public_get_task",
                "public_list_comments",
                "public_list_relations",
                "public_list_task_trash",
                "public_get_trashed_task",
                "public_list_task_activity",
                "public_list_task_github_links",
                "public_list_project_trash",
                "public_get_trashed_project",
                "public_list_notifications",
                "public_list_views",
                "public_get_view",
                "public_list_task_attachments",
                "public_list_comment_attachments",
                "public_download_task_attachment",
                "public_download_comment_attachment",
            ],
        ),
        (
            TasksWrite,
            vec![
                "public_create_project",
                "public_create_task",
                "public_update_task",
                "public_delete_task",
                "public_create_comment",
                "public_update_comment",
                "public_create_label",
                "public_update_label",
                "public_create_relation",
                "public_delete_relation",
                "public_restore_task",
                "public_create_milestone",
                "public_update_milestone",
                "public_create_cycle",
                "public_update_cycle",
                "public_bulk_update_tasks",
                "public_reorder_tasks",
                "public_reorder_milestones",
                "public_move_unfinished_cycle_tasks",
                "public_read_notification",
                "public_read_all_notifications",
                "public_create_view",
                "public_update_view",
                "public_favorite_view",
                "public_unfavorite_view",
                "public_reorder_view_favorites",
                "public_upload_task_attachment",
                "public_upload_comment_attachment",
            ],
        ),
        (
            TasksManage,
            vec![
                "public_update_project",
                "public_create_status",
                "public_update_status",
                "public_reorder_statuses",
                "public_delete_status",
                "public_delete_label",
                "public_delete_milestone",
                "public_delete_cycle",
                "public_delete_comment",
                "public_delete_task_attachment",
                "public_delete_comment_attachment",
                "public_delete_project",
                "public_restore_project",
                "public_delete_view",
            ],
        ),
        (
            DocsRead,
            vec![
                "public_list_docs",
                "public_get_doc",
                "public_list_doc_versions",
                "public_get_doc_version",
                "public_list_doc_comments",
                "public_list_teamspaces",
                "public_list_doc_trash",
                "public_search_docs",
                "public_list_recent_docs",
                "public_list_favorite_docs",
                "public_download_doc_file",
            ],
        ),
        (
            DocsWrite,
            vec![
                "public_create_doc",
                "public_update_doc",
                // As before: the REST move and trash of a doc need `docs:write`.
                "public_move_doc",
                "public_delete_doc",
                "public_restore_doc",
                "public_create_doc_comment",
                "public_update_doc_comment",
                "public_resolve_doc_thread",
                "public_reopen_doc_thread",
                "public_add_favorite_doc",
                "public_remove_favorite_doc",
                "public_move_favorite_doc",
                "public_duplicate_doc",
                "public_restore_doc_version",
                "public_upload_doc_file",
            ],
        ),
        (
            DocsManage,
            vec![
                "public_lock_doc",
                "public_delete_doc_comment",
                "public_delete_doc_thread",
            ],
        ),
        (
            ChatRead,
            vec![
                "public_list_conversations",
                "public_list_messages",
                "public_get_thread",
                "public_search_messages",
            ],
        ),
    ] {
        for operation in operations {
            all.insert(operation, Some(scope));
        }
    }
    all
}

/// The public operations with a workspace path: `(method, path with real or unknown ids, id)`.
fn public_operations(f: &Fixture) -> Vec<(String, String, String)> {
    let document: Value = serde_json::from_str(&openapi_json().unwrap()).unwrap();
    let mut out = Vec::new();
    for (path, item) in document["paths"].as_object().unwrap() {
        let Some(path) = path.strip_prefix("/api/public/v1/workspaces/{workspace_id}") else {
            continue;
        };
        let mut path = path
            .replace("{project_id}", &f.project.to_string())
            .replace("{task}", &f.task.to_string());
        while let Some(start) = path.find('{') {
            let end = path[start..].find('}').unwrap() + start;
            path.replace_range(start..=end, &Id::new_v7().to_string());
        }
        for (method, operation) in item.as_object().unwrap() {
            out.push((
                method.clone(),
                path.clone(),
                operation["operationId"].as_str().unwrap().to_owned(),
            ));
        }
    }
    out
}

#[tokio::test]
async fn every_tool_has_one_scope_and_true_annotations() {
    let f = Fixture::new().await;
    let matrix = tool_matrix();
    let every = f.everything().await;
    let listed = f.tool_list(&every).await;
    assert_eq!(
        listed
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect::<BTreeSet<_>>(),
        matrix.iter().map(|(name, _, _)| *name).collect(),
    );
    assert_eq!(listed.len(), matrix.len(), "no tool is listed twice");
    for tool in &listed {
        let name = tool["name"].as_str().unwrap();
        let kind = matrix.iter().find(|(n, _, _)| *n == name).unwrap().2;
        assert_eq!(
            tool["annotations"]["readOnlyHint"],
            json!(kind == "read"),
            "{name}"
        );
        assert_eq!(
            tool["annotations"]["destructiveHint"],
            json!(kind == "destroy"),
            "{name}"
        );
        assert_eq!(tool["annotations"]["openWorldHint"], json!(false), "{name}");
    }
    // Exactly the tools of the scopes, and a call outside them is refused before its arguments.
    for scopes in [
        vec![TasksRead],
        vec![TasksWrite],
        vec![TasksWrite, TasksManage],
        vec![DocsRead],
        vec![DocsWrite],
        vec![DocsWrite, DocsManage],
        vec![ChatRead],
    ] {
        let token = f.limited(&scopes).await;
        let may = |scope: ApiTokenScope| {
            scopes.contains(&scope)
                || (scope == TasksRead && scopes.contains(&TasksWrite))
                || (scope == DocsRead && scopes.contains(&DocsWrite))
        };
        let expected: BTreeSet<String> = matrix
            .iter()
            .filter(|(_, scope, _)| scope.is_none_or(may))
            .map(|(name, _, _)| (*name).to_owned())
            .collect();
        assert_eq!(f.tool_names(&token).await, expected, "{scopes:?}");
        for (name, scope, _) in &matrix {
            if scope.is_some_and(|scope| !may(scope)) {
                let text = f.refused(&token, name, json!({})).await;
                assert!(text.contains("lacks the required permission"), "{text}");
            }
        }
    }
    // Deletes for ever, members, sharing and teamspace changes are no tools at all.
    for gone in [
        "purge_doc",
        "empty_doc_trash",
        "empty_task_trash",
        "purge_project",
        "add_project_member",
        "remove_project_member",
        "update_teamspace",
        "delete_teamspace",
    ] {
        f.refused(&every, gone, json!({})).await;
    }
}

#[tokio::test]
async fn every_rest_operation_needs_its_scope_and_a_live_person() {
    let f = Fixture::new().await;
    let matrix = rest_matrix();
    let operations = public_operations(&f);
    let names: BTreeSet<&str> = operations
        .iter()
        .map(|(_, _, operation)| operation.as_str())
        .collect();
    let mut expected: BTreeSet<&str> = matrix.keys().copied().collect();
    expected.remove("public_get_me");
    assert_eq!(names, expected);

    let other = Id::new_v7().to_string();
    let admin = f.person("admin", "admin").await;
    let admin_token = f.token_by(admin, TokenProjects::All, &ALL).await;
    let every = f.everything().await;
    let mut without = BTreeMap::new();
    for scope in ALL {
        // Everything but `scope` (and what would include it).
        let rest: Vec<ApiTokenScope> = ALL
            .into_iter()
            .filter(|s| match scope {
                TasksRead => !matches!(s, TasksRead | TasksWrite | TasksManage),
                TasksWrite => !matches!(s, TasksWrite | TasksManage),
                DocsRead => !matches!(s, DocsRead | DocsWrite | DocsManage),
                DocsWrite => !matches!(s, DocsWrite | DocsManage),
                other => *s != other,
            })
            .collect();
        without.insert(scope, f.token_by(f.owner, TokenProjects::All, &rest).await);
    }
    let call = |token: String, method: String, path: String| {
        let f = &f;
        async move {
            let body = (method != "get").then(|| ("application/json", b"{}".to_vec()));
            let (status, bytes, _) = f.rest_full(&token, &method, &path, body, &[]).await;
            let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
            (status, body["code"].as_str().unwrap_or_default().to_owned())
        }
    };
    for (method, path, operation) in &operations {
        let scope = matrix[operation.as_str()].unwrap();
        let (status, code) = call(without[&scope].clone(), method.clone(), path.clone()).await;
        assert_eq!(
            (status, code.as_str()),
            (StatusCode::FORBIDDEN, "insufficient_scope"),
            "{operation}"
        );
        // Another workspace's path answers as an unknown one.
        let body = (method != "get").then(|| Body::from("{}"));
        let response = f
            .rest
            .clone()
            .oneshot(
                Request::builder()
                    .method(method.to_ascii_uppercase().as_str())
                    .uri(format!("/api/public/v1/workspaces/{other}{path}"))
                    .header(header::AUTHORIZATION, format!("Bearer {every}"))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(body.unwrap_or(Body::empty()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{operation}");
    }
    // A guest's, a suspended member's and a removed member's token reaches nothing.
    let refused_everywhere = async |why: &str| {
        for (method, path, operation) in &operations {
            let (status, _) = call(admin_token.clone(), method.clone(), path.clone()).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED, "{why}: {operation}");
        }
        let (status, _) = f.rpc(&admin_token, "tools/list", json!({})).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{why}");
    };
    f.set_role(admin, "guest").await;
    refused_everywhere("guest").await;
    f.set_role(admin, "admin").await;
    let (status, _) = f.rest(&admin_token, "get", "/projects", None).await;
    assert_eq!(status, StatusCode::OK);
    sqlx::query("UPDATE users SET suspended_at = 1 WHERE id = $1")
        .bind(admin.to_string())
        .execute(f.database.pool())
        .await
        .unwrap();
    refused_everywhere("suspended").await;
    sqlx::query("UPDATE users SET suspended_at = NULL WHERE id = $1")
        .bind(admin.to_string())
        .execute(f.database.pool())
        .await
        .unwrap();
    sqlx::query("DELETE FROM memberships WHERE user_id = $1")
        .bind(admin.to_string())
        .execute(f.database.pool())
        .await
        .unwrap();
    refused_everywhere("removed").await;
}

#[tokio::test]
async fn manage_scopes_come_with_write_and_never_with_a_service_account() {
    let f = Fixture::new().await;
    for scopes in [
        vec![TasksManage],
        vec![TasksRead, TasksManage],
        vec![DocsManage],
        vec![DocsRead, DocsManage],
        vec![TasksWrite, DocsManage],
    ] {
        let refused = f
            .tokens
            .create(
                f.workspace,
                f.owner,
                "no".into(),
                TokenProjects::All,
                &scopes,
                false,
                None,
                "test",
                TimestampMillis::now(),
            )
            .await;
        assert!(
            matches!(refused, Err(ApiTokenError::NotFound)),
            "{scopes:?}"
        );
    }
    for scopes in [vec![TasksWrite, TasksManage], vec![TasksWrite, DocsManage]] {
        assert!(f.service_account(&scopes).await.is_err(), "{scopes:?}");
    }
    // A service account has no inbox, views, comment edits or uploads: those are a person's.
    let account = f.service_account(&[TasksWrite]).await.unwrap();
    let comment = f
        .tool(
            &account,
            "create_comment",
            json!({"task_id": f.task, "body": "Hi"}),
        )
        .await;
    for (name, arguments) in [
        ("list_notifications", json!({})),
        ("mark_all_notifications_read", json!({})),
        ("list_views", json!({})),
        (
            "update_comment",
            json!({"task_id": f.task, "comment_id": comment["id"], "expected_version": comment["version"], "body": "x"}),
        ),
        (
            "upload_task_attachment",
            json!({"task_id": f.task, "filename": "a.txt", "content_base64": "YQ=="}),
        ),
    ] {
        f.refused(&account, name, arguments).await;
    }
    // Its plan writes are its own in the audit log, not the owner's.
    let milestone = f
        .tool(
            &account,
            "create_milestone",
            json!({"project_id": f.project, "name": "By a service"}),
        )
        .await;
    let (actor, metadata): (Option<String>, String) =
        sqlx::query_as("SELECT actor_id, metadata_json FROM audit_events WHERE resource_id = $1")
            .bind(milestone["id"].as_str().unwrap())
            .fetch_one(f.database.pool())
            .await
            .unwrap();
    assert_eq!(actor, None);
    assert!(metadata.contains("actor_service_account_id"), "{metadata}");
}

#[tokio::test]
async fn a_limited_token_stays_inside_its_projects() {
    let f = Fixture::new().await;
    let limited = f.limited(&ALL).await;
    let every = f.everything().await;
    let (project, secret) = (f.project.to_string(), f.secret.to_string());
    let task = f
        .tool(&limited, "get_task", json!({"task_id": f.task}))
        .await;
    let hushed = f
        .tool(&every, "get_task", json!({"task_id": f.secret_task}))
        .await;
    let secret_status = f.status(f.secret).await.to_string();

    // Records of the secret project, made with the owner's full token.
    let label = f
        .tool(
            &every,
            "create_label",
            json!({"project_id": secret, "name": "Hush label"}),
        )
        .await;
    let milestone = f
        .tool(
            &every,
            "create_milestone",
            json!({"project_id": secret, "name": "Hush milestone"}),
        )
        .await;
    let cycle = f
        .tool(
            &every,
            "create_cycle",
            json!({"project_id": secret, "name": "Hush cycle"}),
        )
        .await;
    let comment = f
        .tool(
            &every,
            "create_comment",
            json!({"task_id": f.secret_task, "body": "Hush comment"}),
        )
        .await;
    let file = f
        .tool(
            &every,
            "upload_task_attachment",
            json!({"task_id": f.secret_task, "filename": "Hush.txt", "content_base64": STANDARD.encode("Hush")}),
        )
        .await;
    let before = f
        .tool(
            &every,
            "list_task_activity",
            json!({"task_id": f.secret_task}),
        )
        .await;

    // Every tool that names something of the secret project, directly or next to an approved one.
    for (name, arguments) in [
        ("list_task_relations", json!({"task_id": f.secret_task})),
        ("list_task_activity", json!({"task_id": f.secret_task})),
        ("list_task_github_links", json!({"task_id": "HSH-1"})),
        ("list_task_attachments", json!({"task_id": f.secret_task})),
        (
            "download_task_attachment",
            json!({"task_id": f.secret_task, "attachment_id": file["id"]}),
        ),
        // The file's id under an approved task is no way in.
        (
            "download_task_attachment",
            json!({"task_id": f.task, "attachment_id": file["id"]}),
        ),
        (
            "delete_task_attachment",
            json!({"task_id": f.task, "attachment_id": file["id"]}),
        ),
        (
            "upload_task_attachment",
            json!({"task_id": f.secret_task, "filename": "a.txt", "content_base64": "YQ=="}),
        ),
        (
            "upload_task_attachment",
            json!({"task_id": f.task, "comment_id": comment["id"], "filename": "a.txt", "content_base64": "YQ=="}),
        ),
        (
            "update_comment",
            json!({"task_id": f.task, "comment_id": comment["id"], "expected_version": 0, "body": "x"}),
        ),
        (
            "delete_comment",
            json!({"task_id": f.secret_task, "comment_id": comment["id"], "expected_version": 0}),
        ),
        (
            "delete_comment",
            json!({"task_id": f.task, "comment_id": comment["id"], "expected_version": 0}),
        ),
        (
            "create_task_relation",
            json!({"task_id": f.task, "type": "related", "other_task_id": f.secret_task}),
        ),
        (
            "create_task_relation",
            json!({"task_id": f.secret_task, "type": "blocks", "other_task_id": f.task}),
        ),
        (
            "create_milestone",
            json!({"project_id": secret, "name": "x"}),
        ),
        (
            "update_milestone",
            json!({"project_id": project, "milestone_id": milestone["id"], "expected_version": milestone["version"], "name": "x"}),
        ),
        (
            "delete_milestone",
            json!({"project_id": secret, "milestone_id": milestone["id"], "expected_version": milestone["version"]}),
        ),
        (
            "reorder_milestones",
            json!({"project_id": project, "items": [{"id": milestone["id"], "expected_version": milestone["version"], "position": 1}]}),
        ),
        ("create_cycle", json!({"project_id": secret})),
        (
            "update_cycle",
            json!({"project_id": project, "cycle_id": cycle["id"], "expected_version": cycle["version"], "name": "x"}),
        ),
        (
            "delete_cycle",
            json!({"project_id": project, "cycle_id": cycle["id"], "expected_version": cycle["version"]}),
        ),
        (
            "move_unfinished_cycle_tasks",
            json!({"project_id": project, "cycle_id": cycle["id"]}),
        ),
        ("create_label", json!({"project_id": secret, "name": "x"})),
        (
            "update_label",
            json!({"project_id": project, "label_id": label["id"], "expected_version": label["version"], "name": "x"}),
        ),
        (
            "delete_label",
            json!({"project_id": project, "label_id": label["id"], "expected_version": label["version"]}),
        ),
        (
            "create_task_status",
            json!({"project_id": secret, "name": "x", "category": "started"}),
        ),
        (
            "update_task_status",
            json!({"project_id": project, "status_id": secret_status, "expected_version": 0, "name": "x"}),
        ),
        (
            "delete_task_status",
            json!({"project_id": project, "status_id": secret_status, "expected_version": 0}),
        ),
        (
            "reorder_task_statuses",
            json!({"project_id": project, "items": [{"id": secret_status, "expected_version": 0, "position": 1}]}),
        ),
        (
            "update_project",
            json!({"project_id": secret, "expected_version": 0, "name": "x"}),
        ),
        (
            "delete_project",
            json!({"project_id": secret, "expected_version": 0}),
        ),
        ("list_project_trash", json!({})),
        ("get_trashed_project", json!({"project_id": secret})),
        (
            "restore_project",
            json!({"project_id": secret, "expected_version": 0}),
        ),
        (
            "bulk_update_tasks",
            json!({"updates": [
                {"task_id": f.task, "expected_version": task["version"], "title": "Changed"},
                {"task_id": f.secret_task, "expected_version": hushed["version"], "title": "Changed"},
            ]}),
        ),
        // A value of the secret project on an approved task.
        (
            "bulk_update_tasks",
            json!({"updates": [{"task_id": f.task, "expected_version": task["version"], "title": "Changed", "milestone_id": milestone["id"]}]}),
        ),
        (
            "bulk_update_tasks",
            json!({"updates": [{"task_id": f.task, "expected_version": task["version"], "title": "Changed", "label_ids": [label["id"]]}]}),
        ),
        (
            "bulk_update_tasks",
            json!({"updates": [{"task_id": f.task, "expected_version": task["version"], "title": "Changed", "status_id": secret_status}]}),
        ),
        (
            "bulk_update_tasks",
            json!({"updates": [{"task_id": f.task, "expected_version": task["version"], "title": "Changed", "parent_task_id": f.secret_task}]}),
        ),
        (
            "reorder_tasks",
            json!({"items": [
                {"id": f.task, "expected_version": task["version"], "position": 9},
                {"id": "HSH-1", "expected_version": hushed["version"], "position": 9},
            ]}),
        ),
        (
            "create_view",
            json!({"name": "v", "visibility": "personal", "state": view(Some(&secret))}),
        ),
        // Shared by all members, so not for a credential of chosen projects.
        (
            "create_view",
            json!({"name": "v", "visibility": "workspace", "state": view(Some(&project))}),
        ),
    ] {
        let text = f.refused(&limited, name, arguments).await;
        f.assert_hushed(&format!("refused {name}"), &text);
    }
    // Nothing was written, in either project.
    let same = f
        .tool(&limited, "get_task", json!({"task_id": f.task}))
        .await;
    assert_eq!(same, task);
    let same = f
        .tool(&every, "get_task", json!({"task_id": f.secret_task}))
        .await;
    assert_eq!(same, hushed);
    let after = f
        .tool(
            &every,
            "list_task_activity",
            json!({"task_id": f.secret_task}),
        )
        .await;
    assert_eq!(after, before);

    // Lists leave the secret project out.
    f.tool(
        &every,
        "create_task_relation",
        json!({"task_id": f.task, "type": "blocks", "other_task_id": f.secret_task}),
    )
    .await;
    f.tool(
        &every,
        "create_view",
        json!({"name": "Hush view", "visibility": "workspace", "state": view(Some(&secret))}),
    )
    .await;
    for (name, arguments) in [
        ("list_task_relations", json!({"task_id": f.task})),
        ("list_task_activity", json!({"task_id": f.task})),
        ("list_views", json!({})),
        ("list_task_trash", json!({})),
        ("list_notifications", json!({})),
    ] {
        let text = f.tool(&limited, name, arguments).await.to_string();
        f.assert_hushed(name, &text);
    }
    // The full token sees the relation it made, so the list above is a real filter.
    let seen = f
        .tool(&every, "list_task_relations", json!({"task_id": f.task}))
        .await;
    assert_eq!(seen["items"].as_array().unwrap().len(), 1, "{seen}");
    let relation = seen["items"][0]["id"].clone();
    f.refused(
        &limited,
        "delete_task_relation",
        json!({"task_id": f.task, "relation_id": relation}),
    )
    .await;

    // The same over REST: 404, or the refusal that names no project.
    for (method, path, body) in [
        ("get", format!("/tasks/{}/activity", f.secret_task), None),
        ("get", format!("/tasks/{}/attachments", f.secret_task), None),
        (
            "delete",
            format!("/projects/{secret}?expected_version=0"),
            None,
        ),
        (
            "post",
            format!("/projects/{secret}/milestones"),
            Some(json!({"name": "x"})),
        ),
        (
            "delete",
            format!(
                "/projects/{project}/milestones/{}?expected_version=0",
                milestone["id"].as_str().unwrap()
            ),
            None,
        ),
        (
            "post",
            "/tasks/bulk".to_owned(),
            Some(json!({"updates": [
                {"task": f.task, "expected_version": task["version"], "title": "Changed"},
                {"task": "HSH-1", "expected_version": hushed["version"], "title": "Changed"},
            ]})),
        ),
    ] {
        let (status, body) = f.rest(&limited, method, &path, body).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{method} {path}: {body}");
    }
}

/// A saved view's state, with a project condition when `project` is set.
fn view(project: Option<&str>) -> Value {
    let children = match project {
        Some(id) => json!([{"field": "project", "operator": "is", "value": [id]}]),
        None => json!([]),
    };
    json!({
        "filter": {"op": "and", "children": children},
        "display": {
            "layout": "list",
            "group_by": "status",
            "sub_group_by": "none",
            "order_by": "manual",
            "order_direction": "asc",
            "properties": ["id", "status"],
            "show_completed": "all",
            "show_empty_groups": false,
            "sub_issues": "nested"
        }
    })
}

#[tokio::test]
async fn trash_and_restore_keep_to_whole_approved_trees() {
    let f = Fixture::new().await;
    let limited = f.limited(&ALL).await;
    let every = f.everything().await;
    let child = f.new_task(f.project, "Child", Some(f.task)).await;
    let task = f
        .tool(&limited, "get_task", json!({"task_id": f.task}))
        .await;
    f.tool(
        &limited,
        "delete_task",
        json!({"task_id": f.task, "expected_version": task["version"]}),
    )
    .await;
    let hushed = f
        .tool(&every, "get_task", json!({"task_id": f.secret_task}))
        .await;
    f.tool(
        &every,
        "delete_task",
        json!({"task_id": f.secret_task, "expected_version": hushed["version"]}),
    )
    .await;
    let trash = f.tool(&limited, "list_task_trash", json!({})).await;
    f.assert_hushed("list_task_trash", &trash.to_string());
    let all = f.tool(&every, "list_task_trash", json!({})).await;
    assert!(all.to_string().contains("Hush task"), "{all}");
    // The hidden task was trashed last: it takes no place on the page, and the cursor names none.
    let first = f
        .tool(&limited, "list_task_trash", json!({"limit": 1}))
        .await;
    assert_eq!(first["items"][0]["project_id"], json!(f.project), "{first}");
    assert_eq!(all["items"][0]["id"], json!(f.secret_task), "{all}");
    f.refused(&limited, "get_trashed_task", json!({"task_id": "HSH-1"}))
        .await;
    f.refused(
        &limited,
        "restore_task",
        json!({"task_id": f.secret_task, "expected_version": 1}),
    )
    .await;
    // A stale version changes nothing; the right one brings the sub-issue back too.
    let trashed = f
        .tool(&limited, "get_trashed_task", json!({"task_id": f.task}))
        .await;
    assert_eq!(trashed["deleted_by"], json!(f.owner));
    let version = trashed["version"].as_u64().unwrap();
    f.refused(
        &limited,
        "restore_task",
        json!({"task_id": f.task, "expected_version": version + 1}),
    )
    .await;
    f.tool(
        &limited,
        "restore_task",
        json!({"task_id": f.task, "expected_version": version}),
    )
    .await;
    f.tool(&limited, "get_task", json!({"task_id": child}))
        .await;

    // A tree that reaches into the secret project: not trashed, not restored, not bulk-changed.
    f.new_task(f.secret, "Hush child", Some(child)).await;
    let child_version = f
        .tool(&limited, "get_task", json!({"task_id": child}))
        .await["version"]
        .clone();
    for (name, arguments) in [
        (
            "bulk_update_tasks",
            json!({"updates": [{"task_id": child, "expected_version": child_version, "title": "x"}]}),
        ),
        (
            "reorder_tasks",
            json!({"items": [{"id": child, "expected_version": child_version, "position": 3}]}),
        ),
    ] {
        let text = f.refused(&limited, name, arguments).await;
        f.assert_hushed(&format!("refused {name}"), &text);
    }
    let child_task = f.tool(&every, "get_task", json!({"task_id": child})).await;
    f.tool(
        &every,
        "delete_task",
        json!({"task_id": child, "expected_version": child_task["version"]}),
    )
    .await;
    let trashed = f
        .tool(&limited, "get_trashed_task", json!({"task_id": child}))
        .await;
    f.refused(
        &limited,
        "restore_task",
        json!({"task_id": child, "expected_version": trashed["version"]}),
    )
    .await;
    f.tool(
        &every,
        "restore_task",
        json!({"task_id": child, "expected_version": trashed["version"]}),
    )
    .await;

    // Projects: trash and restore with a token for all projects only.
    let projects = f.tool(&every, "list_projects", json!({})).await;
    let secret = projects["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|project| project["id"] == json!(f.secret))
        .unwrap()
        .clone();
    f.tool(
        &every,
        "delete_project",
        json!({"project_id": f.secret, "expected_version": secret["version"]}),
    )
    .await;
    let trash = f.tool(&every, "list_project_trash", json!({})).await;
    assert_eq!(trash["items"][0]["id"], json!(f.secret), "{trash}");
    for (name, arguments) in [
        ("list_project_trash", json!({})),
        ("get_trashed_project", json!({"project_id": f.secret})),
        (
            "restore_project",
            json!({"project_id": f.secret, "expected_version": trash["items"][0]["version"]}),
        ),
    ] {
        let text = f.refused(&limited, name, arguments).await;
        f.assert_hushed(&format!("refused {name}"), &text);
    }
    f.tool(
        &every,
        "restore_project",
        json!({"project_id": f.secret, "expected_version": trash["items"][0]["version"]}),
    )
    .await;
}

#[tokio::test]
async fn a_private_project_closes_when_its_person_no_longer_sees_it() {
    let f = Fixture::new().await;
    let admin = f.person("admin", "admin").await;
    let token = f.token_by(admin, vec![f.project, f.secret], &ALL).await;
    f.tool(
        &token,
        "list_task_activity",
        json!({"task_id": f.secret_task}),
    )
    .await;
    // A member off the private project's list: gone at the next request.
    f.set_role(admin, "member").await;
    for name in [
        "list_task_activity",
        "list_task_relations",
        "list_task_attachments",
    ] {
        let text = f
            .refused(&token, name, json!({"task_id": f.secret_task}))
            .await;
        f.assert_hushed("refused", &text);
    }
    let task = f.tool(&token, "get_task", json!({"task_id": f.task})).await;
    f.refused(
        &token,
        "bulk_update_tasks",
        json!({"updates": [
            {"task_id": f.task, "expected_version": task["version"], "title": "x"},
            {"task_id": f.secret_task, "expected_version": 0, "title": "x"},
        ]}),
    )
    .await;
    f.refused(
        &token,
        "create_milestone",
        json!({"project_id": f.secret, "name": "x"}),
    )
    .await;
}

#[tokio::test]
async fn doc_tools_never_reach_another_members_private_doc() {
    let f = Fixture::new().await;
    let admin = f.person("admin", "admin").await;
    let token = f.token_by(admin, TokenProjects::All, &ALL).await;
    let mine = f.doc(admin, "Mine", None, false).await;
    let theirs = f.doc(f.owner, "Hush doc", None, true).await;
    let version = f.doc_version(theirs).await;
    let owner_token = f.everything().await;
    let thread = f
        .tool(
            &owner_token,
            "create_doc_comment",
            json!({"page_id": theirs, "quote": "Body", "body": "Hush remark"}),
        )
        .await;
    let file = f
        .tool(
            &owner_token,
            "upload_doc_file",
            json!({"page_id": theirs, "filename": "Hush.txt", "content_base64": STANDARD.encode("Hush")}),
        )
        .await;
    for (name, arguments) in [
        ("list_doc_versions", json!({"page_id": theirs})),
        (
            "get_doc_version",
            json!({"page_id": theirs, "version_id": Id::new_v7()}),
        ),
        (
            "restore_doc_version",
            json!({"page_id": theirs, "version_id": Id::new_v7(), "expected_version": version}),
        ),
        ("list_doc_comments", json!({"page_id": theirs})),
        (
            "create_doc_comment",
            json!({"page_id": theirs, "quote": "Body", "body": "x"}),
        ),
        (
            "update_doc_comment",
            json!({"page_id": theirs, "thread_id": thread["thread_id"], "comment_id": thread["id"], "body": "x"}),
        ),
        // The thread's ids under a doc of one's own are no way in.
        (
            "update_doc_comment",
            json!({"page_id": mine, "thread_id": thread["thread_id"], "comment_id": thread["id"], "body": "x"}),
        ),
        (
            "delete_doc_comment",
            json!({"page_id": mine, "thread_id": thread["thread_id"], "comment_id": thread["id"]}),
        ),
        (
            "resolve_doc_thread",
            json!({"page_id": mine, "thread_id": thread["thread_id"]}),
        ),
        (
            "delete_doc_thread",
            json!({"page_id": mine, "thread_id": thread["thread_id"]}),
        ),
        ("duplicate_doc", json!({"page_id": theirs})),
        ("set_doc_lock", json!({"page_id": theirs, "locked": true})),
        ("add_favorite_doc", json!({"page_id": theirs})),
        (
            "delete_doc",
            json!({"page_id": theirs, "expected_version": version}),
        ),
        (
            "move_doc",
            json!({"page_id": theirs, "expected_version": version, "dry_run": true}),
        ),
        (
            "move_doc",
            json!({"page_id": mine, "expected_version": f.doc_version(mine).await, "parent_id": theirs, "dry_run": true}),
        ),
        (
            "download_doc_file",
            json!({"page_id": theirs, "file_id": file["id"]}),
        ),
        (
            "download_doc_file",
            json!({"page_id": mine, "file_id": file["id"]}),
        ),
        (
            "upload_doc_file",
            json!({"page_id": theirs, "filename": "a.txt", "content_base64": "YQ=="}),
        ),
    ] {
        let text = f.refused(&token, name, arguments).await;
        assert!(!text.contains("Hush"), "{name}: {text}");
    }
    let found = f
        .tool(&token, "search_docs", json!({"query": "Hush"}))
        .await;
    assert_eq!(found["items"], json!([]), "{found}");
    // In the trash it stays the owner's.
    f.tool(
        &owner_token,
        "delete_doc",
        json!({"page_id": theirs, "expected_version": f.doc_version(theirs).await}),
    )
    .await;
    let trash = f.tool(&token, "list_doc_trash", json!({})).await;
    assert!(!trash.to_string().contains("Hush"), "{trash}");
    let own_trash = f.tool(&owner_token, "list_doc_trash", json!({})).await;
    let trashed = own_trash["items"][0].clone();
    f.refused(
        &token,
        "restore_doc",
        json!({"page_id": theirs, "expected_version": trashed["version"]}),
    )
    .await;
    f.tool(
        &owner_token,
        "restore_doc",
        json!({"page_id": theirs, "expected_version": trashed["version"]}),
    )
    .await;
}

#[tokio::test]
async fn a_dry_run_moves_nothing_and_a_restore_names_its_sub_docs() {
    let f = Fixture::new().await;
    let token = f.everything().await;
    let parent = f.doc(f.owner, "Parent", None, false).await;
    let child = f.doc(f.owner, "Child", Some(parent), false).await;
    let grandchild = f.doc(f.owner, "Grandchild", Some(child), false).await;
    let target = f.doc(f.owner, "Target", None, false).await;
    let version = f.doc_version(child).await;
    let audit = "SELECT COUNT(*) FROM audit_events WHERE resource_id = $1";
    let events = f.count(audit, &child.to_string()).await;

    for (dry, answer) in [
        (
            true,
            f.tool(
                &token,
                "move_doc",
                json!({"page_id": child, "expected_version": version, "parent_id": target, "dry_run": true}),
            )
            .await,
        ),
        (
            true,
            f.rest(
                &token,
                "post",
                &format!("/docs/{child}/move"),
                Some(json!({"expected_version": version, "parent_id": target, "dry_run": true})),
            )
            .await
            .1,
        ),
    ] {
        assert!(dry);
        assert_eq!(answer["parent_id"], json!(target), "{answer}");
        assert_eq!(answer["version"], json!(version), "{answer}");
    }
    assert_eq!(f.doc_version(child).await, version);
    assert_eq!(f.count(audit, &child.to_string()).await, events);
    let parent_of = "SELECT COUNT(*) FROM pages WHERE parent_id = $1";
    assert_eq!(f.count(parent_of, &parent.to_string()).await, 1);
    // Its checks are the real ones.
    f.refused(
        &token,
        "move_doc",
        json!({"page_id": parent, "expected_version": f.doc_version(parent).await, "parent_id": grandchild, "dry_run": true}),
    )
    .await;
    f.refused(
        &token,
        "move_doc",
        json!({"page_id": child, "expected_version": version + 1, "parent_id": target, "dry_run": true}),
    )
    .await;
    // The same body without `dry_run` moves it.
    let moved = f
        .tool(
            &token,
            "move_doc",
            json!({"page_id": child, "expected_version": version, "parent_id": target}),
        )
        .await;
    assert_eq!(moved["parent_id"], json!(target));
    assert_eq!(f.count(parent_of, &parent.to_string()).await, 0);

    // A stored version comes back with the version that was read, and the edit is kept as one too.
    sqlx::query("UPDATE pages SET created_at = created_at - 3600000 WHERE id = $1")
        .bind(target.to_string())
        .execute(f.database.pool())
        .await
        .unwrap();
    f.tool(
        &token,
        "update_doc",
        json!({"page_id": target, "expected_version": f.doc_version(target).await, "markdown": "Edited\n"}),
    )
    .await;
    let versions = f
        .tool(&token, "list_doc_versions", json!({"page_id": target}))
        .await;
    let stored = versions["items"][0]["id"].clone();
    f.refused(
        &token,
        "restore_doc_version",
        json!({"page_id": target, "version_id": stored, "expected_version": 1}),
    )
    .await;
    let back = f
        .tool(
            &token,
            "restore_doc_version",
            json!({"page_id": target, "version_id": stored, "expected_version": f.doc_version(target).await}),
        )
        .await;
    assert!(
        back["markdown"].as_str().unwrap().contains("Body text"),
        "{back}"
    );
    f.refused(
        &token,
        "get_doc_version",
        json!({"page_id": parent, "version_id": stored}),
    )
    .await;

    // Trash the target with both sub-docs, then restore it.
    f.tool(
        &token,
        "delete_doc",
        json!({"page_id": target, "expected_version": f.doc_version(target).await}),
    )
    .await;
    let trash = f.tool(&token, "list_doc_trash", json!({})).await;
    assert_eq!(trash["items"].as_array().unwrap().len(), 1, "{trash}");
    let (status, restored) = f
        .rest(
            &token,
            "post",
            &format!("/docs/{target}/restore"),
            Some(json!({"expected_version": trash["items"][0]["version"]})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{restored}");
    let mut ids: Vec<&str> = restored["restored_descendant_ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|id| id.as_str().unwrap())
        .collect();
    ids.sort_unstable();
    let mut expected = [child.to_string(), grandchild.to_string()];
    expected.sort_unstable();
    assert_eq!(ids, expected);
}

#[tokio::test]
async fn comments_are_edited_by_their_author_and_docs_comments_too() {
    let f = Fixture::new().await;
    let admin = f.person("admin", "admin").await;
    let owner = f.everything().await;
    let other = f.token_by(admin, TokenProjects::All, &ALL).await;
    let comment = f
        .tool(
            &owner,
            "create_comment",
            json!({"task_id": f.task, "body": "First"}),
        )
        .await;
    f.refused(
        &other,
        "update_comment",
        json!({"task_id": f.task, "comment_id": comment["id"], "expected_version": comment["version"], "body": "Mine now"}),
    )
    .await;
    let (status, problem) = f
        .rest(
            &owner,
            "patch",
            &format!(
                "/tasks/{}/comments/{}",
                f.task,
                comment["id"].as_str().unwrap()
            ),
            Some(json!({"expected_version": 99, "body": "Stale"})),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{problem}");
    let edited = f
        .tool(
            &owner,
            "update_comment",
            json!({"task_id": f.task, "comment_id": comment["id"], "expected_version": comment["version"], "body": "Second"}),
        )
        .await;
    assert_eq!(edited["body"], "Second");
    // A write token cannot delete it; a manage token of its author can.
    let write = f.limited(&[TasksWrite]).await;
    let delete = json!({"task_id": f.task, "comment_id": comment["id"], "expected_version": edited["version"]});
    f.refused(&write, "delete_comment", delete.clone()).await;
    f.tool(&owner, "delete_comment", delete).await;

    let page = f.doc(f.owner, "Doc", None, false).await;
    let first = f
        .tool(
            &owner,
            "create_doc_comment",
            json!({"page_id": page, "quote": "Body text", "body": "**Why?**\n\n- a\n- b"}),
        )
        .await;
    let reply = f
        .tool(
            &other,
            "create_doc_comment",
            json!({"page_id": page, "thread_id": first["thread_id"], "body": "Because"}),
        )
        .await;
    for (token, id) in [(&other, &first), (&owner, &reply)] {
        f.refused(
            token,
            "update_doc_comment",
            json!({"page_id": page, "thread_id": first["thread_id"], "comment_id": id["id"], "body": "x"}),
        )
        .await;
        f.refused(
            token,
            "delete_doc_comment",
            json!({"page_id": page, "thread_id": first["thread_id"], "comment_id": id["id"]}),
        )
        .await;
    }
    let threads = f
        .tool(&owner, "list_doc_comments", json!({"page_id": page}))
        .await;
    assert_eq!(
        threads["threads"][0]["comments"].as_array().unwrap().len(),
        2,
        "{threads}"
    );
    let resolved = f
        .tool(
            &other,
            "resolve_doc_thread",
            json!({"page_id": page, "thread_id": first["thread_id"]}),
        )
        .await;
    assert_eq!(resolved["resolved"], true);
}

#[tokio::test]
async fn notifications_and_activity_show_only_what_the_token_may_read() {
    let f = Fixture::new().await;
    let admin = f.person("admin", "admin").await;
    let page = f.doc(f.owner, "Hush plan", None, false).await;
    let now = TimestampMillis::now().as_millis();
    for (recipient, kind, task, page) in [
        (f.owner, "task_assigned", Some(f.task), None),
        (f.owner, "task_assigned", Some(f.secret_task), None),
        (f.owner, "page_mentioned", None, Some(page)),
        (admin, "task_assigned", Some(f.task), None),
    ] {
        sqlx::query(
            "INSERT INTO notifications (id, workspace_id, recipient_user_id, actor_user_id, kind, task_id, \
             page_id, dedupe_key, created_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        )
        .bind(Id::new_v7().to_string())
        .bind(f.workspace.to_string())
        .bind(recipient.to_string())
        .bind(admin.to_string())
        .bind(kind)
        .bind(task.map(|id| id.to_string()))
        .bind(page.map(|id| id.to_string()))
        .bind(Id::new_v7().to_string())
        .bind(now)
        .execute(f.database.pool())
        .await
        .unwrap();
    }
    let tasks_only = f.limited(&[TasksWrite]).await;
    let listed = f.tool(&tasks_only, "list_notifications", json!({})).await;
    let items = listed["items"].as_array().unwrap();
    assert_eq!(items.len(), 1, "{listed}");
    assert_eq!(items[0]["task_id"], json!(f.task));
    // "Mark all" touches only what the token lists, and nobody else's.
    let marked = f
        .tool(&tasks_only, "mark_all_notifications_read", json!({}))
        .await;
    assert_eq!(marked["updated"], 1);
    let unread = "SELECT COUNT(*) FROM notifications WHERE read_at IS NULL AND workspace_id = $1";
    assert_eq!(f.count(unread, &f.workspace.to_string()).await, 3);
    let every = f.everything().await;
    let all = f.tool(&every, "list_notifications", json!({})).await;
    assert_eq!(all["items"].as_array().unwrap().len(), 3, "{all}");
    let hidden = all["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["task_id"] == json!(f.secret_task))
        .unwrap()["id"]
        .clone();
    f.refused(
        &tasks_only,
        "mark_notification_read",
        json!({"notification_id": hidden}),
    )
    .await;
    let admins: String =
        sqlx::query_scalar("SELECT id FROM notifications WHERE recipient_user_id = $1")
            .bind(admin.to_string())
            .fetch_one(f.database.pool())
            .await
            .unwrap();
    f.refused(
        &every,
        "mark_notification_read",
        json!({"notification_id": admins}),
    )
    .await;

    // The activity of a task with a linked doc: the title needs docs access, also for the service
    // account, which acts as the owner.
    sqlx::query(
        "INSERT INTO audit_events (id, workspace_id, actor_id, action, outcome, resource_type, resource_id, \
         request_id, metadata_json, occurred_at) VALUES ($1, $2, $3, 'task.doc_linked', 'success', 'task', $4, 'test', $5, $6)",
    )
    .bind(Id::new_v7().to_string())
    .bind(f.workspace.to_string())
    .bind(f.owner.to_string())
    .bind(f.task.to_string())
    .bind(json!({"page_id": page, "api_token_id": "tok", "related_task_id": f.secret_task, "related_task_project_id": f.secret, "related_task_title": "Hush task"}).to_string())
    .bind(now)
    .execute(f.database.pool())
    .await
    .unwrap();
    let account = f.service_account(&[TasksRead]).await.unwrap();
    for token in [&tasks_only, &account] {
        let activity = f
            .tool(token, "list_task_activity", json!({"task_id": f.task}))
            .await
            .to_string();
        assert!(activity.contains("task.doc_linked"), "{activity}");
        assert!(!activity.contains(&page.to_string()), "{activity}");
        assert!(!activity.contains("api_token_id"), "{activity}");
        f.assert_hushed("activity", &activity);
    }
    let activity = f
        .tool(&every, "list_task_activity", json!({"task_id": f.task}))
        .await
        .to_string();
    assert!(activity.contains("Hush plan"), "{activity}");
}

#[tokio::test]
async fn files_are_checked_and_large_bodies_need_a_write_token() {
    let f = Fixture::new().await;
    let write = f.limited(&[TasksWrite]).await;
    // The type comes from the bytes, and the name is a name, never a path.
    for (name, bytes, media) in [
        (
            "notes.png",
            b"plain text".as_slice(),
            "application/octet-stream",
        ),
        (
            "../../etc/passwd",
            b"plain text".as_slice(),
            "application/octet-stream",
        ),
        ("a.html", b"<script>alert(1)</script>".as_slice(), ""),
    ] {
        let file = f
            .tool(
                &write,
                "upload_task_attachment",
                json!({"task_id": f.task, "filename": name, "content_base64": STANDARD.encode(bytes)}),
            )
            .await;
        if !media.is_empty() {
            assert!(
                file["media_type"].as_str().unwrap().starts_with(media),
                "{file}"
            );
        }
        let path = file["download_path"].as_str().unwrap();
        let path = path
            .split_once(&f.workspace.to_string())
            .unwrap()
            .1
            .to_owned();
        let (status, body, headers) = f.rest_full(&write, "get", &path, None, &[]).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, bytes);
        assert_eq!(headers["x-content-type-options"], "nosniff");
        let disposition = headers["content-disposition"].to_str().unwrap();
        assert!(!disposition.contains('/'), "{disposition}");
        if name == "a.html" {
            assert!(disposition.starts_with("attachment"), "{disposition}");
        }
    }
    for (name, content) in [
        ("a\u{0}b.txt", "YQ==".to_owned()),
        ("", "YQ==".to_owned()),
        ("a.txt", "not base64!".to_owned()),
        // More than 1 MiB decoded.
        ("big.bin", STANDARD.encode(vec![1_u8; 1024 * 1024 + 1])),
    ] {
        f.refused(
            &write,
            "upload_task_attachment",
            json!({"task_id": f.task, "filename": name, "content_base64": content}),
        )
        .await;
    }
    let mib = STANDARD.encode(vec![7_u8; 1024 * 1024]);
    let file = f
        .tool(
            &write,
            "upload_task_attachment",
            json!({"task_id": f.task, "filename": "big.bin", "content_base64": mib}),
        )
        .await;
    assert_eq!(file["byte_size"], 1024 * 1024);
    // Inline downloads stop at 64 KiB: the path stays.
    let download = f
        .tool(
            &write,
            "download_task_attachment",
            json!({"task_id": f.task, "attachment_id": file["id"]}),
        )
        .await;
    assert_eq!(download["content_base64"], Value::Null);
    assert_eq!(download["download_path"], file["download_path"]);

    // A read token, a chat token and a service account keep the small body; more than 1.5 MiB is
    // too much for everyone.
    let account = f.service_account(&[TasksWrite]).await.unwrap();
    for (token, size, expected) in [
        (f.limited(&[TasksRead]).await, 65_000, StatusCode::OK),
        (
            f.limited(&[TasksRead, DocsRead, ChatRead]).await,
            70_000,
            StatusCode::PAYLOAD_TOO_LARGE,
        ),
        (account, 70_000, StatusCode::PAYLOAD_TOO_LARGE),
        (write.clone(), 70_000, StatusCode::OK),
        (
            write.clone(),
            1536 * 1024 + 1,
            StatusCode::PAYLOAD_TOO_LARGE,
        ),
    ] {
        let (status, _) = f
            .rpc(
                &token,
                "tools/call",
                json!({"name": "get_me", "arguments": {}, "_meta": {"pad": "x".repeat(size)}}),
            )
            .await;
        assert_eq!(status, expected, "{size}");
    }
    // No valid credential: 401 whatever the size, so the body was not needed.
    let (status, _) = f
        .rpc(
            "orb_wrong",
            "tools/call",
            json!({"pad": "x".repeat(1 << 20)}),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    // Deep JSON is refused, not parsed.
    let deep = format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{{\"name\":\"get_me\",\"arguments\":{}{}}}}}",
        "[".repeat(5_000),
        "]".repeat(5_000)
    );
    let request = Request::builder()
        .method("POST")
        .uri("/mcp")
        .header(header::HOST, "orbit.test")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::CONTENT_LENGTH, deep.len())
        .header(header::ACCEPT, "application/json, text/event-stream")
        .header(header::AUTHORIZATION, format!("Bearer {write}"))
        .body(Body::from(deep))
        .unwrap();
    let response = f.mcp.clone().oneshot(request).await.unwrap();
    assert!(response.status().is_client_error(), "{}", response.status());

    // REST: the delete needs `tasks:manage`, and then the file is gone.
    let id = file["id"].as_str().unwrap();
    let path = format!("/tasks/{}/attachments/{id}", f.task);
    let (status, _) = f.rest(&write, "delete", &path, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let manage = f.limited(&[TasksWrite, TasksManage]).await;
    let (status, _) = f.rest(&manage, "delete", &path, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = f
        .rest(&manage, "get", &format!("{path}/download"), None)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn bulk_writes_are_all_or_none_and_plan_conflicts_answer_409() {
    let f = Fixture::new().await;
    let token = f.limited(&ALL).await;
    let second = f.new_task(f.project, "Second", None).await;
    let version = |task: Id| {
        let (f, token) = (&f, &token);
        async move { f.tool(token, "get_task", json!({"task_id": task})).await["version"].clone() }
    };
    let (one, two) = (version(f.task).await, version(second).await);
    // One stale version: neither is written.
    let (status, problem) = f
        .rest(
            &token,
            "post",
            "/tasks/bulk",
            Some(json!({"updates": [
                {"task": f.task, "expected_version": one, "title": "Changed"},
                {"task": second, "expected_version": two.as_u64().unwrap() + 1, "title": "Changed"},
            ]})),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{problem}");
    assert!(!problem.to_string().contains("Second"), "{problem}");
    assert_eq!(version(f.task).await, one);
    // The same task twice, none, and more than 100.
    let item = json!({"task_id": f.task, "expected_version": one, "title": "x"});
    for updates in [
        json!([item, item]),
        json!([]),
        Value::Array(vec![item.clone(); 101]),
    ] {
        f.refused(&token, "bulk_update_tasks", json!({"updates": updates}))
            .await;
    }
    let done = f
        .tool(
            &token,
            "bulk_update_tasks",
            json!({"updates": [
                {"task_id": f.task, "expected_version": one, "title": "Changed", "due_at": "2026-12-01"},
                {"task_id": second, "expected_version": two, "title": "Changed too"},
            ]}),
        )
        .await;
    assert_eq!(done["items"].as_array().unwrap().len(), 2);
    assert_eq!(done["items"][0]["due_at"], "2026-12-01T12:00:00.000Z");

    // Plan conflicts.
    for body in [json!({"name": "M1"}), json!({"name": "M1"})] {
        let (status, _) = f
            .rest(
                &token,
                "post",
                &format!("/projects/{}/milestones", f.project),
                Some(body),
            )
            .await;
        assert!(
            status == StatusCode::CREATED || status == StatusCode::CONFLICT,
            "{status}"
        );
    }
    let (status, problem) = f
        .rest(
            &token,
            "post",
            &format!("/projects/{}/milestones", f.project),
            Some(json!({"name": "M1"})),
        )
        .await;
    assert_eq!(
        (status, problem["code"].as_str()),
        (StatusCode::CONFLICT, Some("conflict")),
        "{problem}"
    );
    let cycles = format!("/projects/{}/cycles", f.project);
    let span = json!({"starts_at": "2027-01-01", "ends_at": "2027-01-15"});
    let (status, _) = f.rest(&token, "post", &cycles, Some(span.clone())).await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, problem) = f.rest(&token, "post", &cycles, Some(span)).await;
    assert_eq!(status, StatusCode::CONFLICT, "{problem}");
    // A status that tasks use.
    let in_use = f.status(f.project).await;
    let statuses = f
        .tool(
            &token,
            "list_task_statuses",
            json!({"project_id": f.project}),
        )
        .await;
    let status_version = statuses["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|status| status["id"] == json!(in_use))
        .unwrap()["version"]
        .clone();
    let (status, problem) = f
        .rest(
            &token,
            "delete",
            &format!(
                "/projects/{}/statuses/{in_use}?expected_version={status_version}",
                f.project
            ),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{problem}");
}

#[tokio::test]
async fn new_creates_take_an_idempotency_key() {
    let f = Fixture::new().await;
    let token = f.limited(&ALL).await;
    let page = f.doc(f.owner, "Doc", None, false).await;
    let other = f.new_task(f.project, "Other", None).await;
    for (path, body, table) in [
        (
            format!("/projects/{}/labels", f.project),
            json!({"name": "L"}),
            "labels",
        ),
        (
            format!("/projects/{}/milestones", f.project),
            json!({"name": "M"}),
            "project_milestones",
        ),
        (
            format!("/projects/{}/statuses", f.project),
            json!({"name": "S", "category": "started"}),
            "",
        ),
        (
            format!("/projects/{}/cycles", f.project),
            json!({"name": "C"}),
            "",
        ),
        (
            format!("/tasks/{}/relations", f.task),
            json!({"type": "related", "task_id": other}),
            "",
        ),
        (
            format!("/docs/{page}/comments"),
            json!({"quote": "Body", "body": "Hi"}),
            "",
        ),
        (format!("/docs/{page}/duplicate"), json!({}), ""),
        (
            "/views".to_owned(),
            json!({"name": "V", "visibility": "personal", "state": view(None)}),
            "",
        ),
    ] {
        let _ = table;
        let key = Id::new_v7().to_string();
        let headers = [("idempotency-key", key.as_str())];
        let send = || {
            f.rest_full(
                &token,
                "post",
                &path,
                Some(("application/json", body.to_string().into_bytes())),
                &headers,
            )
        };
        let (status, first, answered) = send().await;
        assert!(
            status.is_success(),
            "{path}: {status} {}",
            String::from_utf8_lossy(&first)
        );
        assert!(!answered.contains_key("idempotent-replayed"), "{path}");
        let (again, second, answered) = send().await;
        assert_eq!((again, &second), (status, &first), "{path}");
        assert_eq!(answered["idempotent-replayed"], "true", "{path}");
    }
}

#[tokio::test]
async fn views_and_favorites_are_the_persons_own() {
    let f = Fixture::new().await;
    let admin = f.person("admin", "admin").await;
    let owner = f.everything().await;
    let other = f.token_by(admin, TokenProjects::All, &ALL).await;
    let personal = f
        .tool(
            &owner,
            "create_view",
            json!({"name": "My view", "visibility": "personal", "state": view(None)}),
        )
        .await;
    let listed = f.tool(&other, "list_views", json!({})).await;
    assert!(!listed.to_string().contains("My view"), "{listed}");
    for (name, arguments) in [
        ("get_view", json!({"view_id": personal["id"]})),
        ("delete_view", json!({"view_id": personal["id"]})),
        (
            "set_view_favorite",
            json!({"view_id": personal["id"], "favorite": true}),
        ),
        (
            "update_view",
            json!({"view_id": personal["id"], "expected_version": personal["version"], "name": "x"}),
        ),
    ] {
        f.refused(&other, name, arguments).await;
    }
    // A shared view: a limited token may read it, but neither change nor delete it.
    let shared = f
        .tool(
            &owner,
            "create_view",
            json!({"name": "Shared", "visibility": "workspace", "state": view(None)}),
        )
        .await;
    let limited = f.limited(&ALL).await;
    f.tool(&limited, "get_view", json!({"view_id": shared["id"]}))
        .await;
    for (name, arguments) in [
        ("delete_view", json!({"view_id": shared["id"]})),
        (
            "update_view",
            json!({"view_id": shared["id"], "expected_version": shared["version"], "name": "x"}),
        ),
        (
            "update_view",
            json!({"view_id": personal["id"], "expected_version": personal["version"], "visibility": "workspace"}),
        ),
    ] {
        f.refused(&limited, name, arguments).await;
    }
    // Favorites and recents are the creator's.
    let page = f.doc(f.owner, "Doc", None, false).await;
    f.tool(&owner, "add_favorite_doc", json!({"page_id": page}))
        .await;
    let favorites = f.tool(&other, "list_favorite_docs", json!({})).await;
    assert_eq!(favorites["items"], json!([]), "{favorites}");
}

#[tokio::test]
async fn ids_of_another_workspace_are_unknown() {
    let f = Fixture::new().await;
    let token = f.everything().await;
    // A second workspace of the same owner: only the token's workspace keeps them apart.
    let now = TimestampMillis::now();
    let elsewhere = WorkspaceRepository::new((*f.database).clone())
        .create(f.owner, "Elsewhere".into(), "test", now)
        .await
        .unwrap()
        .id;
    let project = f
        .tasks
        .create_project(
            elsewhere,
            f.owner,
            "Else".into(),
            "ELS".into(),
            "#111111".into(),
            false,
            "test",
            now,
        )
        .await
        .unwrap()
        .id;
    let milestone = f
        .tasks
        .create_milestone(
            elsewhere,
            project,
            f.owner,
            "Else".into(),
            None,
            None,
            "test",
            now,
        )
        .await
        .unwrap()
        .id;
    let page = f
        .pages
        .create_page(
            elsewhere,
            f.owner,
            CreatePage {
                parent_id: None,
                space: SpaceRequest {
                    teamspace_id: None,
                    private: None,
                },
                title: "Else".into(),
                icon: None,
                position: None,
            },
            "test",
            now,
        )
        .await
        .unwrap()
        .id;
    for (name, arguments) in [
        (
            "create_milestone",
            json!({"project_id": project, "name": "x"}),
        ),
        (
            "update_milestone",
            json!({"project_id": f.project, "milestone_id": milestone, "expected_version": 0, "name": "x"}),
        ),
        ("create_label", json!({"project_id": project, "name": "x"})),
        (
            "update_project",
            json!({"project_id": project, "expected_version": 0, "name": "x"}),
        ),
        (
            "delete_project",
            json!({"project_id": project, "expected_version": 0}),
        ),
        ("get_trashed_project", json!({"project_id": project})),
        (
            "restore_project",
            json!({"project_id": project, "expected_version": 0}),
        ),
        ("list_doc_comments", json!({"page_id": page})),
        ("duplicate_doc", json!({"page_id": page})),
        (
            "move_doc",
            json!({"page_id": page, "expected_version": 0, "dry_run": true}),
        ),
        ("add_favorite_doc", json!({"page_id": page})),
        (
            "upload_doc_file",
            json!({"page_id": page, "filename": "a.txt", "content_base64": "YQ=="}),
        ),
    ] {
        f.refused(&token, name, arguments).await;
    }
    let untouched: String = sqlx::query_scalar("SELECT name FROM projects WHERE id = $1")
        .bind(project.to_string())
        .fetch_one(f.database.pool())
        .await
        .unwrap();
    assert_eq!(untouched, "Else");
}
