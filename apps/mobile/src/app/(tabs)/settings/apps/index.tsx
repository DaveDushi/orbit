import { router } from 'expo-router'
import { useState } from 'react'
import { Linking, Pressable, View } from 'react-native'

import { AppRolesField, SecretOnce, SignInSwitch, useAppWrites, useManagedApps } from '../../../../components/settings/Apps'
import { Dialog } from '../../../../components/settings/Dialog'
import { Note, RowIcon, RowText, SettingsCard, SettingsRow, SettingsShell, WORKSPACE_NAV } from '../../../../components/settings/Shell'
import { appFormReady, appIconName, APP_ROLES, redirectLines } from '../../../../data/apps'
import { useOrbit } from '../../../../orbit-context'
import { errorMessage } from '../../../../session-context'
import { Button, Input } from '../../../../ui'

/** The web's Settings → Apps: the workspace's web apps, to add one or open its settings. Owners and admins (`apps.manage`). */
export default function AppsScreen() {
  const { workspace } = useOrbit()
  const canManage = workspace.permissions.includes('apps.manage')
  const apps = useManagedApps(canManage)
  const [adding, setAdding] = useState(false)
  return (
    <SettingsShell title="Settings" nav={WORKSPACE_NAV} refreshing={apps.isRefetching} onRefresh={canManage ? () => void apps.refetch() : undefined}>
      {!canManage ? (
        <SettingsCard title="Apps">
          <Note>Only workspace owners and administrators can set up apps.</Note>
        </SettingsCard>
      ) : (
        <>
          <SecretOnce />
          <SettingsCard title="Apps" description="Web apps your team opens from the Orbit sidebar. On a phone they open in the browser." actions={<Button label="Add app" size="sm" disabled={!apps.data} onPress={() => setAdding(true)} />} flush>
            {apps.isPending ? <View className="p-4"><Note loading>Loading apps…</Note></View> : null}
            {apps.isError ? <View className="p-4"><Note error>Apps could not be loaded.</Note></View> : null}
            {apps.data?.items.length === 0 ? <View className="p-4"><Note>No apps yet.</Note></View> : null}
            {apps.data?.items.map((app, index) => (
              <SettingsRow key={app.id} first={index === 0}>
                {/* the link and Open side by side: a button inside a button is not allowed */}
                <Pressable accessibilityRole="link" accessibilityLabel={`Settings of ${app.name}`} onPress={() => router.push(`/settings/apps/${app.id}`)} className="flex-1 min-w-0 flex-row items-center gap-3 active:opacity-70">
                  <RowIcon icon={appIconName(app.icon)} />
                  <RowText title={app.name} subtitle={app.url} />
                </Pressable>
                <Button label="Open" variant="outline" size="sm" onPress={() => void Linking.openURL(app.url)} />
              </SettingsRow>
            ))}
          </SettingsCard>
          {apps.data ? <AddAppDialog visible={adding} canSignIn={apps.data.can_sign_in} onClose={() => setAdding(false)} /> : null}
        </>
      )}
    </SettingsShell>
  )
}

function AddAppDialog({ visible, canSignIn, onClose }: { visible: boolean; canSignIn: boolean; onClose: () => void }) {
  const { create } = useAppWrites()
  const [form, setForm] = useState({ name: '', url: '', roles: [...APP_ROLES], signIn: false, redirects: '' })
  const set = (fields: Partial<typeof form>) => setForm((current) => ({ ...current, ...fields }))
  const ready = appFormReady(form)
  const submit = () => {
    if (!ready || create.isPending) return
    create.mutate(
      { name: form.name.trim(), url: form.url.trim(), allowed_roles: form.roles, ...(form.signIn ? { redirect_uris: redirectLines(form.redirects) } : {}) },
      {
        onSuccess: (created) => {
          onClose()
          setForm({ name: '', url: '', roles: [...APP_ROLES], signIn: false, redirects: '' })
          // the secret and the values the app needs are on its settings page
          if (form.signIn) router.push(`/settings/apps/${created.id}`)
        },
      },
    )
  }
  return (
    <Dialog
      visible={visible}
      title="Add app"
      onClose={onClose}
      footer={
        <>
          <Button label="Cancel" variant="outline" disabled={create.isPending} onPress={onClose} />
          <Button label="Add app" loading={create.isPending} disabled={!ready} onPress={submit} />
        </>
      }
    >
      <Input label="Name" required maxLength={80} placeholder="Reports" value={form.name} onChangeText={(name) => set({ name })} />
      <Input label="Address" required autoCapitalize="none" autoCorrect={false} keyboardType="url" maxLength={2048} placeholder="https://reports.example.com" value={form.url} onChangeText={(url) => set({ url })} />
      <AppRolesField value={form.roles} onChange={(roles) => set({ roles })} />
      <SignInSwitch checked={form.signIn} disabled={!canSignIn} onChange={(signIn) => set({ signIn })} />
      {form.signIn ? (
        <Input label="Redirect URIs" multiline autoCapitalize="none" autoCorrect={false} placeholder="https://reports.example.com/auth/callback" value={form.redirects} onChangeText={(redirects) => set({ redirects })} description="Where Orbit sends people back after they sign in. One per line, exactly as the app sends them." />
      ) : null}
      {create.isError ? <Note error>{errorMessage(create.error)}</Note> : null}
    </Dialog>
  )
}
