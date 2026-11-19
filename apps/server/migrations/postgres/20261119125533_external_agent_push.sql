-- Postgres: the same change as apps/server/migrations/20261119125533_external_agent_push.sql
-- External agents (phase 2, push): Orbit sends each input of an external agent to its Push URL, signed like chat
-- webhook events. `push_secret`: the signing secret, encrypted with the app key (`secret_box`). `push_since`: when the
-- URL was set; older inputs are not pushed (they stay for `wait_for_messages`).
ALTER TABLE agent_profiles ADD COLUMN push_url TEXT;
ALTER TABLE agent_profiles ADD COLUMN push_secret BYTEA;
ALTER TABLE agent_profiles ADD COLUMN push_since BIGINT;

-- Push tries of one input: `push_attempts` sends so far (at most 4: at once, then after 10 s, 60 s and 5 min), and
-- when the next one is due. A 2xx sets `delivered_at` and acknowledges the input.
ALTER TABLE agent_inputs ADD COLUMN push_attempts BIGINT NOT NULL DEFAULT 0;
ALTER TABLE agent_inputs ADD COLUMN push_next_at BIGINT;
CREATE INDEX agent_inputs_push ON agent_inputs (agent_user_id, message_id)
    WHERE acked_at IS NULL AND delivered_at IS NULL AND agent_user_id IS NOT NULL;

-- The newest 20 push tries of an agent, for its card.
CREATE TABLE agent_push_deliveries (
    id TEXT PRIMARY KEY,
    agent_id TEXT NOT NULL REFERENCES agent_profiles(id) ON DELETE CASCADE,
    event_id TEXT NOT NULL,
    attempt BIGINT NOT NULL,
    status BIGINT,
    error TEXT,
    duration_ms BIGINT NOT NULL,
    created_at BIGINT NOT NULL
);
CREATE INDEX agent_push_deliveries_agent ON agent_push_deliveries (agent_id, created_at);
