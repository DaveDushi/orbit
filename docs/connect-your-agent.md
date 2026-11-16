# Connect your agent to Orbit

You can use Orbit from your own AI agent (Codex CLI, Claude Code or another MCP client) on your own computer. The
agent reads and writes tasks, docs, chat and Drive with your permissions, only in the workspaces and projects that you
approve.

**You need:**

- A member account (owner, admin or member). Guests cannot connect agents.
- The **MCP address** of your Orbit. When Orbit is behind a login gateway, the administrator sets a separate public
  address for agents (`ORBIT_PUBLIC_MCP_URL`, see [Operations](operations.md#configuration)), for example
  `https://orbit-hooks.example.com/mcp`. Without that setting, use `<your Orbit address>/mcp`. Note: the commands
  in **Settings → Agentic** show the main Orbit address. Replace it with the MCP address.
- A browser on the same computer, signed in to Orbit. The agent opens Orbit's consent page there.

The agent does not use a password or an API token. It signs in with OAuth: you approve it once in the browser, and
the agent keeps the token.

## Codex CLI

```bash
codex mcp add orbit --url https://orbit-hooks.example.com/mcp
codex mcp login orbit
```

1. `codex mcp login` opens your browser on Orbit's consent page. If it does not, open the link that it prints.
2. Select the workspaces and the projects that the agent can use. Select the optional permissions (for example,
   write tasks or read chat). Then click **Allow**.
3. The browser shows that the sign-in is done. Go back to the terminal.
4. Test it: `codex exec "Use orbit to list my open tasks"`, or enter `/mcp` in a Codex session.

When Codex runs on a remote computer (SSH), use `codex mcp login orbit --no-browser`. Open the printed link in
your own browser, select **Client runs on a remote server** on the consent page, click **Allow**, and paste the
callback URL that Orbit shows into the terminal within one minute. See the
[Codex MCP documentation](https://developers.openai.com/codex/mcp/).

## Claude Code

```bash
claude mcp add --scope user --transport http orbit https://orbit-hooks.example.com/mcp
```

Start `claude`, enter `/mcp`, select **orbit**, then **Authenticate**. Approve the access in the browser as in
step 2 of the Codex section above. Test it: ask "Use orbit to list my open tasks".

## Other MCP clients

Use the Streamable HTTP transport and the MCP address. Orbit follows the
[MCP authorization specification](https://modelcontextprotocol.io/specification/2025-11-25/basic/authorization):

- `/mcp` answers `401` with `WWW-Authenticate: Bearer resource_metadata="…"`. The client finds the protected resource
  metadata and the authorization server metadata on the MCP address.
- OAuth 2.1 authorization code flow with PKCE (`S256`), dynamic client registration (`/oauth/register`), and public
  clients only (`token_endpoint_auth_method: none`).
- Callback addresses: `https://…`, or `http://127.0.0.1`, `http://[::1]` or `http://localhost` on any port.
- The client must send `resource=<MCP address>` (RFC 8707). A token works only on the address that it was issued for.
- Scopes: `tasks:read` always, plus `tasks:write`, `docs:read`, `docs:write`, `chat:read`, `drive:read` and
  `drive:write` if you approve them.

## Good to know

- An access token is valid for 15 minutes. The agent renews it automatically for 30 days after its last use. After
  that, log in again (`codex mcp login orbit`, or **Authenticate** in Claude Code).
- To stop an agent, open **Settings → Agentic → Connected apps** and click **Disconnect**. If you leave a
  workspace or become a guest, the connection stops for that workspace.
- A connection that you made on the main Orbit address continues to work there, but not on the public MCP address.
  Connect again with the MCP address.
- On the public MCP address, only OAuth works. Workspace API tokens work only on the main Orbit address.
- The tools are described in [Orbit MCP](mcp.md#tools).
