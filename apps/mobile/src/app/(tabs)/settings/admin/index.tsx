import { useDeferredValue, useState } from 'react'
import { Share, View } from 'react-native'
import { TextInput } from '../../../../ui/scaled'

import { ActionSheet, ConfirmDialog, type ConfirmRequest, Dialog, type SheetAction } from '../../../../components/settings/Dialog'
import { AdminShell, Badge, type BadgeTone, Note, RowText, SettingsCard, SettingsRow } from '../../../../components/settings/Shell'
import { type AdminAction, adminActions, adminBadges, type AdminUser, type RecoveryLink } from '../../../../data/admin-api'
import { shortDate } from '../../../../data/format'
import { useAdminUserWrites, useAdminUsers } from '../../../../hooks/admin'
import { useOrbit } from '../../../../orbit-context'
import { errorMessage } from '../../../../session-context'
import { alpha, useTheme } from '../../../../theme'
import { Avatar, Button, Icon, Input } from '../../../../ui'

const BADGE_TONE: Record<string, BadgeTone> = { Root: 'primary', Admin: 'secondary', '2FA': 'outline', Suspended: 'destructive' }

/** The web's Admin → Users: every account on the instance, searched on the server, with the Manage menu. */
export default function AdminUsersScreen() {
  const { user: viewer } = useOrbit()
  const { colors } = useTheme()
  const [query, setQuery] = useState('')
  const search = useDeferredValue(query.trim())
  const users = useAdminUsers(search)
  const writes = useAdminUserWrites()
  const [managing, setManaging] = useState<AdminUser | null>(null)
  const [confirm, setConfirm] = useState<ConfirmRequest | null>(null)
  const [link, setLink] = useState<{ user: AdminUser; link: RecoveryLink } | null>(null)
  const list = users.data?.pages.flatMap((page) => page.items) ?? []
  const failed = [writes.suspend, writes.setAdmin, writes.resetTwoFactor, writes.recoveryLink].find((write) => write.isError)?.error

  const run = (user: AdminUser, action: AdminAction) => {
    switch (action) {
      case 'make-admin':
        return setConfirm({
          title: `Make ${user.display_name} an instance admin?`,
          description: 'They can manage accounts (except you and other admins), email, registration, backups and the audit log.',
          confirmLabel: 'Make admin',
          onConfirm: () => writes.setAdmin.mutate({ userId: user.id, isAdmin: true }),
        })
      case 'remove-admin':
        return writes.setAdmin.mutate({ userId: user.id, isAdmin: false })
      case 'recovery-link':
        return writes.recoveryLink.mutate(user.id, { onSuccess: (created) => setLink({ user, link: created }) })
      case 'reset-two-factor':
        return setConfirm({
          title: `Reset two-factor sign-in for ${user.display_name}?`,
          description: 'Their authenticator app and recovery codes stop working, and they sign in with only their password until they set it up again. Do this only after you have confirmed who is asking.',
          confirmLabel: 'Reset two-factor',
          danger: true,
          onConfirm: () => writes.resetTwoFactor.mutate(user.id),
        })
      case 'suspend':
        return setConfirm({
          title: `Suspend ${user.display_name}?`,
          description: 'They are signed out everywhere and cannot sign in until you reinstate the account. Their API tokens stop working.',
          confirmLabel: 'Suspend account',
          danger: true,
          onConfirm: () => writes.suspend.mutate({ userId: user.id, suspended: true }),
        })
      case 'reinstate':
        return writes.suspend.mutate({ userId: user.id, suspended: false })
    }
  }
  const LABELS: Record<AdminAction, string> = {
    'make-admin': 'Make admin',
    'remove-admin': 'Remove admin',
    'recovery-link': 'Create password recovery link',
    'reset-two-factor': 'Reset two-factor',
    suspend: 'Suspend account',
    reinstate: 'Reinstate account',
  }
  const sheet: SheetAction[] = managing
    ? adminActions(managing, viewer).map((action) => ({ label: LABELS[action], destructive: action === 'suspend' || action === 'reset-two-factor', onPress: () => run(managing, action) }))
    : []

  return (
    <AdminShell refreshing={users.isRefetching} onRefresh={() => void users.refetch()}>
      <SettingsCard title="Users" description="Every account on this Orbit instance." flush>
        <View className="p-3 border-b border-border">
          <View className="h-8 flex-row items-center gap-2 px-2.5 rounded-lg border border-input">
            <Icon name="search" size={14} color={alpha(colors.mutedForeground, 0.7)} />
            <TextInput
              accessibilityLabel="Search users"
              placeholder="Search by name or email…"
              placeholderTextColor={colors.mutedForeground}
              value={query}
              onChangeText={setQuery}
              autoCapitalize="none"
              autoCorrect={false}
              className="font-noto text-[14px] leading-[1.45] flex-1 py-0 text-foreground"
            />
          </View>
        </View>
        {users.isPending ? <View className="p-4"><Note>Loading users…</Note></View> : null}
        {users.isError ? <View className="p-4"><Note error>Users could not be loaded.</Note></View> : null}
        {search && users.data && list.length === 0 ? <View className="p-4"><Note>No user matches "{search}".</Note></View> : null}
        {list.map((user, index) => (
          <SettingsRow key={user.id} first={index === 0}>
            <Avatar id={user.id} name={user.display_name || user.email} size={32} />
            <RowText
              title={user.display_name}
              subtitle={`${user.email}\n${user.workspace_count} ${user.workspace_count === 1 ? 'workspace' : 'workspaces'} · ${user.last_active_at ? `Active ${shortDate(user.last_active_at)}` : 'Never signed in'}`}
              badges={adminBadges(user).map((badge) => <Badge key={badge} label={badge} tone={BADGE_TONE[badge]} />)}
            />
            {adminActions(user, viewer).length > 0 ? <Button label="Manage" size="sm" variant="outline" onPress={() => setManaging(user)} /> : null}
          </SettingsRow>
        ))}
        {users.hasNextPage ? (
          <View className="p-2 items-center border-t border-border">
            <Button label={users.isFetchingNextPage ? 'Loading…' : 'Load more users'} variant="ghost" disabled={users.isFetchingNextPage} onPress={() => void users.fetchNextPage()} />
          </View>
        ) : null}
        {failed ? <View className="p-4"><Note error>{errorMessage(failed)}</Note></View> : null}
      </SettingsCard>

      <ActionSheet visible={managing !== null} title={managing?.display_name} actions={sheet} onClose={() => setManaging(null)} />
      <ConfirmDialog request={confirm} onClose={() => setConfirm(null)} />
      <Dialog
        visible={link !== null}
        title={`Recovery link for ${link?.user.display_name ?? ''}`}
        description={link ? `Send this link to ${link.user.email}. It sets a new password, works once, and expires at ${new Date(link.link.expires_at).toLocaleTimeString()}.` : undefined}
        onClose={() => setLink(null)}
        footer={
          <>
            <Button label="Share link" icon="send" variant="outline" onPress={() => link && void Share.share({ message: link.link.url })} />
            <Button label="Done" onPress={() => setLink(null)} />
          </>
        }
      >
        <Input label="Recovery link" value={link?.link.url ?? ''} editable={false} selectTextOnFocus />
      </Dialog>
    </AdminShell>
  )
}
