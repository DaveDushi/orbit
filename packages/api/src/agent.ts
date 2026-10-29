/**
 * The team agent's routes for the signed-in person (`/api/v1/agent`). They are not in the OpenAPI contract, so the
 * agent stays a separate module; see `.ai/agent-design.md`.
 */
import type { Client } from './generated/client/index'

/** A ChatGPT sign-in in progress (device code) or its result. */
export interface AgentLogin {
  state: 'pending' | 'connected' | 'failed'
  user_code?: string
  verification_uri?: string
  expires_at?: number
  error?: string
}

export interface AgentConnection {
  connected: boolean
  /** A token refresh failed: connect again. */
  invalid: boolean
  account_id: string | null
  connected_at: number | null
}

export type AgentStatus =
  | { enabled: false }
  | {
      enabled: true
      /** The agent service answers. */
      online: boolean
      /** The server can store a sign-in (it has an app key). */
      can_connect: boolean
      bot_user_id: string | null
      /** The agent's name, which is also its @handle in chat. */
      name: string
      chatgpt: AgentConnection
      login: AgentLogin | null
    }

export async function readAgentStatus(client: Client): Promise<AgentStatus> {
  const { data } = await client.get({ url: '/api/v1/agent', throwOnError: true })
  return data as AgentStatus
}

/** Starts the device-code sign-in, or returns the one in progress. */
export async function startChatgptLogin(client: Client): Promise<AgentLogin | null> {
  const { data } = await client.post({ url: '/api/v1/agent/chatgpt/login', throwOnError: true })
  return (data as { login: AgentLogin | null }).login
}

export async function disconnectChatgpt(client: Client): Promise<void> {
  await client.delete({ url: '/api/v1/agent/chatgpt', throwOnError: true })
}

/** Admin → AI agent: the agent's one name for the whole installation (installation admins only). */
export interface AgentIdentity {
  name: string
  bot_user_id: string | null
  min_name_chars: number
  max_name_chars: number
}

export async function readAgentIdentity(client: Client): Promise<AgentIdentity> {
  const { data } = await client.get({ url: '/api/v1/admin/agent', throwOnError: true })
  return data as AgentIdentity
}

/** Renames the agent. 409 `agent_name_taken` when a member already has the name. */
export async function saveAgentName(client: Client, name: string): Promise<AgentIdentity> {
  const { data } = await client.put({ url: '/api/v1/admin/agent', body: { name }, headers: { 'Content-Type': 'application/json' }, throwOnError: true })
  return data as AgentIdentity
}

/** Workspace settings → AI agent: instructions for every run in the workspace. Every member reads them. */
export interface TeamInstructions {
  instructions: string
  /** Owners and admins. */
  can_edit: boolean
  max_chars: number
  updated_at: number | null
  updated_by_name: string | null
}

export async function readTeamInstructions(client: Client, workspaceId: string): Promise<TeamInstructions> {
  const { data } = await client.get({ url: `/api/v1/workspaces/${workspaceId}/agent`, throwOnError: true })
  return data as TeamInstructions
}

export async function saveTeamInstructions(client: Client, workspaceId: string, instructions: string): Promise<TeamInstructions> {
  const { data } = await client.put({ url: `/api/v1/workspaces/${workspaceId}/agent`, body: { instructions }, headers: { 'Content-Type': 'application/json' }, throwOnError: true })
  return data as TeamInstructions
}
