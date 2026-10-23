import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Stack, useLocalSearchParams } from 'expo-router'
import { useHeaderHeight } from 'expo-router/react-navigation'
import { type ReactNode, useState } from 'react'
import { Alert, KeyboardAvoidingView, Platform, Pressable, RefreshControl, ScrollView, View } from 'react-native'

import { Composer } from '../../components/Composer'
import { errorMessage } from '../../session-context'
import { keys } from '../../data/keys'
import { markdownToText } from '../../data/markdown'
import * as api from '../../data/orbit'
import { dueLabel, identifier, isVersionConflict, PRIORITY_LABEL, priorityOf, projectStatuses, replaceTask } from '../../data/tasks'
import { useOrbit } from '../../orbit-context'
import { usePeople, useProjects, useStatuses } from '../../queries'
import { useTheme } from '../../theme'
import { shortTime } from '../../time'
import { Avatar, ErrorView, LoadingView, PickerSheet, PriorityIcon, StatusIcon, Text } from '../../ui'

export default function TaskScreen() {
  const { id } = useLocalSearchParams<{ id: string }>()
  const { client, workspaceId } = useOrbit()
  const queryClient = useQueryClient()
  const { colors, radius, spacing } = useTheme()
  const headerHeight = useHeaderHeight()
  const projects = useProjects()
  const statuses = useStatuses()
  const people = usePeople()
  const [picking, setPicking] = useState(false)
  const [draft, setDraft] = useState('')

  const task = useQuery({
    queryKey: keys.task(workspaceId, id),
    queryFn: () => api.task(client, workspaceId, id),
    // the list row the user tapped, until the full task arrives
    placeholderData: () => queryClient.getQueryData<api.TaskRecord[]>(keys.tasks(workspaceId))?.find((item) => item.id === id),
  })
  const comments = useQuery({ queryKey: keys.comments(workspaceId, id), queryFn: () => api.comments(client, workspaceId, id) })

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

  if (!task.data) {
    return task.isError ? <ErrorView error={task.error} onRetry={() => void task.refetch()} /> : <LoadingView />
  }

  const current = task.data
  const status = statuses.data?.find((item) => item.id === current.status_id)
  const ownStatuses = statuses.data ? projectStatuses(statuses.data, current.project_id) : []
  const nameOf = (userId: string) => people.find((person) => person.id === userId)?.name ?? 'Unknown'
  const description = markdownToText(current.description)

  return (
    <KeyboardAvoidingView style={{ flex: 1 }} behavior={Platform.OS === 'ios' ? 'padding' : undefined} keyboardVerticalOffset={headerHeight}>
      <Stack.Screen options={{ title: identifier(current, projects.data ?? []) }} />
      <ScrollView
        contentContainerStyle={{ padding: spacing[4], gap: spacing[4] }}
        keyboardShouldPersistTaps="handled"
        refreshControl={<RefreshControl refreshing={task.isRefetching} onRefresh={() => void Promise.all([task.refetch(), comments.refetch()])} tintColor={colors.mutedForeground} />}
      >
        <Text variant="title" selectable>
          {current.title}
        </Text>

        <View style={{ borderWidth: 1, borderColor: colors.border, borderRadius: radius.xl, overflow: 'hidden' }}>
          <Property label="Status" onPress={ownStatuses.length > 0 ? () => setPicking(true) : undefined}>
            <StatusIcon status={status} />
            <Text variant="label">{status?.name ?? 'Unknown'}</Text>
          </Property>
          <Property label="Priority">
            <PriorityIcon priority={current.priority} />
            <Text variant="label">{PRIORITY_LABEL[priorityOf(current)]}</Text>
          </Property>
          <Property label="Assignees">
            {current.assignee_ids.length === 0 ? (
              <Text variant="label" color="muted">
                Unassigned
              </Text>
            ) : (
              <View style={{ gap: spacing[1.5], flex: 1 }}>
                {current.assignee_ids.map((userId) => (
                  <View key={userId} style={{ flexDirection: 'row', alignItems: 'center', gap: spacing[2] }}>
                    <Avatar id={userId} name={nameOf(userId)} size={20} />
                    <Text variant="label" numberOfLines={1}>
                      {nameOf(userId)}
                    </Text>
                  </View>
                ))}
              </View>
            )}
          </Property>
          <Property label="Due date" last>
            {current.due_at ? (
              <Text variant="label" color={dueLabel(current.due_at, new Date()) === 'Overdue' ? 'destructive' : 'default'}>
                {new Date(current.due_at).toLocaleDateString(undefined, { weekday: 'short', month: 'short', day: 'numeric', year: 'numeric' })}
              </Text>
            ) : (
              <Text variant="label" color="muted">
                None
              </Text>
            )}
          </Property>
        </View>

        {description ? (
          <Text selectable style={{ lineHeight: 22 }}>
            {description}
          </Text>
        ) : (
          <Text color="muted">No description.</Text>
        )}

        <View style={{ gap: spacing[3] }}>
          <Text variant="heading">Comments</Text>
          {comments.isError ? (
            <Pressable onPress={() => void comments.refetch()}>
              <Text color="destructive">Could not load comments. Tap to retry.</Text>
            </Pressable>
          ) : !comments.data ? (
            <LoadingView />
          ) : comments.data.length === 0 ? (
            <Text color="muted">No comments yet.</Text>
          ) : (
            comments.data.map((comment) => (
              <View key={comment.id} style={{ flexDirection: 'row', gap: spacing[2.5] }}>
                <Avatar id={comment.author_id} name={nameOf(comment.author_id)} size={28} />
                <View style={{ flex: 1, gap: 2 }}>
                  <View style={{ flexDirection: 'row', alignItems: 'baseline', gap: spacing[2] }}>
                    <Text variant="label">{nameOf(comment.author_id)}</Text>
                    <Text variant="caption" color="muted">
                      {shortTime(comment.created_at)}
                    </Text>
                  </View>
                  <Text selectable>{markdownToText(comment.body)}</Text>
                </View>
              </View>
            ))
          )}
        </View>
      </ScrollView>
      <Composer value={draft} onChangeText={setDraft} placeholder="Add a comment" sending={sendComment.isPending} onSend={() => sendComment.mutate(draft.trim())} />
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
    </KeyboardAvoidingView>
  )
}

function Property({ label, children, onPress, last = false }: { label: string; children: ReactNode; onPress?: () => void; last?: boolean }) {
  const { colors, spacing } = useTheme()
  return (
    <Pressable
      accessibilityRole={onPress ? 'button' : undefined}
      accessibilityLabel={onPress ? `Change ${label.toLowerCase()}` : undefined}
      disabled={!onPress}
      onPress={onPress}
      style={({ pressed }) => ({
        flexDirection: 'row',
        alignItems: 'center',
        minHeight: 44,
        paddingHorizontal: spacing[3],
        paddingVertical: spacing[2],
        gap: spacing[3],
        backgroundColor: pressed ? colors.accent : colors.card,
        borderBottomWidth: last ? 0 : 1,
        borderBottomColor: colors.border,
      })}
    >
      <Text variant="caption" color="muted" style={{ width: 72 }}>
        {label}
      </Text>
      <View style={{ flex: 1, flexDirection: 'row', alignItems: 'center', gap: spacing[2] }}>{children}</View>
    </Pressable>
  )
}
