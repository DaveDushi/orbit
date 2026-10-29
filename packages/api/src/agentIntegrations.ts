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
  /** A secret is saved. Orbit never sends it back. */
  secret_set: boolean
  allowed_roles: AgentIntegrationRole[]
  allowed_methods: AgentIntegrationMethod[]
  /** MCP: `null` until the first successful test. */
  tools: AgentIntegrationTool[] | null
  tools_checked_at: number | null
  created_at: number
  updated_at: number
}

export interface AgentIntegrationList {
  items: AgentIntegration[]
  /** The server has an app key, so it can keep secrets. */
  can_keep_secrets: boolean
}

/** A change. Leave `secret` out to keep the saved one; `''` removes it. */
export interface AgentIntegrationInput {
  name?: string
  slug?: string
  kind?: AgentIntegrationKind
  url?: string
  auth_header?: string
  secret?: string
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

/** MCP: reads the server's tools. HTTP: a GET of the base URL. */
export async function testAgentIntegration(client: Client, workspaceId: string, id: string): Promise<AgentIntegrationTest> {
  const { data } = await client.post({ url: `${base(workspaceId)}/${id}/test`, throwOnError: true })
  return data as AgentIntegrationTest
}
