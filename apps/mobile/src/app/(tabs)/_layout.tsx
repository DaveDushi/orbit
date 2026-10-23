import { router } from 'expo-router'
import { Tabs } from 'expo-router/js-tabs'
import { Pressable } from 'react-native'

import { useLiveUpdates } from '../../live'
import { useChatUnreadCount, useUnreadNotificationCount } from '../../queries'
import { useTheme } from '../../theme'
import { Icon, type IconName } from '../../ui'

const TABS: { name: string; title: string; icon: IconName }[] = [
  { name: 'index', title: 'Home', icon: 'home' },
  { name: 'tasks', title: 'Tasks', icon: 'tasks' },
  { name: 'chat', title: 'Chat', icon: 'chat' },
  { name: 'inbox', title: 'Inbox', icon: 'inbox' },
  { name: 'account', title: 'Account', icon: 'user' },
]

const badge = (count: number) => (count > 0 ? (count > 99 ? '99+' : count) : undefined)

export default function TabsLayout() {
  const { colors, fonts, spacing } = useTheme()
  useLiveUpdates()
  const chatUnread = useChatUnreadCount()
  const inboxUnread = useUnreadNotificationCount().data ?? 0
  const badges: Record<string, number> = { chat: chatUnread, inbox: inboxUnread }

  return (
    <Tabs
      screenOptions={{
        tabBarActiveTintColor: colors.primary,
        tabBarInactiveTintColor: colors.mutedForeground,
        tabBarStyle: { backgroundColor: colors.background, borderTopColor: colors.border, borderTopWidth: 1 },
        tabBarLabelStyle: { fontFamily: fonts.medium, fontSize: 10 },
        tabBarBadgeStyle: { backgroundColor: colors.primary, color: colors.primaryForeground, fontFamily: fonts.semibold, fontSize: 10 },
        headerStyle: { backgroundColor: colors.background, borderBottomColor: colors.border, borderBottomWidth: 1 },
        headerShadowVisible: false,
        headerTitleAlign: 'left',
        headerTitleStyle: { fontFamily: fonts.semibold, fontSize: 17, color: colors.foreground },
        sceneStyle: { backgroundColor: colors.background },
      }}
    >
      {TABS.map((tab) => (
        <Tabs.Screen
          key={tab.name}
          name={tab.name}
          options={{
            title: tab.title,
            tabBarBadge: badge(badges[tab.name] ?? 0),
            tabBarIcon: ({ color, size }) => <Icon name={tab.icon} color={color} size={Math.min(size, 24)} />,
            headerRight:
              tab.name === 'tasks'
                ? () => (
                    <Pressable accessibilityRole="button" accessibilityLabel="New task" hitSlop={8} onPress={() => router.push('/task/new')} style={{ paddingHorizontal: spacing[4] }}>
                      <Icon name="plus" size={22} color={colors.foreground} />
                    </Pressable>
                  )
                : undefined,
          }}
        />
      ))}
    </Tabs>
  )
}
