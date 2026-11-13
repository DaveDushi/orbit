//! Recurring tasks: completing an occurrence creates the next one, with its due date in the time
//! zone of the member who set the rule, and never a second one for the same occurrence.

mod task_support;

use axum::http::StatusCode;
use orbit_platform::Row;
use serde_json::{Value, json};
use task_support::{Fixture, Person};

/// Every task called `title`: (id, due_at, recurrence_id, status category), oldest first.
async fn series(fixture: &Fixture, title: &str) -> Vec<(String, Option<i64>, Option<String>, String)> {
    sqlx::query(
        "SELECT tasks.id, tasks.due_at, tasks.recurrence_id, task_statuses.category FROM tasks \
         JOIN task_statuses ON task_statuses.id = tasks.status_id \
         WHERE tasks.title = $1 AND tasks.deleted_at IS NULL ORDER BY tasks.id",
    )
    .bind(title)
    .fetch_all(fixture.database.pool())
    .await
    .unwrap()
    .into_iter()
    .map(|row| {
        (
            row.get("id"),
            row.get("due_at"),
            row.get("recurrence_id"),
            row.get("category"),
        )
    })
    .collect()
}

fn millis(rfc3339: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(rfc3339)
        .unwrap()
        .timestamp_millis()
}

async fn set_time_zone(fixture: &Fixture, person: &Person, zone: &str) {
    sqlx::query("UPDATE users SET timezone = $1 WHERE id = $2")
        .bind(zone)
        .bind(person.id.to_string())
        .execute(fixture.database.pool())
        .await
        .unwrap();
}

async fn complete(fixture: &Fixture, person: &Person, task: &Value) -> Value {
    let done = fixture.status("completed").await;
    fixture
        .update(person, task, json!({ "status_id": done }))
        .await
}

#[tokio::test]
async fn a_daily_task_repeats_once_at_the_same_local_time_across_a_clock_change() {
    let fixture = Fixture::new().await;
    let owner = &fixture.owner;
    set_time_zone(&fixture, owner, "America/New_York").await;
    // 17:00 in New York on the day before summer time starts (UTC-5).
    let task = fixture
        .task(owner, "Water the plants", json!({ "due_at": "2027-03-13T22:00:00Z" }))
        .await;
    let task = fixture
        .update(
            owner,
            &task,
            json!({ "recurrence": { "frequency": "daily", "interval": 1, "weekdays": [], "base": "due" } }),
        )
        .await;
    assert_eq!(task["recurrence"]["frequency"], "daily");

    let task = complete(&fixture, owner, &task).await;
    let tasks = series(&fixture, "Water the plants").await;
    assert_eq!(tasks.len(), 2, "{tasks:?}");
    let next = &tasks[1];
    // Still 17:00 in New York, now UTC-4: 23 hours later.
    assert_eq!(next.1, Some(millis("2027-03-14T21:00:00Z")));
    assert_eq!(next.2, tasks[0].2, "the occurrences share the rule");
    assert_eq!(next.3, "unstarted");
    // The creator followed the first occurrence and follows the next one too.
    let followers: Vec<String> =
        sqlx::query_scalar("SELECT user_id FROM task_followers WHERE task_id = $1")
            .bind(&next.0)
            .fetch_all(fixture.database.pool())
            .await
            .unwrap();
    assert_eq!(followers, [owner.id.to_string()]);

    // Reopened and completed again, the first occurrence makes no second successor.
    let open = fixture.status("unstarted").await;
    let task = fixture
        .update(owner, &task, json!({ "status_id": open }))
        .await;
    let task = complete(&fixture, owner, &task).await;
    let task = fixture
        .update(owner, &task, json!({ "status_id": open }))
        .await;
    complete(&fixture, owner, &task).await;
    assert_eq!(series(&fixture, "Water the plants").await.len(), 2);
}

#[tokio::test]
async fn weekly_rules_follow_their_weekdays_and_completion_rules_count_from_today() {
    let fixture = Fixture::new().await;
    let owner = &fixture.owner;
    // Monday 4 January 2027, 09:00 UTC; repeats on Mondays and Wednesdays.
    let task = fixture
        .task(owner, "Stand-up notes", json!({ "due_at": "2027-01-04T09:00:00Z" }))
        .await;
    let task = fixture
        .update(
            owner,
            &task,
            json!({ "recurrence": { "frequency": "weekly", "interval": 1, "weekdays": ["mon", "wed"], "base": "due" } }),
        )
        .await;
    complete(&fixture, owner, &task).await;
    let tasks = series(&fixture, "Stand-up notes").await;
    assert_eq!(tasks[1].1, Some(millis("2027-01-06T09:00:00Z")));

    // Counted from the day it is completed (today, UTC), at the time of the old due date.
    let task = fixture
        .task(owner, "Change the filter", json!({ "due_at": "2020-01-01T08:30:00Z" }))
        .await;
    let task = fixture
        .update(
            owner,
            &task,
            json!({ "recurrence": { "frequency": "daily", "interval": 3, "weekdays": [], "base": "completion" } }),
        )
        .await;
    let today = chrono::Utc::now().date_naive();
    complete(&fixture, owner, &task).await;
    let expected = (today + chrono::Days::new(3))
        .and_hms_opt(8, 30, 0)
        .unwrap()
        .and_utc()
        .timestamp_millis();
    assert_eq!(series(&fixture, "Change the filter").await[1].1, Some(expected));
}

#[tokio::test]
async fn a_stopped_or_invalid_rule_makes_no_occurrence() {
    let fixture = Fixture::new().await;
    let owner = &fixture.owner;
    let task = fixture
        .task(owner, "Pay rent", json!({ "due_at": "2027-02-01T10:00:00Z" }))
        .await;
    // Weekdays belong to weekly rules from the due date only.
    let (status, _) = fixture
        .call(
            owner,
            "PATCH",
            &format!("/tasks/{}", task["id"].as_str().unwrap()),
            Some(json!({
                "expected_version": task["version"],
                "recurrence": { "frequency": "monthly", "interval": 1, "weekdays": ["mon"], "base": "due" },
            })),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let task = fixture
        .update(
            owner,
            &task,
            json!({ "recurrence": { "frequency": "monthly", "interval": 1, "weekdays": [], "base": "due" } }),
        )
        .await;
    let task = fixture
        .update(owner, &task, json!({ "recurrence": null }))
        .await;
    assert_eq!(task["recurrence"], Value::Null);
    complete(&fixture, owner, &task).await;
    assert_eq!(series(&fixture, "Pay rent").await.len(), 1);
}
