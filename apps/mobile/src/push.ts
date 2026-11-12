/**
 * Push notifications on the iPhone: after sign-in the app asks once for permission, then registers its APNs device
 * token with the Orbit server, which pushes through APNs itself (no Expo push service). A tapped notification opens
 * its conversation or Activity. `push.web.ts` replaces this in the web preview, which has no APNs.
 */
import * as Notifications from 'expo-notifications'
import { router } from 'expo-router'
import { useEffect } from 'react'
import { Platform } from 'react-native'

import type { OrbitClient } from './data/orbit'
import { appPathOf, registerDevice, unregisterDevice } from './data/push'

// A push that arrives while the app is open shows as a banner too.
Notifications.setNotificationHandler({
  handleNotification: async () => ({ shouldShowBanner: true, shouldShowList: true, shouldPlaySound: true, shouldSetBadge: false }),
})

/** The token this app registered for the current session, so sign-out can remove it. */
let registered: string | null = null

async function register(client: OrbitClient) {
  if (Platform.OS !== 'ios') return
  const permission = await Notifications.getPermissionsAsync()
  const granted = permission.granted || (permission.canAskAgain && (await Notifications.requestPermissionsAsync()).granted)
  if (!granted) return
  const { data } = await Notifications.getDevicePushTokenAsync()
  if (typeof data !== 'string') return
  await registerDevice(client, data, Platform.isPad ? 'iPad' : 'iPhone')
  registered = data
}

/** Registers the phone for the signed-in session, and opens the screen of a tapped notification. */
export function usePushNotifications(client: OrbitClient) {
  useEffect(() => {
    register(client).catch(() => undefined)
  }, [client])

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
  if (token) await unregisterDevice(client, token).catch(() => undefined)
}
