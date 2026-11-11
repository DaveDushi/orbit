-- Agent settings (step 11): the default agent is a card in Settings → Agentic → Agents too, and each agent has
-- its own connected services. The default agent's name and avatar stay those of its bot user (`agent_bot`), so
-- the name has one source and nothing moves.

-- The default agent's settings in one workspace: its own instructions (besides the team instructions, which
-- every agent reads) and its model and thinking level. `model`/`thinking` NULL: the installation's choice.
CREATE TABLE agent_default_profiles (
    workspace_id TEXT PRIMARY KEY REFERENCES workspaces(id) ON DELETE CASCADE,
    instructions TEXT NOT NULL DEFAULT '',
    model TEXT,
    thinking TEXT CHECK (thinking IS NULL OR thinking IN ('minimal', 'low', 'medium', 'high', 'xhigh', 'max')),
    updated_by TEXT REFERENCES users(id) ON DELETE SET NULL,
    updated_at BIGINT NOT NULL
);

-- The integrations a custom agent may use: none until ticked, also for integrations added later.
CREATE TABLE agent_profile_integrations (
    agent_id TEXT NOT NULL REFERENCES agent_profiles(id) ON DELETE CASCADE,
    integration_id TEXT NOT NULL REFERENCES agent_integrations(id) ON DELETE CASCADE,
    PRIMARY KEY (agent_id, integration_id)
);
CREATE INDEX agent_profile_integrations_integration ON agent_profile_integrations (integration_id);

-- The integrations the default agent may not use: every other one, also integrations added later.
CREATE TABLE agent_default_integrations_off (
    integration_id TEXT PRIMARY KEY REFERENCES agent_integrations(id) ON DELETE CASCADE
);

-- Custom agents that had the `use_integration` tool keep every integration of their workspace.
INSERT INTO agent_profile_integrations (agent_id, integration_id)
SELECT p.id, i.id FROM agent_profiles p JOIN agent_integrations i ON i.workspace_id = p.workspace_id
WHERE p.deleted_at IS NULL AND p.tools LIKE '%"use_integration"%';
