/**
 * "Share to Orbit" from other iPhone apps (the iOS share extension of `expo-sharing` opens the app with what was
 * shared): what was shared, where it may go, and the order of the writes. No React, so a Bun test checks the same
 * functions. The server checks every write as the person, so these lists only hide what would be refused.
 */
import { isVisible } from './chat'
import type { ConversationRecord } from './orbit'

/** At most this many files in one share: a chat message takes 10 files. */
export const MAX_SHARED_FILES = 10

export type ShareTarget = 'chat' | 'drive' | 'task'

/** Where a person can send a share. A guest has only Chat: the server refuses it Drive and tasks. */
export function shareTargets(guest: boolean): ShareTarget[] {
  return guest ? ['chat'] : ['chat', 'drive', 'task']
}

/** One shared item as `expo-sharing` resolves it (`ResolvedSharePayload`). */
export interface SharedPayload {
  contentType?: 'text' | 'audio' | 'file' | 'video' | 'image' | 'website' | null
  contentUri?: string | null
  contentMimeType?: string | null
  contentSize?: number | null
  originalName?: string | null
  /** The text, or the URL that was shared. */
  value?: string
}

export interface SharedFile {
  uri: string
  name: string
  mimeType: string
  size: number | null
}

export interface SharedContent {
  files: SharedFile[]
  /** Shared text and web links, in order. */
  texts: string[]
}

/** Splits a share into files and text: a web page or link is text (its URL), everything with a file is a file. */
export function sharedContent(payloads: readonly SharedPayload[]): SharedContent {
  const files: SharedFile[] = []
  const texts: string[] = []
  for (const payload of payloads) {
    if (payload.contentType === 'website' || payload.contentType === 'text' || !payload.contentType || !payload.contentUri) {
      const text = (payload.contentType === 'website' ? (payload.value || payload.contentUri) : payload.value)?.trim()
      if (text) texts.push(text)
      continue
    }
    const name = payload.originalName?.trim() || payload.contentUri.split('/').pop() || 'file'
    files.push({ uri: payload.contentUri, name: decodeURIComponent(name), mimeType: payload.contentMimeType || 'application/octet-stream', size: payload.contentSize ?? null })
  }
  return { files: files.slice(0, MAX_SHARED_FILES), texts }
}

/** The text that goes with the share: the person's note first, then the shared text and links. */
export function shareText(note: string, texts: readonly string[]): string {
  return [note.trim(), ...texts].filter((part) => part !== '').join('\n')
}

/** A first title for a new task: the shared text's first line, else the first file's name without its extension. */
export function taskTitleOf(content: SharedContent): string {
  const line = content.texts[0]?.split('\n')[0]?.trim()
  if (line) return line.slice(0, 200)
  const name = content.files[0]?.name
  if (name) return name.replace(/\.[^.]+$/, '') || name
  return ''
}

/**
 * The conversations a share can go to: the ones the person is a member of and that are not archived (what the Chat
 * list shows), the agent's DM first, then by the newest message.
 */
export function shareConversations(conversations: readonly ConversationRecord[], userId: string, agentId: string | null): ConversationRecord[] {
  const agentFirst = (conversation: ConversationRecord) => (agentId !== null && conversation.kind === 'dm' && conversation.member_ids.includes(agentId) && conversation.member_ids.length === 2 ? 0 : 1)
  return conversations
    .filter((conversation) => isVisible(conversation, userId))
    .sort((a, b) => agentFirst(a) - agentFirst(b) || (b.last_message_at ?? '').localeCompare(a.last_message_at ?? ''))
}

/**
 * New task from a share: the task is made first, then each file goes to the new task's id as an attachment, one at a
 * time. A file that fails does not stop the others; its name comes back in `failed`.
 */
export async function shareToNewTask<T extends { id: string }>(
  steps: { createTask: () => Promise<T>; attach: (taskId: string, file: SharedFile) => Promise<unknown> },
  files: readonly SharedFile[],
): Promise<{ task: T; failed: string[] }> {
  const task = await steps.createTask()
  const failed: string[] = []
  for (const file of files) {
    try {
      await steps.attach(task.id, file)
    } catch {
      failed.push(file.name)
    }
  }
  return { task, failed }
}
