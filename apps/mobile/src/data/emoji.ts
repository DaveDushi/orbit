/**
 * Settings → Emoji without React: the server's rules for a new custom emoji (the web's `lib/customEmoji.ts`), the
 * words for its refusals, and the calls. Owners and admins only (`chat.manage`).
 */
import { createChatEmoji, deleteChatEmoji } from '@orbit/api/generated/sdk.gen'
import type { CustomEmojiRecord } from '@orbit/api/generated/types.gen'
import { ApiProblem } from '@orbit/api/problem'

import { multipart, type UploadFile } from './account-api'
import type { OrbitClient } from './orbit'

const NAME = /^[a-z0-9_]{2,32}$/
export const EMOJI_IMAGE_TYPES = ['image/png', 'image/jpeg', 'image/webp', 'image/gif']
export const EMOJI_MAX_BYTES = 256 * 1024
export const EMOJI_LIMIT = 200

/** A name to start from, made of a file name: `Party Parrot.gif` gives `party_parrot`. */
export function emojiNameFromFile(fileName: string): string {
  return fileName
    .replace(/\.[^.]*$/, '')
    .toLowerCase()
    .replace(/[^a-z0-9_]+/g, '_')
    .replace(/^_+|_+$/g, '')
    .slice(0, 32)
}

/** Why a new emoji cannot be added, in words for the form; `null` when the server will take it. */
export function emojiProblem(file: { type: string; size: number | null }, name: string, taken: (name: string) => boolean): string | null {
  if (file.type !== '' && !EMOJI_IMAGE_TYPES.includes(file.type)) return 'Choose a PNG, JPEG, WebP or GIF image.'
  if (file.size !== null && file.size > EMOJI_MAX_BYTES) return 'The image is too large. The limit is 256 KB.'
  if (!NAME.test(name)) return 'A name has 2 to 32 characters: lower case letters, digits and underscores.'
  if (taken(name)) return `:${name}: exists already. Choose another name.`
  return null
}

const SERVER_PROBLEMS: Record<string, string> = {
  chat_forbidden: 'Only workspace owners and administrators can manage custom emoji.',
  emoji_too_large: 'The image is too large. The limit is 256 KB.',
  invalid_emoji: 'The file is not a PNG, JPEG, WebP or GIF image.',
  validation_failed: 'A name has 2 to 32 characters: lower case letters, digits and underscores.',
  emoji_name_taken: 'An emoji with this name exists already. Choose another name.',
  emoji_limit_reached: 'The workspace has 200 custom emoji, which is the limit. Delete one first.',
  storage_quota_exceeded: 'The workspace storage is full. Delete files, or ask the administrator of this Orbit for a higher quota.',
}

/** The reason in words: ours for a known code, else what the server said, else that it did not answer. */
export function emojiServerProblem(error: unknown, fallback: string): string {
  if (!(error instanceof ApiProblem)) return `${fallback} Orbit did not answer. Check your connection and try again.`
  return SERVER_PROBLEMS[error.code] ?? `${fallback} ${error.detail} (${error.status} ${error.code})`
}

export async function addEmoji(client: OrbitClient, workspaceId: string, name: string, file: UploadFile): Promise<CustomEmojiRecord> {
  return (await createChatEmoji({ client, path: { workspace_id: workspaceId }, query: { name }, body: { file: file as Blob }, bodySerializer: multipart(file), throwOnError: true })).data
}

export async function removeEmoji(client: OrbitClient, workspaceId: string, emoji: Pick<CustomEmojiRecord, 'id'>): Promise<void> {
  await deleteChatEmoji({ client, path: { workspace_id: workspaceId, emoji_id: emoji.id }, throwOnError: true })
}
