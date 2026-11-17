import type { ChatEvent } from '@orbit/api/generated/types.gen'
import { useQueryClient } from '@tanstack/react-query'
import * as Network from 'expo-network'
import { useEffect, useState } from 'react'
import { AppState, Platform } from 'react-native'

import { chatKeys, keys, workspaceDataKeys } from './data/keys'
import {
  chatResyncKeys,
  createRefreshQueue,
  LiveCursor,
  type NetworkSnapshot,
  networkReconnects,
  parseLiveFrame,
  parseWorkspaceEvent,
  socketUrl,
  workspaceEventChangesProfile,
  workspaceEventKeys,
} from './data/live'
import { currentUser } from './data/orbit'
import { reconnectingSocket, type SocketLike } from './data/socket'
import { applyChatEvents } from './chat-cache'
import { useOrbit } from './orbit-context'

/** React Native's `WebSocket` takes the upgrade headers as a third argument. The DOM typings in scope do not know it. */
const NativeWebSocket = WebSocket as unknown as new (url: string, protocols: null, options: { headers: Record<string, string> }) => SocketLike

const isChatEvent = (value: unknown): value is ChatEvent => typeof value === 'object' && value !== null && typeof (value as { type?: unknown }).type === 'string'

const snapshot = (state: Network.NetworkState): NetworkSnapshot => ({ connected: state.isConnected !== false, type: state.type })

/** `live`: both sockets are open. `connecting`: one is down and retries. Only the native app has sockets. */
export type LiveStatus = 'live' | 'connecting'

/**
 * Keeps the workspace's queries fresh while the app is open: the workspace events socket marks task, project, inbox,
 * docs and Drive queries stale, and the live socket patches chat in place. Both send the session as a bearer token in
 * the upgrade. In the background both close; back in the foreground, online again or on another network they connect
 * at once. After any gap the app fetches again what it shows, so nothing needs a pull to refresh.
 */
export function useLiveUpdates(): LiveStatus {
  const { client, session, workspaceId, setUser } = useOrbit()
  const queryClient = useQueryClient()
  const [status, setStatus] = useState<LiveStatus>('live')

  useEffect(() => {
    // A browser WebSocket cannot send the bearer header, so the web preview runs without live updates.
    if (Platform.OS === 'web') return
    const { server, token } = session
    const create = (url: string) => new NativeWebSocket(url, null, { headers: { Authorization: `Bearer ${token}` } })
    const queue = createRefreshQueue({
      invalidate: (queryKey) => void queryClient.invalidateQueries({ queryKey }),
      busy: () => queryClient.isMutating() > 0,
    })
    // A closed session: the next request gets the 401 that signs out.
    const ended = () => void queryClient.invalidateQueries({ queryKey: keys.workspaces })

    const open = new Set<string>()
    const opened = (name: string) => {
      open.add(name)
      if (open.size === 2) setStatus('live')
    }
    const dropped = (name: string) => {
      open.delete(name)
      setStatus('connecting')
    }

    let after: string | undefined
    const events = reconnectingSocket({
      url: () => socketUrl(server, `/api/v1/workspaces/${workspaceId}/events`, { after }),
      create,
      onEnded: ended,
      onOpen: (resumed) => {
        opened('events')
        // the badge and the workspace list can change in a gap too (another device, another workspace)
        if (resumed) queue.add([...workspaceDataKeys(workspaceId), keys.workspaces, keys.appBadge])
      },
      onDrop: () => dropped('events'),
      onMessage: (raw) => {
        const event = parseWorkspaceEvent(raw)
        if (!event) return
        after = event.sequence
        queue.add(workspaceEventKeys(event, workspaceId))
        if (workspaceEventChangesProfile(event)) currentUser(client).then(setUser, () => undefined)
      },
    })

    const cursor = new LiveCursor()
    const live = reconnectingSocket({
      url: () => socketUrl(server, `/api/v1/workspaces/${workspaceId}/live`, cursor.query()),
      create,
      onEnded: ended,
      silenceMs: 45_000,
      onOpen: (resumed) => {
        opened('live')
        if (resumed) queue.add(chatKeys(workspaceId))
      },
      onDrop: () => dropped('live'),
      onMessage: (raw) => {
        const frame = parseLiveFrame(raw)
        if (!frame) return
        cursor.read(frame)
        if (frame.type === 'resync') queue.add(chatResyncKeys(workspaceId))
        if (frame.type === 'event' && frame.topic === 'chat' && isChatEvent(frame.event)) {
          applyChatEvents(queryClient, workspaceId, [frame.event])
        }
      },
    })

    const reconnect = () => {
      events.reconnect()
      live.reconnect()
    }
    const appState = AppState.addEventListener('change', (next) => {
      if (next === 'active') reconnect()
      else if (next === 'background') {
        events.close()
        live.close()
      }
    })
    let network: NetworkSnapshot | null = null
    void Network.getNetworkStateAsync().then(
      (state) => (network ??= snapshot(state)),
      () => undefined,
    )
    const networkChange = Network.addNetworkStateListener((state) => {
      const now = snapshot(state)
      if (networkReconnects(network, now) && AppState.currentState === 'active') reconnect()
      network = now
    })
    return () => {
      appState.remove()
      networkChange.remove()
      queue.dispose()
      events.close()
      live.close()
    }
  }, [client, queryClient, session, workspaceId])

  return status
}
