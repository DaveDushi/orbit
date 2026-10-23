import { type Href, router, useNavigation, usePathname } from 'expo-router'
import type { ReactNode } from 'react'
import { KeyboardAvoidingView, Platform, Pressable, RefreshControl, ScrollView, Switch, View } from 'react-native'
import { SafeAreaView } from 'react-native-safe-area-context'

import { useOrbit } from '../../orbit-context'
import { alpha, cssColors, setSchemePreference, type, useTheme } from '../../theme'
import { Icon, type IconName, IconButton, PaneHeader, Text } from '../../ui'
import { MenuButton } from '../Drawer'

export interface SettingsNavItem {
  href: Href & string
  label: string
  icon: IconName
}

/**
 * The web's settings areas at phone size. API tokens, Emoji, Stickers, GitHub and Billing are desktop setup work, and
 * Keyboard shortcuts and Storage (this browser's chat cache) have no phone counterpart, so they are left out.
 */
export const WORKSPACE_NAV: readonly SettingsNavItem[] = [
  { href: '/settings', label: 'General', icon: 'settings' },
  { href: '/settings/members', label: 'Members', icon: 'people' },
]

export const ACCOUNT_NAV: readonly SettingsNavItem[] = [
  { href: '/settings/profile', label: 'Profile', icon: 'user' },
  { href: '/settings/notifications', label: 'Notifications', icon: 'bell' },
  { href: '/settings/security', label: 'Security', icon: 'lock' },
  { href: '/settings/sessions', label: 'Sessions', icon: 'shield' },
]

/** The instance's General (registration, limits, email) and Storage (S3) pages are operator setup: on the web only. */
export const ADMIN_NAV: readonly SettingsNavItem[] = [
  { href: '/settings/admin', label: 'Users', icon: 'people' },
  { href: '/settings/admin/audit', label: 'Audit log', icon: 'clipboardText' },
  { href: '/settings/admin/backups', label: 'Backups', icon: 'archiveBox' },
  { href: '/settings/admin/jobs', label: 'Background jobs', icon: 'cpu' },
]

/**
 * The web's `SettingsShell` on a phone: the topbar, the two-column sub-navigation between hairlines, then the cards.
 * A page opened from another screen gets a back button; the area's own pages swap in place.
 */
export function SettingsShell({ title, nav, children, refreshing, onRefresh }: { title: string; nav: readonly SettingsNavItem[]; children: ReactNode; refreshing?: boolean; onRefresh?: () => void }) {
  const { colors, scheme } = useTheme()
  const navigation = useNavigation()
  const pathname = usePathname()
  const muted = alpha(colors.mutedForeground, 0.7)
  return (
    <SafeAreaView edges={['top', 'left', 'right']} style={{ flex: 1, backgroundColor: colors.background }}>
      <PaneHeader>
        {/* only this stack's history: the tab navigator around it can always go back */}
        {(navigation.getState()?.index ?? 0) > 0 ? <IconButton icon="back" label="Back" color={muted} onPress={() => navigation.goBack()} /> : <MenuButton />}
        <Text numberOfLines={1} style={[type(13.5, 'medium'), { flex: 1 }]}>
          {title}
        </Text>
        <View style={{ flexDirection: 'row', gap: 6 }}>
          <IconButton icon="search" label="Search" color={muted} onPress={() => router.navigate('/tasks?search=1')} />
          <IconButton icon={scheme === 'dark' ? 'sun' : 'moon'} label="Toggle theme" iconSize={15} color={muted} onPress={() => setSchemePreference(scheme === 'dark' ? 'light' : 'dark')} />
        </View>
      </PaneHeader>
      <KeyboardAvoidingView style={{ flex: 1 }} behavior={Platform.OS === 'ios' ? 'padding' : undefined}>
        <ScrollView
          contentContainerStyle={{ paddingHorizontal: 20, paddingTop: 20, paddingBottom: 32, gap: 24 }}
          keyboardShouldPersistTaps="handled"
          refreshControl={onRefresh ? <RefreshControl refreshing={refreshing ?? false} onRefresh={onRefresh} tintColor={colors.mutedForeground} /> : undefined}
        >
          <View accessibilityRole="tablist" accessibilityLabel={title} style={{ flexDirection: 'row', flexWrap: 'wrap', gap: 2, paddingVertical: 12, borderTopWidth: 1, borderBottomWidth: 1, borderColor: colors.border }}>
            {nav.map((item) => {
              const active = pathname === item.href
              return (
                <Pressable
                  key={item.href}
                  accessibilityRole="tab"
                  accessibilityLabel={item.label}
                  accessibilityState={{ selected: active }}
                  onPress={active ? undefined : () => router.replace(item.href)}
                  style={({ pressed }) => ({
                    width: '49.6%',
                    height: 32,
                    flexDirection: 'row',
                    alignItems: 'center',
                    gap: 10,
                    paddingHorizontal: 10,
                    borderRadius: 10,
                    backgroundColor: active ? colors.sidebarAccent : pressed ? colors.muted : 'transparent',
                  })}
                >
                  <Icon name={item.icon} size={18} color={alpha(active ? colors.sidebarAccentForeground : colors.foreground, 0.9)} />
                  <Text numberOfLines={1} style={[type(13, 'medium'), { flex: 1 }]}>
                    {item.label}
                  </Text>
                </Pressable>
              )
            })}
          </View>
          {children}
        </ScrollView>
      </KeyboardAvoidingView>
    </SafeAreaView>
  )
}

/** The Admin area: only instance admins see it (the server refuses the others anyway). */
export function AdminShell({ children, refreshing, onRefresh }: { children: ReactNode; refreshing?: boolean; onRefresh?: () => void }) {
  const { user } = useOrbit()
  return (
    <SettingsShell title="Admin" nav={ADMIN_NAV} refreshing={refreshing} onRefresh={onRefresh}>
      {user.installation_admin ? children : <Note>Only the instance admins of this Orbit server see the Admin area.</Note>}
    </SettingsShell>
  )
}

/** The web `SettingsCard`: a muted title strip (with its actions) on the card surface, the body on an inset panel. */
export function SettingsCard({ title, description, actions, flush = false, children }: { title: string; description?: string; actions?: ReactNode; flush?: boolean; children?: ReactNode }) {
  const { colors } = useTheme()
  return (
    <View style={{ borderRadius: 10, backgroundColor: colors.card, boxShadow: `0 0 0 1px ${colors.border}` }}>
      <View style={{ minHeight: 48, justifyContent: 'center', gap: 8, paddingVertical: 10, paddingRight: 8, paddingLeft: 16 }}>
        <View>
          <Text style={[type(14, 'medium'), { lineHeight: 20, color: colors.mutedForeground }]}>{title}</Text>
          {description ? <Text style={[type(12, 'medium'), { lineHeight: 16, marginTop: 2, color: colors.mutedForeground }]}>{description}</Text> : null}
        </View>
        {actions ? <View style={{ flexDirection: 'row', flexWrap: 'wrap', gap: 8 }}>{actions}</View> : null}
      </View>
      {children !== undefined ? (
        <View style={{ padding: flush ? 0 : 16, borderRadius: 10, overflow: 'hidden', backgroundColor: colors.background, boxShadow: `0 0 0 1px ${colors.muted}` }}>{children}</View>
      ) : null}
    </View>
  )
}

/** One row of a flush card; rows after the first get the hairline above. Only a row with `onPress` is a button. */
export function SettingsRow({ children, first = false, onPress, label }: { children: ReactNode; first?: boolean; onPress?: () => void; label?: string }) {
  const { colors } = useTheme()
  const style = { minHeight: 48, flexDirection: 'row', alignItems: 'center', gap: 12, paddingHorizontal: 16, paddingVertical: 12, borderTopWidth: first ? 0 : 1, borderTopColor: colors.border } as const
  if (!onPress) return <View style={style}>{children}</View>
  return (
    <Pressable accessibilityRole="button" accessibilityLabel={label} onPress={onPress} style={({ pressed }) => [style, { backgroundColor: pressed ? colors.muted : 'transparent' }]}>
      {children}
    </Pressable>
  )
}

/** The square icon tile at the start of a settings row. */
export function RowIcon({ icon }: { icon: IconName }) {
  const { colors } = useTheme()
  return (
    <View style={{ width: 36, height: 36, borderRadius: 8, backgroundColor: colors.muted, alignItems: 'center', justifyContent: 'center' }}>
      <Icon name={icon} size={18} color={colors.mutedForeground} />
    </View>
  )
}

/** A row's title and its quiet second line. */
export function RowText({ title, subtitle, badges }: { title: string; subtitle?: string; badges?: ReactNode }) {
  const { colors } = useTheme()
  return (
    <View style={{ flex: 1, minWidth: 0, gap: 2 }}>
      <View style={{ flexDirection: 'row', alignItems: 'center', gap: 6, flexWrap: 'wrap' }}>
        <Text numberOfLines={1} style={[type(13, 'medium'), { flexShrink: 1 }]}>
          {title}
        </Text>
        {badges}
      </View>
      {subtitle ? (
        <Text numberOfLines={2} style={[type(12), { color: alpha(colors.mutedForeground, 0.85) }]}>
          {subtitle}
        </Text>
      ) : null}
    </View>
  )
}

export type BadgeTone = 'primary' | 'secondary' | 'outline' | 'destructive'

/** The web `Badge`: a 20px pill. */
export function Badge({ label, tone = 'primary' }: { label: string; tone?: BadgeTone }) {
  const { colors, scheme } = useTheme()
  const look = {
    primary: { background: colors.primary, text: colors.primaryForeground, border: 'transparent' },
    secondary: { background: colors.secondary, text: colors.secondaryForeground, border: 'transparent' },
    outline: { background: 'transparent', text: colors.foreground, border: colors.border },
    destructive: { background: alpha(colors.destructive, scheme === 'dark' ? 0.2 : 0.1), text: colors.destructive, border: 'transparent' },
  }[tone]
  return (
    <View style={{ height: 20, justifyContent: 'center', paddingHorizontal: 8, borderRadius: 999, borderWidth: 1, borderColor: look.border, backgroundColor: look.background }}>
      <Text style={[type(12, 'medium'), { lineHeight: 16, color: look.text }]}>{label}</Text>
    </View>
  )
}

/** The web `Switch` in the theme's colors. */
export function Toggle({ value, label, onChange, disabled }: { value: boolean; label: string; onChange: (value: boolean) => void; disabled?: boolean }) {
  const { colors } = useTheme()
  return (
    <Switch
      accessibilityLabel={label}
      value={value}
      disabled={disabled}
      onValueChange={onChange}
      trackColor={{ false: colors.input, true: colors.primary }}
      thumbColor={cssColors.white}
      ios_backgroundColor={colors.input}
      // react-native-web paints the on thumb teal unless told otherwise
      {...({ activeThumbColor: cssColors.white } as object)}
    />
  )
}

/** A choice between a few options in one row (the web's small `Select`s). */
export function Segmented<T extends string>({ value, options, onChange, label }: { value: T; options: readonly { value: T; label: string }[]; onChange: (value: T) => void; label: string }) {
  const { colors } = useTheme()
  return (
    <View accessibilityRole="radiogroup" accessibilityLabel={label} style={{ flexDirection: 'row', padding: 2, gap: 2, borderRadius: 10, backgroundColor: colors.muted }}>
      {options.map((option) => {
        const selected = option.value === value
        return (
          <Pressable
            key={option.value}
            accessibilityRole="radio"
            accessibilityLabel={option.label}
            accessibilityState={{ selected }}
            onPress={() => onChange(option.value)}
            style={{ flex: 1, height: 28, alignItems: 'center', justifyContent: 'center', borderRadius: 8, backgroundColor: selected ? colors.background : 'transparent', boxShadow: selected ? `0 1px 2px ${alpha(colors.foreground, 0.12)}` : undefined }}
          >
            <Text style={[type(13, 'medium'), { color: selected ? colors.foreground : colors.mutedForeground }]}>{option.label}</Text>
          </Pressable>
        )
      })}
    </View>
  )
}

/** A quiet status or error line inside a card. */
export function Note({ children, error = false }: { children: ReactNode; error?: boolean }) {
  const { colors } = useTheme()
  return <Text style={[type(13), { lineHeight: 19, color: error ? colors.destructive : colors.mutedForeground }]}>{children}</Text>
}
