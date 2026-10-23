import { Pressable, View } from 'react-native'

import type { Person } from '../data/chat'
import type { StatusRecord, TaskRecord } from '../data/orbit'
import { alpha, type, useTheme } from '../theme'
import { Avatar, PriorityIcon, StatusIcon, Text } from '../ui'

export interface TaskRowProps {
  task: TaskRecord
  status: StatusRecord | undefined
  assignee: Person | undefined
  onPress: () => void
}

/**
 * The web list row at phone width: the 28px select strip, priority, the tree gutter, status, title, assignee. The
 * identifier column is hidden below 480px on the web too.
 */
export function TaskRow({ task, status, assignee, onPress }: TaskRowProps) {
  const { colors } = useTheme()
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={task.title}
      onPress={onPress}
      style={({ pressed }) => ({
        minHeight: 40,
        flexDirection: 'row',
        alignItems: 'center',
        gap: 6,
        paddingRight: 12,
        paddingVertical: 6,
        borderBottomWidth: 1,
        borderBottomColor: colors.border,
        backgroundColor: pressed ? alpha(colors.foreground, 0.02) : 'transparent',
      })}
    >
      <View style={{ width: 28 }} />
      <Glyph>
        <PriorityIcon priority={task.priority} size={16} />
      </Glyph>
      <View style={{ width: 16 }} />
      <Glyph>
        <StatusIcon status={status} size={16} />
      </Glyph>
      <Text numberOfLines={1} style={[type(13, 'medium'), { flex: 1 }]}>
        {task.title || 'Untitled'}
      </Text>
      <View style={{ width: 28, alignItems: 'flex-end' }}>{assignee ? <Avatar id={assignee.id} name={assignee.name} size={18} /> : null}</View>
    </Pressable>
  )
}

/** The 22px ghost button the web wraps each row picker in. */
function Glyph({ children }: { children: React.ReactNode }) {
  return <View style={{ width: 22, height: 22, alignItems: 'center', justifyContent: 'center' }}>{children}</View>
}
