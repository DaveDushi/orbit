/**
 * Forward, Copy link and Create task of the message sheet, as on the web (`features/chat/lib/forward.ts`,
 * `chatRoutes.ts`, `chatPageLib.ts`). No React, so the tests drive it.
 */
import type { ConversationRecord, MessageRecord } from './orbit'

/** How many conversations one forward can go to (Discord's limit). */
export const FORWARD_LIMIT = 5

/**
 * Where a message can be forwarded to: the channels the reader is in and the DMs, without archived ones, latest activity
 * first. `search` keeps those whose title contains it, without regard to case; a `#` at its start is ignored.
 */
export function forwardDestinations(conversations: readonly ConversationRecord[], userId: string, titleOf: (conversation: ConversationRecord) => string, search = ''): ConversationRecord[] {
  const query = search.trim().toLowerCase().replace(/^#/, '')
  const activity = (conversation: ConversationRecord) => conversation.last_message_at ?? conversation.created_at
  return conversations
    .filter((conversation) => conversation.member_ids.includes(userId) && !conversation.archived)
    .filter((conversation) => !query || titleOf(conversation).toLowerCase().replace(/^#/, '').includes(query))
    .sort((a, b) => activity(b).localeCompare(activity(a)))
}

/** Selects a destination or takes it out. One more than `FORWARD_LIMIT` changes nothing. */
export function toggleDestination(selectedIds: readonly string[], conversationId: string): string[] {
  if (selectedIds.includes(conversationId)) return selectedIds.filter((id) => id !== conversationId)
  return selectedIds.length < FORWARD_LIMIT ? [...selectedIds, conversationId] : [...selectedIds]
}

export interface ForwardResult {
  /** The destinations that have the forward, in order. */
  sentIds: string[]
  /** What the first failed destination was refused with. Those after it were not tried. */
  error?: unknown
}

/**
 * Forwards to each destination in turn. `comment` sends the reader's own message to a destination once its forward is
 * there, so it lands right after it. The first failure stops the run.
 */
export async function forwardToEach(conversationIds: readonly string[], forward: (conversationId: string) => Promise<unknown>, comment?: (conversationId: string) => Promise<unknown>): Promise<ForwardResult> {
  const sentIds: string[] = []
  for (const conversationId of conversationIds) {
    try {
      await forward(conversationId)
    } catch (error) {
      return { sentIds, error }
    }
    // a comment that fails does not undo the forward; the next destination still gets its forward
    await comment?.(conversationId)?.catch(() => undefined)
    sentIds.push(conversationId)
  }
  return { sentIds }
}

/** The web address of a message ("Copy link"): in its conversation, or a reply in its thread. */
export function messageLink(server: string, message: Pick<MessageRecord, 'id' | 'conversation_id' | 'thread_root_id'>): string {
  const base = server.replace(/\/+$/, '')
  return message.thread_root_id
    ? `${base}/chat/${message.conversation_id}/thread/${message.thread_root_id}?m=${encodeURIComponent(message.id)}`
    : `${base}/chat/${message.conversation_id}?m=${encodeURIComponent(message.id)}`
}

/** The title of a task made from a message: its first line with text, cut at a word where possible. */
export function taskTitleFromMessage(text: string, limit = 80): string {
  const line = text.split('\n').map((part) => part.trim()).find(Boolean) ?? ''
  if (line.length <= limit) return line
  const cut = line.slice(0, limit)
  const space = cut.lastIndexOf(' ')
  return `${(space > limit * 0.6 ? cut.slice(0, space) : cut).trimEnd()}…`
}
