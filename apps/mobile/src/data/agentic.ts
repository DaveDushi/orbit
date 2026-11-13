/**
 * The Agentic settings' rules without React, so Bun tests check them: which tabs and web links a person sees, and how
 * the agent and routine forms become the API's bodies (the web's `AgentsPage` and `RoutinesPage`).
 */
import { AGENT_DEFAULT_CHOICE as DEFAULT, ANSWER_LEVELS, type AgentProfile, type AgentProfileInput, type AgentProfiles, type AnswerLevel, type DefaultAgentInput } from '@orbit/api/agent'
import { ApiProblem } from '@orbit/api/problem'
import type { Routine, RoutineChange, Script } from '@orbit/api/scheduled'
import type { ConversationRecord, Permission } from '@orbit/api/generated/types.gen'

import { multipart, type UploadFile } from './account-api'
import type { OrbitClient } from './orbit'

export type AgenticTab = 'you' | 'workspace' | 'agents' | 'routines' | 'bots'

/** The web's `AGENTIC_TABS`: Workspace, Agents and Routines need the team agent. The account settings show only You. */
export function agenticTabs(agent: boolean, personal: boolean): { value: AgenticTab; label: string }[] {
  if (personal) return [{ value: 'you', label: 'You' }]
  return [
    { value: 'you', label: 'You' },
    ...(agent
      ? [
          { value: 'workspace' as const, label: 'Workspace' },
          { value: 'agents' as const, label: 'Agents' },
          { value: 'routines' as const, label: 'Routines' },
        ]
      : []),
    { value: 'bots', label: 'Bots' },
  ]
}

/** The tab a link asks for (`?tab=routines`), when this person has it; else You. */
export function pickedTab(tabs: readonly { value: AgenticTab }[], wanted: string | undefined): AgenticTab {
  return tabs.find((tab) => tab.value === wanted)?.value ?? 'you'
}

/** A setup page that stays on the web: the app opens it in an in-app browser. */
export interface WebSetupLink {
  path: string
  label: string
  description: string
}

/**
 * The rare admin pages the app does not rebuild, with the web's own access rules: agent tools for
 * `integrations.manage`, API tokens for `api_tokens.manage`. Bots are on their own tab.
 */
export function webSetupLinks(permissions: readonly Permission[], agent: boolean): WebSetupLink[] {
  return [
    ...(agent && permissions.includes('integrations.manage')
      ? [{ path: '/settings/agentic/workspace', label: 'Agent tools', description: 'Connect MCP servers and HTTP APIs that agents may use.' }]
      : []),
    ...(permissions.includes('api_tokens.manage') ? [{ path: '/settings/api-tokens', label: 'API tokens', description: 'Tokens for scripts and AI clients that cannot sign in with OAuth.' }] : []),
  ]
}

/** The tool that calls connected services: on exactly when a service is ticked, so it is not in the tool list. */
export const INTEGRATION_TOOL = 'use_integration'

/** The tools by group, in the server's order, without the services tool. */
export function toolGroups(tools: AgentProfiles['tools']): [string, AgentProfiles['tools']][] {
  const groups = new Map<string, AgentProfiles['tools']>()
  for (const tool of tools) {
    if (tool.name !== INTEGRATION_TOOL) groups.set(tool.group, [...(groups.get(tool.group) ?? []), tool])
  }
  return [...groups.entries()]
}

export const answerLevelLabel = (level: AnswerLevel) => ANSWER_LEVELS.find((option) => option.value === level)?.label ?? 'Normal'

export function channelsLabel(agent: Pick<AgentProfile, 'all_channels' | 'channels'>): string {
  if (agent.all_channels) return 'Every channel and direct messages'
  if (agent.channels.length === 0) return 'Direct messages only'
  return `${agent.channels.map((channel) => `#${channel.name}`).join(', ')} and direct messages`
}

/** How many of the workspace's connected services the agent may use (owners and admins see them). */
export function servicesLabel(profiles: Pick<AgentProfiles, 'can_manage' | 'integrations'>, ids: readonly string[]): string {
  if (!profiles.can_manage || profiles.integrations.length === 0) return 'No connected services'
  const on = profiles.integrations.filter((integration) => ids.includes(integration.id)).length
  if (on === profiles.integrations.length) return 'All connected services'
  return on === 0 ? 'No connected services' : `${on} of ${profiles.integrations.length} connected services`
}

/** What the agent sheet edits. `model` and `thinking` hold `DEFAULT` for the installation's choice. */
export interface AgentForm {
  name: string
  instructions: string
  tools: string[]
  services: string[]
  model: string
  thinking: string
  answerLevel: AnswerLevel
  allChannels: boolean
  channelIds: string[]
}

export function agentFormOf(agent: AgentProfile | null, defaultAgent?: AgentProfiles['default_agent']): AgentForm {
  const source = agent ?? defaultAgent ?? null
  return {
    name: source?.name ?? '',
    instructions: source?.instructions ?? '',
    tools: (agent?.tools ?? []).filter((tool) => tool !== INTEGRATION_TOOL),
    services: source?.integration_ids ?? [],
    model: source?.model ?? DEFAULT,
    thinking: source?.thinking ?? DEFAULT,
    answerLevel: source?.answer_level ?? 'normal',
    allChannels: agent?.all_channels ?? false,
    channelIds: agent?.channels.map((channel) => channel.id) ?? [],
  }
}

const shared = (form: AgentForm) => ({
  instructions: form.instructions,
  model: form.model === DEFAULT ? null : form.model,
  thinking: form.thinking === DEFAULT ? null : form.thinking,
  answer_level: form.answerLevel,
  integration_ids: form.services,
})

/** A custom agent's body: the services tool is on exactly when a service is ticked. */
export function customAgentInput(form: AgentForm): AgentProfileInput {
  return {
    ...shared(form),
    name: form.name.trim(),
    tools: form.services.length ? [...form.tools, INTEGRATION_TOOL] : form.tools,
    all_channels: form.allChannels,
    channel_ids: form.channelIds,
  }
}

/** The default agent's body: the name only when it changed, since a rename needs an installation admin. */
export function defaultAgentInput(form: AgentForm, savedName: string): DefaultAgentInput {
  const name = form.name.trim()
  return { ...shared(form), ...(name !== savedName ? { name } : {}) }
}

/** The web's rule for the name field: 2 to 40 characters. */
export const agentNameValid = (name: string) => name.trim().length >= 2 && name.trim().length <= 40

/** The channels an agent can be given: open channels and private ones the person is in, not archived, no DMs. */
export function agentChannelChoices(conversations: readonly ConversationRecord[], userId: string): ConversationRecord[] {
  return conversations.filter((conversation) => conversation.kind !== 'dm' && !conversation.archived && (conversation.kind === 'public' || conversation.member_ids.includes(userId)))
}

/** Where a routine may post: conversations the person is in, not archived; a DM only when the agent is in it. */
export function routineTargets(conversations: readonly ConversationRecord[], userId: string, agentUserId: string): ConversationRecord[] {
  return conversations.filter((conversation) => conversation.member_ids.includes(userId) && !conversation.archived && (conversation.kind !== 'dm' || conversation.member_ids.includes(agentUserId)))
}

/** The script choice "none". */
export const NO_SCRIPT = 'none'

export interface RoutineForm {
  name: string
  agentId: string
  prompt: string
  schedule: string
  timeZone: string
  target: string
  scriptId: string
}

export function routineFormOf(routine: Routine): RoutineForm {
  return {
    name: routine.name,
    agentId: routine.agent_user_id,
    prompt: routine.prompt,
    schedule: routine.schedule,
    timeZone: routine.time_zone,
    target: routine.conversation_id ?? '',
    scriptId: routine.script_id ?? NO_SCRIPT,
  }
}

export const routineFormValid = (form: RoutineForm) => form.name.trim() !== '' && form.prompt.trim() !== '' && form.schedule.trim() !== ''

/**
 * The routine's change, as the web's Edit routine sends it: the target and agent only when they changed, and a newly
 * picked script with its code hash, since picking it with its code in view is the creator's yes to that version.
 */
export function routineChange(routine: Routine, form: RoutineForm, scripts: readonly Script[]): RoutineChange {
  const newScript = form.scriptId !== (routine.script_id ?? NO_SCRIPT)
  const script = scripts.find((item) => item.id === form.scriptId)
  return {
    name: form.name,
    prompt: form.prompt,
    schedule: form.schedule,
    time_zone: form.timeZone,
    ...(form.target && form.target !== routine.conversation_id ? { conversation_id: form.target } : {}),
    ...(form.agentId !== routine.agent_user_id ? { agent_user_id: form.agentId } : {}),
    ...(newScript ? { script_id: form.scriptId === NO_SCRIPT ? '' : form.scriptId, ...(script ? { approve_script_hash: script.code_hash } : {}) } : {}),
  }
}

/** Where a routine posts, in words. */
export function routineTarget(routine: Pick<Routine, 'conversation_kind' | 'conversation_name'>): string {
  if (routine.conversation_kind === 'dm') return 'Direct message'
  return routine.conversation_name ? `#${routine.conversation_name}` : 'Deleted conversation'
}

/** The actions on a routine's row, from the server's `can_edit` and `can_stop`. */
export function routineActions(routine: Pick<Routine, 'status' | 'can_edit' | 'can_stop'>) {
  return {
    pause: routine.status === 'active' && routine.can_stop,
    resume: routine.status !== 'active' && routine.can_edit,
    edit: routine.can_edit,
    test: routine.can_edit,
    remove: routine.can_stop,
  }
}

/** "Today 17:00", "Tomorrow 09:00", or "Mon, Nov 16, 09:00" (local), as the web's `whenLabel`. */
export function whenLabel(at: number | Date, now: Date = new Date()): string {
  const date = new Date(at)
  const time = date.toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' })
  const day = new Date(date.getFullYear(), date.getMonth(), date.getDate()).getTime()
  const today = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime()
  const days = Math.round((day - today) / 86_400_000)
  if (days === 0) return `Today ${time}`
  if (days === 1) return `Tomorrow ${time}`
  return `${date.toLocaleDateString(undefined, { weekday: 'short', month: 'short', day: 'numeric' })}, ${time}`
}

/** The server's reason for a refusal, or `fallback`. */
export function problemDetail(error: unknown, fallback: string): string {
  return error instanceof ApiProblem && error.detail ? error.detail : fallback
}

/**
 * An agent's avatar (`id` "default": the default agent), with the app's multipart part: a picked file only implements
 * `Blob`, so the shared form serializer would send it as JSON. `null` removes it.
 */
export async function setAgentAvatar(client: OrbitClient, workspaceId: string, id: string, file: UploadFile | null): Promise<void> {
  const url = `/api/v1/workspaces/${workspaceId}/agents/${id}/avatar`
  if (file) await client.put({ url, body: { file }, bodySerializer: multipart(file), headers: { 'Content-Type': null }, throwOnError: true })
  else await client.delete({ url, throwOnError: true })
}

/** The routine's time zone choices: the zones this engine knows, with the saved one kept when it is not among them. */
export function timeZoneChoices(saved: string): string[] {
  let zones: string[] = []
  try {
    zones = (Intl as { supportedValuesOf?: (key: string) => string[] }).supportedValuesOf?.('timeZone') ?? []
  } catch {
    zones = []
  }
  return zones.includes(saved) ? zones : [saved, ...zones]
}
