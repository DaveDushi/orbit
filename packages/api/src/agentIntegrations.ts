/**
 * The team agent's tools outside Orbit (`/api/v1/workspaces/{id}/agent-integrations`): remote MCP servers and HTTP
 * APIs that owners and admins set up. Not in the OpenAPI contract, like the other agent routes; see
 * `.ai/agent-design.md`, "Integrations".
 */
import type { Client } from './generated/client/index'

export type AgentIntegrationKind = 'mcp' | 'http'
export type AgentIntegrationRole = 'owner' | 'admin' | 'member'
export type AgentIntegrationMethod = 'GET' | 'HEAD' | 'POST' | 'PUT' | 'PATCH' | 'DELETE'

/** An MCP server's tool, as the last test found it. */
export interface AgentIntegrationTool {
  name: string
  title: string | null
  description: string
  /** The server marks it read-only: it runs without the driver's approval. */
  read_only: boolean
  /** The agent may use it. */
  enabled: boolean
}

export interface AgentIntegration {
  id: string
  /** The prefix of the agent's tool names: `<slug>__<tool>`. */
  slug: string
  name: string
  kind: AgentIntegrationKind
  url: string
  auth_header: string | null
  /**
   * `header`: a fixed credential. `oauth`: Orbit gets the token with a client ID and secret. `connect`: an admin signed
   * in to the MCP server (OAuth) with Connect.
   */
  auth_type: 'none' | 'header' | 'oauth' | 'connect'
  /** MCP: the shared sign-in, after Connect. */
  connection: AgentIntegrationConnection | null
  oauth_token_url: string | null
  /** HTTP: what Test does a GET of, below the base URL. */
  test_path: string | null
  /** HTTP: the one path where a GraphQL query can run without the driver's approval. */
  graphql_read_path: string | null
  /** Off by default. Mutations and other POST paths always need approval. */
  graphql_reads_enabled: boolean
  /** A secret or an OAuth client is saved. Orbit never sends them back. */
  secret_set: boolean
  allowed_roles: AgentIntegrationRole[]
  allowed_methods: AgentIntegrationMethod[]
  /** MCP: `null` until the first successful test. */
  tools: AgentIntegrationTool[] | null
  tools_checked_at: number | null
  created_at: number
  updated_at: number
}

/** An MCP server's sign-in. Orbit keeps the tokens; the browser only sees this. */
export interface AgentIntegrationConnection {
  /** Who signed in, when the server told. */
  account: string | null
  connected_at: number | null
  /** When an admin must connect again. `null`: the server did not say. */
  grant_expires_at: number | null
  /** A token refresh was refused: only Reconnect helps. */
  needs_reconnect: boolean
}

export type AgentIntegrationConnectStart = { ok: true; authorize_url: string } | { ok: false; detail: string }

export interface AgentIntegrationList {
  items: AgentIntegration[]
  /** The server has an app key, so it can keep secrets. */
  can_keep_secrets: boolean
}

/**
 * A change. Leave `secret` out to keep the saved one; `''` removes it. A fixed secret and an OAuth client exclude each
 * other. A new `url` or `oauth_token_url` of an OAuth integration needs `oauth_client_id` and `oauth_client_secret` again.
 */
export interface AgentIntegrationInput {
  name?: string
  slug?: string
  kind?: AgentIntegrationKind
  url?: string
  auth_header?: string
  secret?: string
  oauth_token_url?: string
  oauth_client_id?: string
  oauth_client_secret?: string
  test_path?: string
  graphql_read_path?: string
  graphql_reads_enabled?: boolean
  allowed_roles?: AgentIntegrationRole[]
  enabled_tools?: string[]
  allowed_methods?: AgentIntegrationMethod[]
}

export interface AgentIntegrationTest {
  ok: boolean
  detail: string
  integration: AgentIntegration
}

const base = (workspaceId: string) => `/api/v1/workspaces/${workspaceId}/agent-integrations`
const json = { 'Content-Type': 'application/json' }

export async function readAgentIntegrations(client: Client, workspaceId: string): Promise<AgentIntegrationList> {
  const { data } = await client.get({ url: base(workspaceId), throwOnError: true })
  return data as AgentIntegrationList
}

export async function createAgentIntegration(client: Client, workspaceId: string, input: AgentIntegrationInput): Promise<AgentIntegration> {
  const { data } = await client.post({ url: base(workspaceId), body: input, headers: json, throwOnError: true })
  return data as AgentIntegration
}

export async function updateAgentIntegration(client: Client, workspaceId: string, id: string, input: AgentIntegrationInput): Promise<AgentIntegration> {
  const { data } = await client.patch({ url: `${base(workspaceId)}/${id}`, body: input, headers: json, throwOnError: true })
  return data as AgentIntegration
}

export async function deleteAgentIntegration(client: Client, workspaceId: string, id: string): Promise<void> {
  await client.delete({ url: `${base(workspaceId)}/${id}`, throwOnError: true })
}

/** MCP: reads the server's tools. HTTP: a GET of the test path or the base URL. OAuth: a new token, and a GET only of a test path. */
export async function testAgentIntegration(client: Client, workspaceId: string, id: string): Promise<AgentIntegrationTest> {
  const { data } = await client.post({ url: `${base(workspaceId)}/${id}/test`, throwOnError: true })
  return data as AgentIntegrationTest
}

/** MCP: starts the sign-in. Open `authorize_url` in this tab; the server sends the browser back to the admin page. */
export async function connectAgentIntegration(client: Client, workspaceId: string, id: string): Promise<AgentIntegrationConnectStart> {
  const { data } = await client.post({ url: `${base(workspaceId)}/${id}/oauth`, throwOnError: true })
  return data as AgentIntegrationConnectStart
}

/** MCP: removes the sign-in. */
export async function disconnectAgentIntegration(client: Client, workspaceId: string, id: string): Promise<AgentIntegration> {
  const { data } = await client.delete({ url: `${base(workspaceId)}/${id}/oauth`, throwOnError: true })
  return data as AgentIntegration
}
