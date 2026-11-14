/**
 * The iPhone's push notifications: the device registration calls and where a tapped notification goes. No React, so
 * a Bun test checks the same functions. The server sends through APNs (`apps/server/src/push/apns.rs`).
 */
import { getPushBadge, registerPushDevice, unregisterPushDevice } from '@orbit/api/generated/sdk.gen'

import type { OrbitClient } from './orbit'

/** Registers this phone's APNs token for the signed-in session; signing out (or the session ending) stops it. */
export async function registerDevice(client: OrbitClient, token: string, label: string): Promise<void> {
  await registerPushDevice({ client, body: { token, label }, throwOnError: true })
}

export async function unregisterDevice(client: OrbitClient, token: string): Promise<void> {
  await unregisterPushDevice({ client, path: { token }, throwOnError: true })
}

/** The number on the app icon: unread chat (as the Chat tab counts it) and unread inbox, in every workspace. */
export async function appBadge(client: OrbitClient): Promise<number> {
  return (await getPushBadge({ client, throwOnError: true })).data.count
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

/** The person turned notifications off on this phone (Settings → Notifications); the app then does not register it. */
export const PUSH_OFF_KEY = 'orbit.push.off'

export type ThisDevice = 'unsupported' | 'blocked' | 'on' | 'off'

/**
 * This phone's row in Notifications: `blocked` when iOS refuses and will not ask again (only the iOS Settings app can
 * allow it), `on` when iOS allows it and the person did not turn it off here, else `off`.
 */
export function thisDeviceState({ ios, granted, canAskAgain, off }: { ios: boolean; granted: boolean; canAskAgain: boolean; off: boolean }): ThisDevice {
  if (!ios) return 'unsupported'
  if (!granted && !canAskAgain) return 'blocked'
  return granted && !off ? 'on' : 'off'
}

/** Registration runs on each start only while iOS allows it, or may still ask, and the person did not turn it off. */
export const shouldRegister = ({ off, granted, canAskAgain }: { off: boolean; granted: boolean; canAskAgain: boolean }) => !off && (granted || canAskAgain)
