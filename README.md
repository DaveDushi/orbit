# Orbit

> PRETTY ALPHA, DO NOT USE IT IN PROD.

Orbit is a self-hosted workspace application for projects, tasks, documents,
mail, and team chat. It uses a Rust server, a React web client, and SQLite. The
production binary serves both the API and the embedded web application.

## Development

You need Rust 1.97.1, Bun 1.3.14 or later, and [Just](https://just.systems/).

```bash
just setup
just dev
```

Open <http://127.0.0.1:8888>. Run `just test` for the test suites or `just
check` for all project checks.

## MCP setup

Orbit provides three read-only task tools: `list_projects`, `list_tasks`, and
`get_task`. Connect with **Streamable HTTP** and **OAuth**. No external OAuth
provider, client secret, or API token is required.

### Claude

Start Orbit with `just dev`, then add the server:

```bash
claude mcp add \
  --scope user --transport http orbit http://127.0.0.1:8888/mcp
```

Install the CLI for your selected backend and make it available on your `PATH`.

1. Start the Claude CLI and enter `/mcp`.
2. Select **orbit** and authenticate. Sign in to Orbit, select a workspace
   and projects, then click **Allow read access**.
3. In Jean, open **Settings → MCP Servers** and enable **orbit** under
   **Claude**. Start a new Claude session.
4. Ask: **“Use orbit to list my projects and tasks.”**

### Codex

With Orbit running locally:

```bash
codex mcp add orbit --url http://127.0.0.1:8888/mcp
codex mcp login orbit
```

Complete Orbit sign-in and project approval in your browser. Use `/mcp` in the
Codex terminal to inspect the connection. In Jean, enable **orbit** under
**Settings → MCP Servers → Codex**, then start a new Codex session.

For configuration details, see the [official Codex MCP documentation](https://developers.openai.com/codex/mcp/).

### Grok

With Orbit running locally:

```bash
grok mcp add --scope user --transport http orbit http://127.0.0.1:8888/mcp
grok mcp doctor orbit
```

Use `/mcps` in the Grok terminal to inspect the server and complete authentication
when requested. In Jean, enable **orbit** under
**Settings → MCP Servers → Grok**, then start a new Grok session.

If your Grok version cannot complete OAuth, use a read-only Orbit API token with
the `--header "Authorization: Bearer <token>"` option when adding the server.
Use user scope, not a shared project configuration, and never commit the token.
See [API-token setup](docs/mcp.md#api-token-alternative).

Each backend has its own server configuration and credentials. Adding Orbit to
Claude does not configure Codex or Grok.

### Remote server through Tailscale

The public origin must match the HTTPS address used by the MCP client. For this
repository's development server, configure Jean's Run command as follows:

```bash
ORBIT__HTTP__PUBLIC_ORIGIN=https://devserver.tail661ee3.ts.net:18888 \
ORBIT_DEV_API_PORT=18080 ORBIT_DEV_WEB_PORT=18888 just dev
```

Replace the hostname with your server's Tailscale hostname. Stop the old Run
environment before starting this command. A Tailscale HTTPS proxy must already
forward port 18888 to `http://127.0.0.1:18888`; setting the public origin does
not enable HTTPS. Use the HTTPS address for Orbit sign-in too.

Add the HTTPS endpoint to your selected backend:

```bash
claude mcp add --scope user --transport http orbit https://devserver.tail661ee3.ts.net:18888/mcp

codex mcp add orbit --url https://devserver.tail661ee3.ts.net:18888/mcp
codex mcp login orbit

grok mcp add --scope user --transport http orbit https://devserver.tail661ee3.ts.net:18888/mcp
grok mcp doctor orbit
```

No fixed callback port is required. Let the client choose its callback port.
If the CLI runs on a remote server and your browser runs on your computer,
the browser must still reach the CLI's loopback callback. Forward the port
shown in the callback URL through SSH, or use an existing remote callback
forwarding method. Tailscale access to Orbit alone does not forward this
callback. If remote OAuth cannot be completed, use the read-only API-token
alternative.

### Disconnect and troubleshooting

- To revoke access, open **Settings → Sessions → Connected apps** in Orbit and
  disconnect the client.
- If the protected-resource address does not match, correct
  `ORBIT__HTTP__PUBLIC_ORIGIN`, restart Orbit, then authenticate again.
- If tool discovery fails after a server update, reconnect the server in
  Claude's `/mcp` menu or start a new Claude session.

For API-token connections, supported clients, and protocol details, see
[docs/mcp.md](docs/mcp.md).

For deployment and operator instructions, see [docs/operations.md](docs/operations.md).
For GitHub webhook setup and current limits, see [docs/github.md](docs/github.md).
The project is licensed under the [Apache License 2.0](LICENSE).
