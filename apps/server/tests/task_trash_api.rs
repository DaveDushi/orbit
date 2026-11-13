//! The tasks trash: owners and admins delete a trashed task or project forever, or empty the
//! whole trash; members only look at it and restore. A private project's trash stays private.

mod task_support;

use axum::http::StatusCode;
use serde_json::{Value, json};
use task_support::{Fixture, Person};

async fn trash_task(fixture: &Fixture, person: &Person, task: &Value) {
    let (status, body) = fixture
        .call(
            person,
            "DELETE",
            &format!(
                "/tasks/{}?expected_version={}",
                task["id"].as_str().unwrap(),
                task["version"]
            ),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
}

async fn count(fixture: &Fixture, sql: &'static str, id: &str) -> i64 {
    sqlx::query_scalar(sql)
        .bind(id)
        .fetch_one(fixture.database.pool())
        .await
        .unwrap()
}

async fn task_rows(fixture: &Fixture, id: &str) -> i64 {
    count(fixture, "SELECT COUNT(*) FROM tasks WHERE id = $1", id).await
}

#[tokio::test]
async fn only_owners_and_admins_delete_forever_and_empty_the_trash() {
    let fixture = Fixture::new().await;
    let owner = &fixture.owner;
    let admin = fixture.person("admin", "admin").await;
    let member = fixture.person("member", "member").await;
    let guest = fixture.person("guest", "guest").await;
    let first = fixture.task(&member, "First", json!({})).await;
    let second = fixture.task(&member, "Second", json!({})).await;
    trash_task(&fixture, &member, &first).await;
    trash_task(&fixture, &member, &second).await;
    let first_id = first["id"].as_str().unwrap();
    let second_id = second["id"].as_str().unwrap();

    // The member sees the trash and the preview, but cannot delete forever.
    let (status, preview) = fixture
        .call(&member, "GET", &format!("/tasks/trash/{first_id}"), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    assert_eq!(preview["title"], "First");
    let purge = format!("/tasks/{first_id}/permanent");
    let (status, _) = fixture.call(&member, "DELETE", &purge, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = fixture
        .call(&member, "POST", "/tasks/trash/empty", None)
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    // A guest reaches neither.
    for (method, path) in [
        ("GET", format!("/tasks/trash/{first_id}")),
        ("DELETE", purge.clone()),
        ("POST", "/tasks/trash/empty".to_owned()),
    ] {
        let (status, _) = fixture.call(&guest, method, &path, None).await;
        assert!(
            status == StatusCode::NOT_FOUND || status == StatusCode::FORBIDDEN,
            "guest {method} {path}: {status}"
        );
    }
    assert_eq!(task_rows(&fixture, first_id).await, 1);

    // An admin deletes one forever; the owner empties the rest.
    let (status, _) = fixture.call(&admin, "DELETE", &purge, None).await;
    assert!(status.is_success(), "{status}");
    assert_eq!(task_rows(&fixture, first_id).await, 0);
    let (status, _) = fixture
        .call(&admin, "GET", &format!("/tasks/trash/{first_id}"), None)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let emptied = fixture
        .ok(owner, "POST", "/tasks/trash/empty", None)
        .await;
    assert_eq!(emptied["purged_tasks"], 1);
    assert_eq!(task_rows(&fixture, second_id).await, 0);
}

#[tokio::test]
async fn a_trashed_project_is_deleted_forever_with_its_tasks_and_stays_private() {
    let fixture = Fixture::new().await;
    let owner = &fixture.owner;
    let alice = fixture.person("alice", "member").await;
    let bob = fixture.person("bob", "member").await;
    let project = fixture
        .ok(
            &alice,
            "POST",
            "/projects",
            Some(json!({"name": "Secret", "key": "SEC", "color": "#336699", "private": true})),
        )
        .await;
    let project_id = project["id"].as_str().unwrap();
    let status: String = sqlx::query_scalar(
        "SELECT id FROM task_statuses WHERE project_id = $1 ORDER BY position LIMIT 1",
    )
    .bind(project_id)
    .fetch_one(fixture.database.pool())
    .await
    .unwrap();
    let task = fixture
        .ok(
            &alice,
            "POST",
            "/tasks",
            Some(json!({"project_id": project_id, "status_id": status, "title": "Plan B"})),
        )
        .await;
    let (code, body) = fixture
        .call(
            &alice,
            "DELETE",
            &format!(
                "/projects/{project_id}?expected_version={}",
                project["version"]
            ),
            None,
        )
        .await;
    assert_eq!(code, StatusCode::NO_CONTENT, "{body}");

    // Alice sees the trashed project and its tasks; Bob does not even see it there.
    fixture
        .ok(&alice, "GET", &format!("/projects/trash/{project_id}"), None)
        .await;
    let tasks = fixture
        .ok(
            &alice,
            "GET",
            &format!("/projects/trash/{project_id}/tasks"),
            None,
        )
        .await;
    assert_eq!(tasks["items"][0]["title"], "Plan B");
    for path in [
        format!("/projects/trash/{project_id}"),
        format!("/projects/trash/{project_id}/tasks"),
    ] {
        let (status, _) = fixture.call(&bob, "GET", &path, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
    }
    let listed = fixture.ok(&bob, "GET", "/projects/trash", None).await;
    assert!(
        listed["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item["id"] != project_id)
    );

    // Alice cannot delete it forever (a member); the owner can.
    let purge = format!("/projects/{project_id}/permanent");
    let (status, _) = fixture.call(&alice, "DELETE", &purge, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = fixture.call(owner, "DELETE", &purge, None).await;
    assert!(status.is_success(), "{status}");
    assert_eq!(
        count(&fixture, "SELECT COUNT(*) FROM projects WHERE id = $1", project_id).await,
        0
    );
    assert_eq!(task_rows(&fixture, task["id"].as_str().unwrap()).await, 0);
    assert_eq!(
        count(
            &fixture,
            "SELECT COUNT(*) FROM project_members WHERE project_id = $1",
            project_id
        )
        .await,
        0
    );
}
