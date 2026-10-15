-- The numbered events of the live socket (apps/server/src/live/events.rs): every server of the database
-- numbers, replays and delivers them from here, so a client can reconnect to any server. Each workspace
-- keeps its newest 1000 events. `recipients_json` is who may see an event; `frame` is the text sent.
CREATE TABLE live_events (
    workspace_id TEXT NOT NULL,
    seq BIGINT NOT NULL,
    recipients_json TEXT NOT NULL,
    frame TEXT NOT NULL,
    PRIMARY KEY (workspace_id, seq)
);
