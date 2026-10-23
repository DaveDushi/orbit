import { Pressable, View } from 'react-native'

import type { Person } from '../data/chat'
import type { StatusRecord, TaskRecord } from '../data/orbit'
import { dueLabel } from '../data/tasks'
import { useTheme } from '../theme'
import { Avatar, PriorityIcon, StatusIcon, Text } from '../ui'

export interface TaskRowProps {
  task: TaskRecord
  status: StatusRecord | undefined
  identifier: string
  assignee: Person | undefined
  now: Date
  onPress: () => void
}

/** Status, identifier, title, then the quiet details: priority, due date, assignee. */
export function TaskRow({ task, status, identifier, assignee, now, onPress }: TaskRowProps) {
  const { colors, spacing } = useTheme()
  const due = task.due_at ? dueLabel(task.due_at, now) : null
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={`${identifier} ${task.title}`}
      onPress={onPress}
      style={({ pressed }) => ({
        flexDirection: 'row',
        alignItems: 'center',
        gap: spacing[2.5],
        minHeight: 48,
        paddingHorizontal: spacing[4],
        paddingVertical: spacing[2],
        backgroundColor: pressed ? colors.accent : 'transparent',
      })}
    >
      <StatusIcon status={status} />
      <View style={{ flex: 1, gap: 2 }}>
        <Text variant="label" numberOfLines={2}>
          {task.title}
        </Text>
        <View style={{ flexDirection: 'row', alignItems: 'center', gap: spacing[2] }}>
          <Text variant="caption" color="muted">
            {identifier}
          </Text>
          {due ? (
            <Text variant="caption" color={due === 'Overdue' ? 'destructive' : 'muted'}>
              {due}
            </Text>
          ) : null}
        </View>
      </View>
      <PriorityIcon priority={task.priority} />
      {assignee ? <Avatar id={assignee.id} name={assignee.name} size={22} /> : null}
    </Pressable>
  )
}
