import { Redirect, usePathname } from 'expo-router'
import { Tabs } from 'expo-router/js-tabs'

import { AskDonnaBar } from '../../components/AskDonna'
import { BillingBanner } from '../../components/BillingBanner'
import { Dock } from '../../components/Dock'
import { DrawerProvider } from '../../components/Drawer'
import { MailServerBanner } from '../../components/MailServerBanner'
import { guestHome, guestMayOpen } from '../../data/guest'
import { useLiveUpdates } from '../../live'
import { useIsGuest, useOrbit } from '../../orbit-context'
import { usePushNotifications } from '../../push'
import { useTheme } from '../../theme'

export default function TabsLayout() {
  const { colors } = useTheme()
  useLiveUpdates()
  usePushNotifications(useOrbit().client)
  const guest = useIsGuest()
  const pathname = usePathname()
  // a guest only opens Chat and its account settings: any other tab sends it to Chat
  if (guest && !guestMayOpen(pathname)) return <Redirect href={guestHome} />
  return (
    <DrawerProvider>
      <Tabs tabBar={(props) => (
          <>
            <AskDonnaBar />
            <MailServerBanner />
            <BillingBanner />
            <Dock {...props} />
          </>
        )} screenOptions={{ headerShown: false, sceneStyle: { backgroundColor: colors.background } }}>
        <Tabs.Screen name="index" />
        <Tabs.Screen name="tasks" />
        <Tabs.Screen name="docs" />
        <Tabs.Screen name="chat" />
        <Tabs.Screen name="settings" />
        {/* reached from the drawer, like the web's Activity: no dock item */}
        <Tabs.Screen name="activity" />
      </Tabs>
    </DrawerProvider>
  )
}
