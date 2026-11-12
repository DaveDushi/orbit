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
  /** Of the newest scheduled run: `started` while it runs, then `done` or `failed`; `skipped` (see `last_error`). */
  last_run_status: 'started' | 'done' | 'failed' | 'skipped' | null
  last_run_message_id: string | null
  last_error: string | null
  created_at: number
  updated_at: number
  /** The saved script it runs on each run; `null`: only its instruction. */
  script_id: string | null
  script_name: string | null
  /** The script's code hash now. */
  script_hash: string | null
  /** The creator approved the script's current code: its runs may write. While `false`, script runs write nothing. */
  script_approved: boolean
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
  /** One of the creator's saved scripts, or `''` for none. Another script needs its own approval. */
  script_id?: string
  /** Approves the routine's script at exactly this code hash (refused when the script changed meanwhile). */
  approve_script_hash?: string
}

/** One tool call of a routine run; `would_do` is a write that a test run did not make. */
export interface RoutineRunCall {
  name: string
  args: string
  status: 'ok' | 'error' | 'dry_run'
  would_do?: string
}

/** One of a routine's last 20 runs. */
export interface RoutineRun {
  id: string
  routine_id: string
  /** A test run: in the creator's DM, nothing written. */
  test: boolean
  status: 'started' | 'skipped' | 'done' | 'failed'
  /** `script`: an approved script; `dry`: a test run or a changed script (nothing written). */
  writes: 'normal' | 'script' | 'dry'
  conversation_id: string | null
  message_id: string | null
  script_name: string | null
  script_hash: string | null
  started_at: number
  finished_at: number | null
  output: string | null
  error: string | null
  tool_calls: RoutineRunCall[]
}

/** A script an agent saved for a person; a routine can run it. */
export interface Script {
  id: string
  workspace_id: string
  owner_id: string
  owner_name: string
  name: string
  description: string
  code: string
  /** SHA-256 of the code: a routine's write approval is bound to it. */
  code_hash: string
  created_at: number
  updated_at: number
  /** The routines that run it, and whether their creator approved this exact code. */
  routines: { id: string; name: string; approved: boolean }[]
  /** Only its owner changes it. */
  can_edit: boolean
  /** Its owner, owners and admins delete it. */
  can_delete: boolean
}

export interface ScriptChange {
  name?: string
  description?: string
  code?: string
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

/** Runs the routine now as a test run (its creator only): the result comes in their DM with the agent. */
export async function testRoutine(client: Client, workspaceId: string, id: string): Promise<RoutineRun> {
  const { data } = await client.post({ url: `${routinesUrl(workspaceId)}/${id}/test`, throwOnError: true })
  return data as RoutineRun
}

export async function listRoutineRuns(client: Client, workspaceId: string, id: string): Promise<RoutineRun[]> {
  const { data } = await client.get({ url: `${routinesUrl(workspaceId)}/${id}/runs`, throwOnError: true })
  return (data as { runs: RoutineRun[] }).runs
}

const scriptsUrl = (workspaceId: string) => `/api/v1/workspaces/${workspaceId}/scripts`

export async function listScripts(client: Client, workspaceId: string): Promise<Script[]> {
  const { data } = await client.get({ url: scriptsUrl(workspaceId), throwOnError: true })
  return (data as { scripts: Script[] }).scripts
}

export async function updateScript(client: Client, workspaceId: string, id: string, change: ScriptChange): Promise<Script> {
  const { data } = await client.patch({ url: `${scriptsUrl(workspaceId)}/${id}`, body: change, headers: json, throwOnError: true })
  return data as Script
}

export async function deleteScript(client: Client, workspaceId: string, id: string): Promise<void> {
  await client.delete({ url: `${scriptsUrl(workspaceId)}/${id}`, throwOnError: true })
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
