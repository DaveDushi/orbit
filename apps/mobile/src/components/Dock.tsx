import { type Href, router } from 'expo-router'
import type { BottomTabBarProps } from 'expo-router/js-tabs'
import { Pressable, View } from 'react-native'
import { useSafeAreaInsets } from 'react-native-safe-area-context'

import { useChatUnreadCount } from '../queries'
import { alpha, type, useTheme } from '../theme'
import { CountBadge, Icon, type IconName, Text } from '../ui'

/** The web `MobileDock`: the tab routes in its order, Mail disabled as "coming soon". `route` is null for Mail. */
const ITEMS: { route: string | null; href: Href; label: string; icon: IconName }[] = [
  { route: 'index', href: '/', label: 'Home', icon: 'home' },
  { route: 'tasks', href: '/tasks', label: 'Tasks', icon: 'tasks' },
  { route: 'docs', href: '/docs', label: 'Docs', icon: 'docs' },
  { route: 'chat', href: '/chat', label: 'Chat', icon: 'chat' },
  { route: null, href: '/', label: 'Mail', icon: 'mail' },
  { route: 'settings', href: '/settings', label: 'Settings', icon: 'settings' },
]

export function Dock({ state }: BottomTabBarProps) {
  const { colors } = useTheme()
  const insets = useSafeAreaInsets()
  const chatBadge = useChatUnreadCount()
  const current = state.routes[state.index]?.name
  return (
    <View accessibilityRole="tablist" style={{ flexDirection: 'row', minHeight: 56, paddingTop: 12, paddingBottom: insets.bottom, backgroundColor: colors.background }}>
      {ITEMS.map((item) => {
        const enabled = item.route !== null
        const active = enabled && item.route === current
        const count = item.route === 'chat' ? chatBadge : 0
        const color = active ? colors.primary : enabled ? colors.mutedForeground : alpha(colors.mutedForeground, 0.5)
        return (
          <Pressable
            key={item.label}
            accessibilityRole="tab"
            accessibilityLabel={count > 0 ? `${item.label}, ${count} unread` : item.label}
            accessibilityState={{ selected: active, disabled: !enabled }}
            disabled={!enabled}
            onPress={() => router.navigate(item.href)}
            style={{ flex: 1, height: 44, alignItems: 'center', justifyContent: 'center', gap: 2 }}
          >
            <View>
              <Icon name={item.icon} size={20} color={color} />
              {count > 0 ? (
                <View style={{ position: 'absolute', top: -4, left: 12 }}>
                  <CountBadge count={count} />
                </View>
              ) : null}
            </View>
            <Text style={[type(10, 'medium'), { color }]}>{item.label}</Text>
          </Pressable>
        )
      })}
    </View>
  )
}
