/** The parts the Settings → Apps screens share: the queries and writes, the roles field, the sign-in switch and the secret. */
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { type AppRole, createManagedApp, deleteManagedApp, type ManagedApp, type ManagedAppInput, readManagedApps, rotateManagedAppSecret, updateManagedApp } from '@orbit/api/apps'
import { useSyncExternalStore } from 'react'
import { Share, View } from 'react-native'

import { APP_ROLES } from '../../data/apps'
import { workspaceAppsKey } from '../../hooks/apps'
import { useOrbit } from '../../orbit-context'
import { Button, Text } from '../../ui'
import { CheckRow } from './Fields'
import { FieldLabel, OptionHint, OptionText, SettingsCard, Toggle } from './Shell'

export const managedAppsKey = (workspaceId: string) => ['managed-apps', workspaceId] as const

/** The secret of the app that was just made or changed. Only in memory: Orbit cannot show it again. */
export type AppSecret = { name: string; value: string } | null

export function useManagedApps(enabled: boolean) {
  const { client, workspaceId } = useOrbit()
  return useQuery({ queryKey: managedAppsKey(workspaceId), queryFn: () => readManagedApps(client, workspaceId), enabled })
}

let secret: AppSecret = null
const listeners = new Set<() => void>()
const subscribe = (listener: () => void) => {
  listeners.add(listener)
  return () => void listeners.delete(listener)
}
const setAppSecret = (next: AppSecret) => {
  secret = next
  for (const listener of listeners) listener()
}

/**
 * The one-time secret, shared by the list and an app's screen. Only in this module's memory: never in a URL, storage
 * or the query cache (which a persister may write to disk).
 */
export function useAppSecret(): [AppSecret, (secret: AppSecret) => void] {
  return [useSyncExternalStore(subscribe, () => secret), setAppSecret]
}

export function useAppWrites() {
  const { client, workspaceId } = useOrbit()
  const queryClient = useQueryClient()
  const onSuccess = (app: ManagedApp) => {
    if (app.client_secret) setAppSecret({ name: app.name, value: app.client_secret })
  }
  const onSettled = () => {
    void queryClient.invalidateQueries({ queryKey: managedAppsKey(workspaceId) })
    void queryClient.invalidateQueries({ queryKey: workspaceAppsKey(workspaceId) })
  }
  return {
    create: useMutation({ mutationFn: (input: ManagedAppInput) => createManagedApp(client, workspaceId, input), onSuccess, onSettled }),
    update: useMutation({ mutationFn: ({ id, input }: { id: string; input: ManagedAppInput }) => updateManagedApp(client, workspaceId, id, input), onSuccess, onSettled }),
    rotate: useMutation({ mutationFn: (id: string) => rotateManagedAppSecret(client, workspaceId, id), onSuccess, onSettled }),
    remove: useMutation({ mutationFn: (id: string) => deleteManagedApp(client, workspaceId, id), onSettled }),
  }
}

export function AppRolesField({ value, onChange }: { value: readonly AppRole[]; onChange: (roles: AppRole[]) => void }) {
  return (
    <View className="gap-1">
      <FieldLabel>Who can see it</FieldLabel>
      <View className="flex-row flex-wrap gap-x-5">
        {APP_ROLES.map((role) => (
          <CheckRow key={role} label={`${role.charAt(0).toUpperCase()}${role.slice(1)}s`} checked={value.includes(role)} onChange={(on) => onChange(on ? [...value.filter((item) => item !== role), role] : value.filter((item) => item !== role))} />
        ))}
      </View>
    </View>
  )
}

/** On: the app signs people in with their Orbit account. */
export function SignInSwitch({ checked, disabled, onChange }: { checked: boolean; disabled: boolean; onChange: (on: boolean) => void }) {
  return (
    <View className="flex-row items-center gap-3">
      <OptionText title="Sign in with Orbit">
        <OptionHint>{disabled ? 'This server has no ORBIT_APP_KEY, so it cannot sign people in to apps.' : 'People use their Orbit account in this app. Leave off if the app has its own login.'}</OptionHint>
      </OptionText>
      <Toggle label="Sign in with Orbit" value={checked} disabled={disabled} onChange={onChange} />
    </View>
  )
}

/** A value to copy: selectable, with Share (the app has no clipboard module). */
export function Copyable({ label, value }: { label: string; value: string }) {
  return (
    <View className="flex-row items-center gap-3 border-t border-border px-4 py-3">
      <View className="flex-1 min-w-0 gap-0.5">
        <Text className="text-[12px] text-muted-foreground/85">{label}</Text>
        <Text selectable className="font-mono text-[13px]">{value}</Text>
      </View>
      <Button label="Share" icon="send" variant="ghost" size="sm" onPress={() => void Share.share({ message: value })} />
    </View>
  )
}

export function SecretOnce() {
  const [secret, setSecret] = useAppSecret()
  if (!secret) return null
  return (
    <SettingsCard title={`Client secret of ${secret.name}`} description="Copy it now and put it in the app's settings. Orbit shows it only this once." actions={<Button label="Done" variant="outline" size="sm" onPress={() => setSecret(null)} />} flush>
      <View className="-mt-px">
        <Copyable label="Client secret" value={secret.value} />
      </View>
    </SettingsCard>
  )
}
