import { router } from 'expo-router'
import { type ReactNode, useState } from 'react'
import { View } from 'react-native'

import { ConfirmDialog, type ConfirmRequest } from '../../../components/settings/Dialog'
import { Note, RowIcon, RowText, SettingsCard, SettingsRow, SettingsShell, useNavFlags } from '../../../components/settings/Shell'
import { WorkspaceLogo } from '../../../components/WorkspaceLogo'
import { settingsSections, type SettingsNavItem } from '../../../data/settings-nav'
import { useOpenWorkspaceApp, useWorkspaceApps } from '../../../hooks/apps'
import { pickImage, useAuthOptions, useMemberWrites, useRenameWorkspace, useSetWorkspaceLogo } from '../../../hooks/account'
import { useIsGuest, useOrbit } from '../../../orbit-context'
import { useMembers } from '../../../queries'
import { errorMessage } from '../../../session-context'
import { useTheme } from '../../../theme'
import { Button, Icon, Input, Text } from '../../../ui'

/**
 * The dock's Settings: one page with a row for every settings page, in three sections (`settingsSections`). Workspace
 * has the name and logo, the workspace's apps and its pages; Account has the account pages and Sign out; Admin, for the
 * instance admins, the installation's pages. A guest gets Account, Sign out and Leave workspace.
 */
/** The server refuses a bigger logo. */
const MAX_LOGO_BYTES = 512 * 1024

export default function SettingsScreen() {
  const { workspace, session, user, signOut } = useOrbit()
  const flags = useNavFlags()
  const [signingOut, setSigningOut] = useState(false)
  const [signOutError, setSignOutError] = useState<string | null>(null)
  const guest = useIsGuest()
  const sections = settingsSections(workspace.permissions, flags, guest, user.installation_admin)
  const subtitles: Record<(typeof sections)[number]['title'], string> = {
    Workspace: workspace.name,
    Account: `${user.email} on ${session.server}`,
    Admin: 'This Orbit server, for every workspace.',
  }

  return (
    <SettingsShell title="Settings" nav={[]}>
      {sections.map((section) => (
        <Section key={section.title} title={section.title} subtitle={subtitles[section.title]}>
          {section.title === 'Workspace' && workspace.permissions.includes('workspace.update') ? <WorkspaceName key={workspace.id} /> : null}
          {section.title === 'Workspace' ? <AppsCard /> : null}
          <NavCard items={section.items}>
            {section.title === 'Account' ? (
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
            ) : null}
          </NavCard>
        </Section>
      ))}

      <LeaveWorkspace key={`leave-${workspace.id}`} />
    </SettingsShell>
  )
}

/** An iOS grouped-list section: its title and quiet line, then its cards. */
function Section({ title, subtitle, children }: { title: string; subtitle: string; children: ReactNode }) {
  return (
    <View className="gap-3">
      <View className="px-1 gap-0.5">
        <Text accessibilityRole="header" className="font-noto-medium text-[18px] leading-[24px] text-foreground">{title}</Text>
        <Text numberOfLines={1} className="text-[12px] text-muted-foreground">
          {subtitle}
        </Text>
      </View>
      {children}
    </View>
  )
}

/** A section's pages, one row each, with what follows them (Sign out) in the same card. */
function NavCard({ items, children }: { items: readonly SettingsNavItem[]; children?: ReactNode }) {
  const { colors } = useTheme()
  return (
    <View className="rounded-lg overflow-hidden bg-card shadow-[0_0_0_1px_var(--color-border)]">
      {items.map((item, index) => (
        <SettingsRow key={item.href} first={index === 0} label={item.label} onPress={() => router.push(item.href)}>
          <RowIcon icon={item.icon} />
          <RowText title={item.label} />
          <Icon name="chevronRight" size={14} color={colors.mutedForeground} />
        </SettingsRow>
      ))}
      {children}
    </View>
  )
}

function WorkspaceName() {
  const { workspace } = useOrbit()
  const rename = useRenameWorkspace()
  const [name, setName] = useState(workspace.name)
  const dirty = name.trim() !== workspace.name
  return (
    <SettingsCard title="Name and logo" description="Everyone in this workspace sees them.">
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
  const { colors } = useTheme()
  const apps = useWorkspaceApps()
  const openApp = useOpenWorkspaceApp()
  if (!apps.data?.length) return null
  return (
    <SettingsCard title="Open an app" description="The workspace's apps open in your browser." flush>
      {apps.data.map((app, index) => (
        <SettingsRow key={app.slug} first={index === 0} label={`Open ${app.name}`} onPress={() => void openApp(app)}>
          <RowIcon icon="category" />
          <RowText title={app.name} />
          <Icon name="chevronRight" size={14} color={colors.mutedForeground} />
        </SettingsRow>
      ))}
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
