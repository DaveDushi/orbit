import { router } from 'expo-router'
import { readApps } from '@orbit/api/apps'
import { useQuery } from '@tanstack/react-query'
import { useState } from 'react'
import { Linking, View } from 'react-native'

import { ConfirmDialog, type ConfirmRequest } from '../../../components/settings/Dialog'
import { WebSetupRow } from '../../../components/settings/Fields'
import { ACCOUNT_NAV, allowedNav, Note, RowIcon, RowText, SettingsCard, SettingsRow, SettingsShell, WORKSPACE_NAV } from '../../../components/settings/Shell'
import { WorkspaceLogo } from '../../../components/WorkspaceLogo'
import { webSetupLinks } from '../../../data/agentic'
import { pickImage, useAuthOptions, useMemberWrites, useRenameWorkspace, useSetWorkspaceLogo } from '../../../hooks/account'
import { useAgentEnabled } from '../../../hooks/agent'
import { useIsGuest, useOrbit } from '../../../orbit-context'
import { useMembers } from '../../../queries'
import { errorMessage } from '../../../session-context'
import { useTheme } from '../../../theme'
import { Button, Icon, Input } from '../../../ui'

/**
 * The dock's Settings: the web's workspace General page (name, leave), and the account pages,
 * the server and Sign out, which the web keeps in the account menu. A guest has no workspace
 * settings: it gets its account pages, Sign out and Leave workspace.
 */
/** The server refuses a bigger logo. */
const MAX_LOGO_BYTES = 512 * 1024

export default function GeneralScreen() {
  const { workspace, session, user, signOut } = useOrbit()
  const { colors } = useTheme()
  const [signingOut, setSigningOut] = useState(false)
  const [signOutError, setSignOutError] = useState<string | null>(null)
  const guest = useIsGuest()

  return (
    <SettingsShell title={guest ? 'Account settings' : 'Settings'} nav={guest ? ACCOUNT_NAV : WORKSPACE_NAV}>
      {workspace.permissions.includes('workspace.update') ? <WorkspaceName key={workspace.id} /> : null}

      {guest ? null : <AppsCard />}

      {guest ? null : <WebSetupCard />}

      <SettingsCard title="Account" description={`${user.email} on ${session.server}`} flush>
        {allowedNav(ACCOUNT_NAV, workspace.permissions, true, guest).map((item, index) => (
          <SettingsRow key={item.href} first={index === 0} label={item.label} onPress={() => router.push(item.href)}>
            <RowIcon icon={item.icon} />
            <RowText title={item.label} />
            <Icon name="chevronRight" size={14} color={colors.mutedForeground} />
          </SettingsRow>
        ))}
        <View className="p-4 border-t border-border gap-2 items-start">
          <Button
            label="Sign out"
            variant="outline"
            icon="logout"
            loading={signingOut}
            onPress={() => {
              setSigningOut(true)
              setSignOutError(null)
              signOut().catch((error: unknown) => {
                setSigningOut(false)
                setSignOutError(errorMessage(error))
              })
            }}
          />
          {signOutError ? <Note error>{signOutError}</Note> : null}
        </View>
      </SettingsCard>

      <LeaveWorkspace key={`leave-${workspace.id}`} />
    </SettingsShell>
  )
}

function WorkspaceName() {
  const { workspace } = useOrbit()
  const rename = useRenameWorkspace()
  const [name, setName] = useState(workspace.name)
  const dirty = name.trim() !== workspace.name
  return (
    <SettingsCard title="Workspace" description="The name and logo of this workspace.">
      <LogoField />
      <Input label="Name" required value={name} onChangeText={setName} maxLength={100} returnKeyType="done" onSubmitEditing={() => dirty && name.trim() && rename.mutate(name.trim())} />
      {rename.isError ? <View className="mt-3"><Note error>Workspace rename failed. {errorMessage(rename.error)}</Note></View> : null}
      {dirty ? (
        <View className="mt-4 flex-row gap-2 justify-end">
          <Button label="Discard" variant="outline" onPress={() => setName(workspace.name)} />
          <Button label="Save" loading={rename.isPending} disabled={!name.trim()} onPress={() => rename.mutate(name.trim())} />
        </View>
      ) : null}
    </SettingsCard>
  )
}

/** The web's logo field: owners and admins pick a picture, cropped to a square, or remove it. */
function LogoField() {
  const { workspace } = useOrbit()
  const setLogo = useSetWorkspaceLogo()
  const [pickError, setPickError] = useState<string | null>(null)
  const choose = () => {
    setPickError(null)
    setLogo.reset()
    pickImage({ square: true, maxBytes: MAX_LOGO_BYTES }).then(
      (file) => file && setLogo.mutate(file),
      (error: unknown) => setPickError(errorMessage(error)),
    )
  }
  const error = pickError ?? (setLogo.isError ? errorMessage(setLogo.error) : null)
  return (
    <View className="mb-4 flex-row items-center gap-4 flex-wrap">
      <WorkspaceLogo workspace={workspace} size={64} />
      <View className="flex-1 min-w-45 gap-2">
        <View className="flex-row gap-2">
          <Button label={setLogo.isPending ? 'Saving…' : workspace.logo_url ? 'Change logo' : 'Upload logo'} variant="outline" disabled={setLogo.isPending} onPress={choose} />
          {workspace.logo_url ? <Button label="Remove" variant="ghost" disabled={setLogo.isPending} onPress={() => setLogo.mutate(null)} /> : null}
        </View>
        <Note error={error !== null}>{error ?? 'PNG, JPEG or WebP, cropped to a square. Everyone in this workspace sees it.'}</Note>
      </View>
    </View>
  )
}

/** The workspace's custom web apps this person may open. On a phone they open in the browser, not in a frame. */
function AppsCard() {
  const { client, workspaceId } = useOrbit()
  const { colors } = useTheme()
  const apps = useQuery({ queryKey: ['apps', workspaceId], queryFn: () => readApps(client, workspaceId) })
  if (!apps.data?.length) return null
  return (
    <SettingsCard title="Apps" description="Open in your browser." flush>
      {apps.data.map((app, index) => (
        <SettingsRow key={app.slug} first={index === 0} label={`Open ${app.name}`} onPress={() => void Linking.openURL(app.url)}>
          <RowIcon icon="category" />
          <RowText title={app.name} />
          <Icon name="chevronRight" size={14} color={colors.mutedForeground} />
        </SettingsRow>
      ))}
    </SettingsCard>
  )
}

/** The rare admin pages that stay on the web (agent tools, API tokens), for the roles the web shows them to. */
function WebSetupCard() {
  const { workspace } = useOrbit()
  const links = webSetupLinks(workspace.permissions, useAgentEnabled())
  if (links.length === 0) return null
  return (
    <SettingsCard title="Set up in the web app" description="These open the web app in this app. Sign in there if it asks." flush>
      {links.map((link, index) => <WebSetupRow key={link.path} first={index === 0} {...link} icon={link.path.endsWith('api-tokens') ? 'key' : 'cpu'} />)}
    </SettingsCard>
  )
}

/** Any member may leave; the owner transfers ownership first (`can_remove` on your own row). */
function LeaveWorkspace() {
  const { workspace, user } = useOrbit()
  const members = useMembers()
  const { leave } = useMemberWrites()
  const [confirm, setConfirm] = useState<ConfirmRequest | null>(null)
  const membership = members.data?.find((member) => member.user_id === user.id)
  // Orbit Cloud refuses ownership transfer.
  const billing = useAuthOptions().data?.billing_enabled ?? false

  if (membership && !membership.can_remove) {
    return (
      <SettingsCard title="Leave workspace" description="You own this workspace.">
        <Note>
          {billing
            ? 'The owner cannot leave. Ownership transfer is not available on Orbit Cloud yet.'
            : 'To leave, first transfer ownership to another member in Members.'}
        </Note>
      </SettingsCard>
    )
  }
  return (
    <SettingsCard title="Leave workspace" description="Remove yourself from this workspace.">
      <View className="items-start gap-2">
        <Button
          label={leave.isPending ? 'Leaving…' : 'Leave workspace'}
          variant="destructive"
          disabled={!membership || leave.isPending}
          onPress={() =>
            setConfirm({
              title: `Leave ${workspace.name}?`,
              description: 'You lose access to its projects, tasks, docs and chat. To come back, someone must invite you again.',
              confirmLabel: 'Leave workspace',
              danger: true,
              onConfirm: () => membership && leave.mutate(membership),
            })
          }
        />
        {leave.isError ? <Note error>You could not leave the workspace. {errorMessage(leave.error)}</Note> : null}
      </View>
      <ConfirmDialog request={confirm} onClose={() => setConfirm(null)} />
    </SettingsCard>
  )
}
