# Team agent

Orbit has an optional AI agent that every member can use from chat. Mention **@Orbit Agent** in a channel, a thread
or a DM with it, and it does the work with Orbit's own tools (tasks, projects, comments, Docs pages, chat messages).
It answers in the thread and shows its progress while it works. Design and its reasons: `.ai/agent-design.md`.

## What it can and cannot do

- It acts **as the person who gives the instruction** (the driver), with that person's permissions in one workspace.
  Anything that person cannot do, the agent cannot do either.
- It has **no shell, no file system and no delete tool**. Its only way to the internet is the **agent tools** that
  workspace owners and admins set up (see below). It cannot change accounts, API tokens, members, invitations,
  integrations, agent tools, storage or workspace settings: Orbit refuses those routes for agent sessions.
- Each person pays with **their own ChatGPT account** (Plus or Pro). They connect it in **Account settings → AI
  agent** with a device code, so it works on a remote server.
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

People see and search the memory in **Account settings → Agent memory**, and delete wrong entries there (a deleted
memory also drops the summaries built from it; the agent writes them again). The agent itself cannot delete memory.
When a member leaves a workspace, their private memory there is deleted. Design: `.ai/agent-design.md`, "Memory".

## Agent tools (integrations)

Owners and admins can give the agent tools outside Orbit in **Workspace settings → Agent tools**:

- **Remote MCP server**: the server's Streamable HTTP URL (for example Cloudflare's or Shopify's MCP server). Press
  **Test** to read its tools, then turn on the tools the agent may use. Tools the server marks read-only are on at
  first; the others are off until an admin turns them on.
- **HTTP API**: a base URL. The agent gets one tool, `<prefix>__api_request` (method, path, query, JSON body). Every
  request stays below the base URL: Orbit refuses `..`, encoded dots and slashes, other hosts and absolute URLs.
  Choose the allowed methods (GET only at first).

Each integration has **one shared credential**: a header name (default `Authorization`) and its value, for example
`Bearer <token>`. Orbit seals it with `ORBIT_APP_KEY`, never shows it again, and never gives it to the agent
service: Orbit makes every call itself, as the driver of the run. Choose which **roles** (owners, admins, members)
may use each integration. The agent sees the tools as `<prefix>__<tool>`.

**Writes need the driver's approval.** An MCP tool that is not marked read-only, and an HTTP method other than GET
or HEAD, runs only after the agent asked the driver in the thread with the exact action and arguments and the
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
| `AGENT_MODEL` | `gpt-6.1-sol` | The ChatGPT (Codex) model id. |
| `AGENT_MAX_RUNS_PER_USER` | `3` | Runs one person can have going at the same time; more wait. |
| `AGENT_MAX_TOOL_CALLS` | `40` | Tool calls per run. |
| `AGENT_MEMORY_TOKENS` | `3000` | Tokens of memory the agent reads at the start of each run (team and private together; `0` turns it off). It is paid by the driver's ChatGPT account. |
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
  `agent_integration_approvals`. Removing an integration deletes both.
- The agent stores transcripts in `AGENT_DATA_DIR`. It stores no tokens.
- One agent process owns its data directory. Several Orbit servers on one Postgres database can share one agent.

## Development

```bash
just agent        # the agent with the mock model, against `just dev` (API on 8080)
just agent-test   # its tests and typecheck
```
