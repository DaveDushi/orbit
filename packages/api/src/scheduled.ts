/**
 * Routines, reminders and scheduled messages (`/api/v1/workspaces/{id}/routines` and `/scheduled-messages`). Like the
 * agent's other routes they are not in the OpenAPI contract; see `apps/server/src/scheduled/`.
 */
import type { Client } from './generated/client/index'

const json = { 'Content-Type': 'application/json' }

/** A recurring job of the agent, done as its creator and posted in a conversation. */
export interface Routine {
  id: string
  workspace_id: string
  agent_user_id: string
  /** The agent's name, as people mention it. */
  agent_name: string
  creator_id: string
  creator_name: string
  /** `null`: the conversation was deleted. */
  conversation_id: string | null
  conversation_name: string | null
  conversation_kind: 'public' | 'private' | 'dm' | null
  name: string
  prompt: string
  /** 5-field cron: minute hour day month weekday. */
  schedule: string
  time_zone: string
  status: 'active' | 'paused'
  /** Why Orbit paused it; `null` when a person did. */
  paused_reason: string | null
  next_run_at: number | null
  last_run_at: number | null
  last_run_status: 'started' | 'skipped' | null
  last_run_message_id: string | null
  last_error: string | null
  created_at: number
  updated_at: number
  /** The caller may change it and resume it (only its creator). */
  can_edit: boolean
  /** The caller may pause and delete it (its creator, owners and admins). */
  can_stop: boolean
}

export interface RoutineChange {
  name?: string
  prompt?: string
  schedule?: string
  time_zone?: string
  conversation_id?: string
  /** Another agent of the workspace runs it. */
  agent_user_id?: string
  status?: 'active' | 'paused'
}

const routinesUrl = (workspaceId: string) => `/api/v1/workspaces/${workspaceId}/routines`

export async function listRoutines(client: Client, workspaceId: string): Promise<Routine[]> {
  const { data } = await client.get({ url: routinesUrl(workspaceId), throwOnError: true })
  return (data as { routines: Routine[] }).routines
}

export async function updateRoutine(client: Client, workspaceId: string, id: string, change: RoutineChange): Promise<Routine> {
  const { data } = await client.patch({ url: `${routinesUrl(workspaceId)}/${id}`, body: change, headers: json, throwOnError: true })
  return data as Routine
}

export async function deleteRoutine(client: Client, workspaceId: string, id: string): Promise<void> {
  await client.delete({ url: `${routinesUrl(workspaceId)}/${id}`, throwOnError: true })
}

/** A message posted as its author later (`message`), or a reminder from the agent in the author's DM with it. */
export interface ScheduledMessage {
  id: string
  workspace_id: string
  kind: 'message' | 'reminder'
  conversation_id: string | null
  conversation_name: string | null
  conversation_kind: 'public' | 'private' | 'dm' | null
  body: string
  send_at: number
  /** A sent message has no row any more; `failed` keeps its `error`. */
  status: 'scheduled' | 'failed'
  error: string | null
  created_at: number
  updated_at: number
}

const scheduledUrl = (workspaceId: string) => `/api/v1/workspaces/${workspaceId}/scheduled-messages`

export async function listScheduled(client: Client, workspaceId: string): Promise<ScheduledMessage[]> {
  const { data } = await client.get({ url: scheduledUrl(workspaceId), throwOnError: true })
  return (data as { scheduled: ScheduledMessage[] }).scheduled
}

/** `sendAt` is an ISO time with an offset. */
export async function scheduleMessage(client: Client, workspaceId: string, conversationId: string, body: string, sendAt: string): Promise<ScheduledMessage> {
  const { data } = await client.post({
    url: scheduledUrl(workspaceId),
    body: { kind: 'message', conversation_id: conversationId, body, send_at: sendAt },
    headers: json,
    throwOnError: true,
  })
  return data as ScheduledMessage
}

export async function updateScheduled(client: Client, workspaceId: string, id: string, change: { body?: string; send_at?: string }): Promise<ScheduledMessage> {
  const { data } = await client.patch({ url: `${scheduledUrl(workspaceId)}/${id}`, body: change, headers: json, throwOnError: true })
  return data as ScheduledMessage
}

export async function deleteScheduled(client: Client, workspaceId: string, id: string): Promise<void> {
  await client.delete({ url: `${scheduledUrl(workspaceId)}/${id}`, throwOnError: true })
}
