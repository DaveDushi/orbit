-- Postgres
-- Agent Skills (https://agentskills.io/specification): a folder with SKILL.md (front matter `name`
-- and `description`, a Markdown body) and optional scripts/, references/ and assets/. Owners and
-- admins add them to a workspace (from GitHub or skills.sh, a .zip, or written in Settings) and turn
-- each one on per agent. The prompt of an agent gets only the name and description of its skills;
-- it loads the body and the files with a tool (`agent::skills`).
CREATE TABLE agent_skills (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    description TEXT NOT NULL,
    body TEXT NOT NULL,
    -- `manual`, `zip`, `github` or `skills_sh`; `source_url` for the last two.
    source TEXT NOT NULL CHECK (source IN ('manual', 'zip', 'github', 'skills_sh')),
    source_url TEXT,
    created_by TEXT REFERENCES users(id) ON DELETE SET NULL,
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL,
    UNIQUE (workspace_id, name)
);

-- The skill's other files (SKILL.md itself is `body`), by path below the skill folder.
CREATE TABLE agent_skill_files (
    skill_id TEXT NOT NULL REFERENCES agent_skills(id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    content BYTEA NOT NULL,
    size_bytes BIGINT NOT NULL,
    PRIMARY KEY (skill_id, path)
);

-- The agents that use a skill: `default` for the default agent, else the bot user id of a custom agent.
CREATE TABLE agent_skill_agents (
    skill_id TEXT NOT NULL REFERENCES agent_skills(id) ON DELETE CASCADE,
    agent TEXT NOT NULL,
    PRIMARY KEY (skill_id, agent)
);
CREATE INDEX agent_skill_agents_agent ON agent_skill_agents (agent);
