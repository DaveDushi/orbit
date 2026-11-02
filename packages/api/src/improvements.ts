/**
 * "Suggest a change" (`/api/v1/admin/improvements`): change requests that installation admins send, and the status
 * an agent outside Orbit reports back with `orbit improvements update`. Not in the OpenAPI contract, like the agent
 * routes; see `docs/improvements.md`. The text is untrusted input: show it as plain text only.
 */
import type { Client } from './generated/client/index'

export type ImprovementStatus = 'new' | 'building' | 'review' | 'live' | 'declined'

export interface ImprovementContext {
  /** The URL path of the page the request was sent from. */
  path: string
  /** `WIDTHxHEIGHT` of the window. */
  viewport: string
  /** The web build's revision. */
  revision: string
}

export interface ImprovementRequest {
  id: string
  status: ImprovementStatus
  text: string
  context: ImprovementContext
  author: { id: string; name: string; email: string }
  /** The agent's short note. */
  note: string | null
  /** A branch, commit or address the agent gave. */
  link: string | null
  created_at: string
  updated_at: string
}

export interface ImprovementSettings {
  enabled: boolean
  max_text_chars: number
}

const BASE = '/api/v1/admin/improvements'
const JSON_BODY = { 'Content-Type': 'application/json' }

export async function readImprovementSettings(client: Client): Promise<ImprovementSettings> {
  const { data } = await client.get({ url: `${BASE}/settings`, throwOnError: true })
  return data as ImprovementSettings
}

export async function saveImprovementSettings(client: Client, enabled: boolean): Promise<ImprovementSettings> {
  const { data } = await client.put({ url: `${BASE}/settings`, body: { enabled }, headers: JSON_BODY, throwOnError: true })
  return data as ImprovementSettings
}

/** Every request, newest first. */
export async function listImprovements(client: Client): Promise<ImprovementRequest[]> {
  const { data } = await client.get({ url: BASE, throwOnError: true })
  return (data as { items: ImprovementRequest[] }).items
}

export async function sendImprovement(client: Client, text: string, context: ImprovementContext): Promise<ImprovementRequest> {
  const { data } = await client.post({ url: BASE, body: { text, context }, headers: JSON_BODY, throwOnError: true })
  return data as ImprovementRequest
}

/** Only a `new` request: it becomes `declined` with the note "Withdrawn". */
export async function withdrawImprovement(client: Client, id: string): Promise<ImprovementRequest> {
  const { data } = await client.post({ url: `${BASE}/${encodeURIComponent(id)}/withdraw`, throwOnError: true })
  return data as ImprovementRequest
}
