-- The model and thinking level of the team agent's conversations, one for the installation
-- (apps/server/src/agent/settings.rs). An installation admin sets them; the sidecar reads them for
-- each run. NULL: the sidecar's default (`AGENT_MODEL`, Pi's default thinking level).
CREATE TABLE agent_model_settings (
    id BIGINT PRIMARY KEY CHECK (id = 1),
    model TEXT CHECK (model IS NULL OR length(model) BETWEEN 1 AND 100),
    thinking TEXT CHECK (thinking IS NULL OR thinking IN ('minimal', 'low', 'medium', 'high', 'xhigh', 'max')),
    updated_by TEXT REFERENCES users(id) ON DELETE SET NULL,
    updated_at BIGINT NOT NULL
);
