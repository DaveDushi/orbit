-- SQLite
-- The workspace whose owner or admin suspended the account (Settings → Members). Only that workspace may resume it.
-- NULL while the account is active, or when an instance admin suspended it (Admin → Users).
ALTER TABLE users ADD COLUMN suspended_workspace_id TEXT;
