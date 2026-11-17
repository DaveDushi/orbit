import { router } from 'expo-router'
import { useState } from 'react'
import { View } from 'react-native'

import { Dialog } from '../../../components/settings/Dialog'
import { Note, SettingsCard, SettingsShell, WORKSPACE_NAV } from '../../../components/settings/Shell'
import { useDeleteWorkspace } from '../../../hooks/workspaces'
import { useOrbit } from '../../../orbit-context'
import { Button, Input, Text } from '../../../ui'

/**
 * The web's Settings → Danger zone: the owner deletes the workspace after typing its name. The app then opens the
 * next workspace, or the no-workspace screen after the last one.
 */
export default function DangerZoneScreen() {
  const { workspace, workspaces } = useOrbit()
  const deletion = useDeleteWorkspace()
  const [open, setOpen] = useState(false)
  const [confirmation, setConfirmation] = useState('')
  const canDelete = workspace.permissions.includes('workspace.delete')
  const confirmed = confirmation === workspace.name
  const submit = () => confirmed && !deletion.isPending && deletion.mutate(undefined, {
      onSuccess: () => {
        setOpen(false)
        router.replace('/settings')
      },
    })
  return (
    <SettingsShell title="Settings" nav={WORKSPACE_NAV}>
      {!canDelete ? (
        <SettingsCard title="Danger zone">
          <Note>Only the workspace owner can delete this workspace.</Note>
        </SettingsCard>
      ) : (
        <View className="rounded-lg border border-destructive p-4 gap-3">
          <View className="gap-0.5">
            <Text className="font-noto-medium text-[14px] text-destructive">Danger zone</Text>
            <Text className="text-[12px] text-muted-foreground">Delete this workspace and remove access for all members.</Text>
          </View>
          <View className="flex-row">
            <Button
              label="Delete workspace"
              variant="destructive"
              onPress={() => {
                setConfirmation('')
                deletion.reset()
                setOpen(true)
              }}
            />
          </View>
        </View>
      )}
      <Dialog
        visible={open}
        title="Delete workspace?"
        description={`Deleting "${workspace.name}" removes access to its projects and tasks for everyone.`}
        onClose={() => !deletion.isPending && setOpen(false)}
        footer={
          <>
            <Button label="Cancel" variant="ghost" disabled={deletion.isPending} onPress={() => setOpen(false)} />
            <Button label={deletion.isPending ? 'Deleting…' : 'Delete workspace'} variant="destructive" loading={deletion.isPending} disabled={!confirmed} onPress={submit} />
          </>
        }
      >
        {workspaces.length === 1 ? <Note error>This is your last workspace. After deletion, you will have no workspace access.</Note> : null}
        <Input
          label="Confirm workspace name"
          description={`Type ${workspace.name} to confirm deletion.`}
          autoCapitalize="none"
          autoCorrect={false}
          autoComplete="off"
          editable={!deletion.isPending}
          value={confirmation}
          onChangeText={setConfirmation}
          returnKeyType="done"
          onSubmitEditing={submit}
        />
        {deletion.isError ? <Note error>Workspace deletion failed. Try again, or refresh if the workspace has changed.</Note> : null}
      </Dialog>
    </SettingsShell>
  )
}
