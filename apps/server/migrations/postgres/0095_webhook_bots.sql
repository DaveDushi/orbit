-- Webhook bots (step 10): bots that answer. An owner or admin gives a bot a name, an avatar, an
-- https URL and the channels where it may be used. A mention of the bot in one of its channels, or
-- a message in a DM with it, is sent to the URL, signed; the bot answers within 30 seconds, in the
-- HTTP answer or through a single-use reply URL. Each bot is a bot user of its own, as an agent
-- profile is (step 6), so people mention it and DM it like a person. Code: apps/server/src/webhook_bots.

-- `signing_secret` is secret_box ciphertext under ORBIT_APP_KEY: Orbit signs with it, so it is
-- encrypted, not hashed (as chat_webhooks.signing_secret). A deleted bot keeps its row and its bot
-- user (suspended), so its messages and DMs stay readable.
CREATE TABLE webhook_bots (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    user_id TEXT NOT NULL UNIQUE REFERENCES users(id),
    url TEXT NOT NULL CHECK (length(url) BETWEEN 1 AND 2048),
    signing_secret BYTEA NOT NULL,
    created_by TEXT REFERENCES users(id) ON DELETE SET NULL,
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL,
    secret_rotated_at BIGINT,
    deleted_at BIGINT
);
CREATE INDEX webhook_bots_workspace ON webhook_bots (workspace_id);

CREATE TABLE webhook_bot_channels (
    bot_id TEXT NOT NULL REFERENCES webhook_bots(id) ON DELETE CASCADE,
    conversation_id TEXT NOT NULL REFERENCES chat_conversations(id) ON DELETE CASCADE,
    PRIMARY KEY (bot_id, conversation_id)
);

-- One message for a bot. `status`: pending (written with the message), sending (Orbit posts it;
-- the reply token is live), answered, late (no answer in 30 seconds), failed (the bot refused or
-- could not be reached), refused (a channel the bot is not in: the author gets a DM), limited (the
-- bot's rate limit). `reply_token_hash` is the SHA-256 of the reply token, NULL once used.
-- `root_message_id`: the thread the answer goes to; NULL is the main list of a DM with the bot.
CREATE TABLE webhook_bot_deliveries (
    id TEXT PRIMARY KEY,
    bot_id TEXT NOT NULL REFERENCES webhook_bots(id) ON DELETE CASCADE,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    conversation_id TEXT NOT NULL REFERENCES chat_conversations(id) ON DELETE CASCADE,
    message_id TEXT NOT NULL,
    root_message_id TEXT,
    author_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    status TEXT NOT NULL CHECK (status IN ('pending', 'sending', 'answered', 'late', 'failed', 'refused', 'limited')),
    reply_token_hash BYTEA,
    http_status BIGINT,
    error TEXT,
    created_at BIGINT NOT NULL,
    expires_at BIGINT NOT NULL,
    finished_at BIGINT
);
CREATE INDEX webhook_bot_deliveries_open ON webhook_bot_deliveries (status, expires_at);
CREATE INDEX webhook_bot_deliveries_bot ON webhook_bot_deliveries (bot_id, created_at);
CREATE UNIQUE INDEX webhook_bot_deliveries_token ON webhook_bot_deliveries (reply_token_hash);
