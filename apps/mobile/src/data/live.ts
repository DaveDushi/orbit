/**
 * The two live sockets of a workspace, as data: what a frame says and which queries it touches. The hook in
 * `src/live.ts` only applies the result.
 */
import type { ChatEvent } from '@orbit/api/generated/types.gen'

import { chatKeys, keys, type QueryKey, workspaceDataKeys } from './keys'
import type { ConversationStateRecord, MessageRecord } from './orbit'

/** `https://orbit.example.com` → `wss://orbit.example.com/api/v1/…`. React Native's `URL` has no `protocol` setter. */
export function socketUrl(server: string, path: string, query: Record<string, string | undefined> = {}): string {
  const params = Object.entries(query)
    .filter((entry): entry is [string, string] => entry[1] !== undefined)
    .map(([name, value]) => `${name}=${encodeURIComponent(value)}`)
    .join('&')
  return `${server.replace(/^http/i, 'ws').replace(/\/+$/, '')}${path}${params ? `?${params}` : ''}`
}

// ---- `/api/v1/workspaces/{id}/events`

export interface WorkspaceEvent {
  version: 1
  kind: 'workspace.changed' | 'resync_required'
  sequence: string
  workspaces_changed?: boolean
  /** The signed-in person's profile changed (name, avatar). */
  profile_changed?: boolean
  /** Every change since the last event is a saved view: only the views refresh. */
  views_only?: boolean
}

export function parseWorkspaceEvent(raw: unknown): WorkspaceEvent | null {
  if (typeof raw !== 'string') return null
  try {
    const value = JSON.parse(raw)
    if (value?.version !== 1 || !['workspace.changed', 'resync_required'].includes(value.kind)) return null
    if (typeof value.sequence !== 'string' || !/^(0|[1-9][0-9]*)$/.test(value.sequence)) return null
    return value
  } catch {
    return null
  }
}

/** The person's own profile is fetched again (the app keeps it outside the query cache). */
export const workspaceEventChangesProfile = (event: WorkspaceEvent) => event.kind === 'resync_required' || event.profile_changed === true

/** The queries a workspace event makes stale. A resync, or a change to the workspace list, also refreshes the list. */
export function workspaceEventKeys(event: WorkspaceEvent, workspaceId: string): QueryKey[] {
  if (event.kind !== 'resync_required' && event.views_only === true) return [keys.views(workspaceId)]
  const list = event.kind === 'resync_required' || event.workspaces_changed === true ? [keys.workspaces] : []
  return [...workspaceDataKeys(workspaceId), ...list]
}

// ---- `/api/v1/workspaces/{id}/live`

export type LiveFrame =
  | { type: 'hello'; epoch: string; seq: number }
  | { type: 'resync' }
  | { type: 'event'; seq: number | null; topic: string; event: unknown }

/** `null` for frames the app has no use for (`ping`, typing, presence without a topic) and for garbage. */
export function parseLiveFrame(raw: unknown): LiveFrame | null {
  if (typeof raw !== 'string') return null
  let frame: { type?: unknown; epoch?: unknown; seq?: unknown; topic?: unknown; event?: unknown }
  try {
    frame = JSON.parse(raw)
  } catch {
    return null
  }
  if (frame === null || typeof frame !== 'object') return null
  if (frame.type === 'hello') return { type: 'hello', epoch: String(frame.epoch ?? ''), seq: typeof frame.seq === 'number' ? frame.seq : 0 }
  if (frame.type === 'resync') return { type: 'resync' }
  if (typeof frame.topic === 'string') return { type: 'event', seq: typeof frame.seq === 'number' ? frame.seq : null, topic: frame.topic, event: frame.event }
  return null
}

/** What a chat event does to the cache. */
export type ChatAction =
  | { kind: 'upsert-message'; conversationId: string; message: MessageRecord }
  | { kind: 'remove-message'; conversationId: string; messageId: string }
  | { kind: 'state'; state: ConversationStateRecord }
  | { kind: 'invalidate'; keys: QueryKey[] }

/**
 * Messages and read states are patched in place: they arrive whole. A new message also moves its conversation in the
 * DM order, so the conversation list refetches. Conversations, categories, custom emoji and stickers refetch; the rest
 * (ephemeral answers) the app does not show. Threads are patched in `chat-cache.ts`.
 */
export function chatActions(event: ChatEvent, workspaceId: string): ChatAction[] {
  switch (event.type) {
    case 'message.created':
      return [
        { kind: 'upsert-message', conversationId: event.message.conversation_id, message: event.message },
        { kind: 'invalidate', keys: [keys.conversations(workspaceId)] },
      ]
    case 'message.updated':
      return [{ kind: 'upsert-message', conversationId: event.message.conversation_id, message: event.message }]
    case 'message.deleted':
      return [{ kind: 'remove-message', conversationId: event.conversation_id, messageId: event.message_id }]
    case 'state.changed':
      return [{ kind: 'state', state: event.state }]
    case 'conversation.changed':
      return [{ kind: 'invalidate', keys: [keys.conversations(workspaceId)] }]
    case 'conversation.removed':
      // its read state and its threads go with it
      return [{ kind: 'invalidate', keys: [keys.conversations(workspaceId), keys.chatStates(workspaceId), ['messages', workspaceId]] }]
    case 'categories.changed':
      return [{ kind: 'invalidate', keys: [keys.chatCategories(workspaceId)] }]
    case 'emoji.changed':
      return [{ kind: 'invalidate', keys: [keys.chatEmoji(workspaceId)] }]
    case 'stickers.changed':
      return [{ kind: 'invalidate', keys: [['chat-stickers', workspaceId], ['custom-stickers', workspaceId]] }]
    default:
      return []
  }
}

/** Events were missed and cannot be replayed: refetch everything chat shows. */
export const chatResyncKeys = (workspaceId: string) => chatKeys(workspaceId)

/**
 * Where the live socket resumes. It sends the epoch and the number of the last event it got, so the server replays what
 * was missed; a new epoch starts the numbers again.
 */
export class LiveCursor {
  private epoch: string | null = null
  private helloSeq = 0
  private lastSeq: number | null = null

  query(): Record<string, string | undefined> {
    return this.epoch !== null && this.lastSeq !== null ? { epoch: this.epoch, after: String(this.lastSeq) } : {}
  }

  read(frame: LiveFrame) {
    if (frame.type === 'hello') {
      if (frame.epoch !== this.epoch) this.lastSeq = null
      this.epoch = frame.epoch
      this.helloSeq = frame.seq
    } else if (frame.type === 'resync') {
      this.lastSeq = this.helloSeq
    } else if (frame.seq !== null) {
      this.lastSeq = frame.seq
    }
  }
}

// ---- Reconnecting

/** What the app needs to know of the phone's network (`expo-network`'s state). */
export interface NetworkSnapshot {
  connected: boolean
  type: string | undefined
}

/**
 * The sockets connect again at once when the phone is back online or changed network (Wi-Fi ↔ cellular): an open
 * socket on the old network can stay silent for minutes before the system reports it closed.
 */
export function networkReconnects(before: NetworkSnapshot | null, now: NetworkSnapshot): boolean {
  if (!before || !now.connected) return false
  return !before.connected || before.type !== now.type
}

export interface RefreshQueue {
  add(keys: readonly QueryKey[]): void
  dispose(): void
}

/**
 * Collects the queries that events make stale and refetches them together a moment later, once no write of the app
 * is on its way: a refetch then could put the old value back over a change the app shows before the server has it.
 */
export function createRefreshQueue({ invalidate, busy, delayMs = 100 }: { invalidate: (key: QueryKey) => void; busy: () => boolean; delayMs?: number }): RefreshQueue {
  const pending = new Map<string, QueryKey>()
  let timer: ReturnType<typeof setTimeout> | undefined
  const flush = () => {
    timer = undefined
    if (busy()) {
      timer = setTimeout(flush, delayMs)
      return
    }
    const due = [...pending.values()]
    pending.clear()
    for (const key of due) invalidate(key)
  }
  return {
    add(keys) {
      for (const key of keys) pending.set(JSON.stringify(key), key)
      if (pending.size > 0 && timer === undefined) timer = setTimeout(flush, delayMs)
    },
    dispose() {
      clearTimeout(timer)
      timer = undefined
      pending.clear()
    },
  }
}
