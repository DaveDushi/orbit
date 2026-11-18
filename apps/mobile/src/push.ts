/**
 * Push notifications on the iPhone: after sign-in the app asks once for permission, then registers its APNs device
 * token with the Orbit server, which pushes through APNs itself (no Expo push service). A tapped notification opens
 * its conversation or Activity. The app icon's badge is the server's count of unread chat and inbox: each push sets
 * it, the server sends it alone when it changes without a shown push (a read on another device), and the app sets it
 * again when it comes to the foreground and when the counts it shows change (something was read, or arrived). `push.web.ts` replaces this in the web preview, which has no APNs.
 */
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import * as Notifications from 'expo-notifications'
import { router, usePathname } from 'expo-router'
import { useEffect, useState } from 'react'
import { Keyboard, Linking, Platform } from 'react-native'

import { chatQueryKeys } from './chat-cache'
import { keys } from './data/keys'
import type { OrbitClient } from './data/orbit'
import { appBadge, appPathOf, isOpenOnScreen, notificationUrl, PUSH_OFF_KEY, registerDevice, shouldRegister, thisDeviceState, type ThisDevice, unregisterDevice, workspaceOf } from './data/push'
import { useIsGuest, useOrbit } from './orbit-context'
import { useChatBadge, useUnreadNotifications } from './queries'
import { storage } from './storage'

/** The screen on show, so a push about it does not show a banner over it. */
let openPath: string | null = null

// A push that arrives while the app is open shows as a banner too (the server pushes also while the person is active in
// Orbit), but not over the conversation or thread it is about: the message shows there, without a sound.
Notifications.setNotificationHandler({
  handleNotification: async ({ request }) => {
    const open = isOpenOnScreen(notificationUrl(request.content.data, request.trigger), openPath)
    return { shouldShowBanner: !open, shouldShowList: true, shouldPlaySound: !open, shouldSetBadge: true }
  },
})

/** The token this app registered for the current session, so sign-out can remove it. */
let registered: string | null = null

async function register(client: OrbitClient) {
  if (Platform.OS !== 'ios') return
  const permission = await Notifications.getPermissionsAsync()
  const off = (await storage.get(PUSH_OFF_KEY)) === '1'
  if (!shouldRegister({ off, granted: permission.granted, canAskAgain: permission.canAskAgain })) return
  const granted = permission.granted || (await Notifications.requestPermissionsAsync()).granted
  if (!granted) return
  const { data } = await Notifications.getDevicePushTokenAsync()
  if (typeof data !== 'string') return
  await registerDevice(client, data, Platform.isPad ? 'iPad' : 'iPhone')
  registered = data
}

/** Registers the phone for the signed-in session, keeps the icon's badge, and opens the screen of a tapped notification. */
export function usePushNotifications(client: OrbitClient) {
  useEffect(() => {
    register(client).catch(() => undefined)
  }, [client])

  const ios = Platform.OS === 'ios'
  const badge = useQuery({ queryKey: keys.appBadge, queryFn: () => appBadge(client), enabled: ios, refetchOnWindowFocus: 'always' })
  // The counts of this workspace that the app shows: when one changes, the total is asked for again.
  const chat = useChatBadge().count
  const inbox = useUnreadNotifications(!useIsGuest()).data?.length ?? 0
  const { refetch } = badge
  useEffect(() => {
    if (ios) void refetch()
  }, [ios, refetch, chat, inbox])
  useEffect(() => {
    if (badge.data !== undefined) Notifications.setBadgeCountAsync(badge.data).catch(() => undefined)
  }, [badge.data])

  const pathname = usePathname()
  useEffect(() => {
    openPath = pathname
  }, [pathname])

  // A tap opens its screen in the push's workspace: when that is not the open one, the app switches first, and opens
  // the screen once the switch is done.
  const { workspaceId, workspaces, selectWorkspace } = useOrbit()
  const queryClient = useQueryClient()
  const [tapped, setTapped] = useState<{ path: string; workspaceId: string | null } | null>(null)
  const response = Notifications.useLastNotificationResponse()
  useEffect(() => {
    if (!response || response.actionIdentifier !== Notifications.DEFAULT_ACTION_IDENTIFIER) return
    const { content, trigger } = response.notification.request
    const url = notificationUrl(content.data, trigger)
    Notifications.clearLastNotificationResponse()
    const target = workspaceOf(url)
    const known = target !== null && workspaces.some((workspace) => workspace.id === target)
    setTapped({ path: appPathOf(url), workspaceId: known ? target : null })
    // Once for each tap: the list of workspaces is a new array on each render.
  }, [response])
  useEffect(() => {
    if (!tapped) return
    if (tapped.workspaceId && tapped.workspaceId !== workspaceId) return selectWorkspace(tapped.workspaceId)
    // The keyboard of the screen that was open closes. A conversation or thread opens over the Chat tab, so Back
    // goes to the chat list.
    Keyboard.dismiss()
    // The list opens at its newest message: a copy from the disk cache that is not yet stale (the app killed less
    // than 30 s before the tap) does not have the new message, so it loads again.
    const thread = /^\/chat\/thread\/([\w-]+)$/.exec(tapped.path)?.[1]
    const conversation = /^\/chat\/([\w-]+)$/.exec(tapped.path)?.[1]
    if (thread) void queryClient.invalidateQueries({ queryKey: chatQueryKeys.thread(workspaceId, thread) })
    else if (conversation) void queryClient.invalidateQueries({ queryKey: keys.messages(workspaceId, conversation) })
    if (tapped.path.startsWith('/chat/')) {
      router.navigate('/chat')
      router.push(tapped.path as never)
    } else router.navigate(tapped.path as never)
    setTapped(null)
    // `selectWorkspace` is a new function on each render that does the same: it does not run this again.
  }, [tapped, workspaceId])
}

/** Stops the pushes to this phone; called before sign-out, while the session still works. */
export async function unregisterPush(client: OrbitClient) {
  const token = registered
  registered = null
  await Notifications.setBadgeCountAsync(0).catch(() => false)
  if (token) await unregisterDevice(client, token).catch(() => undefined)
}

const deviceKey = ['push-this-device'] as const

/**
 * Settings → Notifications → This iPhone: whether it gets notifications, and turning them on or off here. Off stays
 * off on the next start (`PUSH_OFF_KEY`); on asks iOS when it still may, and opens the iOS Settings when it refuses.
 */
export function useThisDevice(client: OrbitClient) {
  const queryClient = useQueryClient()
  const state = useQuery({
    queryKey: deviceKey,
    queryFn: async (): Promise<ThisDevice> => {
      if (Platform.OS !== 'ios') return 'unsupported'
      const permission = await Notifications.getPermissionsAsync()
      const off = (await storage.get(PUSH_OFF_KEY)) === '1'
      return thisDeviceState({ ios: true, granted: permission.granted, canAskAgain: permission.canAskAgain, off })
    },
    refetchOnWindowFocus: 'always',
  })
  const refresh = () => void queryClient.invalidateQueries({ queryKey: deviceKey })
  const turnOn = useMutation({
    mutationFn: async () => {
      const permission = await Notifications.getPermissionsAsync()
      if (!permission.granted && !permission.canAskAgain) {
        await Linking.openSettings()
        return
      }
      await storage.remove(PUSH_OFF_KEY)
      await register(client)
    },
    onSettled: refresh,
  })
  const turnOff = useMutation({
    mutationFn: async () => {
      await storage.set(PUSH_OFF_KEY, '1')
      const token = registered ?? (await Notifications.getDevicePushTokenAsync().then(({ data }) => (typeof data === 'string' ? data : null)).catch(() => null))
      registered = null
      if (token) await unregisterDevice(client, token)
    },
    onSettled: refresh,
  })
  return { state, turnOn, turnOff }
}
