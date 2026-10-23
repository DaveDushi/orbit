import { type Href, router, useGlobalSearchParams, usePathname } from 'expo-router'
import { createContext, type ReactNode, use, useEffect, useRef, useState } from 'react'
import { Animated, Easing, Modal, Pressable, ScrollView, useWindowDimensions, View } from 'react-native'
import { useSafeAreaInsets } from 'react-native-safe-area-context'

import { useOrbit } from '../orbit-context'
import { useChatUnreadCount, useUnreadNotifications } from '../queries'
import { alpha, cssColors, fonts, mix, type, useTheme } from '../theme'
import { Avatar, CountBadge, Icon, type IconName, IconButton, PickerSheet, Text } from '../ui'

const DrawerContext = createContext<(() => void) | null>(null)

/** The ☰ button of every tab header: opens the sidebar drawer, like the web's mobile topbar. */
export function MenuButton() {
  const { colors } = useTheme()
  const open = use(DrawerContext)
  return <IconButton icon="menu" label="Menu" iconSize={18} color={alpha(colors.mutedForeground, 0.7)} onPress={open ?? undefined} />
}

/** Holds the drawer of the tab screens. Mount once, around the tabs. */
export function DrawerProvider({ children }: { children: ReactNode }) {
  const [open, setOpen] = useState(false)
  return (
    <DrawerContext value={() => setOpen(true)}>
      {children}
      <Drawer open={open} onClose={() => setOpen(false)} />
    </DrawerContext>
  )
}

interface NavLink {
  label: string
  icon: IconName
  href: Href
  active: boolean
  badge?: number
  dot?: boolean
}

/** The web's `SideSheet` with `SidebarNav`: workspace, Search, the products, Personal, Manage, the user. */
function Drawer({ open, onClose }: { open: boolean; onClose: () => void }) {
  const { colors, scheme } = useTheme()
  const { user, workspace, workspaces, selectWorkspace } = useOrbit()
  const insets = useSafeAreaInsets()
  const { width: screen } = useWindowDimensions()
  const width = Math.min(280, screen * 0.84)
  const pathname = usePathname()
  const { view } = useGlobalSearchParams<{ view?: string }>()
  const chatBadge = useChatUnreadCount()
  const inboxUnread = (useUnreadNotifications().data?.length ?? 0) > 0
  const [mounted, setMounted] = useState(open)
  const [switching, setSwitching] = useState(false)
  const progress = useRef(new Animated.Value(0)).current

  useEffect(() => {
    if (open) setMounted(true)
    Animated.timing(progress, { toValue: open ? 1 : 0, duration: open ? 200 : 150, easing: Easing.out(Easing.cubic), useNativeDriver: true }).start(({ finished }) => {
      if (finished && !open) setMounted(false)
    })
  }, [open, progress])

  const go = (href: Href) => {
    onClose()
    router.navigate(href)
  }
  const onTasks = pathname === '/tasks' || pathname.startsWith('/tasks/')
  const onSettings = pathname === '/settings' || pathname.startsWith('/settings/')
  const onAdmin = pathname.startsWith('/settings/admin')
  const taskView = (name: string) => pathname === '/tasks' && view === name
  const workspaceLinks: NavLink[] = [
    { label: 'Home', icon: 'home', href: '/', active: pathname === '/' },
    { label: 'Tasks', icon: 'tasks', href: '/tasks', active: onTasks && !view },
  ]
  const personal: NavLink[] = [
    { label: 'Activity', icon: 'inbox', href: '/activity', active: pathname === '/activity', dot: inboxUnread },
    { label: 'My tasks', icon: 'tasks', href: '/tasks?view=mine', active: taskView('mine') },
    { label: 'This week', icon: 'calendar', href: '/tasks?view=current_week', active: taskView('current_week') },
    { label: 'My week', icon: 'calendarTick', href: '/tasks?view=my_week', active: taskView('my_week') },
    { label: 'Overdue', icon: 'danger', href: '/tasks?view=overdue', active: taskView('overdue') },
    { label: 'Due soon', icon: 'timer', href: '/tasks?view=due_soon', active: taskView('due_soon') },
  ]
  const dark = scheme === 'dark'

  return (
    <Modal visible={mounted} transparent animationType="none" onRequestClose={onClose} statusBarTranslucent>
      <Animated.View style={{ position: 'absolute', inset: 0, backgroundColor: alpha(cssColors.black, 0.5), opacity: progress }}>
        <Pressable accessibilityLabel="Close menu" style={{ flex: 1 }} onPress={onClose} />
      </Animated.View>
      <Animated.View
        accessibilityLabel="Navigation"
        style={{
          position: 'absolute',
          top: 0,
          bottom: 0,
          left: 0,
          width,
          backgroundColor: colors.sidebar,
          borderRightWidth: 1,
          borderRightColor: colors.sidebarEdge,
          paddingHorizontal: 12,
          paddingTop: insets.top,
          paddingBottom: 12 + insets.bottom,
          transform: [{ translateX: progress.interpolate({ inputRange: [0, 1], outputRange: [-width, 0] }) }],
        }}
      >
        <Pressable
          accessibilityRole="button"
          accessibilityLabel={`Workspace: ${workspace.name}`}
          onPress={() => setSwitching(true)}
          style={{ height: 48, marginBottom: 8, flexDirection: 'row', alignItems: 'center', gap: 7, paddingHorizontal: 6 }}
        >
          <Text numberOfLines={1} style={[type(17, 'bold'), { letterSpacing: -0.34, flexShrink: 1 }]}>
            {workspace.name}
          </Text>
          <Icon name="chevronDown" size={13} color={alpha(colors.mutedForeground, 0.7)} />
        </Pressable>

        <View style={{ paddingHorizontal: 4, paddingBottom: 12 }}>
          <Pressable
            accessibilityRole="button"
            accessibilityLabel="Search"
            onPress={() => go('/tasks?search=1')}
            style={({ pressed }) => ({
              height: 32,
              flexDirection: 'row',
              alignItems: 'center',
              gap: 10,
              paddingLeft: 10,
              paddingRight: 6,
              borderRadius: 8,
              backgroundColor: dark ? alpha(cssColors.white, pressed ? 0.06 : 0.03) : pressed ? colors.border : colors.muted,
              borderWidth: dark ? 1 : 0,
              borderColor: alpha(cssColors.white, 0.06),
            })}
          >
            <Icon name="search" size={16} color={alpha(colors.mutedForeground, 0.9)} />
            <Text style={[type(13, 'medium'), { color: colors.mutedForeground }]}>Search</Text>
          </Pressable>
        </View>

        <ScrollView style={{ flex: 1, marginHorizontal: -4 }} contentContainerStyle={{ flexGrow: 1, gap: 2, paddingHorizontal: 4, paddingBottom: 8 }}>
          {workspaceLinks.map((link) => (
            <NavRow key={link.label} link={link} onPress={() => go(link.href)} />
          ))}
          <SubRow label="Views" onPress={() => go('/tasks')} />
          <NavRow link={{ label: 'Docs', icon: 'docs', href: '/docs', active: pathname.startsWith('/docs') }} onPress={() => go('/docs')} />
          <NavRow link={{ label: 'Chat', icon: 'chat', href: '/chat', active: pathname.startsWith('/chat'), badge: chatBadge }} onPress={() => go('/chat')} />
          <NavRow link={{ label: 'Mail', icon: 'mail', href: '/', active: false }} comingSoon />
          <Section label="Personal" />
          {personal.map((link) => (
            <NavRow key={link.label} link={link} onPress={() => go(link.href)} />
          ))}
          <Section label="Manage" />
          <NavRow link={{ label: 'Settings', icon: 'settings', href: '/settings', active: onSettings && !onAdmin }} onPress={() => go('/settings')} />
          {user.installation_admin ? (
            <>
              {/* the installation, not the workspace: it sits apart at the bottom */}
              <View style={{ flex: 1 }} />
              <NavRow link={{ label: 'Admin', icon: 'admin', href: '/settings/admin', active: onAdmin }} onPress={() => go('/settings/admin')} />
            </>
          ) : null}
        </ScrollView>

        <Pressable
          accessibilityRole="button"
          accessibilityLabel={`Account: ${user.display_name}`}
          onPress={() => go('/settings/profile')}
          style={({ pressed }) => ({
            height: 32,
            marginTop: 8,
            flexDirection: 'row',
            alignItems: 'center',
            gap: 8,
            paddingHorizontal: 8,
            borderRadius: 10,
            borderWidth: 1,
            borderColor: dark ? alpha(cssColors.white, 0.08) : colors.border,
            backgroundColor: dark ? alpha(cssColors.white, pressed ? 0.07 : 0.04) : pressed ? colors.muted : colors.background,
          })}
        >
          <Avatar id={user.id} name={user.display_name} size={20} presence={user.status.presence} />
          <Text numberOfLines={1} style={[type(12, 'medium'), { flex: 1 }]}>
            {user.display_name}
          </Text>
          <Icon name="chevronDown" size={14} color={dark ? colors.sidebarNavMuted : alpha(colors.mutedForeground, 0.7)} />
        </Pressable>
      </Animated.View>
      <PickerSheet
        visible={switching}
        title="Workspaces"
        options={workspaces.map((item) => ({ key: item.id, label: item.name, subtitle: item.role, selected: item.id === workspace.id }))}
        onSelect={(id) => {
          setSwitching(false)
          onClose()
          selectWorkspace(id)
        }}
        onClose={() => setSwitching(false)}
      />
    </Modal>
  )
}

/** The web `SidebarNavItem`: a 36px row with a 30px icon tile; the current row's tile turns pink. */
function NavRow({ link, onPress, comingSoon = false }: { link: NavLink; onPress?: () => void; comingSoon?: boolean }) {
  const { colors } = useTheme()
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={link.badge ? `${link.label}, ${link.badge} unread` : link.dot ? `${link.label}, unread` : link.label}
      accessibilityState={{ selected: link.active, disabled: comingSoon }}
      disabled={comingSoon || !onPress}
      onPress={onPress}
      style={{ height: 36, flexDirection: 'row', alignItems: 'center', gap: 10, paddingLeft: 4, paddingRight: 10, opacity: comingSoon ? 0.48 : 1 }}
    >
      {({ pressed }) => (
        <>
          <Tile icon={link.icon} active={link.active} pressed={pressed} />
          <Text numberOfLines={1} style={[type(13, 'medium'), { flex: 1, color: link.active || pressed ? colors.sidebarNavActive : colors.sidebarNav }]}>
            {link.label}
          </Text>
          {comingSoon ? <Text style={[type(10, 'medium'), { color: alpha(colors.mutedForeground, 0.7) }]}>Coming soon</Text> : null}
          {link.badge ? <CountBadge count={link.badge} /> : link.dot ? <View style={{ width: 8, height: 8, marginRight: 4, borderRadius: 4, backgroundColor: colors.primary }} /> : null}
        </>
      )}
    </Pressable>
  )
}

function Tile({ icon, active, pressed }: { icon: IconName; active: boolean; pressed: boolean }) {
  const { colors, scheme } = useTheme()
  const dark = scheme === 'dark'
  const look = active
    ? dark
      ? { background: colors.primary, border: mix(colors.primary, 0.75, cssColors.white), edge: mix(colors.primary, 0.6, cssColors.black), icon: cssColors.white }
      : { background: cssColors.white, border: alpha(colors.primary, 0.45), edge: mix(colors.primary, 0.45, cssColors.white), icon: colors.primary }
    : dark
      ? { background: alpha(cssColors.white, 0.03), border: alpha(cssColors.white, 0.07), edge: alpha(cssColors.white, 0.05), icon: colors.sidebarNav }
      : { background: colors.sidebar, border: alpha(cssColors.black, 0.09), edge: alpha(cssColors.black, 0.22), icon: colors.sidebarNav }
  return (
    <View
      style={{
        width: 30,
        height: 30,
        borderRadius: 10,
        borderWidth: 1,
        borderColor: look.border,
        backgroundColor: look.background,
        alignItems: 'center',
        justifyContent: 'center',
        boxShadow: pressed ? undefined : `0 2px 0 ${look.edge}`,
        transform: [{ translateY: pressed ? 2 : 0 }],
      }}
    >
      <Icon name={icon} size={16} color={look.icon} />
    </View>
  )
}

/** "Views" nested under Tasks behind a 1px guide line. */
function SubRow({ label, onPress }: { label: string; onPress: () => void }) {
  const { colors, scheme } = useTheme()
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel="Task views"
      onPress={onPress}
      style={{ height: 28, marginLeft: 20, justifyContent: 'center', paddingLeft: 12, borderLeftWidth: 1, borderLeftColor: alpha(scheme === 'dark' ? cssColors.white : cssColors.black, scheme === 'dark' ? 0.06 : 0.08) }}
    >
      <Text style={[type(13, 'medium'), { color: colors.sidebarNav }]}>{label}</Text>
    </Pressable>
  )
}

function Section({ label }: { label: string }) {
  const { colors, scheme } = useTheme()
  const dark = scheme === 'dark'
  return (
    <Text
      style={{
        marginTop: 12,
        paddingHorizontal: 10,
        paddingVertical: 4,
        fontSize: dark ? 10.5 : 11,
        lineHeight: 16,
        fontFamily: dark ? fonts.mono : fonts.medium,
        letterSpacing: dark ? 0.84 : 0,
        color: colors.sidebarNavMuted,
      }}
    >
      {dark ? label.toUpperCase() : label}
    </Text>
  )
}
