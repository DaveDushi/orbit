-- The root user: the account created at setup. `users.installation_admin` now means "can open Admin" and is also
-- held by instance admins; only the root user makes or removes instance admins, and nobody can suspend or reset root.
ALTER TABLE installation_state ADD COLUMN root_user_id TEXT REFERENCES users(id);

-- Until now the setup account was the only installation admin; on older databases take the oldest one.
UPDATE installation_state
SET root_user_id = (
    SELECT id FROM users WHERE installation_admin = 1 ORDER BY created_at, id LIMIT 1
)
WHERE id = 1;
