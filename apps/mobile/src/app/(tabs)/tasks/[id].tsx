import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { router, useLocalSearchParams } from 'expo-router'
import { type ReactNode, useMemo, useState } from 'react'
import { Alert, KeyboardAvoidingView, Platform, Pressable, RefreshControl, ScrollView, TextInput, View } from 'react-native'
import { SafeAreaView } from 'react-native-safe-area-context'

import { activityRows, buildFeed, type ActivityRow, type CommentThread, foldRun } from '../../../data/activity'
import { fullDate, relativeAgo } from '../../../data/format'
import { keys } from '../../../data/keys'
import { markdownToText } from '../../../data/markdown'
import * as api from '../../../data/orbit'
import { identifier, isVersionConflict, PRIORITY_LABEL, priorityOf, projectStatuses, replaceTask } from '../../../data/tasks'
import { useOrbit } from '../../../orbit-context'
import { useLabels, usePeople, useProjects, useStatuses } from '../../../queries'
import { errorMessage } from '../../../session-context'
import { useOpenOnWeb } from '../../../web'
import { alpha, type, useTheme } from '../../../theme'
import { Avatar, Button, ErrorView, Icon, type IconName, IconButton, LoadingView, PaneHeader, PickerSheet, PriorityIcon, StatusIcon, Text } from '../../../ui'

export default function TaskScreen() {
  const { id } = useLocalSearchParams<{ id: string }>()
  const { client, workspaceId } = useOrbit()
  const queryClient = useQueryClient()
  const { colors, scheme } = useTheme()
  const projects = useProjects()
  const statuses = useStatuses()
  const labels = useLabels()
  const people = usePeople()
  const [picking, setPicking] = useState(false)
  const [draft, setDraft] = useState('')
  const [unfolded, setUnfolded] = useState<readonly string[]>([])
  const openOnWeb = useOpenOnWeb()
  const onWeb = () => openOnWeb(`/tasks/${id}`)

  const task = useQuery({
    queryKey: keys.task(workspaceId, id),
    queryFn: () => api.task(client, workspaceId, id),
    // the list row the user tapped, until the full task arrives
    placeholderData: () => queryClient.getQueryData<api.TaskRecord[]>(keys.tasks(workspaceId))?.find((item) => item.id === id),
  })
  const comments = useQuery({ queryKey: keys.comments(workspaceId, id), queryFn: () => api.comments(client, workspaceId, id) })
  const activity = useQuery({ queryKey: keys.taskActivity(workspaceId, id), queryFn: () => api.taskActivity(client, workspaceId, id) })

  const setTask = (next: api.TaskRecord) => {
    queryClient.setQueryData(keys.task(workspaceId, id), next)
    queryClient.setQueryData<api.TaskRecord[]>(keys.tasks(workspaceId), (list) => list && replaceTask(list, next))
  }

  const changeStatus = useMutation({
    mutationFn: ({ current, statusId }: { current: api.TaskRecord; statusId: string }) =>
      api.changeTask(client, workspaceId, current.id, { status_id: statusId, expected_version: current.version }),
    onMutate: async ({ current, statusId }) => {
      await queryClient.cancelQueries({ queryKey: keys.task(workspaceId, id) })
      setTask({ ...current, status_id: statusId })
      return current
    },
    onError: (error, _input, previous) => {
      if (previous) setTask(previous)
      void queryClient.invalidateQueries({ queryKey: keys.task(workspaceId, id) })
      if (isVersionConflict(error)) Alert.alert('Task changed', 'Someone else changed this task. Showing the latest version; try again.')
      else Alert.alert('Could not change the status', errorMessage(error))
    },
    onSuccess: (updated) => {
      setTask(updated)
      // closing a task can close its parent or sub-issues too
      void queryClient.invalidateQueries({ queryKey: keys.tasks(workspaceId) })
      void queryClient.invalidateQueries({ queryKey: keys.taskActivity(workspaceId, id) })
    },
  })

  const sendComment = useMutation({
    mutationFn: (body: string) => api.addComment(client, workspaceId, id, body),
    onSuccess: (comment) => {
      setDraft('')
      queryClient.setQueryData<api.CommentRecord[]>(keys.comments(workspaceId, id), (list) => (list ? [...list, comment] : [comment]))
    },
    onError: (error) => Alert.alert('Could not add the comment', errorMessage(error)),
  })

  const trash = useMutation({
    mutationFn: (current: api.TaskRecord) => api.trashTask(client, workspaceId, current),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: keys.tasks(workspaceId) })
      close()
    },
    onError: (error) => Alert.alert('Task deletion failed', errorMessage(error)),
  })

  const close = () => (router.canGoBack() ? router.back() : router.navigate('/tasks'))
  const nameOf = (userId: string) => people.find((person) => person.id === userId)?.name ?? 'Unknown'
  const current = task.data
  const project = projects.data?.find((item) => item.id === current?.project_id)

  const feed = useMemo(() => {
    if (!activity.data || !comments.data) return []
    const known = queryClient.getQueryData<api.TaskRecord[]>(keys.tasks(workspaceId)) ?? []
    const rows = activityRows(activity.data, {
      statuses: statuses.data ?? [],
      people,
      labels: labels.data ?? [],
      projects: projects.data ?? [],
      identifierOf: (taskId) => {
        const other = known.find((item) => item.id === taskId)
        return other ? identifier(other, projects.data ?? []) : 'another task'
      },
    })
    return buildFeed(rows, comments.data)
  }, [activity.data, comments.data, labels.data, people, projects.data, queryClient, statuses.data, workspaceId])

  const header = (
    <PaneHeader style={{ paddingHorizontal: 12 }}>
      <IconButton icon="back" label="Back to tasks" color={alpha(colors.mutedForeground, 0.7)} onPress={close} />
      <View style={{ flex: 1, flexDirection: 'row', alignItems: 'center', gap: 6, minWidth: 0 }}>
        {project ? (
          <Pressable accessibilityRole="link" onPress={() => router.navigate('/tasks')} style={{ flexDirection: 'row', alignItems: 'center', gap: 6, flexShrink: 1 }}>
            <View style={{ width: 6, height: 6, borderRadius: 3, backgroundColor: project.color }} />
            <Text numberOfLines={1} style={[type(12, 'medium'), { lineHeight: 16, flexShrink: 1 }]}>
              {project.name}
            </Text>
          </Pressable>
        ) : null}
        {project ? <Icon name="chevronRight" size={14} color={colors.mutedForeground} /> : null}
        <Text style={[type(12), { lineHeight: 16, color: colors.mutedForeground }]}>{current ? identifier(current, projects.data ?? []) : 'Task'}</Text>
      </View>
      {current ? (
        <Button
          label="Delete"
          variant="destructive"
          disabled={trash.isPending}
          onPress={() =>
            Alert.alert('Move to trash?', 'You can restore it from the trash in Orbit on the web.', [
              { text: 'Cancel', style: 'cancel' },
              { text: 'Delete', style: 'destructive', onPress: () => trash.mutate(current) },
            ])
          }
        />
      ) : null}
    </PaneHeader>
  )

  if (!current) {
    return (
      <SafeAreaView edges={['top', 'left', 'right']} style={{ flex: 1, backgroundColor: colors.background }}>
        {header}
        {task.isError ? <ErrorView error={task.error} onRetry={() => void task.refetch()} /> : <LoadingView />}
      </SafeAreaView>
    )
  }

  const status = statuses.data?.find((item) => item.id === current.status_id)
  const ownStatuses = statuses.data ? projectStatuses(statuses.data, current.project_id) : []
  const description = markdownToText(current.description)
  const taskLabels = (labels.data ?? []).filter((label) => current.label_ids.includes(label.id))

  return (
    <SafeAreaView edges={['top', 'left', 'right']} style={{ flex: 1, backgroundColor: colors.background }}>
      {header}
      <KeyboardAvoidingView style={{ flex: 1 }} behavior={Platform.OS === 'ios' ? 'padding' : undefined}>
        <ScrollView
          contentContainerStyle={{ paddingHorizontal: 14, paddingTop: 16, paddingBottom: 28, gap: 14 }}
          keyboardShouldPersistTaps="handled"
          refreshControl={<RefreshControl refreshing={task.isRefetching} onRefresh={() => void Promise.all([task.refetch(), comments.refetch(), activity.refetch()])} tintColor={colors.mutedForeground} />}
        >
          <View>
            <Text selectable style={[type(20, 'semibold'), { lineHeight: 26 }]}>
              {current.title || 'Untitled'}
            </Text>
            {description ? (
              <Text selectable style={{ ...type(16), lineHeight: 24, marginTop: 19 }}>
                {description}
              </Text>
            ) : null}
            <View style={{ flexDirection: 'row', marginTop: description ? 43 : 14 }}>
              <QuietAction icon="relation" label="Add relation" onPress={onWeb} />
              <QuietAction icon="subIssue" label="Add sub-issue" onPress={onWeb} />
            </View>
          </View>

          <View style={{ borderTopWidth: 1, borderBottomWidth: 1, borderColor: colors.border, paddingVertical: 14, flexDirection: 'row', flexWrap: 'wrap', columnGap: 12, rowGap: 14 }}>
            <PropertyGroup title="Properties" full>
              <View style={{ flexDirection: 'row', flexWrap: 'wrap', gap: 4 }}>
                <Chip pull label={`Status: ${status?.name ?? 'None'}`} onPress={ownStatuses.length > 0 ? () => setPicking(true) : undefined}>
                  <StatusIcon status={status} size={16} />
                  <ChipText>{status?.name ?? 'No status'}</ChipText>
                </Chip>
                <Chip pull label={`Priority: ${PRIORITY_LABEL[priorityOf(current)]}`} onPress={onWeb}>
                  <PriorityIcon priority={current.priority} size={16} />
                  <ChipText>{PRIORITY_LABEL[priorityOf(current)]}</ChipText>
                </Chip>
                <Chip pull label="Assignees" onPress={onWeb}>
                  {current.assignee_ids.length === 0 ? (
                    <>
                      <Icon name="user" size={14} color={colors.mutedForeground} />
                      <ChipText muted>Assignee</ChipText>
                    </>
                  ) : (
                    <>
                      {current.assignee_ids.slice(0, 2).map((userId) => (
                        <Avatar key={userId} id={userId} name={nameOf(userId)} size={16} />
                      ))}
                      <ChipText>{current.assignee_ids.map(nameOf).join(', ')}</ChipText>
                    </>
                  )}
                </Chip>
              </View>
              <View style={{ flexDirection: 'row', marginTop: 4, marginLeft: -8 }}>
                <Chip label="Parent" onPress={onWeb}>
                  <Icon name="subIssue" size={14} color={colors.mutedForeground} />
                  {current.parent ? (
                    <>
                      <ChipText muted>{identifier({ project_id: current.parent.project_id, number: current.parent.number }, projects.data ?? [])}</ChipText>
                      <ChipText>{current.parent.title || 'Untitled'}</ChipText>
                    </>
                  ) : (
                    <ChipText muted>Set parent</ChipText>
                  )}
                </Chip>
              </View>
            </PropertyGroup>
            <PropertyGroup title="Labels">
              {taskLabels.length === 0 ? (
                <View style={{ marginLeft: -8, flexDirection: 'row' }}>
                  <Chip label="Add label" onPress={onWeb}>
                    <Icon name="tag" size={14} color={colors.mutedForeground} />
                    <ChipText muted>Add label</ChipText>
                  </Chip>
                </View>
              ) : (
                <View style={{ flexDirection: 'row', flexWrap: 'wrap', gap: 4 }}>
                  {taskLabels.map((label) => (
                    <Pill key={label.id} color={label.color} name={label.name} />
                  ))}
                </View>
              )}
            </PropertyGroup>
            <PropertyGroup title="Project">{project ? <Pill color={project.color} name={project.name} /> : <ChipText muted>—</ChipText>}</PropertyGroup>
            <PropertyGroup title="Source">
              <View style={{ marginLeft: -8, flexDirection: 'row' }}>
                <Chip label="Source" onPress={onWeb}>
                  <ChipText muted>{current.source_url ? current.source_url.replace(/^https?:\/\//, '').split('/')[0] : 'Add source'}</ChipText>
                </Chip>
              </View>
            </PropertyGroup>
            <PropertyGroup title="Due date">
              <View style={{ marginLeft: -8, flexDirection: 'row' }}>
                <Chip label="Due date" onPress={onWeb}>
                  <Icon name="calendar" size={14} color={colors.foreground} />
                  <ChipText>{current.due_at ? fullDate(current.due_at) : 'Set due date'}</ChipText>
                </Chip>
              </View>
            </PropertyGroup>
          </View>

          <View>
            <Text style={[type(14, 'semibold'), { lineHeight: 20, marginBottom: 8 }]}>Activity</Text>
            {activity.isError || comments.isError ? (
              <Pressable onPress={() => void Promise.all([activity.refetch(), comments.refetch()])}>
                <Text style={[type(12), { color: colors.destructive }]}>Could not load the activity. Tap to retry.</Text>
              </Pressable>
            ) : (
              feed.map((entry) =>
                entry.kind === 'thread' ? (
                  <Thread key={entry.thread.root.id} thread={entry.thread} nameOf={nameOf} />
                ) : (
                  <Timeline
                    key={entry.items[0]!.id}
                    run={entry.items}
                    unfolded={unfolded.includes(entry.items[0]!.id)}
                    onToggle={() => {
                      const runId = entry.items[0]!.id
                      setUnfolded((list) => (list.includes(runId) ? list.filter((item) => item !== runId) : [...list, runId]))
                    }}
                    nameOf={nameOf}
                    statusOf={(statusId) => statuses.data?.find((item) => item.id === statusId)}
                  />
                ),
              )
            )}
            <CommentComposer value={draft} onChangeText={setDraft} sending={sendComment.isPending} onSend={() => sendComment.mutate(draft.trim())} onAttach={onWeb} />
          </View>
        </ScrollView>
      </KeyboardAvoidingView>
      <PickerSheet
        visible={picking}
        title="Status"
        options={ownStatuses.map((item) => ({ key: item.id, label: item.name, leading: <StatusIcon status={item} />, selected: item.id === current.status_id }))}
        onSelect={(statusId) => {
          setPicking(false)
          if (statusId !== current.status_id) changeStatus.mutate({ current, statusId })
        }}
        onClose={() => setPicking(false)}
      />
    </SafeAreaView>
  )
}

/** "Add relation" / "Add sub-issue": the web's quiet ghost buttons under the description. */
function QuietAction({ icon, label, onPress }: { icon: IconName; label: string; onPress: () => void }) {
  const { colors } = useTheme()
  const color = alpha(colors.mutedForeground, 0.7)
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={label}
      onPress={onPress}
      style={({ pressed }) => ({ height: 32, flexDirection: 'row', alignItems: 'center', gap: 6, paddingHorizontal: 10, borderRadius: 10, backgroundColor: pressed ? colors.muted : 'transparent' })}
    >
      <Icon name={icon} size={14} color={color} />
      <Text style={[type(12, 'medium'), { lineHeight: 16, color }]}>{label}</Text>
    </Pressable>
  )
}

function PropertyGroup({ title, full = false, children }: { title: string; full?: boolean; children: ReactNode }) {
  const { colors } = useTheme()
  return (
    <View style={{ width: full ? '100%' : '47%', flexGrow: full ? 0 : 1, alignItems: 'flex-start' }}>
      <Text style={[type(12, 'medium'), { lineHeight: 16, marginBottom: full ? 10 : 8, color: alpha(colors.mutedForeground, 0.7) }]}>{title}</Text>
      {children}
    </View>
  )
}

/** The web `PropertyButton` at phone size: a 28px ghost button with 12px text. */
function Chip({ label, onPress, pull = false, children }: { label: string; onPress?: () => void; /** The web's `-ml-2`: each chip of a wrapping row starts 8px left. */ pull?: boolean; children: ReactNode }) {
  const { colors } = useTheme()
  return (
    <Pressable
      accessibilityRole={onPress ? 'button' : undefined}
      accessibilityLabel={label}
      disabled={!onPress}
      onPress={onPress}
      style={({ pressed }) => ({ height: 28, maxWidth: '100%', marginLeft: pull ? -8 : 0, flexDirection: 'row', alignItems: 'center', gap: 6, paddingHorizontal: 6, borderRadius: 10, backgroundColor: pressed ? colors.muted : 'transparent' })}
    >
      {children}
    </Pressable>
  )
}

function ChipText({ children, muted = false }: { children: ReactNode; muted?: boolean }) {
  const { colors } = useTheme()
  return (
    <Text numberOfLines={1} style={[type(12, 'medium'), { lineHeight: 16, flexShrink: 1, color: muted ? colors.mutedForeground : colors.foreground }]}>
      {children}
    </Text>
  )
}

/** A project or label: a round outline pill with its color dot. */
function Pill({ color, name }: { color: string; name: string }) {
  const { colors } = useTheme()
  return (
    <View style={{ height: 20, flexDirection: 'row', alignItems: 'center', gap: 4, paddingHorizontal: 8, borderRadius: 10, borderWidth: 1, borderColor: colors.input, backgroundColor: alpha(colors.input, 0.3) }}>
      <View style={{ width: 6, height: 6, borderRadius: 3, backgroundColor: color }} />
      <Text numberOfLines={1} style={[type(12, 'medium'), { lineHeight: 16 }]}>
        {name}
      </Text>
    </View>
  )
}

/** A run of activity rows on a connector line, folded to its first and latest rows like the web. */
function Timeline({
  run,
  unfolded,
  onToggle,
  nameOf,
  statusOf,
}: {
  run: ActivityRow[]
  unfolded: boolean
  onToggle: () => void
  nameOf: (userId: string) => string
  statusOf: (statusId: string) => api.StatusRecord | undefined
}) {
  const { colors } = useTheme()
  const folded = foldRun(run)
  const items = unfolded ? run : folded.items
  const rows: ReactNode[] = []
  items.forEach((item, index) => {
    const status = item.statusId ? statusOf(item.statusId) : undefined
    const actor = item.actorName ?? (item.actorId ? nameOf(item.actorId) : 'Someone')
    rows.push(
      <TimelineRow
        key={item.id}
        first={index === 0}
        last={index === items.length - 1 && folded.hidden === 0}
        glyph={status ? <StatusIcon status={status} size={14} /> : <Avatar id={item.actorId} name={actor} size={14} />}
      >
        <Text numberOfLines={1} style={[type(12), { lineHeight: 16, flex: 1, color: colors.mutedForeground }]}>
          <Text style={[type(12, 'medium'), { lineHeight: 16, color: colors.foreground }]}>{actor}</Text> {item.text} · {relativeAgo(item.createdAt)}
        </Text>
      </TimelineRow>,
    )
    if (folded.hidden > 0 && index === 0) {
      rows.push(
        <TimelineRow key={`${item.id}-fold`} first={false} last={items.length === 1} glyph={null}>
          <Pressable accessibilityRole="button" accessibilityState={{ expanded: unfolded }} onPress={onToggle} style={{ flexDirection: 'row', alignItems: 'center', gap: 8 }}>
            <View style={{ transform: [{ rotate: unfolded ? '180deg' : '0deg' }] }}>
              <Icon name="chevronDown" size={14} color={colors.mutedForeground} />
            </View>
            <Text style={[type(12, 'medium'), { lineHeight: 16, color: colors.mutedForeground }]}>{unfolded ? 'Show less' : `Show ${folded.hidden} more`}</Text>
          </Pressable>
        </TimelineRow>,
      )
    }
  })
  return <View>{rows}</View>
}

/** One timeline row: the 1px connector runs above and below the 14px glyph, the first and last drop their outer half. */
function TimelineRow({ first, last, glyph, children }: { first: boolean; last: boolean; glyph: ReactNode; children: ReactNode }) {
  const { colors } = useTheme()
  const line = { position: 'absolute' as const, left: 12.5, width: 1, backgroundColor: colors.border }
  return (
    <View style={{ minHeight: 30, flexDirection: 'row', alignItems: 'center', gap: 10, paddingVertical: 7, paddingLeft: 6 }}>
      {first ? null : <View style={[line, { top: 0, height: 7 }]} />}
      {last ? null : <View style={[line, { bottom: 0, height: 7 }]} />}
      <View style={{ width: 14, height: 14, alignItems: 'center', justifyContent: 'center' }}>{glyph}</View>
      {children}
    </View>
  )
}

/** A comment and its replies in one card. */
function Thread({ thread, nameOf }: { thread: CommentThread; nameOf: (userId: string) => string }) {
  const { colors } = useTheme()
  return (
    <View style={{ marginVertical: 12, borderRadius: 10, borderWidth: 1, borderColor: colors.border, backgroundColor: colors.card, overflow: 'hidden' }}>
      {[thread.root, ...thread.replies].map((comment, index) => (
        <View key={comment.id} style={{ flexDirection: 'row', gap: 8, paddingHorizontal: 14, paddingVertical: 10, borderTopWidth: index === 0 ? 0 : 1, borderTopColor: colors.border }}>
          <Avatar id={comment.author_id} name={nameOf(comment.author_id)} size={20} />
          <View style={{ flex: 1, minWidth: 0 }}>
            <View style={{ minHeight: 20, flexDirection: 'row', alignItems: 'center', gap: 6 }}>
              <Text numberOfLines={1} style={[type(13, 'semibold'), { flexShrink: 1 }]}>
                {nameOf(comment.author_id)}
              </Text>
              <Text style={{ fontSize: 11, color: alpha(colors.mutedForeground, 0.7) }}>·</Text>
              <Text style={[type(11, 'medium'), { color: colors.mutedForeground }]}>
                {relativeAgo(comment.created_at)}
                {comment.updated_at !== comment.created_at ? ' (edited)' : ''}
              </Text>
            </View>
            <Text selectable style={[type(13), { lineHeight: 19, marginTop: 2 }]}>
              {markdownToText(comment.body)}
            </Text>
          </View>
        </View>
      ))}
    </View>
  )
}

/** The web comment box: a card with the field on top and attach + a round send button below. */
function CommentComposer({ value, onChangeText, sending, onSend, onAttach }: { value: string; onChangeText: (text: string) => void; sending: boolean; onSend: () => void; onAttach: () => void }) {
  const { colors, scheme } = useTheme()
  const [focused, setFocused] = useState(false)
  const canSend = value.trim().length > 0 && !sending
  return (
    <View style={{ marginTop: 12, borderRadius: 10, borderWidth: 1, borderColor: focused ? colors.ring : colors.border, backgroundColor: colors.card }}>
      <TextInput
        value={value}
        onChangeText={onChangeText}
        onFocus={() => setFocused(true)}
        onBlur={() => setFocused(false)}
        placeholder="Leave a comment… Use @ to mention"
        placeholderTextColor={colors.mutedForeground}
        selectionColor={colors.primary}
        keyboardAppearance={scheme}
        multiline
        numberOfLines={1}
        accessibilityLabel="Comment"
        style={{ minHeight: 40, maxHeight: 160, paddingHorizontal: 12, paddingTop: 10, paddingBottom: 0, fontSize: 16, lineHeight: 24, fontFamily: type(16).fontFamily, color: colors.foreground }}
      />
      <View style={{ flexDirection: 'row', justifyContent: 'flex-end', alignItems: 'center', gap: 4, padding: 6 }}>
        <IconButton icon="attach" label="Attach" color={colors.mutedForeground} onPress={onAttach} />
        <Pressable
          accessibilityRole="button"
          accessibilityLabel={sending ? 'Sending…' : 'Send'}
          accessibilityState={{ disabled: !canSend }}
          disabled={!canSend}
          onPress={onSend}
          style={{ width: 28, height: 28, borderRadius: 14, alignItems: 'center', justifyContent: 'center', backgroundColor: colors.primary, opacity: canSend ? 1 : 0.5 }}
        >
          <Icon name="arrowUp" size={16} color={colors.primaryForeground} />
        </Pressable>
      </View>
    </View>
  )
}
