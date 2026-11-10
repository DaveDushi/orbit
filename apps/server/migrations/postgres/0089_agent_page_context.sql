-- The Ask bar (on every page but Chat) sends a message to the person's DM with the agent, with
-- the page they were on: the area and the open task, Docs page or Drive folder (id and title).
-- It is stored as JSON on the agent input only, never on the chat message, and the sidecar shows
-- it to the model as data.
ALTER TABLE agent_inputs ADD COLUMN page_context TEXT;
