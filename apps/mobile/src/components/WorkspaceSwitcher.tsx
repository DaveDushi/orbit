import { useState } from 'react'
import { Pressable, View } from 'react-native'

import { useOrbit } from '../orbit-context'
import { useTheme } from '../theme'
import { Icon, PickerSheet, Text } from '../ui'

/** The current workspace name. With more than one workspace it opens a sheet to switch. */
export function WorkspaceSwitcher() {
  const { workspace, workspaces, selectWorkspace } = useOrbit()
  const { colors, spacing } = useTheme()
  const [open, setOpen] = useState(false)
  const switchable = workspaces.length > 1

  return (
    <>
      <Pressable
        accessibilityRole={switchable ? 'button' : undefined}
        accessibilityLabel={switchable ? `Workspace ${workspace.name}. Switch workspace` : undefined}
        disabled={!switchable}
        onPress={() => setOpen(true)}
        style={{ flexDirection: 'row', alignItems: 'center', gap: spacing[1], alignSelf: 'flex-start' }}
      >
        <Text color="muted" numberOfLines={1}>
          {workspace.name}
        </Text>
        {switchable ? <Icon name="chevronDown" size={16} color={colors.mutedForeground} /> : null}
      </Pressable>
      <PickerSheet
        visible={open}
        title="Workspaces"
        options={workspaces.map((item) => ({ key: item.id, label: item.name, subtitle: item.role, selected: item.id === workspace.id }))}
        onSelect={(id) => {
          setOpen(false)
          selectWorkspace(id)
        }}
        onClose={() => setOpen(false)}
      />
    </>
  )
}

/** Every workspace as a row, the current one checked: the Account tab's switcher. */
export function WorkspaceList() {
  const { workspace, workspaces, selectWorkspace } = useOrbit()
  const { colors, radius, spacing } = useTheme()
  return (
    <View style={{ gap: spacing[1.5] }}>
      <Text variant="caption" color="muted">
        Workspaces
      </Text>
      <View style={{ borderWidth: 1, borderColor: colors.border, borderRadius: radius.xl, overflow: 'hidden' }}>
        {workspaces.map((item, index) => (
          <Pressable
            key={item.id}
            accessibilityRole="button"
            accessibilityState={{ selected: item.id === workspace.id }}
            onPress={() => selectWorkspace(item.id)}
            style={({ pressed }) => ({
              flexDirection: 'row',
              alignItems: 'center',
              minHeight: 48,
              paddingHorizontal: spacing[4],
              backgroundColor: pressed ? colors.accent : 'transparent',
              borderTopWidth: index === 0 ? 0 : 1,
              borderTopColor: colors.border,
            })}
          >
            <Text variant="label" style={{ flex: 1 }} numberOfLines={1}>
              {item.name}
            </Text>
            {item.id === workspace.id ? <Icon name="check" size={18} color={colors.primary} /> : null}
          </Pressable>
        ))}
      </View>
    </View>
  )
}
