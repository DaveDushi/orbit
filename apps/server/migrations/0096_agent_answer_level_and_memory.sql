-- Agent settings cleanup and memory per agent (step 13). Design: .ai/agent-design.md, "Agent profiles" (P8)
-- and "Memory" (M10).

-- 1. Answer level per agent (step 13): set on each agent's card in Settings → Agentic → Agents, no longer per person.
ALTER TABLE agent_profiles ADD COLUMN answer_level TEXT NOT NULL DEFAULT 'normal'
    CHECK (answer_level IN ('short', 'normal', 'thorough'));
ALTER TABLE agent_default_profiles ADD COLUMN answer_level TEXT NOT NULL DEFAULT 'normal'
    CHECK (answer_level IN ('short', 'normal', 'thorough'));

-- Each workspace's level until now: the most common level of its people (owners, admins and members, not
-- guests or agents; no choice counts as normal). A tie keeps normal, then short. Every agent of the workspace,
-- the default one too, starts with it.
CREATE TEMPORARY TABLE workspace_answer_levels AS
WITH levels AS (
    SELECT m.workspace_id, COALESCE(p.answer_level, 'normal') AS level
    FROM memberships m
    JOIN users u ON u.id = m.user_id
    LEFT JOIN agent_preferences p ON p.user_id = m.user_id
    WHERE m.role <> 'guest'
      AND NOT EXISTS (SELECT 1 FROM agent_profiles a WHERE a.user_id = m.user_id)
      AND NOT EXISTS (SELECT 1 FROM agent_bot b WHERE b.user_id = m.user_id)
), counts AS (
    SELECT workspace_id, level, COUNT(*) AS people FROM levels GROUP BY workspace_id, level
), ranked AS (
    SELECT workspace_id, level, ROW_NUMBER() OVER (
        PARTITION BY workspace_id
        ORDER BY people DESC, CASE level WHEN 'normal' THEN 0 WHEN 'short' THEN 1 ELSE 2 END
    ) AS place
    FROM counts
)
SELECT workspace_id, level FROM ranked WHERE place = 1;

UPDATE agent_profiles SET answer_level = COALESCE(
    (SELECT level FROM workspace_answer_levels l WHERE l.workspace_id = agent_profiles.workspace_id), 'normal');

INSERT INTO agent_default_profiles (workspace_id, answer_level, updated_at)
SELECT workspace_id, level, CAST(strftime('%s', 'now') AS INTEGER) * 1000 FROM workspace_answer_levels WHERE level <> 'normal'
ON CONFLICT (workspace_id) DO UPDATE SET answer_level = excluded.answer_level;

DROP TABLE workspace_answer_levels;
DROP TABLE agent_preferences;

-- 2. Memory per agent: each agent has its own team memory and private memories. NULL is the default agent, as in
-- every other `agent_user_id` column, so the memory written so far stays the default agent's.
ALTER TABLE agent_memory_scopes ADD COLUMN agent_user_id TEXT REFERENCES users(id) ON DELETE CASCADE;
DROP INDEX agent_memory_scopes_team;
DROP INDEX agent_memory_scopes_private;
CREATE UNIQUE INDEX agent_memory_scopes_team ON agent_memory_scopes (workspace_id, COALESCE(agent_user_id, ''))
    WHERE owner_user_id IS NULL;
CREATE UNIQUE INDEX agent_memory_scopes_private
    ON agent_memory_scopes (workspace_id, owner_user_id, COALESCE(agent_user_id, ''))
    WHERE owner_user_id IS NOT NULL;
