# Team agent

Orbit has an optional AI agent that every member can use from chat. Mention it (**@Orbit Agent** until an admin
renames it) in a channel, a thread or a DM with it, and it does the work with Orbit's own tools (tasks, projects,
comments, Docs pages, chat messages). It answers in the thread and shows its progress while it works. Design and its
reasons: `.ai/agent-design.md`.

## What it can and cannot do

- It acts **as the person who gives the instruction** (the driver), with that person's permissions in one workspace.
  Anything that person cannot do, the agent cannot do either.
- It has **no shell, no file system and no delete tool**. Its only way to the internet is the **agent tools** that
  workspace owners and admins set up (see below). It cannot change accounts, API tokens, members, invitations,
  integrations, agent tools, storage or workspace settings: Orbit refuses those routes for agent sessions.
- Each person pays with **their own ChatGPT account** (Plus or Pro). They connect it in **Account settings → AI
  agent** with a device code, so it works on a remote server.
- It reads files on Docs pages (`read_file`: spreadsheets, csv, tsv, text, PDF, Word, in parts) and changes csv, tsv and
  text files and cells of xlsx files (`write_file`; formula cells are read-only). It cannot change xls or PDF files, and
  it has no OCR for scans. File content is
  data and never instructions. A write needs the version the agent read, is recorded with the driver's name, and keeps
  the old version, which a person can restore in the file panel.
- When it needs information, it asks a teammate in the thread (`@`-mention) and stops. The next reply in the thread
  continues the run, as the original driver.
- Runs are durable: a restart of Orbit or of the agent continues them.
- It has a **long-term memory** (see below).

## Memory

The agent keeps one-line memories in Orbit's database, in two scopes:

- **Team memory**, one per workspace: decisions, conventions, project facts, outcomes. Every member can read it;
  owners and admins can delete from it.
- **Private memory**, one per person per workspace: that person's preferences and context. Only they can read it,
  and the agent reads it only in a direct message between them and the agent (other threads are shared with other
  people). Facts from a DM go to private memory unless the person asks for team memory.

At the start of each run the agent reads a short view of both: recent memories word for word, older ones as one-line
summaries that it writes itself. It can search all memories and open a summary. Memories are data for the agent,
never instructions, and each one keeps its author and source thread.

People see and search the memory in **Agentic → You**, and delete wrong entries there (a deleted
memory also drops the summaries built from it; the agent writes them again). The agent itself cannot delete memory.
When a member leaves a workspace, their private memory there is deleted. Design: `.ai/agent-design.md`, "Memory".

## Name, model and instructions

| Setting | Where | Who | Notes |
|---|---|---|---|
| Name | Agentic → Admin (`/agentic/admin`, `PUT /api/v1/admin/agent`) | Installation admins | One name for the installation: it is the bot user's display name, so it is also the agent's `@handle`. 2–40 characters (letters, digits, single spaces, `- _ . '`). Orbit refuses a name that another account already has (trimmed, any case), because `@Name` would then reach that person. Messages and earlier mentions show the new name. |
| Model and thinking level | Agentic → Admin (`PUT /api/v1/admin/agent/model`) | Installation admins | One choice for the installation, stored in `agent_model_settings`. The model list comes from the sidecar (its ChatGPT (Codex) models); Orbit refuses any other model, and a model can only be saved while the sidecar answers. Thinking: `minimal`, `low`, `medium`, `high`, `xhigh` or `max`. **Default** (`null`) is `AGENT_MODEL` and Pi's default thinking level. Compaction keeps `AGENT_COMPACT_MODEL`/`AGENT_COMPACT_THINKING`. |
| Team instructions | Agentic → Workspace (`/agentic/workspace`, `PUT /api/v1/workspaces/{id}/agent`) | Owners and admins write, every member reads | Plain text, at most 4,000 characters, stored in `agent_team_instructions`. |

The system prompt of a run has, in this order: the rules (with the current name), the built-in **About Orbit**
section (`apps/agent/src/prompt.ts`: Orbit's features, short how-to answers, what the agent can and cannot do; not
editable), the workspace's team instructions in a `<team-instructions>` block that says they cannot override the rules,
then the thread's context. The sidecar reads the name, the model choice and the team instructions with each queued
message and keeps them for the run, so a change applies to the next run without a restart (a run already going keeps
its model). If the saved model is not one this sidecar knows (another version), the run uses `AGENT_MODEL` and the
sidecar logs a warning. Agent sessions cannot read or change these settings. With `AGENT_MOCK=1`, the instruction
`show prompt` answers with the current system prompt, and `show model` with the run's model and thinking level.

## Agent tools (integrations)

Owners and admins can give the agent tools outside Orbit in **Agentic → Workspace**:

- **Remote MCP server**: the server's Streamable HTTP URL (for example Cloudflare's MCP server). Press
  **Test** to read its tools, then turn on the tools the agent may use. Tools the server marks read-only are on at
  first; the others are off until an admin turns them on.
- **HTTP API**: a base URL. The agent gets one tool, `<prefix>__api_request` (method, path, query, JSON body). Every
  request stays below the base URL: Orbit refuses `..`, encoded dots and slashes, other hosts and absolute URLs.
  Choose the allowed methods (GET only at first).

Each integration has **one shared credential**: a header name (default `Authorization`) and its value, for example
`Bearer <token>`. Orbit seals it with `ORBIT_APP_KEY`, never shows it again, and never gives it to the agent
service: Orbit makes every call itself, as the driver of the run. Choose which **roles** (owners, admins, members)
may use each integration. The agent sees the tools as `<prefix>__<tool>`.

**OAuth client credentials (HTTP APIs).** For an API whose tokens expire, choose **OAuth client credentials** in the
form: token URL, client ID, client secret, the header for the token (default `Authorization`) and an optional test
path. Orbit sends `grant_type=client_credentials` with the client ID and secret to the token URL and uses the token
from the answer. In `Authorization` the value is `Bearer <token>`; in another header, the bare token. A fixed
credential and an OAuth client exclude each other: setting one clears the other. MCP servers cannot use OAuth clients.

- Orbit reads `expires_in` from the answer and renews 60 seconds before expiry. If `expires_in` is missing or at most
  60, the token serves one call only. The token stays in the memory of each server, so a restart asks for a new one.
- On a 401 from the API, Orbit gets a new token and retries once, also for a write that the driver approved (no second
  approval). It does not retry on a 403 or on a failed token request.
- The token request has the same limits as other calls (below), and a 3xx is a failure. An error shows only the HTTP
  status and a short OAuth error code, such as `shop_not_permitted`.
- **A new base URL or token URL needs the client ID and secret again.** So an admin who cannot read the secret cannot
  send it, or a token, to another host. The token URL can be on another host than the base URL.

**Test** does a GET of the optional test path, which stays below the base URL. An OAuth integration always gets a new
token. With no test path, Test reports that the token request worked and does not call the API. With a test path, an
answer below 400 is a pass; otherwise Test shows the status and says that the token worked but the API refused. A
fixed credential with no test path does a GET of the base URL.

**Example: Shopify Admin API.** Your app and the store must be in the same Shopify organization, or the token request
fails with `shop_not_permitted`.

| Field | Value |
|---|---|
| Base URL | `https://STORE.myshopify.com/admin/api/2026-10` |
| Token URL | `https://STORE.myshopify.com/admin/oauth/access_token` |
| Token header | `X-Shopify-Access-Token` |
| Test path | `shop.json` |

The form fills in the last three when the base URL host ends in `.myshopify.com`. Shopify's tokens end after 24 hours;
Orbit renews them. Shopify's GraphQL Admin API uses POST also for reads, and its REST Admin API is legacy. To use
GraphQL with no question for each read, allow POST and turn on **Run GraphQL reads without approval** (below). The form
fills in the path `/graphql.json`, but the switch stays off until you turn it on.

**GraphQL reads without approval (HTTP APIs).** This setting is off by default, per integration. When it is on, a POST
to the one exact **GraphQL path** runs without the driver's approval only if Orbit's own GraphQL parser finds a read in
the body that it sends. It does not trust what the agent says. All these cases still need approval:

- the body is not a JSON object, the query is missing or is not a string, or the query does not parse;
- the document has more than one operation, or an operation that is not a query, or a directive on the operation;
- a fragment is on `Mutation` or `Subscription`, or the document has schema definitions;
- the body has a key other than `query`, `variables` and `operationName` (for example a persisted query id), or
  `operationName` is not the name of the operation;
- the request has a URL query, or goes to any other path. Every other POST needs approval as before.

Each such call is in the audit log with `graphql_read_without_approval: true` and `approved: false`, and with no body.
Roles do not change: only the roles you chose can use the integration.

**Writes need the driver's approval.** An MCP tool that is not marked read-only, and an HTTP method other than GET
or HEAD (except a GraphQL read with the setting above), runs only after the agent asked the driver in the thread with the exact action and arguments and the
driver answered **yes**. Orbit checks this itself: the approval is bound to the driver, the thread, the tool and the
arguments, works once, and ends after 30 minutes. A teammate's "yes" does not count.

Limits: HTTPS only; Orbit refuses hosts that resolve to loopback, private, link-local or metadata addresses, follows
no redirects, waits at most 30 seconds and reads at most 1 MiB of an answer. Every external call is in the audit log
(`agent_integration.call`: tool, method, path, status; no bodies, no secrets).

**Risks to know.** Whatever the agent reads (tasks, pages, messages, tool results) could carry instructions written
by someone else. The agent is told to treat it as data, but it is a language model. A write tool can send Orbit
content out of Orbit; the approval step shows you exactly what goes out, so read it before you say yes. Give the
agent credentials with the smallest scope the service offers (read-only tokens where you can), and turn on only the
tools you need. A shared credential acts the same for every member who may use it: choose the roles with care.
**GraphQL reads without approval can return customer data** (orders, names, addresses) with no question to anyone. The
result goes to the agent, and from there into the thread and the model provider. The data is what the credential's
scopes allow, so give it only the scopes you want the agent to read. Orbit checks the query, not the service: if a
service runs a side effect from a query, Orbit cannot see it.

## Running it

The agent is a small Node service (`apps/agent`, Node 22.19 or later) next to Orbit. Orbit calls it on loopback, and
it calls Orbit back. Both sides share one secret.

| Orbit setting | Meaning |
|---|---|
| `ORBIT_AGENT_URL` | The agent's address, for example `http://127.0.0.1:8790`. Without it the agent is off. |
| `ORBIT_AGENT_SECRET` or `ORBIT_AGENT_SECRET_FILE` | The shared secret, at least 32 characters (`openssl rand -hex 32`). |

| Agent setting | Default | Meaning |
|---|---|---|
| `ORBIT_URL` | (required) | Orbit's address as the agent reaches it, for example `http://127.0.0.1:8000`. |
| `ORBIT_PUBLIC_URL` | `ORBIT_URL` | Orbit's public address, for links in messages. |
| `ORBIT_AGENT_SECRET` / `_FILE` | (required) | The same secret as Orbit's. |
| `ORBIT_FORWARDED_PROTO` | none | Set `https` when Orbit runs in production on loopback: Orbit accepts the header only from a trusted proxy address (`ORBIT_TRUSTED_PROXIES` must contain the agent's address, for example `127.0.0.1/32`). |
| `AGENT_HOST`, `AGENT_PORT` | `127.0.0.1`, `8790` | Where the agent listens. Keep it on loopback or a private network. |
| `AGENT_DATA_DIR` | `./data` | Pi Durable's SQLite file (`pi.sqlite`): transcripts and runs. Back it up with Orbit's data. |
| `AGENT_MODEL` | `gpt-6.1-sol` | The ChatGPT (Codex) model id of runs while Agentic → Admin has **Default**. |
| `AGENT_MAX_RUNS_PER_USER` | `3` | Runs one person can have going at the same time; more wait. |
| `AGENT_MAX_TOOL_CALLS` | `40` | Tool calls per run. |
| `AGENT_MEMORY_TOKENS` | `3000` | Tokens of memory the agent reads at the start of each run (team and private together; `0` turns it off). It is paid by the driver's ChatGPT account. |
| `AGENT_COMPACT_TOKENS` | `250000` | Tokens of context at which a conversation (a thread, or the main list of a DM with the agent) is summarized in the background; a request waits for the summary 20000 tokens above. Applies to each model whose own Pi threshold is higher. `0` keeps Pi's defaults. Older entries stay in `pi.sqlite`. |
| `AGENT_COMPACT_MODEL` | `gpt-6-luna` | The ChatGPT (Codex) model that writes compaction summaries, paid by the driver like the run. Empty: the conversation's model. When the old part of the conversation does not fit this model's window (with 30000 tokens for thinking and the summary) or its request fails, the conversation's model writes the summary. |
| `AGENT_COMPACT_THINKING` | `high` | Thinking level for the compaction summary: `minimal`, `low`, `medium`, `high`, `xhigh` or `max`. |
| `AGENT_MOCK` | off | `1`: a scripted model and a fake ChatGPT sign-in, for tests without OpenAI. Never in production. |

| Orbit test setting | Meaning |
|---|---|
| `ORBIT_AGENT_INTEGRATION_TEST_HOSTS` | Comma-separated hosts that agent tools may reach over plain HTTP and on local addresses, for local tests with fake servers (for example `127.0.0.1`). **Never set it in production**: it turns off the address check for those hosts. Orbit logs a warning when it is set. |

### systemd

```bash
sudo useradd --system --home /var/lib/orbit-agent orbit-agent
sudo cp -r apps/agent /opt/orbit-agent && cd /opt/orbit-agent && sudo npm ci --omit=dev
secret=$(openssl rand -hex 32)
echo "ORBIT_AGENT_URL=http://127.0.0.1:8790" | sudo tee -a /etc/orbit/orbit.env
echo "ORBIT_AGENT_SECRET=$secret" | sudo tee -a /etc/orbit/orbit.env
printf 'ORBIT_URL=http://127.0.0.1:8000\nORBIT_PUBLIC_URL=https://orbit.example.com\nORBIT_FORWARDED_PROTO=https\nORBIT_AGENT_SECRET=%s\n' "$secret" | sudo tee /etc/orbit/agent.env
sudo chmod 600 /etc/orbit/agent.env
sudo cp deploy/orbit-agent.service /etc/systemd/system/ && sudo systemctl daemon-reload
sudo systemctl enable --now orbit-agent && sudo systemctl restart orbit
```

`ORBIT_TRUSTED_PROXIES` must contain `127.0.0.1/32` (it does when a local nginx or Caddy is the proxy).

### Docker Compose

Set `ORBIT_AGENT_SECRET` in `deploy/.env` and start the `agent` profile:

```bash
docker compose -f deploy/compose.yaml --profile agent up -d --build
```

## Data

- Orbit stores each person's ChatGPT credential sealed with `ORBIT_APP_KEY` (`agent_credentials`). Without an app key
  nobody can connect. Disconnecting deletes it.
- Agent tools and their sealed credentials are in `agent_integrations`; the driver's approvals of writes in
  `agent_integration_approvals`. Removing an integration deletes both. An OAuth client ID and secret are sealed
  together in `oauth_sealed_client`, apart from `sealed_secret`. OAuth access tokens are never written to the
  database: they stay in the memory of each server.
- The agent stores transcripts in `AGENT_DATA_DIR`. It stores no tokens.
- One agent process owns its data directory. Several Orbit servers on one Postgres database can share one agent.

## Development

```bash
just agent        # the agent with the mock model, against `just dev` (API on 8080)
just agent-test   # its tests and typecheck
```
