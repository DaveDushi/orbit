-- The fork's webhook bots (0095_webhook_bots, step 10) are replaced by two-way chat webhooks
-- (20261113100400_chat_webhook_events, apps/server/src/chat_routes/events.rs). A live bot's user is
-- suspended and renamed "<name> (deleted)", as deleting a bot did, so its messages and DMs stay
-- readable and its name is free again. Then the bot tables go.
UPDATE users
SET display_name = display_name || ' (deleted)',
    suspended_at = (SELECT b.updated_at FROM webhook_bots b WHERE b.user_id = users.id),
    updated_at = (SELECT b.updated_at FROM webhook_bots b WHERE b.user_id = users.id),
    version = version + 1
WHERE id IN (SELECT user_id FROM webhook_bots WHERE deleted_at IS NULL);

DROP TABLE webhook_bot_deliveries;
DROP TABLE webhook_bot_channels;
DROP TABLE webhook_bots;
