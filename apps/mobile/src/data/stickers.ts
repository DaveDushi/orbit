/** The workspace's stickers for Settings → Stickers: the calls and the server's rules (`chat/stickers`), without React. */
import { createChatSticker, deleteChatSticker, listChatStickers } from '@orbit/api/generated/sdk.gen'
import type { CustomStickerRecord } from '@orbit/api/generated/types.gen'
import { ApiProblem } from '@orbit/api/problem'

import { multipart, type UploadFile } from './account-api'
import type { OrbitClient } from './orbit'

export type { CustomStickerRecord }

export const STICKER_IMAGE_TYPES = ['image/png', 'image/jpeg', 'image/webp', 'image/gif']
export const STICKER_MAX_BYTES = 512 * 1024
export const STICKER_NAME_MAX = 30
export const STICKER_LIMIT = 100

/** Two names are the same sticker name when only the case of their ASCII letters differs, as on the server. */
const foldCase = (name: string) => name.replace(/[A-Z]/g, (letter) => letter.toLowerCase())

/** A name to start from, made of a file name: `Party Parrot.gif` gives `Party Parrot`. */
export function stickerNameFromFile(fileName: string): string {
  const base = fileName.replace(/\.[^.]*$/, '').trim()
  return Array.from(base).slice(0, STICKER_NAME_MAX).join('').trim()
}

/** Why a new sticker cannot be added, in words for the form; `null` when the server will take it (the web's rules). */
export function stickerProblem(file: { type: string; size: number | null }, name: string, names: readonly string[]): string | null {
  const trimmed = name.trim()
  const length = Array.from(trimmed).length
  // a picker that does not know the type sends none: the server decides by the bytes
  if (file.type !== '' && !STICKER_IMAGE_TYPES.includes(file.type)) return 'Choose a PNG, JPEG, WebP or GIF image.'
  if ((file.size ?? 0) > STICKER_MAX_BYTES) return 'The image is too large. The limit is 512 KB.'
  if (length < 2 || length > STICKER_NAME_MAX || /\p{Cc}/u.test(trimmed)) return 'A name has 2 to 30 characters.'
  if (names.some((used) => foldCase(used) === foldCase(trimmed))) return `A sticker named ${trimmed} exists already. Choose another name.`
  return null
}

const SERVER_PROBLEMS: Record<string, string> = {
  chat_forbidden: 'Only workspace owners and administrators can manage stickers.',
  sticker_too_large: 'The image is too large. The limit is 512 KB.',
  storage_quota_exceeded: 'The workspace storage is full. Delete files, or ask the administrator of this Orbit for a higher quota.',
  invalid_sticker: 'The file is not a PNG, JPEG, WebP or GIF image.',
  validation_failed: 'A name has 2 to 30 characters.',
  sticker_name_taken: 'A sticker with this name exists already. Choose another name.',
  sticker_limit_reached: 'The workspace has 100 stickers, which is the limit. Delete one first.',
}

/** The reason in words: ours for a known code, else what the server said, else that it did not answer. */
export function stickerServerProblem(error: unknown, fallback: string): string {
  if (!(error instanceof ApiProblem)) return `${fallback} Orbit did not answer. Check your connection and try again.`
  return SERVER_PROBLEMS[error.code] ?? `${fallback} ${error.detail} (${error.status} ${error.code})`
}

export async function stickers(client: OrbitClient, workspaceId: string): Promise<CustomStickerRecord[]> {
  return (await listChatStickers({ client, path: { workspace_id: workspaceId }, throwOnError: true })).data
}

export async function addSticker(client: OrbitClient, workspaceId: string, name: string, file: UploadFile): Promise<CustomStickerRecord> {
  return (await createChatSticker({ client, path: { workspace_id: workspaceId }, query: { name }, body: { file: file as Blob }, bodySerializer: multipart(file), throwOnError: true })).data
}

export async function removeSticker(client: OrbitClient, workspaceId: string, stickerId: string): Promise<void> {
  await deleteChatSticker({ client, path: { workspace_id: workspaceId, sticker_id: stickerId }, throwOnError: true })
}
