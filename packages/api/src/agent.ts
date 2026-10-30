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

/** `team`: the workspace's memory, every member reads it. `private`: the signed-in person's own. */
export type AgentMemoryScope = 'team' | 'private'

/** Where a memory came from: a chat thread, or the main list of a DM with the agent. */
export interface AgentMemorySource {
  conversation_id: string | null
  root_message_id: string | null
}

export interface AgentMemory {
  position: number
  body: string
  author_id: string | null
  author_name: string | null
  created_at: string
  source: AgentMemorySource | null
}

export interface AgentMemoryPage {
  items: AgentMemory[]
  /** Pass as `before` for older memories. */
  next_before: number | null
  /** The person may delete memories and summaries of this scope. */
  can_delete: boolean
}

/** A line of what the agent reads: a memory (`lo === hi`) or the summary of memories `lo`–`hi`. */
export interface AgentMemoryLine {
  lo: number
  hi: number
  body: string
  author_name: string | null
  created_at: string | null
}

export interface AgentMemoryWake {
  total: number
  lines: AgentMemoryLine[]
  text: string
}

const memoryUrl = (workspaceId: string, scope: AgentMemoryScope) => `/api/v1/workspaces/${workspaceId}/agent-memory/${scope}`

/** Memories, newest first; with `query`, only those containing every word. */
export async function readAgentMemories(
  client: Client,
  workspaceId: string,
  scope: AgentMemoryScope,
  options: { query?: string; before?: number | null } = {},
): Promise<AgentMemoryPage> {
  const params = new URLSearchParams({ limit: '50' })
  if (options.query?.trim()) params.set('query', options.query.trim())
  if (options.before != null) params.set('before', String(options.before))
  const { data } = await client.get({ url: `${memoryUrl(workspaceId, scope)}?${params}`, throwOnError: true })
  return data as AgentMemoryPage
}

/** What a run of the agent reads from this memory, in about `bytes` bytes. */
export async function readAgentMemoryWake(client: Client, workspaceId: string, scope: AgentMemoryScope, bytes = 12_000): Promise<AgentMemoryWake> {
  const { data } = await client.get({ url: `${memoryUrl(workspaceId, scope)}/wake?bytes=${bytes}`, throwOnError: true })
  return data as AgentMemoryWake
}

/** Erases a memory; the summaries built from it are written again. */
export async function deleteAgentMemory(client: Client, workspaceId: string, scope: AgentMemoryScope, position: number): Promise<void> {
  await client.delete({ url: `${memoryUrl(workspaceId, scope)}/memories/${position}`, throwOnError: true })
}

/** Drops the summary of memories `lo`–`hi` (and those built from it); the agent writes it again. */
export async function forgetAgentMemorySummary(client: Client, workspaceId: string, scope: AgentMemoryScope, lo: number, hi: number): Promise<void> {
  await client.delete({ url: `${memoryUrl(workspaceId, scope)}/summaries/${lo}-${hi}`, throwOnError: true })
}

/** The chat link of a memory's source thread. */
export function agentMemorySourcePath(source: AgentMemorySource | null): string | null {
  if (!source?.conversation_id) return null
  return source.root_message_id ? `/chat/${source.conversation_id}/thread/${source.root_message_id}` : `/chat/${source.conversation_id}`
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

/** Admin → AI agent: the model and thinking level of every run. `null` is the agent's default. */
export interface AgentModelSettings {
  model: string | null
  thinking: string | null
  /** What the agent offers now; `null` when it is offline (then only the default model can be saved). */
  available: { default_model: string; models: { id: string; name: string }[] } | null
  thinking_levels: string[]
  updated_at: number | null
  updated_by_name: string | null
}

/** The value of the "default" choice in a picker; the API stores it as `null`. */
export const AGENT_DEFAULT_CHOICE = 'default'

/** The model and thinking choices of a picker. A saved model stays listed while the agent is offline or no longer offers it. */
export function agentModelChoices(settings: AgentModelSettings): { models: { value: string; label: string }[]; levels: { value: string; label: string }[] } {
  const available = settings.available
  const defaultName = available?.models.find((option) => option.id === available.default_model)?.name ?? available?.default_model
  const models = [
    { value: AGENT_DEFAULT_CHOICE, label: defaultName ? `Default (${defaultName})` : 'Default' },
    ...(available?.models ?? []).map((option) => ({ value: option.id, label: option.name })),
  ]
  if (settings.model && !models.some((option) => option.value === settings.model)) models.push({ value: settings.model, label: settings.model })
  const levels = [{ value: AGENT_DEFAULT_CHOICE, label: 'Default' }, ...settings.thinking_levels.map((level) => ({ value: level, label: level }))]
  return { models, levels }
}

export async function readAgentModel(client: Client): Promise<AgentModelSettings> {
  const { data } = await client.get({ url: '/api/v1/admin/agent/model', throwOnError: true })
  return data as AgentModelSettings
}

/** 422 for a model the agent does not offer or an unknown thinking level; 502 `agent_offline` for a model while the agent is offline. */
export async function saveAgentModel(client: Client, choice: { model: string | null; thinking: string | null }): Promise<AgentModelSettings> {
  const { data } = await client.put({ url: '/api/v1/admin/agent/model', body: choice, headers: { 'Content-Type': 'application/json' }, throwOnError: true })
  return data as AgentModelSettings
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
