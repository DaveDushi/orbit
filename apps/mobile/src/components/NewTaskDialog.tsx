import { useMutation, useQueryClient } from '@tanstack/react-query'
import { router } from 'expo-router'
import { type ReactNode, useState } from 'react'
import { KeyboardAvoidingView, Modal, Platform, Pressable, ScrollView, TextInput, useWindowDimensions, View } from 'react-native'

import { keys } from '../data/keys'
import * as api from '../data/orbit'
import { defaultStatusOf, PRIORITY_LABEL, type Priority, projectStatuses } from '../data/tasks'
import { useOrbit } from '../orbit-context'
import { usePeople, useProjects, useStatuses } from '../queries'
import { errorMessage } from '../session-context'
import { alpha, cssColors, mix, type, useTheme } from '../theme'
import { Avatar, Button, Icon, type IconName, IconButton, PickerSheet, PriorityIcon, StatusIcon, Text } from '../ui'

type Picker = 'project' | 'status' | 'priority' | 'assignee'
const PRIORITIES: Priority[] = ['none', 'urgent', 'high', 'medium', 'low']

export interface NewTaskDefaults {
  projectId?: string
  statusId?: string
}

/**
 * The web's New task dialog: a card 14% from the top over a faint scrim, with property chips and a footer. Mounted
 * while open, so each opening starts from `defaults`.
 */
export function NewTaskDialog({ defaults, onClose }: { defaults: NewTaskDefaults; onClose: () => void }) {
  const params = defaults
  const { client, workspaceId } = useOrbit()
  const queryClient = useQueryClient()
  const { colors, scheme } = useTheme()
  const { height } = useWindowDimensions()
  const projects = useProjects()
  const statuses = useStatuses()
  const people = usePeople()
  const [title, setTitle] = useState('')
  const [description, setDescription] = useState('')
  const [projectId, setProjectId] = useState<string | null>(params.projectId ?? null)
  const [statusId, setStatusId] = useState<string | null>(params.statusId ?? null)
  const [priority, setPriority] = useState<Priority>('none')
  const [assigneeIds, setAssigneeIds] = useState<string[]>([])
  const [createMore, setCreateMore] = useState(false)
  const [picker, setPicker] = useState<Picker | null>(null)

  const create = useMutation({
    mutationFn: (input: api.NewTask) => api.newTask(client, workspaceId, input),
    onSuccess: (task) => {
      void queryClient.invalidateQueries({ queryKey: keys.tasks(workspaceId) })
      if (createMore) {
        setTitle('')
        setDescription('')
        return
      }
      onClose()
      router.push(`/tasks/${task.id}`)
    },
  })

  const project = projects.data?.find((item) => item.id === projectId) ?? projects.data?.[0]
  const own = project && statuses.data ? projectStatuses(statuses.data, project.id) : []
  const status = own.find((item) => item.id === statusId) ?? (project && statuses.data ? defaultStatusOf(statuses.data, project.id) : undefined)
  const canCreate = title.trim().length > 0 && project !== undefined && status !== undefined
  const assignees = people.filter((person) => assigneeIds.includes(person.id))

  return (
    <Modal visible transparent animationType="fade" onRequestClose={onClose}>
      <Pressable accessibilityLabel="Close" onPress={onClose} style={{ position: 'absolute', inset: 0, backgroundColor: alpha(cssColors.black, 0.1) }} />
      <KeyboardAvoidingView behavior={Platform.OS === 'ios' ? 'padding' : undefined} style={{ flex: 1 }} pointerEvents="box-none">
        <View
          accessibilityViewIsModal
          style={{
            marginTop: Math.round(height * 0.14),
            marginHorizontal: 16,
            maxHeight: height * 0.8,
            borderRadius: 14,
            backgroundColor: colors.popover,
            boxShadow: `0 0 0 1px ${alpha(colors.foreground, 0.1)}`,
            overflow: 'hidden',
          }}
        >
          <ScrollView contentContainerStyle={{ padding: 16, paddingBottom: 12, gap: 12 }} keyboardShouldPersistTaps="handled">
            <View style={{ flexDirection: 'row', alignItems: 'center', gap: 6, paddingRight: 32 }}>
              <Pressable
                accessibilityRole="button"
                accessibilityLabel={`Project: ${project?.name ?? 'none'}`}
                onPress={() => setPicker('project')}
                style={{ height: 24, flexDirection: 'row', alignItems: 'center', gap: 4, paddingHorizontal: 8, borderRadius: 8, borderWidth: 1, borderColor: colors.input, backgroundColor: alpha(colors.input, 0.3), boxShadow: `0 2px 0 ${alpha(scheme === 'dark' ? cssColors.white : cssColors.black, scheme === 'dark' ? 0.08 : 0.22)}` }}
              >
                {project ? <View style={{ width: 6, height: 6, borderRadius: 3, backgroundColor: project.color }} /> : null}
                <Text numberOfLines={1} style={[type(12), { lineHeight: 16, maxWidth: 170 }]}>
                  {project?.name ?? 'No project'}
                </Text>
              </Pressable>
              <Icon name="chevronRight" size={12} color={alpha(colors.mutedForeground, 0.7)} />
              <Text style={[type(12), { lineHeight: 16, color: colors.mutedForeground }]}>New task</Text>
            </View>
            <View style={{ position: 'absolute', top: 8, right: 8 }}>
              <IconButton icon="close" label="Close" onPress={onClose} />
            </View>

            <View style={{ gap: 6 }}>
              <TextInput
                accessibilityLabel="Task title"
                placeholder="Task title"
                placeholderTextColor={colors.mutedForeground}
                selectionColor={colors.primary}
                keyboardAppearance={scheme}
                autoFocus
                value={title}
                onChangeText={setTitle}
                style={{ ...type(16, 'medium'), lineHeight: 25, padding: 0, color: colors.foreground }}
              />
              <TextInput
                accessibilityLabel="Description"
                placeholder="Add description…"
                placeholderTextColor={colors.mutedForeground}
                selectionColor={colors.primary}
                keyboardAppearance={scheme}
                multiline
                value={description}
                onChangeText={setDescription}
                style={{ ...type(16), lineHeight: 24, minHeight: 80, padding: 0, paddingTop: 2, textAlignVertical: 'top', color: colors.foreground }}
              />
              <View style={{ flexDirection: 'row', marginLeft: -8 }}>
                <Quiet icon="attach" label="Attach" />
                <Quiet icon="relation" label="Add relation" />
              </View>
            </View>

            <View style={{ flexDirection: 'row', flexWrap: 'wrap', gap: 6 }}>
              <Chip label={`Status: ${status?.name ?? 'none'}`} onPress={() => setPicker('status')}>
                <StatusIcon status={status} size={14} />
                <ChipText>{status?.name ?? 'No status'}</ChipText>
              </Chip>
              <Chip label={`Priority: ${PRIORITY_LABEL[priority]}`} onPress={() => setPicker('priority')}>
                <PriorityIcon priority={priority} size={14} />
                <ChipText>{priority === 'none' ? 'Priority' : PRIORITY_LABEL[priority]}</ChipText>
              </Chip>
              <Chip label="Assignee" onPress={() => setPicker('assignee')}>
                {assignees.length === 0 ? <Icon name="user" size={14} color={colors.mutedForeground} /> : <Avatar id={assignees[0]!.id} name={assignees[0]!.name} size={14} />}
                <ChipText>{assignees.length === 0 ? 'Assignee' : assignees.map((person) => person.name).join(', ')}</ChipText>
              </Chip>
              <Chip label="Labels">
                <Icon name="tag" size={14} color={colors.mutedForeground} />
                <ChipText>Labels</ChipText>
              </Chip>
              <Chip label="Due date">
                <Icon name="calendar" size={14} color={colors.foreground} />
                <ChipText>Due date</ChipText>
              </Chip>
              <Chip label="Parent">
                <Icon name="subIssue" size={14} color={colors.mutedForeground} />
                <ChipText>Parent</ChipText>
              </Chip>
            </View>
            {create.isError ? <Text style={[type(12), { color: colors.destructive }]}>{errorMessage(create.error)}</Text> : null}
          </ScrollView>

          <View style={{ flexDirection: 'row', alignItems: 'center', justifyContent: 'center', gap: 12, paddingHorizontal: 16, paddingTop: 10, paddingBottom: 18, borderTopWidth: 1, borderTopColor: colors.border, backgroundColor: alpha(colors.muted, 0.5) }}>
            <Pressable accessibilityRole="switch" accessibilityState={{ checked: createMore }} onPress={() => setCreateMore((value) => !value)} style={{ flexDirection: 'row', alignItems: 'center', gap: 8 }}>
              <View style={{ width: 24, height: 14, borderRadius: 7, padding: 1, alignItems: createMore ? 'flex-end' : 'flex-start', backgroundColor: createMore ? colors.primary : alpha(colors.foreground, 0.12) }}>
                <View style={{ width: 12, height: 12, borderRadius: 6, backgroundColor: createMore ? colors.primaryForeground : colors.foreground }} />
              </View>
              <Text style={[type(12), { lineHeight: 16, color: colors.mutedForeground }]}>Create more</Text>
            </Pressable>
            <Button
              label="Create task"
              loading={create.isPending}
              disabled={!canCreate}
              onPress={() =>
                project && status && create.mutate({ title: title.trim(), description: description.trim(), projectId: project.id, statusId: status.id, priority, assigneeIds })
              }
            />
          </View>
        </View>
      </KeyboardAvoidingView>

      <PickerSheet
        visible={picker === 'project'}
        title="Project"
        options={(projects.data ?? []).map((item) => ({ key: item.id, label: item.name, subtitle: item.key, selected: item.id === project?.id, leading: <View style={{ width: 8, height: 8, borderRadius: 4, backgroundColor: item.color }} /> }))}
        onSelect={(id) => {
          setPicker(null)
          setProjectId(id)
          // a status belongs to one project: the new project starts at its default
          setStatusId(null)
        }}
        onClose={() => setPicker(null)}
      />
      <PickerSheet
        visible={picker === 'status'}
        title="Status"
        options={own.map((item) => ({ key: item.id, label: item.name, leading: <StatusIcon status={item} />, selected: item.id === status?.id }))}
        onSelect={(id) => {
          setPicker(null)
          setStatusId(id)
        }}
        onClose={() => setPicker(null)}
      />
      <PickerSheet
        visible={picker === 'priority'}
        title="Priority"
        options={PRIORITIES.map((item) => ({ key: item, label: PRIORITY_LABEL[item], leading: <PriorityIcon priority={item} />, selected: item === priority }))}
        onSelect={(key) => {
          setPicker(null)
          setPriority(key as Priority)
        }}
        onClose={() => setPicker(null)}
      />
      <PickerSheet
        visible={picker === 'assignee'}
        title="Assignee"
        options={people.map((person) => ({ key: person.id, label: person.name, leading: <Avatar id={person.id} name={person.name} size={20} />, selected: assigneeIds.includes(person.id) }))}
        onSelect={(id) => {
          setPicker(null)
          setAssigneeIds((list) => (list.includes(id) ? list.filter((item) => item !== id) : [...list, id]))
        }}
        onClose={() => setPicker(null)}
      />
    </Modal>
  )
}

/** The web's outline `sm` button: a 28px chip with the 2px edge. */
function Chip({ label, onPress, children }: { label: string; onPress?: () => void; children: ReactNode }) {
  const { colors, scheme } = useTheme()
  const edge = alpha(scheme === 'dark' ? cssColors.white : cssColors.black, scheme === 'dark' ? 0.08 : 0.22)
  return (
    <Pressable
      accessibilityRole={onPress ? 'button' : undefined}
      accessibilityLabel={label}
      disabled={!onPress}
      onPress={onPress}
      style={({ pressed }) => ({
        height: 28,
        flexDirection: 'row',
        alignItems: 'center',
        gap: 4,
        paddingHorizontal: 10,
        borderRadius: 8,
        borderWidth: 1,
        borderColor: colors.input,
        backgroundColor: alpha(colors.input, pressed ? 0.5 : 0.3),
        boxShadow: pressed ? undefined : `0 2px 0 ${edge}`,
        transform: [{ translateY: pressed ? 2 : 0 }],
      })}
    >
      {children}
    </Pressable>
  )
}

function ChipText({ children }: { children: ReactNode }) {
  return (
    <Text numberOfLines={1} style={[type(12.8), { lineHeight: 18, maxWidth: 200 }]}>
      {children}
    </Text>
  )
}

function Quiet({ icon, label }: { icon: IconName; label: string }) {
  const { colors } = useTheme()
  const color = alpha(colors.mutedForeground, 0.7)
  return (
    <View style={{ height: 32, flexDirection: 'row', alignItems: 'center', gap: 6, paddingHorizontal: 10 }}>
      <Icon name={icon} size={14} color={color} />
      <Text style={[type(12, 'medium'), { lineHeight: 16, color }]}>{label}</Text>
    </View>
  )
}
