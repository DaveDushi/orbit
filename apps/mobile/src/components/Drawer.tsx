import { type Href, router, useGlobalSearchParams, usePathname } from 'expo-router'
import { createContext, type ReactNode, use, useRef, useState } from 'react'
import { Animated, Easing, Modal, PanResponder, type PanResponderGestureState, Pressable, ScrollView, useWindowDimensions, View } from 'react-native'
import { NativeText as RNText } from '../ui/scaled'
import { useSafeAreaInsets } from 'react-native-safe-area-context'
import Svg, { Defs, LinearGradient, Rect, Stop } from 'react-native-svg'

import { useOrbit } from '../orbit-context'
import { useChatUnreadCount, useUnreadNotifications } from '../queries'
import { splitAlpha } from '../data/color'
import { alpha, cssColors, mix, useTheme } from '../theme'
import { Avatar, CountBadge, Icon, type IconName, IconButton, PickerSheet } from '../ui'

const DrawerContext = createContext<(() => void) | null>(null)

/** The ☰ button of every tab header: opens the sidebar drawer, like the web's mobile topbar. */
export function MenuButton() {
  const { colors } = useTheme()
  const open = use(DrawerContext)
  return <IconButton icon="menu" label="Menu" iconSize={18} color={alpha(colors.mutedForeground, 0.7)} onPress={open ?? undefined} />
}

/** The tab roots: on a pushed screen the same edge swipe is iOS's back gesture. */
const SWIPE_ROOTS = new Set(['/', '/tasks', '/docs', '/chat', '/settings', '/activity'])
/** A swipe opens the drawer only when it starts this close to the left edge. */
const EDGE = 24

/** A clear horizontal move, so taps and vertical scrolling stay with the screen. */
const horizontal = (gesture: PanResponderGestureState, direction: 1 | -1) =>
  gesture.dx * direction > 10 && Math.abs(gesture.dx) > Math.abs(gesture.dy) * 1.5

const clamp = (value: number) => Math.min(1, Math.max(0, value))

/**
 * Holds the drawer of the tab screens. Mount once, around the tabs. The ☰ button opens it; a swipe from the left
 * edge of a tab root drags it open and a swipe to the left drags it closed, following the finger.
 */
export function DrawerProvider({ children }: { children: ReactNode }) {
  const { width: screen } = useWindowDimensions()
  const width = Math.min(280, screen * 0.84)
  const pathname = usePathname()
  const [mounted, setMounted] = useState(false)
  const progress = useRef(new Animated.Value(0)).current
  // The pan responders are created once: they read the current values through this ref.
  const live = useRef({ width, mounted, swipeable: false })
  live.current = { width, mounted, swipeable: SWIPE_ROOTS.has(pathname) }

  const settle = (open: boolean, velocity = 0) => {
    if (open) setMounted(true)
    const duration = Math.abs(velocity) > 1 ? 120 : open ? 200 : 150
    Animated.timing(progress, { toValue: open ? 1 : 0, duration, easing: Easing.out(Easing.cubic), useNativeDriver: true }).start(({ finished }) => {
      if (finished && !open) setMounted(false)
    })
  }

  // Where the touch began: `gesture.x0` is not that on every platform (react-native-web), so record it.
  const touchStartX = useRef(Number.POSITIVE_INFINITY)
  const edge = useRef(
    PanResponder.create({
      onStartShouldSetPanResponderCapture: (event) => {
        touchStartX.current = event.nativeEvent.pageX
        return false
      },
      onMoveShouldSetPanResponderCapture: (_, gesture) => live.current.swipeable && !live.current.mounted && touchStartX.current <= EDGE && horizontal(gesture, 1),
      onPanResponderGrant: () => {
        progress.setValue(0)
        setMounted(true)
      },
      onPanResponderMove: (_, gesture) => progress.setValue(clamp(gesture.dx / live.current.width)),
      onPanResponderRelease: (_, gesture) => settle(gesture.dx > live.current.width / 3 || gesture.vx > 0.5, gesture.vx),
      onPanResponderTerminate: () => settle(false),
    }),
  ).current

  const back = useRef(
    PanResponder.create({
      onMoveShouldSetPanResponderCapture: (_, gesture) => horizontal(gesture, -1),
      onPanResponderMove: (_, gesture) => progress.setValue(clamp(1 + gesture.dx / live.current.width)),
      onPanResponderRelease: (_, gesture) => settle(!(gesture.dx < -live.current.width / 3 || gesture.vx < -0.5), gesture.vx),
      onPanResponderTerminate: () => settle(true),
    }),
  ).current

  return (
    <DrawerContext value={() => settle(true)}>
      <View className="flex-1" {...edge.panHandlers}>
        {children}
      </View>
      <Drawer mounted={mounted} progress={progress} width={width} panHandlers={back.panHandlers} onClose={() => settle(false)} />
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
function Drawer({
  mounted,
  progress,
  width,
  panHandlers,
  onClose,
}: {
  mounted: boolean
  progress: Animated.Value
  width: number
  panHandlers: ReturnType<typeof PanResponder.create>['panHandlers']
  onClose: () => void
}) {
  const { colors, scheme } = useTheme()
  const { user, workspace, workspaces, selectWorkspace } = useOrbit()
  const insets = useSafeAreaInsets()
  const pathname = usePathname()
  const { view } = useGlobalSearchParams<{ view?: string }>()
  const chatBadge = useChatUnreadCount()
  const inboxUnread = (useUnreadNotifications().data?.length ?? 0) > 0
  const [switching, setSwitching] = useState(false)

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
      <View className="flex-1" {...panHandlers}>
      <Animated.View style={{ position: 'absolute', inset: 0, backgroundColor: alpha(cssColors.black, 0.5), opacity: progress }}>
        <Pressable accessibilityLabel="Close menu" className="flex-1" onPress={onClose} />
      </Animated.View>
      {/* Animated: only the slide; the look is the web's classes on the inner view */}
      <Animated.View
        accessibilityLabel="Navigation"
        style={{ position: 'absolute', top: 0, bottom: 0, left: 0, width, transform: [{ translateX: progress.interpolate({ inputRange: [0, 1], outputRange: [-width, 0] }) }] }}
      >
        <View className="flex-1 border-r border-sidebar-edge bg-sidebar px-3" style={{ paddingTop: insets.top, paddingBottom: 12 + insets.bottom }}>
          <Wash color={colors.sidebarWash} width={width} />
          <Pressable
            accessibilityRole="button"
            accessibilityLabel={`Workspace: ${workspace.name}`}
            onPress={() => setSwitching(true)}
            className="mb-2 h-12 flex-row items-center gap-[7px] px-1.5"
          >
            <RNText numberOfLines={1} className="shrink font-noto-bold text-[17px] tracking-[-0.34px] text-foreground">
              {workspace.name}
            </RNText>
            <Icon name="chevronDown" size={13} color={alpha(colors.mutedForeground, 0.7)} />
          </Pressable>

          <View className="px-1 pb-3">
            <Pressable
              accessibilityRole="button"
              accessibilityLabel="Search"
              onPress={() => go('/tasks?search=1')}
              className="h-8 flex-row items-center gap-2.5 rounded-md bg-neutral-100 pr-1.5 pl-2.5 active:bg-neutral-200 dark:border dark:border-sidebar-tint/10 dark:bg-sidebar-tint/6 dark:active:bg-sidebar-tint/10"
            >
              <Icon name="search" size={16} color={alpha(colors.mutedForeground, 0.9)} />
              <RNText className="font-noto-medium text-[13px] text-muted-foreground">Search</RNText>
            </Pressable>
          </View>

          <ScrollView className="-mx-1 flex-1" contentContainerClassName="grow gap-0.5 px-1 pb-2">
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
                <View className="flex-1" />
                <NavRow link={{ label: 'Admin', icon: 'admin', href: '/settings/admin', active: onAdmin }} tone="neutral" onPress={() => go('/settings/admin')} />
              </>
            ) : null}
          </ScrollView>

          <Pressable
            accessibilityRole="button"
            accessibilityLabel={`Account: ${user.display_name}`}
            onPress={() => go('/settings/profile')}
            className="mt-2 h-8 flex-row items-center gap-2 rounded-[10px] border border-border bg-background px-2 active:bg-muted dark:border-sidebar-tint/24 dark:bg-primary/14 dark:active:border-sidebar-tint/34 dark:active:bg-primary/18"
          >
            <Avatar id={user.id} name={user.display_name} size={20} presence={user.status.presence} />
            <RNText numberOfLines={1} className="flex-1 font-noto-medium text-[12px] text-foreground dark:text-[#fbe9ee]">
              {user.display_name}
            </RNText>
            <Icon name="chevronDown" size={14} color={dark ? colors.sidebarTint : alpha(colors.mutedForeground, 0.7)} />
          </Pressable>
        </View>
      </Animated.View>
      </View>
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
function NavRow({ link, onPress, comingSoon = false, tone = 'brand' }: { link: NavLink; onPress?: () => void; comingSoon?: boolean; tone?: TileTone }) {
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={link.badge ? `${link.label}, ${link.badge} unread` : link.dot ? `${link.label}, unread` : link.label}
      accessibilityState={{ selected: link.active, disabled: comingSoon }}
      disabled={comingSoon || !onPress}
      onPress={onPress}
      className={`h-9 flex-row items-center gap-2.5 pr-2.5 pl-1 ${comingSoon ? 'opacity-[0.48]' : ''}`}
    >
      {({ pressed }) => (
        <>
          <Tile icon={link.icon} active={link.active} pressed={pressed} tone={tone} />
          <RNText numberOfLines={1} className={`flex-1 font-noto-medium text-[13px] ${link.active || pressed ? 'text-sidebar-nav-active' : 'text-sidebar-nav'}`}>
            {link.label}
          </RNText>
          {comingSoon ? <RNText className="font-noto-medium text-[10px] text-muted-foreground/70">Coming soon</RNText> : null}
          {link.badge ? <CountBadge count={link.badge} /> : link.dot ? <View className="mr-1 size-2 rounded-full bg-primary" /> : null}
        </>
      )}
    </Pressable>
  )
}

/** `brand`: the workspace's apps, pink-tinted in dark mode. `neutral`: Admin, which is the installation, not the workspace. */
type TileTone = 'brand' | 'neutral'

/**
 * The web's `sidebarNavTileVariants`. The 2px ledge (`shadow-[0_2px_0_var(--tile-depth)]`) and the current tile's
 * `color-mix` border stay styles: they are computed colors.
 */
function Tile({ icon, active, pressed, tone }: { icon: IconName; active: boolean; pressed: boolean; tone: TileTone }) {
  const { colors, scheme } = useTheme()
  const dark = scheme === 'dark'
  const className = active
    ? 'bg-white border-primary/45 dark:bg-primary'
    : tone === 'brand'
      ? 'bg-sidebar border-black/9 dark:border-sidebar-tint/13 dark:bg-primary/7'
      : 'bg-sidebar border-black/9 dark:border-white/7 dark:bg-white/3'
  const edge = active
    ? dark ? mix(colors.primary, 0.6, cssColors.black) : mix(colors.primary, 0.45, cssColors.white)
    : dark ? alpha(cssColors.white, 0.05) : alpha(cssColors.black, 0.22)
  const icon_ = active ? (dark ? cssColors.white : colors.primary) : dark ? (tone === 'brand' ? colors.sidebarTintForeground : NEUTRAL_TILE_ICON_DARK) : colors.sidebarNav
  return (
    <View
      className={`size-[30px] items-center justify-center rounded-[10px] border ${className}`}
      style={{
        borderColor: active && dark ? mix(colors.primary, 0.75, cssColors.white) : undefined,
        boxShadow: pressed ? undefined : `0 2px 0 ${edge}`,
        transform: [{ translateY: pressed ? 2 : 0 }],
      }}
    >
      <Icon name={icon} size={16} color={icon_} />
    </View>
  )
}

/** "Views" nested under Tasks behind a 1px guide line. */
function SubRow({ label, onPress }: { label: string; onPress: () => void }) {
  return (
    <Pressable accessibilityRole="button" accessibilityLabel="Task views" onPress={onPress} className="ml-5 h-7 justify-center border-l border-black/8 pl-3 dark:border-sidebar-tint/10">
      <RNText className="font-noto-medium text-[13px] text-sidebar-nav">{label}</RNText>
    </Pressable>
  )
}

/** The web's section label: mono, uppercase and pink-tinted in dark mode. */
function Section({ label }: { label: string }) {
  const { scheme } = useTheme()
  return (
    <RNText className="mt-3 px-2.5 py-1 font-noto-medium text-[11px] leading-4 text-sidebar-nav-muted dark:font-mono dark:text-[10.5px] dark:tracking-[0.84px] dark:text-sidebar-tint/55">
      {scheme === 'dark' ? label.toUpperCase() : label}
    </RNText>
  )
}

/** The web's `oklch(0.72 0 0)` neutral tile icon (dark). */
const NEUTRAL_TILE_ICON_DARK = '#a4a4a4'

/** The web sidebar's `bg-linear-to-b from-sidebar-wash to-transparent to-[520px]`: a pink wash fading out from the top. */
function Wash({ color, width }: { color: string; width: number }) {
  const wash = splitAlpha(color)
  return (
    <Svg pointerEvents="none" width={width} height={520} style={{ position: 'absolute', top: 0, left: 0 }}>
      <Defs>
        <LinearGradient id="sidebar-wash" x1="0" y1="0" x2="0" y2="1">
          <Stop offset="0" stopColor={wash.color} stopOpacity={wash.opacity} />
          <Stop offset="1" stopColor={wash.color} stopOpacity={0} />
        </LinearGradient>
      </Defs>
      <Rect width="100%" height="100%" fill="url(#sidebar-wash)" />
    </Svg>
  )
}
