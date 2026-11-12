-- GitHub comment sync (both ways). `github_comment_id` is the GitHub comment of a note: set when
-- the note came from GitHub, and when Orbit posted the note to GitHub. `github_author_login` is set
-- only for notes from GitHub (their author is the GitHub service account); such notes are never
-- sent back. SQLite: ../0100_github_comment_sync.sql.
ALTER TABLE task_comments ADD COLUMN github_comment_id BIGINT;
ALTER TABLE task_comments ADD COLUMN github_author_login TEXT;
ALTER TABLE task_comments ADD COLUMN github_author_avatar_url TEXT;

CREATE UNIQUE INDEX task_comments_github_comment ON task_comments (workspace_id, github_comment_id)
    WHERE github_comment_id IS NOT NULL;

-- A note from GitHub notifies as written by its GitHub author, not by a member: such a
-- notification has `actor_github_login` and no `actor_user_id` (exactly one of the two is set).
ALTER TABLE notifications ALTER COLUMN actor_user_id DROP NOT NULL;
ALTER TABLE notifications ADD COLUMN actor_github_login TEXT
    CHECK (actor_github_login IS NULL OR length(actor_github_login) BETWEEN 1 AND 100);
ALTER TABLE notifications ADD CONSTRAINT notifications_actor_check
    CHECK ((actor_user_id IS NULL) <> (actor_github_login IS NULL));
