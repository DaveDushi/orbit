import { useMutation } from '@tanstack/react-query'
import { router } from 'expo-router'
import { type ReactNode, useState } from 'react'
import { Alert, KeyboardAvoidingView, Platform, Pressable, ScrollView, View } from 'react-native'
import { SafeAreaView } from 'react-native-safe-area-context'

import { MenuButton } from '../../components/Drawer'
import * as api from '../../data/orbit'
import { phoneValid, profileChanges, profileFormOf, type ProfileForm } from '../../data/profile'
import { useOrbit } from '../../orbit-context'
import { errorMessage } from '../../session-context'
import { useOpenOnWeb } from '../../web'
import { alpha, setSchemePreference, type, useTheme } from '../../theme'
import { Avatar, Button, Icon, type IconName, IconButton, Input, PaneHeader, Text } from '../../ui'

/** The web's account pages. Profile is here; the others open in Orbit on the web. */
const ACCOUNT_LINKS: { path: string; label: string; icon: IconName }[] = [
  { path: '/profile', label: 'Profile', icon: 'user' },
  { path: '/profile/notifications', label: 'Notifications', icon: 'inbox' },
  { path: '/profile/security', label: 'Security', icon: 'lock' },
  { path: '/profile/sessions', label: 'Sessions', icon: 'shield' },
  { path: '/profile/shortcuts', label: 'Keyboard shortcuts', icon: 'keyboard' },
  { path: '/profile/storage', label: 'Storage', icon: 'driver' },
]

/** The Settings tab: the web's Profile page (the account area), with the server and Sign out. */
export default function SettingsScreen() {
  const { client, session, user, setUser, signOut } = useOrbit()
  const { colors, scheme } = useTheme()
  const openOnWeb = useOpenOnWeb()
  const saved = profileFormOf(user)
  const [form, setForm] = useState<ProfileForm>(saved)
  const [signingOut, setSigningOut] = useState(false)
  const set = (part: Partial<ProfileForm>) => setForm((current) => ({ ...current, ...part }))
  const changes = profileChanges(user, form)
  const phoneInvalid = form.phone.trim() !== '' && !phoneValid(form.phone.trim())

  const save = useMutation({
    mutationFn: (body: api.UpdateMeBody) => api.updateProfile(client, body),
    onSuccess: (updated) => {
      setUser(updated)
      setForm(profileFormOf(updated))
    },
  })

  return (
    <SafeAreaView edges={['top', 'left', 'right']} style={{ flex: 1, backgroundColor: colors.background }}>
      <PaneHeader>
        <MenuButton />
        <Text numberOfLines={1} style={[type(13.5, 'medium'), { flex: 1 }]}>
          Profile
        </Text>
        <View style={{ flexDirection: 'row', gap: 6 }}>
          <IconButton icon="search" label="Search" color={alpha(colors.mutedForeground, 0.7)} onPress={() => router.navigate('/tasks?search=1')} />
          <IconButton icon={scheme === 'dark' ? 'sun' : 'moon'} label="Toggle theme" iconSize={15} color={alpha(colors.mutedForeground, 0.7)} onPress={() => setSchemePreference(scheme === 'dark' ? 'light' : 'dark')} />
        </View>
      </PaneHeader>
      <KeyboardAvoidingView style={{ flex: 1 }} behavior={Platform.OS === 'ios' ? 'padding' : undefined}>
        <ScrollView contentContainerStyle={{ paddingHorizontal: 20, paddingTop: 20, paddingBottom: 32, gap: 24 }} keyboardShouldPersistTaps="handled">
          <View style={{ flexDirection: 'row', flexWrap: 'wrap', gap: 2, paddingVertical: 12, borderTopWidth: 1, borderBottomWidth: 1, borderColor: colors.border }}>
            {ACCOUNT_LINKS.map((link) => {
              const active = link.path === '/profile'
              return (
                <Pressable
                  key={link.path}
                  accessibilityRole="link"
                  accessibilityState={{ selected: active }}
                  accessibilityLabel={active ? link.label : `${link.label}, opens Orbit on the web`}
                  onPress={active ? undefined : () => openOnWeb(link.path)}
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
                  <Icon name={link.icon} size={18} color={alpha(active ? colors.sidebarAccentForeground : colors.foreground, 0.9)} />
                  <Text numberOfLines={1} style={[type(13, 'medium'), { flex: 1 }]}>
                    {link.label}
                  </Text>
                </Pressable>
              )
            })}
          </View>

          <SettingsCard title="Profile details" description="Your picture, name and what the people in your workspaces see of you.">
            <Avatar id={user.id} name={user.display_name} size={64} />
            <View style={{ marginTop: 16, flexDirection: 'row' }}>
              <Button label="Upload picture" variant="outline" onPress={() => openOnWeb('/profile')} />
            </View>
            <Text style={[type(12), { lineHeight: 16, marginTop: 8, color: colors.mutedForeground }]}>Cropped to a square. People in your workspaces see it.</Text>
            <View style={{ marginTop: 16, gap: 16 }}>
              <Input label="Name" required value={form.name} onChangeText={(name) => set({ name })} autoComplete="name" />
              <Input label="Email" value={user.email} editable={false} />
              <Input label="Title" value={form.title} onChangeText={(title) => set({ title })} placeholder="Product designer" maxLength={100} />
              <Input label="Pronouns" value={form.pronouns} onChangeText={(pronouns) => set({ pronouns })} placeholder="she/her" maxLength={40} />
              <Input
                label="Phone number"
                value={form.phone}
                onChangeText={(phone) => set({ phone })}
                placeholder="+1 555 010 0000"
                keyboardType="phone-pad"
                autoComplete="tel"
                description="The people in your workspaces can see it."
                error={phoneInvalid ? 'Use digits, spaces and + - ( ) . only.' : undefined}
              />
              <Input label="Time zone" value={form.timezone} onChangeText={(timezone) => set({ timezone })} placeholder="Europe/Budapest" autoCapitalize="none" autoCorrect={false} />
              <Input label="Bio" value={form.bio} onChangeText={(bio) => set({ bio })} multiline maxLength={500} />
            </View>
            {save.isError ? <Text style={[type(12), { marginTop: 12, color: colors.destructive }]}>{errorMessage(save.error)}</Text> : null}
            {changes ? (
              <View style={{ marginTop: 16, flexDirection: 'row', gap: 8, justifyContent: 'flex-end' }}>
                <Button label="Discard" variant="outline" onPress={() => setForm(saved)} />
                <Button label="Save" loading={save.isPending} disabled={phoneInvalid} onPress={() => save.mutate(changes)} />
              </View>
            ) : null}
          </SettingsCard>

          <SettingsCard title="Session" description={`Signed in to ${session.server}.`}>
            <View style={{ flexDirection: 'row' }}>
              <Button
                label="Sign out"
                variant="outline"
                icon="logout"
                loading={signingOut}
                onPress={() => {
                  setSigningOut(true)
                  signOut().catch((error: unknown) => {
                    setSigningOut(false)
                    Alert.alert('Could not sign out', errorMessage(error))
                  })
                }}
              />
            </View>
          </SettingsCard>
        </ScrollView>
      </KeyboardAvoidingView>
    </SafeAreaView>
  )
}

/** The web `SettingsCard`: a muted title strip on the card surface, the body on an inset background panel. */
function SettingsCard({ title, description, children }: { title: string; description?: string; children: ReactNode }) {
  const { colors } = useTheme()
  return (
    <View style={{ borderRadius: 10, backgroundColor: colors.card, boxShadow: `0 0 0 1px ${colors.border}` }}>
      <View style={{ minHeight: 48, justifyContent: 'center', paddingVertical: 10, paddingRight: 8, paddingLeft: 16 }}>
        <Text style={[type(14, 'medium'), { lineHeight: 20, color: colors.mutedForeground }]}>{title}</Text>
        {description ? <Text style={[type(12, 'medium'), { lineHeight: 16, marginTop: 2, color: colors.mutedForeground }]}>{description}</Text> : null}
      </View>
      <View style={{ padding: 16, borderRadius: 10, backgroundColor: colors.background, boxShadow: `0 0 0 1px ${colors.muted}` }}>{children}</View>
    </View>
  )
}
