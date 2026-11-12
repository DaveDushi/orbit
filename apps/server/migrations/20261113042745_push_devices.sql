-- Apple Push Notification service (APNs): the iPhone app's device tokens (docs/operations.md, "iPhone push").
--
-- One row per installed app. The token is unique, so a phone that signs in as somebody else moves its row. As with
-- push_subscriptions, the row goes with the session that made it: signing out, or a session that ends, stops the
-- pushes. Additive only: a server without this table's code ignores it.
CREATE TABLE push_devices (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id),
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    -- The APNs device token, lowercase hex.
    token TEXT NOT NULL UNIQUE CHECK (length(token) BETWEEN 32 AND 200),
    -- What the user sees in the list of devices, e.g. "iPhone".
    label TEXT NOT NULL CHECK (length(label) <= 120),
    created_at INTEGER NOT NULL,
    -- Pushes that failed in a row; the server removes the device after a few (see push_subscriptions.failures).
    failures INTEGER NOT NULL DEFAULT 0 CHECK (failures >= 0)
);

CREATE INDEX push_devices_user ON push_devices (user_id);
-- For the cascade when a session is deleted.
CREATE INDEX push_devices_session ON push_devices (session_id);
