/** A member's profile card (the web's `ProfilePopover`): the profile, the private note, and how they show. */
import { getMemberProfile, putMemberNote } from '@orbit/api/generated/sdk.gen'
import type { MemberProfile } from '@orbit/api/generated/types.gen'

import type { OrbitClient } from './orbit'

export type { MemberProfile }

export const profileKey = (workspaceId: string, userId: string) => ['members', workspaceId, 'profile', userId] as const

/** The member's profile and the caller's private note about them; `null` when they left the workspace. */
export async function memberProfile(client: OrbitClient, workspaceId: string, userId: string): Promise<MemberProfile | null> {
  try {
    return (await getMemberProfile({ client, path: { workspace_id: workspaceId, user_id: userId }, throwOnError: true })).data
  } catch (error) {
    if ((error as { status?: unknown }).status === 404) return null
    throw error
  }
}

/** Saves the private note; an empty text removes it. */
export async function saveMemberNote(client: OrbitClient, workspaceId: string, userId: string, body: string): Promise<void> {
  await putMemberNote({ client, path: { workspace_id: workspaceId, user_id: userId }, body: { body }, throwOnError: true })
}

/** "3:42 PM local time" in the member's zone; `null` when the zone is missing or unknown here. */
export function localTimeLabel(timezone: string | null | undefined, now: Date | number = Date.now(), locale?: string): string | null {
  if (!timezone) return null
  try {
    return `${new Intl.DateTimeFormat(locale, { hour: 'numeric', minute: '2-digit', timeZone: timezone }).format(now)} local time`
  } catch {
    return null
  }
}

/** The number for a `tel:` link: the digits, with a leading `+` kept. */
export const phoneHref = (phone: string) => `tel:${phone.trim().startsWith('+') ? '+' : ''}${phone.replace(/\D/g, '')}`

/** The text to send for a note, or `null` when it is the saved note. An empty text removes the note. */
export function noteToSave(saved: string | null | undefined, draft: string): string | null {
  const next = draft.trim()
  return next === (saved ?? '').trim() ? null : next
}

/** The role as the card shows it. */
export const roleLabel = (role: string) => role.charAt(0).toUpperCase() + role.slice(1)
