import { useState } from 'react'
import { View } from 'react-native'

import { WorkspaceList } from '../../components/WorkspaceSwitcher'
import { useOrbit } from '../../orbit-context'
import { useTheme } from '../../theme'
import { Avatar, Button, Icon, ListRow, Screen, Text } from '../../ui'

export default function AccountScreen() {
  const { session, user, workspaces, signOut } = useOrbit()
  const { colors, spacing, radius } = useTheme()
  const [signingOut, setSigningOut] = useState(false)

  return (
    <Screen edges={['left', 'right']} scroll>
      <View style={{ alignItems: 'center', gap: spacing[2], paddingVertical: spacing[4] }}>
        <Avatar id={user.id} name={user.display_name} size={64} />
        <Text variant="heading">{user.display_name}</Text>
        <Text color="muted">{user.email}</Text>
      </View>
      {workspaces.length > 1 ? <WorkspaceList /> : null}
      <View style={{ borderWidth: 1, borderColor: colors.border, borderRadius: radius.xl, overflow: 'hidden' }}>
        <ListRow title="Server" subtitle={session.server} leading={<Icon name="settings" color={colors.mutedForeground} />} />
      </View>
      <Button
        label="Sign out"
        variant="secondary"
        loading={signingOut}
        leading={<Icon name="logout" size={18} color={colors.secondaryForeground} />}
        onPress={async () => {
          setSigningOut(true)
          await signOut()
        }}
      />
    </Screen>
  )
}
