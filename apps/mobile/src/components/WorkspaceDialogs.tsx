import { useEffect, useState } from 'react'
import { Image, View } from 'react-native'

import { invitationForOther, invitationToken, type InvitationPreview } from '../data/workspaces'
import { useCreateWorkspace, useJoinWorkspace } from '../hooks/workspaces'
import { errorMessage, useSession, useSignedIn } from '../session-context'
import { useOrbit } from '../orbit-context'
import { useTheme } from '../theme'
import { Button, Icon, type IconName, Input, PickerSheet, Text } from '../ui'
import { Dialog } from './settings/Dialog'
import { Note } from './settings/Shell'
import type { WorkspaceRecord } from '../data/orbit'
import { WorkspaceLogo } from './WorkspaceLogo'

/** The switcher's rows below the workspaces, as on the web; Join takes an invitation link pasted from the email. */
const CREATE = '__create'
const JOIN = '__join'
const WORKSPACE_ACTIONS: readonly { key: typeof CREATE | typeof JOIN; label: string; icon: IconName }[] = [
  { key: CREATE, label: 'Create workspace', icon: 'plus' },
  { key: JOIN, label: 'Join with an invitation link', icon: 'link2' },
]

/**
 * The web's workspace switcher: the person's workspaces, then Create workspace and Join with an invitation link. Opened
 * from the drawer and from Settings. `onSwitched` runs before another workspace opens (the drawer closes itself).
 */
export function WorkspaceSwitcher({ visible, onClose, onSwitched, surface = 'popover' }: { visible: boolean; onClose: () => void; onSwitched?: () => void; surface?: 'popover' | 'sidebarPopover' }) {
  const { colors } = useTheme()
  const { workspace, workspaces, selectWorkspace } = useOrbit()
  const [dialog, setDialog] = useState<typeof CREATE | typeof JOIN | null>(null)
  const open = (id: string) => {
    onSwitched?.()
    selectWorkspace(id)
  }
  return (
    <>
      <PickerSheet
        visible={visible}
        title="Workspaces"
        surface={surface}
        options={[
          ...workspaces.map((item) => ({ key: item.id, label: item.name, subtitle: item.role, selected: item.id === workspace.id, leading: <WorkspaceLogo workspace={item} size={24} /> })),
          ...WORKSPACE_ACTIONS.map((action) => ({ ...action, leading: <Icon name={action.icon} size={18} color={colors.mutedForeground} /> })),
        ]}
        onSelect={(id) => {
          onClose()
          if (id === CREATE || id === JOIN) setDialog(id)
          else open(id)
        }}
        onClose={onClose}
      />
      <CreateWorkspaceDialog visible={dialog === CREATE} onClose={() => setDialog(null)} onCreated={(created) => open(created.id)} />
      <JoinWorkspaceDialog visible={dialog === JOIN} onClose={() => setDialog(null)} onJoined={open} />
    </>
  )
}

/** The web switcher's "Create workspace" form. `onCreated` selects the new workspace. */
export function CreateWorkspaceDialog({ visible, onClose, onCreated }: { visible: boolean; onClose: () => void; onCreated?: (workspace: WorkspaceRecord) => void }) {
  const [name, setName] = useState('')
  const create = useCreateWorkspace((workspace) => {
    onClose()
    onCreated?.(workspace)
  })
  useEffect(() => {
    if (!visible) return
    setName('')
    create.reset()
    // only when the dialog opens
  }, [visible])
  const submit = () => name.trim() && !create.isPending && create.mutate(name.trim())
  return (
    <Dialog
      visible={visible}
      title="Create workspace"
      description="A workspace is where your team works together. It holds your projects, tasks, documents and chat. You can invite people later."
      onClose={onClose}
      footer={
        <>
          <Button label="Cancel" variant="outline" disabled={create.isPending} onPress={onClose} />
          <Button label={create.isPending ? 'Creating…' : 'Create workspace'} loading={create.isPending} disabled={!name.trim()} onPress={submit} />
        </>
      }
    >
      <Input
        label="Workspace name"
        required
        autoFocus
        placeholder="Acme Inc."
        maxLength={100}
        value={name}
        onChangeText={setName}
        returnKeyType="done"
        onSubmitEditing={submit}
        error={create.isError ? errorMessage(create.error) : undefined}
      />
    </Dialog>
  )
}

/**
 * Joins a workspace with an invitation link pasted from the email (the app has no universal links, so the link opens
 * the browser). Shows the workspace first, as the web's accept page does; an invitation for another email cannot be
 * accepted with this account.
 */
export function JoinWorkspaceDialog({ visible, onClose, onJoined }: { visible: boolean; onClose: () => void; onJoined?: (workspaceId: string) => void }) {
  const { user } = useSignedIn()
  const { state } = useSession()
  const server = state.status === 'signed_in' ? state.session.server : ''
  const [link, setLink] = useState('')
  const [problem, setProblem] = useState<string | null>(null)
  const [found, setFound] = useState<{ token: string; preview: InvitationPreview } | null>(null)
  const { preview, accept } = useJoinWorkspace((workspaceId) => {
    onClose()
    onJoined?.(workspaceId)
  })
  useEffect(() => {
    if (!visible) return
    setLink('')
    setProblem(null)
    setFound(null)
    preview.reset()
    accept.reset()
    // only when the dialog opens
  }, [visible])

  const check = () => {
    const parsed = invitationToken(link, server)
    if ('problem' in parsed) {
      setProblem(parsed.problem)
      return
    }
    setProblem(null)
    preview.mutate(parsed.token, { onSuccess: (data) => setFound({ token: parsed.token, preview: data }) })
  }

  const other = found ? invitationForOther(found.preview, user.email) : false
  const busy = preview.isPending || accept.isPending
  const error = problem ?? (preview.isError ? `This invitation could not be loaded. ${errorMessage(preview.error)}` : accept.isError ? errorMessage(accept.error) : null)

  return (
    <Dialog
      visible={visible}
      title={found ? `Join ${found.preview.workspace_name}` : 'Join a workspace'}
      description={found ? undefined : 'Paste the invitation link from your email.'}
      onClose={onClose}
      footer={
        <>
          <Button label="Cancel" variant="outline" disabled={busy} onPress={onClose} />
          {found ? (
            <Button label="Accept invitation" loading={accept.isPending} disabled={other} onPress={() => accept.mutate(found.token)} />
          ) : (
            <Button label="Continue" loading={preview.isPending} disabled={!link.trim()} onPress={check} />
          )}
        </>
      }
    >
      {found ? (
        <View className="gap-3">
          {found.preview.workspace_logo ? <Image accessibilityIgnoresInvertColors source={{ uri: found.preview.workspace_logo }} style={{ width: 48, height: 48, borderRadius: 12 }} /> : null}
          <Text className="text-[13px] leading-[19px]">
            {other
              ? `This invitation is for ${found.preview.email}, and you are signed in as ${user.email}. Sign in as ${found.preview.email} to accept it, or ask the inviter for a new invitation.`
              : `Accept to join this workspace with your account (${user.email}).`}
          </Text>
          {error ? <Note error>{error}</Note> : null}
        </View>
      ) : (
        <Input
          label="Invitation link"
          autoFocus
          autoCapitalize="none"
          autoCorrect={false}
          keyboardType="url"
          placeholder="https://…/accept-invitation?token=…"
          value={link}
          onChangeText={setLink}
          returnKeyType="go"
          onSubmitEditing={() => link.trim() && check()}
          error={error ?? undefined}
        />
      )}
    </Dialog>
  )
}
