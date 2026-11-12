/**
 * The iPhone's push notifications: the device registration calls and where a tapped notification goes. No React, so
 * a Bun test checks the same functions. The server sends through APNs (`apps/server/src/push/apns.rs`).
 */
import { registerPushDevice, unregisterPushDevice } from '@orbit/api/generated/sdk.gen'

import type { OrbitClient } from './orbit'

/** Registers this phone's APNs token for the signed-in session; signing out (or the session ending) stops it. */
export async function registerDevice(client: OrbitClient, token: string, label: string): Promise<void> {
  await registerPushDevice({ client, body: { token, label }, throwOnError: true })
}

export async function unregisterDevice(client: OrbitClient, token: string): Promise<void> {
  await unregisterPushDevice({ client, path: { token }, throwOnError: true })
}

/**
 * The app screen for a notification's `url` (a path of the web app): a conversation, a thread reply in it, or the
 * Activity list; anything else opens Home.
 */
export function appPathOf(url: unknown): string {
  if (typeof url !== 'string' || !url.startsWith('/')) return '/'
  const [path = '', query = ''] = url.split('?')
  const chat = /^\/chat\/([\w-]+)$/.exec(path)
  if (chat) {
    const thread = new URLSearchParams(query).get('thread')
    return thread && /^[\w-]+$/.test(thread) ? `/chat/thread/${thread}` : `/chat/${chat[1]}`
  }
  if (path === '/activity') return '/activity'
  if (path === '/profile/notifications') return '/settings/notifications'
  return '/'
}
