/** The main-column sections of the task page under the description: Duplicate banner, Sub-issues, Relations. */
import { router } from 'expo-router'
import { type ReactNode, useState } from 'react'
import { Pressable, TextInput, View } from 'react-native'

import type { StatusRecord, TaskRecord } from '../../data/orbit'
import type { TaskRelationRecord } from '../../data/task-api'
import { defaultStatusOf, groupRelations, identifier, isClosed, refIdentifier } from '../../data/tasks'
import { useProjects } from '../../queries'
import { alpha, type, useTheme } from '../../theme'
import { Button, Icon, IconButton, StatusIcon, Text } from '../../ui'

const openTask = (taskId: string) => router.push(`/tasks/${taskId}`)

function SectionTitle({ title, count, children }: { title: string; count: ReactNode; children?: ReactNode }) {
  const { colors } = useTheme()
  return (
    <View style={{ minHeight: 28, flexDirection: 'row', alignItems: 'center', gap: 6, marginBottom: 4 }}>
      <Text style={[type(12, 'semibold'), { color: colors.mutedForeground }]}>{title}</Text>
      {typeof count === 'number' ? <Text style={[type(12), { color: alpha(colors.mutedForeground, 0.6) }]}>{count}</Text> : count}
      <View style={{ flex: 1 }} />
      {children}
    </View>
  )
}

/** One task line: status, identifier and title; a tap opens the task. */
function TaskLine({ status, taskId, id, title, label, trailing }: { status: StatusRecord | undefined; taskId: string; id: string; title: string; label?: string; trailing?: ReactNode }) {
  const { colors } = useTheme()
  return (
    <View style={{ minHeight: 36, flexDirection: 'row', alignItems: 'center', gap: 8 }}>
      {label !== undefined ? (
        <Text numberOfLines={1} style={[type(12), { width: 72, color: alpha(colors.mutedForeground, 0.7) }]}>
          {label}
        </Text>
      ) : null}
      <Pressable accessibilityRole="link" accessibilityLabel={`${id} ${title}`} onPress={() => openTask(taskId)} style={{ flex: 1, flexDirection: 'row', alignItems: 'center', gap: 8, minWidth: 0 }}>
        <StatusIcon status={status} size={14} />
        <Text style={[type(12), { color: colors.mutedForeground }]}>{id}</Text>
        <Text numberOfLines={1} style={[type(13), { flex: 1 }]}>
          {title || 'Untitled'}
        </Text>
      </Pressable>
      {trailing}
    </View>
  )
}

/** The web's strip above the title while the task is a duplicate: the canonical task (opens it) and Unmark. */
export function DuplicateBanner({ task, status, pending, onUnmark }: { task: TaskRecord; status: StatusRecord | undefined; pending: boolean; onUnmark: () => void }) {
  const { colors } = useTheme()
  const target = task.duplicate_of
  if (!target) return null
  return (
    <View accessibilityLabel={`Duplicate of ${refIdentifier(target)}`} style={{ minHeight: 40, flexDirection: 'row', alignItems: 'center', gap: 8, paddingBottom: 8, marginBottom: 6, borderBottomWidth: 1, borderBottomColor: colors.border }}>
      <StatusIcon status={status} size={14} />
      <Text style={[type(13), { color: colors.mutedForeground }]}>Duplicate of</Text>
      <Pressable accessibilityRole="link" onPress={() => openTask(target.id)} style={{ flex: 1, flexDirection: 'row', gap: 6, minWidth: 0 }}>
        <Text style={[type(13), { flexShrink: 0, color: colors.mutedForeground }]}>{refIdentifier(target)}</Text>
        <Text numberOfLines={1} style={[type(13, 'medium'), { flexShrink: 1 }]}>
          {target.title || 'Untitled'}
        </Text>
      </Pressable>
      <Button label="Unmark" variant="ghost" size="xs" disabled={pending} onPress={onUnmark} />
    </View>
  )
}

/**
 * Sub-issues: the closed count, the direct sub-issues, and the inline composer of the web (a title, Enter creates it
 * with the parent's project, default status, priority and assignees). The composer stays for the next title.
 */
export function SubIssuesSection({
  tasks,
  statuses,
  composing,
  creating,
  failed,
  onCompose,
  onAddExisting,
  onCreate,
  onCloseComposer,
  projectId,
}: {
  tasks: readonly TaskRecord[]
  statuses: readonly StatusRecord[]
  composing: boolean
  creating: boolean
  failed: boolean
  onCompose: () => void
  onAddExisting: () => void
  onCreate: (title: string) => void
  onCloseComposer: () => void
  projectId: string
}) {
  const { colors, scheme } = useTheme()
  const projects = useProjects()
  const [title, setTitle] = useState('')
  if (tasks.length === 0 && !composing) return null
  const statusOf = (statusId: string) => statuses.find((status) => status.id === statusId)
  const closed = tasks.filter((task) => isClosed(statusOf(task.status_id))).length
  const submit = () => {
    const text = title.trim()
    if (!text) return
    setTitle('')
    onCreate(text)
  }
  return (
    <View style={{ borderTopWidth: 1, borderTopColor: colors.border, paddingTop: 12 }}>
      <SectionTitle
        title="Sub-issues"
        count={
          <View style={{ height: 22, flexDirection: 'row', alignItems: 'center', gap: 4, paddingHorizontal: 7, borderRadius: 11, borderWidth: 1, borderColor: colors.border }}>
            <Icon name="circle" size={12} color={colors.mutedForeground} />
            <Text style={[type(12), { color: colors.mutedForeground }]}>{`${closed}/${tasks.length}`}</Text>
          </View>
        }
      >
        <Button label="Add existing" variant="ghost" size="xs" muted onPress={onAddExisting} />
        <IconButton icon="plus" label="Add sub-issue" size={24} iconSize={14} color={colors.mutedForeground} onPress={onCompose} />
      </SectionTitle>
      {tasks.map((task) => (
        <TaskLine key={task.id} taskId={task.id} status={statusOf(task.status_id)} id={identifier(task, projects.data ?? [])} title={task.title} />
      ))}
      {composing ? (
        <View style={{ minHeight: 36, flexDirection: 'row', alignItems: 'center', gap: 8, paddingHorizontal: 8, marginHorizontal: -8, borderRadius: 8, backgroundColor: alpha(colors.muted, 0.5) }}>
          <StatusIcon status={defaultStatusOf(statuses, projectId)} size={14} />
          <TextInput
            autoFocus
            value={title}
            onChangeText={setTitle}
            onSubmitEditing={submit}
            submitBehavior="submit"
            returnKeyType="done"
            placeholder="Sub-issue title"
            accessibilityLabel="Sub-issue title"
            placeholderTextColor={colors.mutedForeground}
            selectionColor={colors.primary}
            keyboardAppearance={scheme}
            style={{ flex: 1, ...type(16), lineHeight: 22, paddingVertical: 6, color: colors.foreground }}
          />
          {creating ? <Text style={[type(12), { color: colors.mutedForeground }]}>Creating…</Text> : null}
          <IconButton icon="close" label="Close the composer" size={24} iconSize={14} color={colors.mutedForeground} onPress={onCloseComposer} />
        </View>
      ) : null}
      {failed ? <Text style={[type(12), { color: colors.destructive, marginTop: 4 }]}>Couldn’t create the sub-issue.</Text> : null}
    </View>
  )
}

/** Relations in the web's groups (Blocked by, Blocks, Related …), each with a remove button. */
export function RelationsSection({ relations, statuses, removingId, onRemove }: { relations: readonly TaskRelationRecord[]; statuses: readonly StatusRecord[]; removingId: string | undefined; onRemove: (relationId: string) => void }) {
  const { colors } = useTheme()
  const projects = useProjects()
  const groups = groupRelations(relations)
  if (groups.length === 0) return null
  const count = groups.reduce((total, group) => total + group.relations.length, 0)
  return (
    <View style={{ borderTopWidth: 1, borderTopColor: colors.border, paddingTop: 12 }}>
      <SectionTitle title="Relations" count={count} />
      {groups.map((group) =>
        group.relations.map((relation, index) => {
          const id = identifier(relation.task, projects.data ?? [])
          return (
            <TaskLine
              key={relation.id}
              taskId={relation.task.id}
              label={index === 0 ? group.label : ''}
              status={statuses.find((status) => status.id === relation.task.status_id)}
              id={id}
              title={relation.task.title}
              trailing={<IconButton icon="close" label={`Remove relation to ${id}`} size={24} iconSize={12} color={colors.mutedForeground} disabled={removingId === relation.id} onPress={() => onRemove(relation.id)} />}
            />
          )
        }),
      )}
    </View>
  )
}
