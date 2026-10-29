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
- may not call these workspace routes: `api-tokens`, `service-accounts`, `integrations`, `members`, `invitations`,
  `transfer-ownership`, `export`, `imports`, `billing`, `storage`, `audit`, and `DELETE` on the workspace itself.

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
  no network tool, no delete tool, no posting outside the current thread, no access to other workspaces.
- Each run is limited to 40 tool calls (a `beforeTool` hook blocks more).
- The agent cannot send what it reads outside Orbit: it has no web or e-mail tool.

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
