import { router, useLocalSearchParams } from 'expo-router'
import type { ManagedApp } from '@orbit/api/apps'
import { useState } from 'react'
import { Linking, View } from 'react-native'

import { AppRolesField, Copyable, SecretOnce, SignInSwitch, useAppWrites, useManagedApps } from '../../../../components/settings/Apps'
import { SelectField } from '../../../../components/settings/Fields'
import { ConfirmDialog, type ConfirmRequest } from '../../../../components/settings/Dialog'
import { Note, SettingsCard, SettingsShell, WORKSPACE_NAV } from '../../../../components/settings/Shell'
import { APP_ICON_OPTIONS, APP_OPEN_MODES, redirectLines, sameRoles } from '../../../../data/apps'
import { useOrbit } from '../../../../orbit-context'
import { errorMessage } from '../../../../session-context'
import { Button, Input, Text } from '../../../../ui'

/** The web's settings of one app (`/settings/apps/:id`): name, address, roles, appearance, sign-in and remove. */
export default function AppSettingsScreen() {
  const { id } = useLocalSearchParams<{ id: string }>()
  const { workspace } = useOrbit()
  const canManage = workspace.permissions.includes('apps.manage')
  const apps = useManagedApps(canManage)
  const app = apps.data?.items.find((item) => item.id === id)
  return (
    <SettingsShell title="Settings" nav={WORKSPACE_NAV} refreshing={apps.isRefetching} onRefresh={canManage ? () => void apps.refetch() : undefined}>
      {!canManage ? <Note>Only workspace owners and administrators can set up apps.</Note> : null}
      {apps.isPending && canManage ? <Note loading>Loading apps…</Note> : null}
      {apps.isError ? <Note error>Apps could not be loaded.</Note> : null}
      {apps.data && !app ? <Note>This app does not exist.</Note> : null}
      {app && apps.data ? <AppSettings key={app.id} app={app} canSignIn={apps.data.can_sign_in} /> : null}
    </SettingsShell>
  )
}

function AppSettings({ app, canSignIn }: { app: ManagedApp; canSignIn: boolean }) {
  const { update, remove } = useAppWrites()
  const [name, setName] = useState(app.name)
  const [url, setUrl] = useState(app.url)
  const [roles, setRoles] = useState(app.allowed_roles)
  const [confirm, setConfirm] = useState<ConfirmRequest | null>(null)
  const changed = name.trim() !== app.name || url.trim() !== app.url || !sameRoles(roles, app.allowed_roles)
  const valid = name.trim() !== '' && url.trim() !== '' && roles.length > 0
  return (
    <>
      <SecretOnce />
      <View className="flex-row justify-end">
        <Button label="Open" variant="outline" size="sm" onPress={() => void Linking.openURL(app.url)} />
      </View>
      <SettingsCard title={app.name}>
        <View className="gap-4">
          <Input label="Name" maxLength={80} value={name} onChangeText={setName} />
          <Input label="Address" autoCapitalize="none" autoCorrect={false} keyboardType="url" maxLength={2048} value={url} onChangeText={setUrl} description="To open inside Orbit on the web, the app must allow it with a frame-ancestors header. A subdomain of Orbit's domain works best." />
          <AppRolesField value={roles} onChange={setRoles} />
          <View className="flex-row">
            <Button label="Save" variant="outline" loading={update.isPending} disabled={!changed || !valid} onPress={() => update.mutate({ id: app.id, input: { name: name.trim(), url: url.trim(), allowed_roles: roles } })} />
          </View>
        </View>
      </SettingsCard>
      <SettingsCard title="Appearance">
        <View className="gap-4">
          <SelectField label="Opens" value={app.open_mode} options={APP_OPEN_MODES} onChange={(value) => update.mutate({ id: app.id, input: { open_mode: value as ManagedApp['open_mode'] } })} description="On the web. On a phone, apps open in the browser." />
          <SelectField label="Icon" value={app.icon} options={APP_ICON_OPTIONS} onChange={(value) => update.mutate({ id: app.id, input: { icon: value } })} />
        </View>
      </SettingsCard>
      {update.isError ? <Note error>The change could not be saved. {errorMessage(update.error)}</Note> : null}
      <SignInSection app={app} canSignIn={canSignIn} />
      <SettingsCard title="Remove app">
        <View className="gap-3">
          <Note>It leaves the sidebar for everyone, and its sign-in ends. The app itself keeps running.</Note>
          <View className="flex-row">
            <Button
              label="Remove"
              variant="destructive"
              disabled={remove.isPending}
              onPress={() =>
                setConfirm({
                  title: `Remove ${app.name}?`,
                  description: 'It leaves the sidebar for everyone. Its sign-in client ends, with all its tokens. The app itself keeps running, and people stay signed in to it until its own session ends.',
                  confirmLabel: 'Remove',
                  danger: true,
                  onConfirm: () => remove.mutate(app.id, { onSuccess: () => router.back() }),
                })
              }
            />
          </View>
          {remove.isError ? <Note error>The app could not be removed. {errorMessage(remove.error)}</Note> : null}
        </View>
      </SettingsCard>
      <ConfirmDialog request={confirm} onClose={() => setConfirm(null)} />
    </>
  )
}

function SignInSection({ app, canSignIn }: { app: ManagedApp; canSignIn: boolean }) {
  const { update, rotate } = useAppWrites()
  const saved = app.sign_in?.redirect_uris.join('\n') ?? ''
  const [redirects, setRedirects] = useState(saved)
  // turned on here, not saved yet: the client exists only after the first redirect URI is saved
  const [turningOn, setTurningOn] = useState(false)
  const [confirm, setConfirm] = useState<ConfirmRequest | null>(null)
  const on = app.sign_in !== null || turningOn
  const saveRedirects = (uris: string[]) => update.mutate({ id: app.id, input: { redirect_uris: uris } }, { onSuccess: () => setTurningOn(false) })
  return (
    <SettingsCard title="Sign-in" flush>
      <View className="gap-4 p-4">
        <SignInSwitch
          checked={on}
          disabled={!canSignIn || update.isPending}
          onChange={(next) => {
            if (next) return setTurningOn(true)
            if (!app.sign_in) return setTurningOn(false)
            setConfirm({
              title: `Turn off sign-in for ${app.name}?`,
              description: 'Its client ID and secret end, with all its tokens. People cannot sign in to it with Orbit until you turn this on again, which makes a new client.',
              confirmLabel: 'Turn off',
              danger: true,
              onConfirm: () => {
                setRedirects('')
                saveRedirects([])
              },
            })
          }}
        />
        {on ? (
          <View className="gap-3">
            <Input label="Redirect URIs" multiline autoCapitalize="none" autoCorrect={false} placeholder="https://reports.example.com/auth/callback" value={redirects} onChangeText={setRedirects} description="Where Orbit sends people back after they sign in. One per line, exactly as the app sends them." />
            <View className="flex-row">
              <Button label="Save" variant="outline" disabled={update.isPending || redirectLines(redirects).length === 0 || redirects.trim() === saved} onPress={() => saveRedirects(redirectLines(redirects))} />
            </View>
          </View>
        ) : null}
      </View>
      {app.sign_in ? (
        <>
          <Copyable label="Client ID" value={app.sign_in.client_id} />
          <Copyable label="Issuer" value={app.sign_in.issuer} />
          <Copyable label="Discovery URL" value={app.sign_in.discovery_url} />
          <View className="gap-2 border-t border-border p-4">
            <Text className="text-[13px] text-muted-foreground">Client secret is hidden. A new one ends the old one and all tokens of this app.</Text>
            <View className="flex-row">
              <Button
                label="Make a new secret"
                variant="outline"
                disabled={rotate.isPending}
                onPress={() =>
                  setConfirm({ title: `Make a new secret for ${app.name}?`, description: 'The old secret stops working at once. Sign-in fails until you put the new one in the app.', confirmLabel: 'Make a new secret', danger: true, onConfirm: () => rotate.mutate(app.id) })
                }
              />
            </View>
            {rotate.isError ? <Note error>The secret could not be changed. {errorMessage(rotate.error)}</Note> : null}
          </View>
        </>
      ) : null}
      <ConfirmDialog request={confirm} onClose={() => setConfirm(null)} />
    </SettingsCard>
  )
}
