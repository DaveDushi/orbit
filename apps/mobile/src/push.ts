/**
 * Push notifications on the iPhone: after sign-in the app asks once for permission, then registers its APNs device
 * token with the Orbit server, which pushes through APNs itself (no Expo push service). A tapped notification opens
 * its conversation or Activity. The app icon's badge is the server's count of unread chat and inbox: each push sets
 * it, and the app sets it again when it comes to the foreground and when the counts it shows change (something was
 * read, or arrived). `push.web.ts` replaces this in the web preview, which has no APNs.
 */
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import * as Notifications from 'expo-notifications'
import { router } from 'expo-router'
import { useEffect } from 'react'
import { Linking, Platform } from 'react-native'

import { keys } from './data/keys'
import type { OrbitClient } from './data/orbit'
import { appBadge, appPathOf, PUSH_OFF_KEY, registerDevice, shouldRegister, thisDeviceState, type ThisDevice, unregisterDevice } from './data/push'
import { useIsGuest } from './orbit-context'
import { useChatBadge, useUnreadNotifications } from './queries'
import { storage } from './storage'

// A push that arrives while the app is open shows as a banner too.
Notifications.setNotificationHandler({
  handleNotification: async () => ({ shouldShowBanner: true, shouldShowList: true, shouldPlaySound: true, shouldSetBadge: true }),
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

  const response = Notifications.useLastNotificationResponse()
  useEffect(() => {
    if (!response || response.actionIdentifier !== Notifications.DEFAULT_ACTION_IDENTIFIER) return
    router.navigate(appPathOf(response.notification.request.content.data?.url) as never)
    Notifications.clearLastNotificationResponse()
  }, [response])
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
