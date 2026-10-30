# Team agent: design and grilling

Status: built on branch `team-agent` (2026-10-29). This file records the hard questions about the design and the
answers. Read it before you change `apps/agent`, `apps/server/src/agent/` or the agent tables.

## Summary

- **Orbit (Rust)** owns identity, permissions, chat, the input queue (`agent_inputs`), thread state
  (`agent_threads`), the encrypted ChatGPT tokens (`agent_credentials`) and short-lived agent sessions
  (`agent_sessions`). Code: `apps/server/src/agent/`, migration `0062_agent.sql`.
- **The sidecar (Node, `apps/agent`)** runs Pi Durable (`@earendil-works/pi-durable` 1.4.0, `pi-ai` 1.4.0,
  `chord` 1.4.0, exact pins). It owns transcripts and runs. It has no shell, no file tools and no database access. Its
  only tools call Orbit's public API with a bearer session of one user.
- **The agent in chat** is one bot user (named `Orbit Agent` until an installation admin renames it in Admin → AI
  agent; no password, no sessions) with a membership in every workspace. People `@`-mention it like a teammate. It answers in the thread and edits one progress message while it
  works.
- **ChatGPT** sign-in is the OpenAI Codex device-code flow from pi-ai (`openai-codex` provider). It works on a remote
  server: the user opens `auth.openai.com/codex/device` on any device and types the code.

```text
browser ──HTTPS──► Orbit (Rust) ◄──────── internal API (shared secret, loopback) ──────── sidecar (Node, Pi Durable)
                     │  chat send → agent_inputs (same transaction)          ▲ poll inputs, post/edit replies,
                     │                                                        │ mint agent sessions, read/write tokens
                     └── public API ◄── Authorization: Bearer <agent session of the driving user> ── Orbit tools
```

## Grilling: questions and answers

### 1. Auth: how does a tool act "as the user"?

The sidecar asks Orbit for an **agent session** of one user in one workspace
(`POST /api/internal/agent/sessions`). Orbit writes a normal `sessions` row (15 minute absolute life) plus an
`agent_sessions` row, and returns a token with the prefix `orbit-agent.`. Tools send it as
`Authorization: Bearer …`, so every request goes through the same `request_session` path, the same repositories and
the same `policy.rs` rules as the user's browser. No new permission code exists for the agent, so it cannot drift from
the real rules.

Orbit mints a session only when the user is an active (not suspended) member of the workspace and has an active
agent thread in it. The sidecar caches the token until one minute before expiry.

### 2. Permission scoping: what can an agent session NOT do?

`agent::scope` is a layer on the whole router. A bearer token with the `orbit-agent.` prefix:

- must exist in `agent_sessions` (else 401), and
- may only call `/api/v1/workspaces/{its workspace}/…`, and
- may not call these workspace routes: `agent-integrations`, `api-tokens`, `audit`, `github`, `imports`,
  `invitations`, `live`, `restore`, `storage`, `transfer-ownership`, any write on `members`, `DELETE` on
  `agent-memory`, and any write on the workspace itself (`agent::scope::DENIED`).

So the agent cannot change the password, e-mail, 2FA, passkeys or sessions, cannot mint long-lived API tokens, cannot
touch another workspace, and cannot reach the installation admin routes, even when the user is an admin. "Full
access" means the user's full access to the work (tasks, projects, docs, chat), not to the account or the server.
The tools also expose no delete operation in this version.

### 3. Which user's ChatGPT account pays, and whose permissions apply, in a shared thread?

One rule: **every run has exactly one driver**, the author of the message that started the run. Tools act as the
driver, and the driver's ChatGPT account pays for the model calls of that run.

- A message that `@`-mentions the agent (or any message in a DM with the agent) is an **instruction**. It starts a
  run driven by its author. If the thread is busy with another person's run, the instruction waits in the queue until
  that run ends.
- A plain reply (no mention) in a thread where the agent **waits for an answer** is an **answer**. It continues the
  waiting run with the same driver. The teammate who answers does not pay and lends no permissions: the answer is
  only information for the driver's task, and the prompt says so.
- A plain reply from the driver while the run is going is a **steer**. Plain replies from others while a run is going
  are ignored (they can `@`-mention the agent to give an instruction).

Result: nobody can make the agent do more than they can do themselves, and nobody spends another person's
subscription except by answering a question that person's run asked.

### 4. Token storage

- The OAuth credential (access token, refresh token, expiry, account id) is JSON, sealed with ChaCha20-Poly1305 under
  `ORBIT_APP_KEY` (the existing `secret_box`), in `agent_credentials`, one row per user.
- The sidecar never writes tokens to disk. Its pi-ai `CredentialStore` reads and writes through the internal API.
  Refresh runs inside `CredentialStore.modify`, serialized per user in the sidecar process.
- Disconnect deletes the row. Backups contain only ciphertext. A changed app key makes the rows unreadable; the user
  then connects again.

### 5. Remote login (no localhost callback)

The browser calls `POST /api/v1/agent/chatgpt/login`; Orbit asks the sidecar to start pi-ai's `openai-codex`
**device-code** login. The UI shows the code and the link `https://auth.openai.com/codex/device`; the sidecar polls
OpenAI, then saves the credential through Orbit. The UI polls `GET /api/v1/agent/chatgpt` until it says connected.
No callback port is opened, so concurrent logins of many users do not collide (the newer "Sign in with ChatGPT"
flow in pi-ai binds port 1455 and allows one login at a time, so we do not use it).

### 6. Concurrency

- One run at a time per thread (the Pi conversation is the unit of serialization). Inputs of a busy thread stay
  unacknowledged in `agent_inputs` and are tried again; their order is the message order.
- Different threads run in parallel. Each user may have at most 3 running runs (`AGENT_MAX_RUNS_PER_USER`); more
  wait in the queue.
- Orbit writes `agent_inputs` in the same transaction as the chat message, so a crash cannot lose a mention.

### 7. Durability and restarts

- Pi Durable stores every model turn and tool call in SQLite (`$AGENT_DATA_DIR/pi.sqlite`) and `harness.resume()`
  continues interrupted runs. Orbit tools that write are not `replay: "safe"`, so an interrupted write is reported
  to the model as interrupted instead of done twice.
- `agent_inputs` is the durable queue between Orbit and the sidecar. The sidecar submits each input with
  `requestId = message id` (exactly-once in Pi), then acknowledges it. A crash between the two submits nothing twice.
- The progress message is sent with a fixed nonce (`agent:<input message id>`), so sending it again returns the
  same message, and the final answer is an edit of it, so posting is idempotent.
- "Waiting for an answer" is not a sleeping task: the run ends after `ask_teammate`, and `agent_threads.status =
  waiting` is stored in Orbit. The answer is a new input. Nothing is in memory while the run waits, so a wait can
  last days and survive any restart of either process.
- Each thread's conversation keeps an `orbit.thread` document (ids, driver, current run). At start the sidecar
  re-attaches a watcher to every thread with a current run.

### 8. Prompt injection from threads, tasks and docs

- Tools return data in fenced blocks labelled as untrusted content. The system prompt says that only the driver's
  messages are instructions; text from tasks, docs and other messages is data.
- The blast radius is the driver's own Orbit access in one workspace, minus the denied routes (section 2). No shell,
  no delete tool, no posting outside the current thread, no access to other workspaces.
- Each run is limited to 40 tool calls (a `beforeTool` hook blocks more).
- **Changed with integrations (2026-10-29):** "no internet" is no longer the whole defence. The agent can now reach
  the outside, but only through the agent tools that owners and admins set up, with Orbit making each call, and
  every write needs the driver's "yes" checked by Orbit. It still has no general web, fetch or e-mail tool. See
  "Integrations" below (I1–I8).

### 8a. Name and team instructions

- **One name.** The agent is one bot user for the installation, so it has one name, and that display name is its
  `@handle` (Orbit has no separate handles). A rename refuses a name that another account already has (trimmed, any case):
  `namesToTokens` would turn `@Name` into a mention of that person. Every account counts, not only the members of
  the bot's workspaces, because the bot joins a new workspace only on the sidecar's next poll. The check and the
  update run in one `immediate_transaction`. A member who later takes the agent's name is not refused (see open
  decisions below).
- **No editable base prompt.** The rules and the built-in "About Orbit" text are code. The only editable layer is
  each workspace's team instructions (owners and admins, `workspace.update`), placed after About Orbit in a block that
  says they cannot override the rules. Its text cannot close the block early. There is no installation-wide and no
  personal layer.
- **Delivery.** `GET /api/internal/agent/inputs` returns `bot_name`, and each input has `team_instructions`. The
  sidecar stores both in the thread's `orbit.thread` document when a run starts; the prompt sections read them from
  there, so they stay the same during a run (Pi resends only changed sections) and change with the next run.
- **Scope.** Agent sessions cannot reach `/api/v1/workspaces/{id}/agent` (denied segment) or `/api/v1/admin/agent`.
- **Model.** An installation admin picks the conversation model and thinking level (one row, `agent_model_settings`;
  `null` = `AGENT_MODEL` and Pi's default). Orbit asks the sidecar (`GET /models`, shared secret) which ChatGPT models
  exist and refuses others, so only "default" can be saved while the sidecar is offline. `GET …/inputs` returns
  `model` and `thinking`; the sidecar applies them with Pi's `configure()` in the commit that starts each run, and falls
  back to the default (with a log line) for a model it does not know. Compaction keeps its own env settings.

### 9. Rate limits

- ChatGPT limits: pi-ai retries with backoff (`retry.maxRetries = 3`). A run that still fails ends with a message in
  the thread that names the error.
- Orbit limits: agent sessions count against the normal per-credential bucket of the user. All sidecar requests
  come from one IP (loopback), so they share one per-IP budget (`ORBIT_RATE_LIMIT_GENERAL_PER_MINUTE`, default 600).
  Raise it if a large team uses the agent heavily.

### 10. Failure UX

| Case | What the user sees |
|---|---|
| Driver has no ChatGPT connection | The agent answers in the thread: "@you, connect ChatGPT first: <link to Account → AI agent>". The input is consumed. |
| Token refresh fails / revoked | Same message; the credential is marked invalid in the UI. |
| Model or tool error | The progress message becomes "I could not finish: <reason>". Mention the agent again to retry. |
| Sidecar down | Account → AI agent shows "Agent service is offline". Mentions stay queued and run when it is back. |
| Thread busy | The input waits; the progress message of the queued input says "Queued". |

### 11. Deployment and supervision

- The sidecar listens on `127.0.0.1:${AGENT_PORT:-8790}` only. Orbit reaches it with `ORBIT_AGENT_URL`; both share
  `ORBIT_AGENT_SECRET` (or `ORBIT_AGENT_SECRET_FILE`). Without `ORBIT_AGENT_URL` the feature is off: no bot user,
  no routes that do anything, no UI.
- Supervision: `deploy/orbit-agent.service` (systemd, `Restart=always`, `After=orbit.service`) and an `agent`
  service in `deploy/compose.yaml`. The sidecar restarts on its own when Pi's harness fails (`harness.closed`).
- One sidecar process owns the Pi storage (Pi has no cross-process lock). Several Orbit servers on Postgres can share
  it, because the queue and the state are in the Orbit database.

### 12. Upstream merge cost

New files: `apps/agent/**`, `apps/server/src/agent/**`, `apps/server/migrations/0062_agent.sql` (+ Postgres),
`apps/web/src/features/agent/**`, `deploy/orbit-agent.service`, `docs/agent.md`. Touched upstream files, each with a
few lines: `chat/messages.rs` (one call that records agent inputs), `chat_routes/mod.rs` (`publish` is
`pub(crate)`), `router.rs`, `app.rs`, `lib.rs`, `migrate.rs`, `backup.rs` (schema version), the schema-version
tests, the account settings navigation and routes in the web and mobile apps. `apps/agent` is not a Bun workspace
(it has its own `package-lock.json` for Node), so `bun.lock` does not change.

## Decisions that need the owner

- ChatGPT models available through the Codex backend change over time; the default is `gpt-6.1-sol`
  (`AGENT_MODEL`). Pi marks the `openai-codex` provider "legacy" (superseded by Sign in with ChatGPT, which needs a
  localhost callback). If OpenAI turns the Codex device flow off, switch to the paste-the-redirect-URL flow.
- The bot user shows in member lists. Removing it from a workspace turns the agent off there.
- A person can still set their own display name to the agent's name (profile, sign-up, invitations). Then `@Name` in
  the agent's answers is ambiguous. Refusing that everywhere a name is set is not built yet.

## Memory

Status: built on branch `agent-memory` (2026-10-29). The design follows OptMem
(https://github.com/VictorTaelin/OptMem): an append-only log of one-line memories plus a binary tree of one-line
summaries that is only a cache. OptMem has no license, so Orbit reimplements the idea; no code or prompt text is
copied. Code: `apps/server/src/agent/memory.rs` (tree, storage, routes), migration `0063_agent_memory.sql`,
`apps/agent/src/memory.ts` (tools and the injected view), `apps/web/src/features/agent/MemoryPage.tsx`.

### Model

- A **scope** is one memory: the **team memory** of a workspace, or the **private memory** of one person in one
  workspace (`agent_memory_scopes`, `owner_user_id` null for the team).
- A **memory** is one line of at most 280 bytes. It gets the next **position** of its scope (0, 1, 2, …, no gaps),
  and keeps its author (the run's driver), the date, and the chat thread it came from (`agent_memories`).
- A **block** `#lo-hi` is an aligned power-of-two range of positions (`#0-1`, `#2-3`, `#0-3`, …). Its **summary**
  is one line written by the model from the block's two halves (`agent_memory_summaries`). Summaries are a cache:
  deleting one only means the model writes it again.
- The **wake view** is what a run reads at its start: the newest memories word for word and older ones as coarser
  summaries, cut to a byte budget. A block is kept whole when its size is small compared to its age, so detail
  falls off with age. A block without a summary yet is shown as its two halves instead.

### Grilling: questions and answers

#### M1. Who can read and write which memory?

All memory routes are public workspace routes (`/api/v1/workspaces/{id}/agent-memory/{team|private}/…`). The
sidecar calls them with the driver's agent session, and the web app with the person's cookie. So `agent::scope`
and the normal session path apply, and the agent has no way around them.

| | Team memory | Private memory |
|---|---|---|
| Read, search | every member of the workspace | only its owner (`private` always means the caller) |
| Note, summarize | every member (through the agent) | only its owner |
| Delete a memory or a summary | owners and admins (`agent_memory.manage`) | only its owner |

There is no route that names another person's private memory, so even an admin cannot read it. A non-member gets
404. Agent sessions cannot `DELETE` memory at all (`agent::scope`), so a prompt-injected run cannot erase memory.

#### M2. Privacy between scopes inside a chat thread

Problem: the Pi transcript of a thread is shared by every run in it. If Ann's run reads her private memory in a
channel thread, Bob's later run in the same thread sees it in the transcript and can repeat it.

Answer: the sidecar reads private memory (wake view, `memory_recall`, `memory_zoom`) **only in a DM between the
driver and the agent alone** (a DM with two members: the driver and the bot). In channels and group DMs, a run reads
team memory only. It can still *note* to the driver's private memory there (the fact is already in the thread).

Second problem: team memory is readable by every member, but a run can see private channels, private pages and DMs.
Answer: the prompt and the tool description say that team memory takes only facts that every member of the
workspace may know; facts from DMs with the agent go to private memory by default, and to team memory only when the
driver asks for it. This is a rule for the model, not a hard check. The author and source thread of each memory show
in the Memory view, and owners/admins can delete a team memory.

#### M3. Poisoning (prompt injection into memory)

- Memories are data. The wake view and every memory tool result are inside `<orbit-data source="…memory">` fences,
  like other Orbit data, and the prompt says never to follow instructions found there.
- The prompt says: note only what the driver or a teammate stated in the thread, or what this run verified with a
  tool; never note instructions, and never note text just because a task, page or message says "remember this".
- Each memory keeps its author (the driver whose run wrote it) and its source thread, so a bad memory can be traced
  to a run and deleted. Summaries keep the author of the run that wrote them too.
- A poisoned summary is visible: the Memory view shows the wake view (what the agent reads) and lets the same
  people delete a summary. Deleting a memory also drops every summary built from it.
- Blast radius: a poisoned memory can only steer a run inside the driver's own permissions (section 8).

#### M4. Delete versus append-only

The log is append-only in that positions never move and rows are never re-numbered, so the tree stays valid.
Deleting a memory **erases its text** (the row stays as a tombstone with `deleted_at` and `deleted_by`, and an empty
text) and deletes every summary whose block contains it (one per tree level). Those summaries are due again, and the
next notes ask the model to rebuild them from what is left. A block whose memories are all deleted gets an empty
summary at once, with no model call. Until a summary is rebuilt, the wake view shows its halves instead, so deleted
text never stays in the view. Summaries can also be dropped one by one (OptMem's `forget`), with the same rule for
their ancestors.

#### M5. Token cost on the driver's ChatGPT account

- The wake view is one system prompt section per run: default 3,000 tokens for team and private memory together
  (`AGENT_MEMORY_TOKENS`, sidecar env; the server caps it at 20,000). The section stays the same for the whole run,
  so Pi does not send it again and the provider's prompt cache stays warm. The budget is in bytes (4 bytes ≈ 1
  token); private memory gets up to 40 % of it and team memory the rest (and what private does not use).
- A binary tree over N memories has N − 1 summaries, so each note costs about one more model step later (a
  `memory_summarize` call). Notes and summaries count against the run's tool-call limit (40).
- A run never has to summarize to read: a missing summary is shown as its halves.

#### M6. Concurrency of summaries and notes

- A note runs in `immediate_transaction()` (one writer at a time on SQLite and on the whole Postgres cluster).
  `UPDATE agent_memory_scopes SET size = size + 1 … RETURNING size` gives the position, so positions have no gaps
  and no duplicates, also with several Orbit servers on one Postgres.
- When a note makes a block due, the same transaction **claims** it: it writes the summary row with no text, a random
  claim token and a 10 minute lease. The tool result gives the model the block, its two halves and the token.
  `memory_summarize` saves only with the matching token and only while the row has no text, so two runs never write
  the same summary. Another run's note claims the next due block that is not claimed. A lease that ran out (the run
  died) is claimed again by a later note.
- A block is due only when both halves exist (a memory, or a summary with text), smallest blocks first.
- A delete that drops a claimed row makes the late `memory_summarize` a no-op ("dropped; it will be rebuilt").

#### M7. Size limits

- One memory or summary: one line, 1–280 bytes (UTF-8).
- 100,000 memories per scope; a note past that fails with a clear error.
- Search (`memory_recall`): words (all must match, case-insensitive), at most 30 results, newest first.
- The wake view is cut to its byte budget; there is no limit on how old a memory may be.

#### M8. A member leaves, a workspace or a user is deleted

- Removing a member (or leaving) deletes their private memory in that workspace in the same transaction: it is
  about them, and nobody else could read it anyway.
- Their team memories stay: they are work facts of the workspace. The author shows as their name.
- Deleting the workspace deletes all its memory (foreign keys). Deleting a user deletes their private memories and
  sets the author of their team memories to null.

#### M9. Why the database and not files

Several Orbit servers on Postgres share one memory; backups and restore include it; the same permission code reads it.
The sidecar stays stateless for memory.

## Compaction

Status: built on branch `agent-compaction` (2026-10-29). Code: `apps/agent/src/compaction.ts`.

The main list of a DM with the agent is one Pi conversation forever, and a busy thread can grow without end. Pi
Durable compacts on its own, but relative to the model's window: for `gpt-6.1-sol` (500k) only near 451k
(background) and 484k (blocking). Orbit compacts at a fixed size instead, `AGENT_COMPACT_TOKENS` (default 250000):

- **Numbers.** Above 250k tokens of context a compaction starts in the background; the run goes on, and the summary
  is placed at the next turn boundary. Above 270k a request waits for it. The last 40k tokens (at most half the
  threshold) stay word for word, so the recent part of a thread (a few tool results and answers) is not paraphrased.
  These are Pi `modelOverrides` for each model in the catalog whose own threshold is higher; models with a 272k window
  keep Pi's defaults (they already compact near 223k).
- **Why 250k.** Every request of a run sends the whole context again, on the driver's ChatGPT plan. 250k instead of
  450k nearly halves what each request costs and how long it takes, and the 270k blocking point keeps each request
  under the model's 272k price tier (pi-ai lists double input cost above it). Models also use a long context less
  well; a summary plus 40k recent tokens keeps what matters for the next instruction.
- **Who pays.** The summarization request carries the conversation's provider session id, so `routedModels` sends it
  to the payer of the conversation: the driver of the newest run. It is one request of about 210k input tokens (no
  prompt cache) per compaction.
- **Summary model.** A `beforeCompact` hook in the `orbit` extension (every conversation selects it) writes the
  summary with `AGENT_COMPACT_MODEL` (`gpt-6-luna`) and `AGENT_COMPACT_THINKING` (`high`), not with the
  conversation's model. Luna's input costs about 1/20 of Sol's on the driver's plan, and the summary needs reading
  more than reasoning; high thinking keeps its quality. Pi 1.4.0 has no compaction-model setting and does not export
  its summary prompt, so `compaction.ts` has a short equivalent with the same sections and Pi's `instructions`. The
  request has the same session id, so the same payer. When the source plus 30k (thinking and summary) does not fit
  Luna's 272k window, or the request fails after 2 retries, the hook returns nothing and Pi summarizes with the
  conversation's model as before (Pi's output limit there: `0.8 * reserveTokens`, capped at the model's 128k). The
  result, or the fallback, is memoized on the compaction task (`api.memo`), so a hook that runs again after a
  restart does not ask again. Pi does not count a hook's request in `pi.usage`; Orbit does not read it.
- **What is kept.** The summary (Pi's structured prompt: goal, progress, decisions, next steps), the recent 40k, and
  the system prompt: after a compaction Pi writes one complete baseline with every section (`orbit`,
  `orbit-context`, `orbit-memory`) and it leads the context. The memory wake view is read again at each run start,
  so compaction never loses memory. Older entries stay in `pi.sqlite`; only the model's view shrinks.


## Integrations

Status: built on branch `agent-integrations` (2026-10-29). Owners and admins give the agent tools outside Orbit:
**remote MCP servers** (Streamable HTTP) and **HTTP APIs** (base URL + one auth header). No CLI connector and no
shell. Code: `apps/server/src/agent/integrations.rs` (settings, routes, approvals), `apps/server/src/agent/outbound.rs`
(the external calls), migration `0065_agent_integrations.sql`,
`apps/agent/src/integrations.ts`, `apps/web/src/features/agentIntegrations/`.

```text
admin (cookie) ──► /workspaces/{id}/agent-integrations ── sealed credential, roles, tools/methods ──► agent_integrations
sidecar (driver's agent session) ──► /workspaces/{id}/agent-tools[/call] ──► Orbit checks role, tool, thread, approval
                                                                               └──► Orbit ──HTTPS──► MCP server / API
```

### Model

- One **integration** per external service, in one workspace: kind (`mcp` | `http`), name, **slug** (the tool
  prefix), URL, one credential (header name + value, sealed with `ORBIT_APP_KEY` in `secret_box`), the **roles**
  that may use it, and the MCP tools that are on (with a snapshot of the server's `tools/list`) or the HTTP methods
  that are allowed.
- The agent sees `<slug>__<tool>` (MCP) or `<slug>__api_request` (HTTP) in a prompt section that the sidecar reads
  as the driver at the start of each run, and calls them through **one** Pi tool, `use_integration(tool, arguments)`.
  Pi Durable's tools are a static registry stored by name in each conversation; one tool per external tool would
  need extensions installed at run time and present again before every resume, and the shared thread transcript
  would mix the tool sets of different drivers. One fixed tool plus a per-run section avoids both.
- No MCP client crate: Orbit speaks the small part of Streamable HTTP it needs (`initialize`,
  `notifications/initialized`, `tools/list`, `tools/call`, JSON or SSE answers, `Mcp-Session-Id`) on its own
  `reqwest` 0.12 client. rmcp's client transport pulls in `reqwest` 0.13 (a second HTTP and TLS stack) and its own
  connection handling, where Orbit must control address checks, redirects and byte limits.

### Grilling: questions and answers

#### I1. Who can set them up, who can use them?

- Admin routes (`/workspaces/{id}/agent-integrations…`) need `integrations.manage` (owners and admins) and a normal
  session: `agent::scope` denies the segment to agent sessions, and the handler checks again. Others get 403,
  non-members 404.
- Agent routes (`/workspaces/{id}/agent-tools…`) accept only an **agent session of that workspace** (a person's
  cookie gets 403, so nobody can use the shared credential past the agent and its approvals). The call also needs a
  **running** `agent_threads` row for the given conversation and root whose driver is the caller.
- The driver's **current role** must be in the integration's roles, for listing and for every call.

#### I2. Credential leakage

- The credential is sealed at rest; the admin API returns only `secret_set` and the header name. Leaving `secret`
  out of an update keeps it; `""` removes it.
- The sidecar never sees it: it calls Orbit with the driver's agent session; Orbit opens the credential for the one
  external request (`HeaderValue::set_sensitive`, `Debug` redacted) and never logs it.
- Errors given to the agent and the audit log carry no URL query, no body and no credential.
- A credential can still leak through the *service*: a tool that echoes request headers, or a write that stores data
  somewhere public. Admins should use tokens with the smallest scope. One credential is shared by everyone in the
  allowed roles: it acts the same for each of them, and the service sees one identity.

#### I3. Exfiltration through write tools

The new risk: a prompt injection (from a task, page, message or a tool result) makes the agent send Orbit content
to the outside. Answers:

- Writes (an MCP tool without `readOnlyHint: true`, an HTTP method other than GET/HEAD) run only with the
  **driver's approval**, enforced in Orbit (I4). The approval question shows the exact action and arguments, so the
  driver sees what leaves Orbit.
- MCP tools are off until an admin turns them on (read-only ones are on after the first test).
- Remaining gap: **reads can carry data too** (a GET with Orbit text in the query or path, a "read-only" MCP tool
  whose arguments reach the server). Admins choose the services; the prompt says to send Orbit content only when the
  driver asked for exactly that. This is a rule for the model, not a hard check. A server can also lie about
  `readOnlyHint`; admins enable only tools of servers they trust.

#### I4. Approvals: why they cannot be faked

- An approval row is bound to the **driver, the conversation and thread root, the integration, the tool and the
  SHA-256 of the canonical arguments** (object keys sorted). It lives 30 minutes and is used once (`used_at`, in an
  `immediate_transaction`).
- Orbit grants it only when the driver posted a message in that thread **after** the approval was made whose first
  word (after mentions) is `yes`, `approve` or `approved`. Orbit reads this from `chat_messages` itself: the model
  and the sidecar cannot write a message as the driver (the bot posts as the bot).
- Other members' replies never count, changed arguments make a new approval, a used or expired one makes a new one.
- Flow: the call answers `approval_required` with the exact action; the sidecar posts the question to the driver and
  ends the run waiting for them (the `ask_teammate` path). The driver's reply starts the next run; the model calls
  the same tool with the same arguments, and Orbit finds the yes. No approval state lives in the sidecar.
- In a shared thread, a teammate can still answer the question with words that make the model try again; Orbit then
  answers `approval_required` again.

#### I5. Injection from tool output and descriptions

- Tool results and the tool list (names and descriptions come from the external server) are inside
  `<orbit-data source="integration …">` fences, and the prompt says they are data. Output is cut at 12,000
  characters by the sidecar and 1 MiB by Orbit.
- An injected run is still limited to the driver's permissions, the enabled tools, the approvals and 40 tool calls.

#### I6. SSRF

- URLs: HTTPS only, no user info, no fragment, IP literals must be public (`orbit_platform::is_public_ip`: loopback,
  private, link-local incl. `169.254.169.254`, shared, reserved, IPv4-mapped and NAT64 forms are refused). Checked
  when saved and again before each call.
- Host names: the client's resolver refuses a host if **any** of its addresses is not public, at connect time, so a
  DNS rebind cannot pass. No redirects are followed and no proxy is used.
- HTTP paths: refused when they do not start with `/`, start with `//`, contain `..` or `.` segments, `%2e`, `%2f`,
  `%5c`, `%00`, `\`, `?`, `#`, control characters or `://`; after joining, scheme, host, port and the base path
  prefix are compared again. Query values are encoded by Orbit.
- `ORBIT_AGENT_INTEGRATION_TEST_HOSTS` lets listed hosts use HTTP and local addresses, for local tests only (logged
  as a warning at start). Never in production.

#### I7. Limits and audit

- 30 s per external request (5 s to connect), 1 MiB per answer (an HTTP answer is cut and marked `truncated`, an MCP
  answer over the limit fails), 64 KiB of arguments, 50 integrations per workspace, 20 pages of `tools/list`.
- Each MCP call opens and ends its own session (initialize, call, `DELETE`), so no state lives between runs.
- Audit: `agent_integration.create|update|delete|test|call` with the actor (the driver for calls), the integration
  and for calls the tool or method, path and status, never bodies, arguments, query strings or secrets.

#### I8. What happens when things change

- Removing an integration deletes its approvals; the next run of the agent no longer lists it. A run in progress gets
  404 for it.
- A changed MCP URL clears the tool snapshot and the enabled tools until the next test. A tool that disappears from
  the server at a test is turned off.
- A role change of a member applies at the next call. A changed app key makes the credential unreadable: the test
  and the calls say so, and an admin enters it again.
