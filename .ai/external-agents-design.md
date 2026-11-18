# Orbit: external agents (design only)

Status: DESIGN for the user's decision, 2026-11-18. No code, no worktree, nothing on prod.
Project: Orbit (`ef9a996e-…`), read on `main` at `c0b8f0e5`.

User's goal: "add external agents that are like the internal ones … connected to the MCP, but I can also talk to
it from within Orbit", and @mention it in one chat or in many chats, like the built-in agent.

## 1. What exists today (checked in code)

| Part | What it does | Limit for this goal |
|---|---|---|
| **Agent profiles** (`docs/agent.md:104-148`, `apps/server/src/agent/profiles.rs`) | Each agent is a bot user (`users` row `agent-{id}@orbit.invalid`, `profiles.rs:1151`) with a `member` membership (`profiles.rs:1164`), an `agent_profiles` row, channels (`all_channels` or `agent_profile_channels`, check `allowed_in` `profiles.rs:405`), DMs, @ list, task assign/mention (`agent_task_runs`), max 20 per workspace (`profiles.rs:45`). Delete = suspend + rename the bot user. | It always runs in Orbit's own sidecar. Messages go to `agent_inputs` (`agent/inputs.rs:372`) and only the sidecar reads that queue. |
| **Two-way chat webhooks** (`docs/chat-webhooks.md`, `chat_routes/events.rs`, `chat_routes/interactions.rs`) | Signed Slack-style `event_callback` to a request URL (HMAC `v0:` + 5 min window), one try, 3 s, no redirects, public-address-only resolver (`interactions.rs:29-47`). Signing secret encrypted with `ORBIT_APP_KEY` (`secret_box.rs`), URL secret stored as hash. Reads the channel with Bearer (`GET …/messages`), thread replies with `thread_ts`. Last 20 deliveries shown. | ONE channel per webhook, not a user: no @ list, no DMs, no tasks, no MCP. `@name` match is text only (`events.rs:90`). |
| **MCP with OAuth** (`docs/connect-your-agent.md`, `oauth.rs`, `mcp.rs`) | OAuth 2.1 code + PKCE, DCR, public clients, `resource` binding, 15 min access / 30 day refresh. Scopes `tasks:read` (always), `tasks:write`, `docs:read`, `docs:write`, `chat:read`, `drive:read`, `drive:write` (`oauth.rs:34`). Grant = person + workspace + projects (`oauth_grants`). Each request re-checks: not guest, not suspended, project still visible (`oauth.rs:181-240`). | Acts as a PERSON. Chat on MCP is **read only**: `list_conversations`, `list_messages`, `get_thread`, `search_messages`; there is no send/reply tool. No inbox. |
| **Service-account API tokens** (`repositories/api_tokens.rs:130-150`, `docs/public-api.md:50-62`) | Non-login author name. | Reads use the **workspace owner's** access (`actor_id`). Too wide for an outside agent: do not reuse this for external agents. |

## 2. Recommendation in one paragraph

An **external agent** is a third kind of agent profile (`agent_profiles.kind = 'external'`). It reuses the bot user,
membership, avatar, @ list, channels, DMs and task assignment of agent profiles. Instead of the sidecar, its inputs go
to the outside agent: **push** (signed POST, the webhook event code) or **pull** (an MCP `wait_for_messages` tool).
The outside agent connects to Orbit's MCP **as the bot user**: an admin approves an OAuth grant that is bound to the
agent (works unchanged in Claude Code and Codex), or copies an agent token for a server. Every MCP call checks the
bot user's own access AND the agent's channel and project lists. It answers with new MCP chat write tools, as itself,
in the thread. Its writes need no per-write yes; the admin's permission set is the limit (section 5).

## 3. Settings and data

**Settings → Agentic → Agents** gets **New agent → External** next to the internal ones. Owners and admins of the
workspace make and edit it (same rules as agent profiles).

| Field | Internal agent | External agent |
|---|---|---|
| Name, avatar, @handle | yes | yes (same rules, same uniqueness) |
| Channels | all or chosen | **chosen only** (all public channels is a tick, private channels only by name; see 4.3) |
| DMs | always | always (a toggle "Allow DMs", on) |
| Projects | through tools | **chosen projects** (or "all public projects"); private projects only when the bot is on the project's member list |
| Permissions | tool subset | **scopes**: `tasks:read` always; `tasks:write`, `docs:read`, `docs:write`, `chat:read`, `chat:write`, `drive:read`, `drive:write` |
| Model, instructions, answer level, connected services | yes | **no** (the outside agent owns its model and prompt) |
| Delivery | sidecar | **Push URL** (https) and/or **Pull over MCP** |
| Connection | — | **Connect** button (OAuth as this agent), **Agent token** (shown once), **Disconnect** |
| Status | — | Last seen, last 20 deliveries (reuse webhook delivery list) |

Data (one migration, additive only):

- `agent_profiles`: `kind TEXT NOT NULL DEFAULT 'internal'`, `push_url TEXT NULL`, `signing_secret BLOB NULL`
  (encrypted, `secret_box`), `scopes TEXT`, `all_public_projects BOOL`, `allow_dms BOOL`, `last_seen_at`.
- `agent_profile_projects (agent_id, project_id)` like `agent_profile_channels`.
- `oauth_grants`: `agent_profile_id NULL`, `approved_by NULL` (the admin). `user_id` = the bot user.
- `agent_tokens (id, agent_id, token_hash, created_by, created_at, expires_at, last_used_at, revoked_at)` — SHA-256
  hash only, like API tokens.
- `agent_inputs`: already has `agent_user_id`; add `delivered_at`, `delivery_attempts` for push and the pull cursor.
  `agent_task_runs` (or a light `agent_events` row) for task assign/mention.

## 4. Design

### 4.1 Identity and MCP: the agent is the bot user

Two ways to get credentials, both bound to ONE agent profile:

1. **OAuth "Connect as agent" (recommended default, works with Claude Code / Codex as they are).** The admin runs
   `claude mcp add … orbit <MCP URL>` or `codex mcp add/login` on the machine of the outside agent. On Orbit's
   consent page, an owner/admin sees a new choice **"Connect as: me / agent @Muse"** (only external agents of
   workspaces where they are owner/admin). With an agent selected, the page shows the agent's fixed channels,
   projects and scopes read only — the admin cannot give more than the agent card allows. The grant row gets
   `user_id = bot user`, `agent_profile_id`, `approved_by = admin`. Refresh, `resource` binding and 15 min access stay
   as today. Optional: a `?agent=<id>` link from the agent card pre-selects it.
2. **Agent token (for a server agent like Muse).** **Agent token → Create** shows `orb_agent_…` once; Orbit stores
   the hash, expires in 90 days (choice: 30/90/365), rotate and revoke on the card. It works as
   `Authorization: Bearer` on `/mcp` **including the public MCP origin** (only for agent tokens; personal workspace
   tokens stay main-origin only, `connect-your-agent.md` "Good to know").

**OAuth client credentials:** not in the MVP. It exists only as the MCP extension
`io.modelcontextprotocol/oauth-client-credentials` (SEP-1046, Final), opt-in on both sides, and SDK support is not
complete (TypeScript PR #1157 draft). The agent token gives the same machine-to-machine result with less code.
Revisit when Claude Code / Codex support the extension.

**Principal and policy (the security core).** `oauth::principal()` (`oauth.rs:236`) and the token path build an
`ApiTokenPrincipal` with `actor_id = creator_id = bot user`, `service_account = None`, plus a new
`agent: Some(AgentLimits { profile_id, channel_ids, all_public_channels, allow_dms, project_ids })`. Each tool then
checks BOTH:

- the bot user's own access (it is a normal `member`, so existing `user_sees_project_sql`, chat membership and Drive
  checks apply — no new bypass), and
- the agent's lists: a conversation is allowed only if it is a DM/group DM the bot is in (and `allow_dms`), or in
  `channel_ids` (or a public channel when `all_public_channels`); a project only if in `project_ids` (or public when
  "all public projects").
- **IDOR:** every ID from a tool argument (`conversation_id`, `thread_ts`/message id, `task_id`, `doc_id`,
  `drive node`) is resolved to its conversation/project/workspace on the server and checked against both lists
  before any read or write; an ID outside gives the same "not found" as an unknown ID (no existence leak). One shared
  helper `agent_scope::conversation_allowed / project_allowed`, used by MCP chat, tasks, docs and Drive tools.
- `list_conversations` and `search_messages` filter by the lists in SQL, not after the fetch.
- Auth query (`oauth.rs:181`) keeps `u.suspended_at IS NULL` and `m.role <> 'guest'`: deleting the agent suspends
  the bot user, so every grant and token stops at once; the delete also sets `revoked_at` on its grants/tokens.
- Guests never see an external agent, cannot DM it, and their messages never reach it (same as agent profiles).

### 4.2 How messages reach it

What counts as an input is the same as for an internal agent (reuse `inputs::record` with `agent_user_id`): a DM
to the bot, an `@mention` in one of its channels, a reply in a thread where it is a participant (opt-in), a task
assigned to it, a task comment that mentions it or is on its task. Each input is one row (already transactional, so
nothing is lost). The sidecar poll skips `kind = 'external'` agents.

**Event shape:** the Slack `event_callback` of `chat-webhooks.md#the-event` (so Bolt-style code works), with
`api_app_id = agent id`, `event.type` in `app_mention` | `message` (DM, `channel_type: "im"`) | `task_assigned` |
`task_comment`, plus `event.thread_ts`, `event_id = Ev<input id>`. Text has `<@id>` tokens. Task events carry
`task_id`, title, link, the comment — not the full task (the agent reads it with `get_task`, checked).

**Push mode** (agents with a server):
- `POST push_url`, signed exactly like webhook events (HMAC-SHA256 `v0:{ts}:{body}`, `X-Orbit-Signature` +
  `X-Slack-Signature`, 5 min window, constant-time compare in the receiver). Reuse `interactions::post_signed`
  (public-address resolver, no redirects, 3 s).
- New: **retries** for agent pushes (not for webhooks): up to 3 tries (10 s, 60 s, 5 min) from the `agent_inputs`
  row until `delivered_at`; then the input stays for pull and the agent card shows "push failing". `event_id` is the
  dedup key for the receiver.
- The agent answers through MCP (below), not through the HTTP response, so a slow model never hits the 3 s limit.

**Pull mode** (Claude Code, Codex, any MCP client without a server):
- New MCP tool **`wait_for_messages`** `{ timeout_seconds ≤ 50, ack? : [event_id] }`: acknowledges the given events,
  then returns at once with pending inputs (oldest first, max 20), or waits (long-poll inside the tool call, wakes on
  the same realtime signal that the chat uses) until one arrives or the timeout ends with `[]`. Not acked events come
  back again (at-least-once). Portable: works in every MCP client because it is a plain tool call.
- The standard spec options are weaker here: 2025-11-25 lets a client open a GET SSE stream for server messages, but
  clients surface notifications to the model inconsistently, and the next revision (2026-07-28) removes the GET
  stream and `resources/subscribe` for `subscriptions/listen`. Do not build on them for the MVP.
- Claude Code-specific nicer option (later): a tiny local stdio "channel" MCP server (`notifications/claude/channel`,
  research preview, needs the development flag for custom channels) that long-polls Orbit and pushes events into a
  running Claude Code session. Phase 3, optional.
- A usage recipe goes in the docs: "Run `claude -p` / `codex exec` with: loop `wait_for_messages`, act, reply".

**Replies and "working" state** (new MCP chat write tools, scope `chat:write`):
- `send_message { conversation_id, text, thread_ts? }` — posts as the bot user; for a mention the reply goes in the
  thread (`thread_ts` = the message's root, as webhooks do). Markdown + mention tokens; same 4,000 char cut, same
  per-message rules as members (no `@channel` for agents in MVP).
- `set_status { conversation_id, thread_ts?, state: working | idle, label? }` — shows "@Muse is working…" in the
  thread like the internal agent's state, expires after 2 min unless renewed; `send_message` clears it.
- `add_reaction` (👀 on receipt) is a cheap extra, optional.
- Bot messages of an external agent create **no agent inputs** for any agent (no loops, same rule as
  `docs/agent.md:169`). A member's later reply that mentions the other agent starts it normally.

### 4.3 Channels: no silent auto-join

Today a mention of an agent in a private channel makes the agent **join** it (`inputs.rs:343-362`). For an external
agent that would send private content to an outside server. Rule: an external agent never auto-joins. A mention in a
channel that is not on its list gets the same refusal DM as custom agents today ("I work in #a, #b"). An admin adds a
private channel on the agent card; Orbit then adds the bot to `chat_members`. "All public channels" never includes
private channels or other people's DMs.

### 4.4 Approvals (decision 3)

Internal agents act **as the driver** with the driver's approvals and ChatGPT payer. An external agent has no driver:
it acts as itself with its own model and cost. Proposal:

- **Default: no per-write yes.** The admin's permission set is the limit: scopes + channels + projects on the card.
  Writes show the agent as author (messages, comments, task changes), are in the audit log
  (`external_agent.write`, tool, ids), and members can see "by @Muse" everywhere.
- **Off by default, admin turns on:** `tasks:write`, `docs:write`, `drive:write`. `delete_task` is never available
  to external agents in the MVP.
- **Later option (phase 3):** per-agent "Ask before writes": write tools return `pending_approval`, Orbit posts a
  card in the thread, the member who mentioned the agent (or an admin) clicks yes. Reuse the internal approval card.

Why: a per-write yes needs a human in each run, which breaks the pull loop of an unattended agent, and the agent's
reach is already limited by Orbit (not by its prompt), so prompt injection can only touch what the card allows.

### 4.5 Migration of Muse (decision 4)

Today Muse is chat webhook `01a1debe-523a-70ab-b79b-db6148a811d8` in #marketing, with events ON to the Ops sink
(`orbit-hooks.easytomanage.com/hooks-sink/15ba…`) and Bearer read (memory #198).

1. Admin makes external agent **@Muse** (same name and avatar; the webhook's display name does not hold the handle,
   because webhooks are not users — check name collision), channels: #marketing (+ others the user picks), DMs on,
   scopes `chat:read chat:write` only, no projects.
2. Push URL = Muse's endpoint (or the same Ops sink path for the switch), new signing secret. Meta's side changes
   only its secret and the reply call (MCP `send_message` with an agent token, or keep a thin adapter at the Ops
   gateway that turns the Slack-style reply into the MCP call).
3. Test in a second channel, then turn off events on the webhook, remove its request URL, delete the webhook (its old
   messages stay), Ops removes the sink entry. Muse never gets a person's token or Donna's MCP chat access (memory
   #198 rule stays: the agent token is Muse's own, limited).
4. Old webhook messages keep the webhook author; new ones show the bot user. No data migration.

## 5. Reuse

| Need | Reuse |
|---|---|
| Bot user, membership, avatar, @ list, channels, delete/suspend | `agent/profiles.rs` (create, `allowed_in`, delete path) |
| Input queue, thread keys, refusals, task triggers | `agent/inputs.rs`, `agent/task_runs.rs` (route to "external" instead of sidecar) |
| Signed push, SSRF-safe client, delivery log | `chat_routes/interactions.rs::post_signed`, `chat_routes/events.rs::event_payload`, `record_delivery` |
| Encrypted signing secret | `secret_box.rs` |
| OAuth, DCR, PKCE, refresh, consent page, Connected apps | `oauth.rs` (add agent choice + `agent_profile_id`) |
| MCP tool list, scope check, workspace pick | `mcp.rs` (`required_scope`, `tools()`, add `chat:write`) |
| Chat read for MCP | existing `list_messages` / `get_thread` + the new agent filter |

## 6. Phases and size

| Phase | Content | Size |
|---|---|---|
| **1. MVP — pull + identity** | Migration; External agent on the card (name, avatar, channels, projects, scopes, DMs); agent token + OAuth "connect as agent"; `AgentLimits` + IDOR helper in all MCP tools; `wait_for_messages`; `send_message`, `set_status`; inputs routed (DM, mention, task assign/mention); no auto-join; revoke on delete; audit log; docs (`agent.md`, `connect-your-agent.md`, `mcp.md`), aboutOrbit/what's new; tests; browser e2e with Claude Code or a scripted MCP client. | L (≈ 1 build session, 2–3 runs) |
| **2. Push + Muse** | Push URL, signing secret, retries, delivery list on the card; Muse migration with Ops (gateway, sink cleanup). | M |
| **3. Extras** | "Ask before writes" cards; Claude Code channel bridge; `add_reaction`; iPhone/desktop card parity check; maybe OAuth client credentials when clients support it. | M |

## 7. Security checklist

- **Secrets:** agent tokens and OAuth tokens stored as SHA-256 hashes; signing secret encrypted with `ORBIT_APP_KEY`
  (`secret_box`), shown only on "Reveal" to owners/admins; never in logs, `instance` fields or audit details.
- **Replay:** signed push with timestamp + 5 min window; `event_id` stable for dedup; pull acks are per agent and
  per input (an agent cannot ack another agent's input — `agent_user_id` check).
- **Rate limits:** per agent: 60 `send_message` / min, 600 MCP calls / min, one open `wait_for_messages` per
  credential (a second one ends the first), max 20 events per pull; pushes counted per agent.
- **SSRF:** push URL https only, public addresses only (existing `PublicResolver`), no redirects, 3 s, re-resolved on
  each send (DNS rebinding), not editable by members.
- **Authorization:** owner/admin only for create/edit/connect; the consent page checks that the approving person is
  owner/admin of the agent's workspace at grant time and the grant stops if the agent is deleted (bot suspended).
  Connected apps list shows agent grants to owners/admins with **Disconnect**.
- **Data reach:** bot is a normal `member` + agent lists (intersection); private projects only with project
  membership; private channels only when an admin adds them; never other people's DMs; no auto-join.
- **Prompt injection:** members' text reaches an outside model by design. Orbit limits the blast radius (scopes,
  lists, no delete, no `@channel`, rate limits, audit); the card shows a warning "Messages in these channels go to
  <push host / connected client>".
- **Loops:** external agent messages start no agent; task writes by it start no task runs (same as agent sessions).
- **Revoke on delete:** suspend bot, set `revoked_at` on grants and tokens, clear push URL and secret, drop pending
  inputs, pause.

## 8. Not in the MVP

- Push mode and Muse migration (phase 2).
- Per-write approvals, the Claude Code channel bridge, OAuth client credentials, MCP notifications/subscriptions.
- External agents in routines, scheduled messages, Drive "Insert from", Shopify tools.
- Files/images in `send_message` (text only), Block Kit buttons from external agents.
- Cross-workspace agents (one agent = one workspace; a second workspace needs a second agent).
- Billing/metering of external agent use; guest access of any kind.

## 9. Conflicts with open branches

- `agent-codemode-2tools` (commits `cdb24176`, `1625ea46`): `apps/agent/*`, `docs/agent.md`. Small overlap in
  `docs/agent.md` only; the build touches the server, not the sidecar logic. Second merger fixes the doc.
- `agent-sandbox-design`: design only. `donna-codemode-eval`: evals. `owner-names-mobile`, `enter-sends-touch-laptop`:
  no overlap.
- Memory #204 Agentic MCP card (`agentic-mcp-address`, web/mobile Agentic): may touch the same Agentic settings
  files; build after it is merged.

## 10. Decisions for the user

1. Credentials: OAuth "connect as agent" + agent token (recommended) / token only / OAuth only.
2. Writes: no per-write yes, admin permission set is the limit (recommended) / always ask the mentioning member.
3. MVP order: pull + identity first, push + Muse next (recommended) / push + Muse first.
4. Channels: chosen channels, no auto-join (recommended) / allow "all channels" incl. auto-join like internal agents.

Sources: [MCP 2025-11-25 transports](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports.md),
[MCP changelog 2026-07-28](https://modelcontextprotocol.io/specification/2026-07-28/changelog),
[MCP OAuth client credentials extension](https://modelcontextprotocol.io/extensions/auth/oauth-client-credentials.md),
[SEP-1046](https://modelcontextprotocol.io/seps/1046-support-oauth-client-credentials-flow-in-authoriza),
[TypeScript SDK PR #1157](https://github.com/modelcontextprotocol/typescript-sdk/pull/1157),
[Claude Code channels reference](https://code.claude.com/docs/en/channels-reference).
