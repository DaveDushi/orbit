/** The scoped MCP endpoint of the Orbit server at `origin` (docs/mcp.md). */
export function mcpUrl(origin: string): string {
  return `${origin.replace(/\/+$/, '')}/mcp`
}

/** The terminal commands that add the Orbit MCP server to each supported client, in the order docs/mcp.md shows them. */
export function mcpClientCommands(url: string): { client: string; command: string; signIn: string }[] {
  return [
    { client: 'Claude', command: `claude mcp add --scope user --transport http orbit ${url}`, signIn: 'Start Claude, enter /mcp, select orbit and authenticate.' },
    { client: 'Codex', command: `codex mcp add orbit --url ${url}\ncodex mcp login orbit`, signIn: 'The login command opens Orbit in your browser.' },
    { client: 'Grok', command: `grok mcp add --scope user --transport http orbit ${url}`, signIn: 'Start Grok, enter /mcps and authenticate.' },
  ]
}

/** The guide for Codex CLI and Claude Code (docs/connect-your-agent.md). */
export const CONNECT_AGENT_GUIDE_URL = 'https://github.com/DaveDushi/orbit/blob/main/docs/connect-your-agent.md'

/**
 * The address and commands that Settings → Agentic shows. With `mcpAddress` (`ORBIT_PUBLIC_MCP_URL`, for a public origin
 * behind a login gateway) they use that address, for the two clients the guide covers. Without it, `origin`.
 */
export function mcpSetup(origin: string, mcpAddress?: string | null): { url: string; commands: ReturnType<typeof mcpClientCommands>; guide: string | null } {
  if (!mcpAddress) {
    const url = mcpUrl(origin)
    return { url, commands: mcpClientCommands(url), guide: null }
  }
  return {
    url: mcpAddress,
    commands: [
      { client: 'Codex', command: `codex mcp add orbit --url ${mcpAddress}\ncodex mcp login orbit`, signIn: 'The login command opens Orbit in your browser.' },
      { client: 'Claude Code', command: `claude mcp add --scope user --transport http orbit ${mcpAddress}`, signIn: 'Then, in Claude Code: /mcp → Authenticate.' },
    ],
    guide: CONNECT_AGENT_GUIDE_URL,
  }
}

/** `GET /api/v1/oauth/grants`: the MCP clients the signed-in person approved, and the separate MCP address (or null). */
export type OAuthConnections = { items: OAuthConnection[]; mcp_url?: string | null }

/** An MCP client the signed-in person approved (`GET /api/v1/oauth/grants`). */
export type OAuthConnection = { id: string; client_name: string; workspace_name: string; project_names: string[]; all_projects: boolean; scope: string; created_at: number }
