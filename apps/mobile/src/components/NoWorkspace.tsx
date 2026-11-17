import { useState } from 'react'
import { View } from 'react-native'

import { saveWorkspaceId } from '../session'
import { useSignedIn } from '../session-context'
import { Button, EmptyState, Screen } from '../ui'
import { DeleteAccountDialog } from './settings/DeleteAccountDialog'
import { CreateWorkspaceDialog, JoinWorkspaceDialog } from './WorkspaceDialogs'

/**
 * What a signed-in person with no workspace sees (the last one is gone, or they left it): the web's onboarding first
 * step. They create a workspace, join one with an invitation link, sign out or delete the account; the root account
 * cannot be deleted. The new workspace opens as soon as the list has it.
 */
export function NoWorkspace() {
  const { user, signOut } = useSignedIn()
  const [dialog, setDialog] = useState<'create' | 'join' | 'delete' | null>(null)
  const close = () => setDialog(null)
  return (
    <Screen centered>
      <EmptyState
        icon="home"
        title="No workspaces"
        description="A workspace holds your team's projects, tasks, documents and chat. Create one, or join one with the invitation link from your email."
        action={
          <View className="items-center gap-2">
            <Button label="Create workspace" icon="plus" onPress={() => setDialog('create')} />
            <Button label="Join with an invitation link" icon="link2" variant="outline" onPress={() => setDialog('join')} />
            <Button label="Sign out" variant="ghost" onPress={() => void signOut()} />
            {user.root ? null : <Button label="Delete account" variant="destructive" onPress={() => setDialog('delete')} />}
          </View>
        }
      />
      <CreateWorkspaceDialog visible={dialog === 'create'} onClose={close} onCreated={(created) => void saveWorkspaceId(created.id)} />
      <JoinWorkspaceDialog visible={dialog === 'join'} onClose={close} onJoined={(id) => void saveWorkspaceId(id)} />
      {dialog === 'delete' ? <DeleteAccountDialog onClose={close} /> : null}
    </Screen>
  )
}
