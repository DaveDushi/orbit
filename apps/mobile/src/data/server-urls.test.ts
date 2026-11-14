import { afterEach, describe, expect, test } from 'bun:test'
import { collabSocket } from '@orbit/editor/collab/target'

import eas from '../../eas.json'
import { serverUrl } from './chat-api'
import { fileDeleted } from './docs-api'
import { absoluteUrl } from './docs'
import { socketUrl } from './live'
import { createClient, DEFAULT_SERVER, normalizeServer } from './orbit'
import { registerDevice } from './push'

// a server the user typed at sign-in, not the default: every address must use it
const SERVER = normalizeServer('orbit.self-hosted.example/')

const realFetch = globalThis.fetch
afterEach(() => {
  globalThis.fetch = realFetch
})

/** Replaces the global fetch and returns the URLs it was called with. */
function recordFetch(status = 204): string[] {
  const urls: string[] = []
  globalThis.fetch = (async (input: RequestInfo | URL) => {
    urls.push(input instanceof Request ? input.url : String(input))
    return new Response(null, { status })
  }) as typeof fetch
  return urls
}

describe('the default server', () => {
  test('is the open Orbit host, in the app and in every EAS build profile that sets it', () => {
    expect(DEFAULT_SERVER).toBe('https://orbit-app.easytomanage.com')
    const profiles = Object.values(eas.build) as { env?: { EXPO_PUBLIC_ORBIT_SERVER?: string } }[]
    const set = profiles.map((profile) => profile.env?.EXPO_PUBLIC_ORBIT_SERVER).filter(Boolean)
    expect(set.length).toBeGreaterThan(0)
    for (const value of set) expect(value).toBe(DEFAULT_SERVER)
  })
})

describe('every address uses the signed-in server', () => {
  test('API calls and the push registration', async () => {
    const urls = recordFetch()
    await registerDevice(createClient(SERVER, () => 'token'), 'apns-token', 'iPhone')
    expect(urls).toEqual(['https://orbit.self-hosted.example/api/v1/push/devices'])
  })

  test('file downloads and images', async () => {
    const urls = recordFetch(410)
    expect(await fileDeleted(SERVER, 'token', '/api/v1/files/1')).toBe(true)
    expect(urls).toEqual(['https://orbit.self-hosted.example/api/v1/files/1'])
    expect(absoluteUrl(SERVER, '/api/v1/files/1')).toBe('https://orbit.self-hosted.example/api/v1/files/1')
    expect(serverUrl(SERVER, '/api/v1/emoji/1')).toBe('https://orbit.self-hosted.example/api/v1/emoji/1')
  })

  test('the live sockets and the page editor socket', () => {
    expect(socketUrl(SERVER, '/api/v1/workspaces/w/live')).toBe('wss://orbit.self-hosted.example/api/v1/workspaces/w/live')
    expect(collabSocket(SERVER, { workspaceId: 'w', kind: 'pages', id: 'p', epoch: '1' }).serverUrl).toBe('wss://orbit.self-hosted.example/api/v1/workspaces/w/pages/p')
  })
})
