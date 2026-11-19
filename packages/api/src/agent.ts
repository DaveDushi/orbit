/**
 * The team agent's routes for the signed-in person (`/api/v1/agent`). They are not in the OpenAPI contract, so the
 * agent stays a separate module; see `.ai/agent-design.md`.
 */
import type { Client } from './generated/client/index'
import { formDataBodySerializer } from './generated/client/index'

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

/**
 * Each agent has its own memory. `agent` is the agent's bot user id (`AgentProfile.user_id`); `null` is the default
 * agent. Orbit gives an agent only its own memory.
 */
const withAgent = (url: string, agent: string | null | undefined) => (agent ? `${url}${url.includes('?') ? '&' : '?'}agent=${encodeURIComponent(agent)}` : url)

/** Memories, newest first; with `query`, only those containing every word. */
export async function readAgentMemories(
  client: Client,
  workspaceId: string,
  scope: AgentMemoryScope,
  options: { query?: string; before?: number | null; agent?: string | null } = {},
): Promise<AgentMemoryPage> {
  const params = new URLSearchParams({ limit: '50' })
  if (options.query?.trim()) params.set('query', options.query.trim())
  if (options.before != null) params.set('before', String(options.before))
  if (options.agent) params.set('agent', options.agent)
  const { data } = await client.get({ url: `${memoryUrl(workspaceId, scope)}?${params}`, throwOnError: true })
  return data as AgentMemoryPage
}

/** What a run of the agent reads from this memory, in about `bytes` bytes. */
export async function readAgentMemoryWake(client: Client, workspaceId: string, scope: AgentMemoryScope, agent: string | null = null, bytes = 12_000): Promise<AgentMemoryWake> {
  const { data } = await client.get({ url: withAgent(`${memoryUrl(workspaceId, scope)}/wake?bytes=${bytes}`, agent), throwOnError: true })
  return data as AgentMemoryWake
}

/** Erases a memory; the summaries built from it are written again. */
export async function deleteAgentMemory(client: Client, workspaceId: string, scope: AgentMemoryScope, position: number, agent: string | null = null): Promise<void> {
  await client.delete({ url: withAgent(`${memoryUrl(workspaceId, scope)}/memories/${position}`, agent), throwOnError: true })
}

/** Drops the summary of memories `lo`–`hi` (and those built from it); the agent writes it again. */
export async function forgetAgentMemorySummary(client: Client, workspaceId: string, scope: AgentMemoryScope, lo: number, hi: number, agent: string | null = null): Promise<void> {
  await client.delete({ url: withAgent(`${memoryUrl(workspaceId, scope)}/summaries/${lo}-${hi}`, agent), throwOnError: true })
}

/** The chat link of a memory's source thread. */
export function agentMemorySourcePath(source: AgentMemorySource | null): string | null {
  if (!source?.conversation_id) return null
  return source.root_message_id ? `/chat/${source.conversation_id}/thread/${source.root_message_id}` : `/chat/${source.conversation_id}`
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

/** Settings → Agentic → Agents: a workspace's agent besides the default one. Owners and admins manage them. */
export interface AgentProfile {
  id: string
  /** Its bot user: people mention and DM it like a person. */
  user_id: string
  name: string
  avatar_url: string | null
  instructions: string
  /** Tool names, a subset of the default agent's tools. Orbit refuses the others. */
  tools: string[]
  /** `null`: the installation's model and thinking level. */
  model: string | null
  thinking: string | null
  answer_level: AnswerLevel
  /** Every channel; else only `channels`. Direct messages always work. */
  all_channels: boolean
  channels: { id: string; name: string; kind: 'public' | 'private' }[]
  /** The connected services it may use (ticked on its card); new ones are off. Orbit refuses the others. */
  integration_ids: string[]
  created_at: number
  updated_at: number
  /** `external`: an outside client (Claude Code, Codex, a server) runs it through MCP; absent on older servers. */
  kind?: 'internal' | 'external'
  /** Only for an external agent. */
  external?: ExternalAgentCard
}

/** The permissions an external agent can have. `tasks:read` is always on; Drive is not offered. */
export const EXTERNAL_AGENT_SCOPES = ['tasks:read', 'tasks:write', 'docs:read', 'docs:write', 'chat:read', 'chat:write'] as const
export type ExternalAgentScope = (typeof EXTERNAL_AGENT_SCOPES)[number]

/** What an external agent's card adds: its limits and, for owners and admins, its credentials. */
export interface ExternalAgentCard {
  scopes: ExternalAgentScope[]
  allow_dms: boolean
  projects: { id: string; name: string }[]
  /** Its last MCP call (to the minute), or `null`. */
  last_seen_at: number | null
  /** Live agent tokens: only the prefix is shown. Empty for members. */
  tokens: { id: string; prefix: string; created_at: number; expires_at: number; last_used_at: number | null }[]
  /** OAuth connections made "as agent" on the consent page. Empty for members. */
  connections: { id: string; client_name: string; approved_by: string | null; created_at: number }[]
  /** Where Orbit pushes its inputs (owners and admins only; `null`: no push). Absent on older servers. */
  push_url?: string | null
  /** After the newest push try: `failing` = the last try of an input failed, so it waits for `wait_for_messages`. */
  push_state?: 'ok' | 'retrying' | 'failing' | null
  /** The newest 20 push tries, newest first (owners and admins only). */
  deliveries?: AgentPushDelivery[]
}

/** One push try: `status` is the receiver's HTTP status, `null` when it did not answer (`error` says why). */
export interface AgentPushDelivery {
  event_id: string
  /** 1 to 4: at once, then after 10 s, 60 s and 5 min. */
  attempt: number
  status: number | null
  error: string | null
  duration_ms: number
  created_at: number
}

export interface ExternalAgentInput {
  name?: string
  channel_ids?: string[]
  project_ids?: string[]
  scopes?: ExternalAgentScope[]
  allow_dms?: boolean
  /** https on a public host; `''` turns push off. */
  push_url?: string
}

/** A new agent token: `token` is shown this once. */
export interface IssuedAgentToken {
  id: string
  token: string
  prefix: string
  created_at: number
  expires_at: number
}

/** The default agent's card: its name and avatar are the installation's; the rest is this workspace's. */
export interface DefaultAgent {
  user_id: string
  name: string
  avatar_url: string | null
  instructions: string
  /** `null`: the installation's model and thinking level (the Default model card in Agentic → Agents). */
  model: string | null
  thinking: string | null
  answer_level: AnswerLevel
  /** The connected services it may use; new ones are on until unticked. */
  integration_ids: string[]
  /** Installation admins change the name and avatar. */
  can_rename: boolean
}

/** A connected service of the workspace (Agentic → Workspace), for the ticks on agent cards. */
export interface AgentIntegrationOption {
  id: string
  name: string
  kind: 'mcp' | 'http'
}

export interface AgentToolInfo {
  name: string
  group: string
  /** It writes, posts or changes something. */
  writes: boolean
}

export interface AgentProfiles {
  default_agent: DefaultAgent
  agents: AgentProfile[]
  /** For owners and admins; else empty. */
  integrations: AgentIntegrationOption[]
  /** Owners and admins. */
  can_manage: boolean
  tools: AgentToolInfo[]
  /** For managers, while the agent service answers; else `null`. */
  models: AgentModelSettings['available']
  thinking_levels: string[]
  max_instructions_chars: number
  max_agents: number
}

export interface AgentProfileInput {
  name?: string
  instructions?: string
  tools?: string[]
  model?: string | null
  thinking?: string | null
  answer_level?: AnswerLevel
  all_channels?: boolean
  channel_ids?: string[]
  integration_ids?: string[]
}

/** The default agent's card. `name` needs an installation admin (403 `installation_admin_required`). */
export interface DefaultAgentInput {
  name?: string
  instructions?: string
  model?: string | null
  thinking?: string | null
  answer_level?: AnswerLevel
  integration_ids?: string[]
}

const agentsUrl = (workspaceId: string) => `/api/v1/workspaces/${workspaceId}/agents`

export async function readAgentProfiles(client: Client, workspaceId: string): Promise<AgentProfiles> {
  const { data } = await client.get({ url: agentsUrl(workspaceId), throwOnError: true })
  return data as AgentProfiles
}

/** 409 `agent_name_taken` when an account already has the name. */
export async function createAgentProfile(client: Client, workspaceId: string, input: AgentProfileInput): Promise<AgentProfile> {
  const { data } = await client.post({ url: agentsUrl(workspaceId), body: input, headers: { 'Content-Type': 'application/json' }, throwOnError: true })
  return data as AgentProfile
}

export async function updateAgentProfile(client: Client, workspaceId: string, id: string, input: AgentProfileInput): Promise<AgentProfile> {
  const { data } = await client.patch({ url: `${agentsUrl(workspaceId)}/${id}`, body: input, headers: { 'Content-Type': 'application/json' }, throwOnError: true })
  return data as AgentProfile
}

/** An external agent: its channels only (never every channel), DMs when allowed. 409 `agent_name_taken`. */
export async function createExternalAgent(client: Client, workspaceId: string, input: ExternalAgentInput): Promise<AgentProfile> {
  const { data } = await client.post({ url: `${agentsUrl(workspaceId)}/external`, body: input, headers: { 'Content-Type': 'application/json' }, throwOnError: true })
  return data as AgentProfile
}

export async function updateExternalAgent(client: Client, workspaceId: string, id: string, input: ExternalAgentInput): Promise<AgentProfile> {
  const { data } = await client.patch({ url: `${agentsUrl(workspaceId)}/${id}/external`, body: input, headers: { 'Content-Type': 'application/json' }, throwOnError: true })
  return data as AgentProfile
}

/** A new `orb_agent_…` token for the agent, valid for 30, 90 or 365 days. */
export async function createAgentToken(client: Client, workspaceId: string, id: string, expiresInDays: 30 | 90 | 365): Promise<IssuedAgentToken> {
  const { data } = await client.post({ url: `${agentsUrl(workspaceId)}/${id}/tokens`, body: { expires_in_days: expiresInDays }, headers: { 'Content-Type': 'application/json' }, throwOnError: true })
  return data as IssuedAgentToken
}

/** The signing secret of the agent's pushes (owners and admins); `rotate` makes a new one, used at once. */
export async function agentPushSecret(client: Client, workspaceId: string, id: string, rotate = false): Promise<string> {
  const url = `${agentsUrl(workspaceId)}/${id}/push-secret`
  const { data } = rotate ? await client.post({ url: `${url}/rotate`, throwOnError: true }) : await client.get({ url, throwOnError: true })
  return (data as { signing_secret: string }).signing_secret
}

/** Revokes an agent token (`tokens`) or disconnects an OAuth connection (`connections`) of the agent. */
export async function revokeAgentCredential(client: Client, workspaceId: string, id: string, kind: 'tokens' | 'connections', credentialId: string): Promise<void> {
  await client.delete({ url: `${agentsUrl(workspaceId)}/${id}/${kind}/${credentialId}`, throwOnError: true })
}

/** The default agent cannot be deleted. 409 `agent_name_taken` when an account already has the name. */
export async function updateDefaultAgent(client: Client, workspaceId: string, input: DefaultAgentInput): Promise<DefaultAgent> {
  const { data } = await client.patch({ url: `${agentsUrl(workspaceId)}/default`, body: input, headers: { 'Content-Type': 'application/json' }, throwOnError: true })
  return data as DefaultAgent
}

/** Its routines pause; its messages and DMs stay readable. */
export async function deleteAgentProfile(client: Client, workspaceId: string, id: string): Promise<void> {
  await client.delete({ url: `${agentsUrl(workspaceId)}/${id}`, throwOnError: true })
}

/** A PNG, JPEG or WebP of at most 512 KiB (`avatarImage` makes a 256 px square); `null` removes it. */
export async function setAgentProfileAvatar(client: Client, workspaceId: string, id: string, image: Blob | null): Promise<void> {
  // `id` "default": the default agent (installation admins only).
  const url = `${agentsUrl(workspaceId)}/${id}/avatar`
  if (image) await client.put({ ...formDataBodySerializer, url, body: { file: image }, headers: { 'Content-Type': null }, throwOnError: true })
  else await client.delete({ url, throwOnError: true })
}

/** How much an agent thinks and writes, set on its card: for every person who uses it. */
export type AnswerLevel = 'short' | 'normal' | 'thorough'

export const ANSWER_LEVELS: readonly { value: AnswerLevel; label: string; help: string }[] = [
  { value: 'short', label: 'Short', help: 'Less thinking, a few lines. Fast.' },
  { value: 'normal', label: 'Normal', help: 'Its model and thinking level as set.' },
  { value: 'thorough', label: 'Thorough', help: 'More thinking, full detail. Slower, and it uses more of each person’s ChatGPT limits.' },
]
