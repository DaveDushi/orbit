/**
 * Agent sandboxes (docs/agent-sandbox-design.md, sections 4 and 17): the egress cards of a conversation, their
 * answers, and an agent's sandbox settings and Internet access rules (owners and admins). Not in the OpenAPI
 * contract, like the other agent routes. Every route answers 404 `sandbox_off` when the server has no sandbox.
 */
import type { Client } from './generated/client/index'

/** `expired`: nobody answered within 30 minutes; the agent asks again at its next try. */
export type EgressStatus = 'pending' | 'expired' | 'allowed' | 'denied'

/** One host an agent's sandbox wanted, on its card in the thread. The `can_*` flags are the viewer's. */
export interface EgressRequest {
  id: string
  /** The agent's card message that asked. */
  message_id: string
  host: string
  port: number
  status: EgressStatus
  /** On the risky list: it always asks and can never be allowed always. */
  risky: boolean
  expires_at: number | null
  driver_id: string
  driver_name: string
  decided_by_name: string | null
  decided_at: number | null
  /** The agent's rule for this host now, if any. */
  rule: 'allow' | 'deny' | null
  can_allow_once: boolean
  can_allow_always: boolean
  can_deny: boolean
}

/** The egress card's message: the sidecar posts it with the nonce `egress:<call>` (`apps/agent/src/agent-sandbox.ts`). */
export const isEgressCard = (nonce: string | null | undefined) => nonce?.startsWith('egress:') === true

export type EgressAnswer = 'allow_once' | 'allow_always' | 'deny'

const workspaceUrl = (workspaceId: string) => `/api/v1/workspaces/${workspaceId}/agents`

/** The egress cards of one conversation, newest first (a card shows its rows oldest first: `cardRequests`). 404 for guests and people who cannot read it. */
export async function readEgressRequests(client: Client, workspaceId: string, conversationId: string): Promise<EgressRequest[]> {
  const { data } = await client.get({ url: `${workspaceUrl(workspaceId)}/egress-requests`, query: { conversation_id: conversationId }, throwOnError: true })
  return (data as { requests: EgressRequest[] }).requests
}

/**
 * Answers a card. Allow once: the driver only; Allow always: owners and admins, never a risky host; Deny: the driver
 * or an owner/admin. 409 when it has an answer already, 410 when it expired.
 */
export async function answerEgressRequest(client: Client, workspaceId: string, requestId: string, answer: EgressAnswer): Promise<void> {
  await client.post({ url: `${workspaceUrl(workspaceId)}/egress-requests/${requestId}`, body: { answer }, headers: { 'Content-Type': 'application/json' }, throwOnError: true })
}

/** The rows of one card message, in the order the agent asked (ids are time-ordered). */
export const cardRequests = (requests: readonly EgressRequest[], messageId: string) => requests.filter((request) => request.message_id === messageId).sort((a, b) => (a.id < b.id ? -1 : 1))

/** What a card shows now: its state line and the buttons this viewer gets (none once it is answered or expired). */
export function egressCardView(request: EgressRequest, now: number) {
  const status: EgressStatus = request.status === 'pending' && request.expires_at !== null && request.expires_at <= now ? 'expired' : request.status
  const open = status === 'pending'
  const by = request.decided_by_name ? ` by ${request.decided_by_name}` : ''
  const mayAnswer = request.can_allow_once || request.can_allow_always || request.can_deny
  const text = {
    pending: mayAnswer ? 'Waiting for your answer.' : `Waiting for ${request.driver_name} or an admin.`,
    expired: 'Nobody answered in 30 minutes. The agent asks again at its next try.',
    allowed: request.rule === 'allow' ? `Allowed always${by}.` : `Allowed once${by}, for 30 minutes in this thread.`,
    denied: request.rule === 'deny' ? `Denied always${by}.` : `Denied${by} in this thread.`,
  }[status]
  const buttons = open
    ? (
        [
          ['allow_once', 'Allow once', request.can_allow_once],
          ['allow_always', 'Allow always', request.can_allow_always],
          ['deny', 'Deny', request.can_deny],
        ] as const
      )
        .filter(([, , may]) => may)
        .map(([answer, label]) => ({ answer: answer as EgressAnswer, label }))
    : []
  // A driver who is not an admin learns why Allow always is missing; a risky host never gets it.
  const note = !open ? null : request.risky ? 'This host always asks: it cannot be allowed always.' : request.can_allow_once && !request.can_allow_always ? 'Only owners and admins can allow always.' : null
  return { status, text, buttons, note }
}

export interface SandboxSettings {
  enabled: boolean
  /** A quiet thread sandbox is deleted after this long (5 to 120); its /work stays. */
  idle_minutes: number
  cpu_minutes_month: number
  egress_bytes_month: number
  model_tokens_month: number
  updated_at: number | null
}

export type SandboxSettingsInput = Omit<SandboxSettings, 'updated_at'>

export interface EgressRule {
  id: string
  /** `api.example.com`, or `*.example.com` (subdomains only). */
  host: string
  port: number
  action: 'allow' | 'deny'
  created_by_name: string | null
  created_at: number
  last_used_at: number | null
}

export interface EgressPending {
  id: string
  host: string
  port: number
  risky: boolean
  driver_name: string
  created_at: number
  expires_at: number | null
  can_allow_once: boolean
  can_allow_always: boolean
  can_deny: boolean
}

export interface EgressLogRow {
  id: string
  host: string
  port: number
  /** `blocked`: allowed by the rules, then refused by the proxy (a private address, a wrong TLS name). */
  decision: 'allow' | 'deny' | 'ask' | 'blocked'
  reason: string
  bytes_up: number
  bytes_down: number
  driver_name: string | null
  created_at: number
}

/** One agent's sandbox: settings, this month's use, Internet access rules, pending requests, the latest 100 connections. */
export interface AgentSandbox {
  settings: SandboxSettings
  usage: { month: string; cpu_ms: number; egress_bytes: number; model_tokens: number }
  rules: EgressRule[]
  pending: EgressPending[]
  log: EgressLogRow[]
}

/** `agentId`: the agent's profile id. Owners and admins; members get 403, guests 404. */
const sandboxUrl = (workspaceId: string, agentId: string) => `${workspaceUrl(workspaceId)}/${agentId}/sandbox`
const rulesUrl = (workspaceId: string, agentId: string) => `${workspaceUrl(workspaceId)}/${agentId}/egress-rules`

export async function readAgentSandbox(client: Client, workspaceId: string, agentId: string): Promise<AgentSandbox> {
  const { data } = await client.get({ url: sandboxUrl(workspaceId, agentId), throwOnError: true })
  return data as AgentSandbox
}

export async function saveAgentSandbox(client: Client, workspaceId: string, agentId: string, input: SandboxSettingsInput): Promise<AgentSandbox> {
  const { data } = await client.put({ url: sandboxUrl(workspaceId, agentId), body: input, headers: { 'Content-Type': 'application/json' }, throwOnError: true })
  return data as AgentSandbox
}

/** 422 `validation_failed` with the reason: an IP, `*`, a wildcard on a public suffix, Orbit's own host, a risky allow. */
export async function addEgressRule(client: Client, workspaceId: string, agentId: string, rule: { host: string; port: number; action: 'allow' | 'deny' }): Promise<AgentSandbox> {
  const { data } = await client.post({ url: rulesUrl(workspaceId, agentId), body: rule, headers: { 'Content-Type': 'application/json' }, throwOnError: true })
  return data as AgentSandbox
}

export async function deleteEgressRule(client: Client, workspaceId: string, agentId: string, ruleId: string): Promise<AgentSandbox> {
  const { data } = await client.delete({ url: `${rulesUrl(workspaceId, agentId)}/${ruleId}`, throwOnError: true })
  return data as AgentSandbox
}


const GB = 1024 * 1024 * 1024

/** The agent card's sandbox switch and limits, as the dialog edits them (traffic in GB, tokens in millions). */
export interface SandboxDraft {
  enabled: boolean
  idle: string
  cpu: string
  trafficGb: string
  tokensM: string
}

export const sandboxDraft = (sandbox: AgentSandbox): SandboxDraft => ({
  enabled: sandbox.settings.enabled,
  idle: String(sandbox.settings.idle_minutes),
  cpu: String(sandbox.settings.cpu_minutes_month),
  trafficGb: String(+(sandbox.settings.egress_bytes_month / GB).toFixed(2)),
  tokensM: String(+(sandbox.settings.model_tokens_month / 1_000_000).toFixed(2)),
})

/** The draft as the route takes it; `null` when a number is not valid (the fields say which). */
export function sandboxInput(draft: SandboxDraft): SandboxSettingsInput | null {
  const idle = Number(draft.idle)
  const cpu = Number(draft.cpu)
  const traffic = Number(draft.trafficGb)
  const tokens = Number(draft.tokensM)
  if (![idle, cpu, traffic, tokens].every((value) => Number.isFinite(value) && value >= 0) || draft.idle === '' || draft.cpu === '') return null
  if (!Number.isInteger(idle) || idle < 5 || idle > 120 || !Number.isInteger(cpu)) return null
  return { enabled: draft.enabled, idle_minutes: idle, cpu_minutes_month: cpu, egress_bytes_month: Math.round(traffic * GB), model_tokens_month: Math.round(tokens * 1_000_000) }
}

/** Bytes as people read them: 1.5 GB, 820 KB. */
export function formatBytes(bytes: number): string {
  const units = ['B', 'KB', 'MB', 'GB', 'TB']
  let value = bytes
  let unit = 0
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024
    unit += 1
  }
  return `${unit === 0 || value >= 10 ? Math.round(value) : value.toFixed(1)} ${units[unit]}`
}
