-- Task followers. A member follows a task they created, are assigned to, wrote a note on or were
-- @mentioned in, and can follow or unfollow it by hand. Followers get an Inbox notification
-- (`task_commented`) for a new note and (`task_status_changed`) for a new status, unless it is
-- their own change; a member the note @mentions gets `comment_mentioned` only. Guests never
-- follow: the API refuses them, and notifications skip a follower who is no longer an active
-- member.
CREATE TABLE task_followers (
    task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at BIGINT NOT NULL,
    PRIMARY KEY (task_id, user_id)
);
CREATE INDEX task_followers_user ON task_followers (user_id);

-- Existing tasks: their creator, assignees and note authors (not service accounts), while they
-- are members of the workspace and not guests.
INSERT INTO task_followers (task_id, user_id, created_at)
SELECT DISTINCT candidates.task_id, candidates.user_id, candidates.created_at
FROM (
    SELECT tasks.id AS task_id, tasks.creator_id AS user_id, tasks.workspace_id, tasks.created_at
    FROM tasks WHERE tasks.creator_service_account_id IS NULL
    UNION ALL
    SELECT tasks.id, task_assignees.user_id, tasks.workspace_id, tasks.created_at
    FROM task_assignees JOIN tasks ON tasks.id = task_assignees.task_id
    UNION ALL
    SELECT task_comments.task_id, task_comments.author_id, task_comments.workspace_id, tasks.created_at
    FROM task_comments JOIN tasks ON tasks.id = task_comments.task_id
    WHERE task_comments.author_service_account_id IS NULL
) AS candidates
WHERE EXISTS (
    SELECT 1 FROM memberships
    WHERE memberships.workspace_id = candidates.workspace_id
      AND memberships.user_id = candidates.user_id AND memberships.role <> 'guest'
)
ON CONFLICT DO NOTHING;

-- Each member chooses whether notes and status changes of followed tasks notify them.
ALTER TABLE notification_prefs ADD COLUMN followed_tasks BIGINT NOT NULL DEFAULT 1
    CHECK (followed_tasks IN (0, 1));

-- The two new kinds (0052 named these CHECKs).
ALTER TABLE notifications DROP CONSTRAINT notifications_kind_check;
ALTER TABLE notifications DROP CONSTRAINT notifications_scope_check;
ALTER TABLE notifications ADD CONSTRAINT notifications_kind_check
    CHECK (kind IN ('task_assigned', 'comment_mentioned', 'description_mentioned',
                    'page_comment_mentioned', 'page_mentioned', 'chat_mentioned',
                    'task_commented', 'task_status_changed'));
ALTER TABLE notifications ADD CONSTRAINT notifications_scope_check
    CHECK (
        (kind IN ('task_assigned', 'comment_mentioned', 'description_mentioned',
                  'task_commented', 'task_status_changed')
            AND task_id IS NOT NULL
            AND page_id IS NULL AND page_thread_id IS NULL AND page_comment_id IS NULL
            AND page_block_id IS NULL)
        OR (kind = 'page_comment_mentioned'
            AND task_id IS NULL AND comment_id IS NULL
            AND page_id IS NOT NULL AND page_thread_id IS NOT NULL AND page_comment_id IS NOT NULL
            AND page_block_id IS NULL)
        OR (kind = 'page_mentioned'
            AND task_id IS NULL AND comment_id IS NULL
            AND page_id IS NOT NULL AND page_thread_id IS NULL AND page_comment_id IS NULL)
        OR (kind = 'chat_mentioned'
            AND task_id IS NULL AND comment_id IS NULL
            AND page_id IS NULL AND page_thread_id IS NULL AND page_comment_id IS NULL
            AND page_block_id IS NULL)
    );
