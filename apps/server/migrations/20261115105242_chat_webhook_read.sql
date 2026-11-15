-- With `can_read` on, the webhook's secret can read its channel and threads without events
-- (apps/server/src/chat_routes/events.rs). Off by default: a post-only URL may sit in a CI config.
ALTER TABLE chat_webhooks ADD COLUMN can_read INTEGER NOT NULL DEFAULT 0 CHECK (can_read IN (0, 1));
