import { daysUntilPurge, filterTrash, trashIdentifier } from '@orbit/api/trash'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { router } from 'expo-router'
import { useState } from 'react'
import { Alert, Pressable, RefreshControl, ScrollView, View } from 'react-native'
import { TextInput } from '../../../ui/scaled'

import { ActionSheet, ConfirmDialog, type ConfirmRequest, type SheetAction } from '../../../components/settings/Dialog'
import { keys } from '../../../data/keys'
import type { ProjectRecord, TaskRecord } from '../../../data/orbit'
import * as trashApi from '../../../data/task-trash'
import { useOrbit } from '../../../orbit-context'
import { useProjects } from '../../../queries'
import { errorMessage } from '../../../session-context'
import { alpha, useTheme } from '../../../theme'
import { Button, EmptyState, ErrorView, Icon, IconButton, LoadingView, PaneHeader, SafeAreaView, SwipeRow, Text } from '../../../ui'

type Entry = { kind: 'project'; project: ProjectRecord } | { kind: 'task'; task: TaskRecord }

/**
 * The web's task Trash: trashed projects (with their tasks), then trashed tasks. Swipe right restores; swipe left (for
 * owners and admins, the server's `pages.purge`) deletes forever after a confirmation; a tap has both.
 */
export default function TaskTrashScreen() {
  const { colors } = useTheme()
  const { client, workspaceId, workspace } = useOrbit()
  const queryClient = useQueryClient()
  const trashKey = keys.taskTrash(workspaceId)
  const trash = useQuery({ queryKey: trashKey, queryFn: () => trashApi.taskTrash(client, workspaceId) })
  const projects = useProjects()
  const [search, setSearch] = useState('')
  const [menu, setMenu] = useState<{ title: string; actions: SheetAction[] } | null>(null)
  const [confirm, setConfirm] = useState<ConfirmRequest | null>(null)
  const canPurge = workspace.permissions.includes('pages.purge')
  const muted = alpha(colors.mutedForeground, 0.7)

  /** Takes the entry off the list at once; an error puts it back. Either way the task lists and projects refresh. */
  const write = useMutation({
    mutationFn: async ({ entry, action }: { entry: Entry; action: 'restore' | 'purge' }) => {
      if (entry.kind === 'task') await (action === 'restore' ? trashApi.restoreTrashedTask(client, workspaceId, entry.task) : trashApi.purgeTrashedTask(client, workspaceId, entry.task.id))
      else await (action === 'restore' ? trashApi.restoreTrashedProject(client, workspaceId, entry.project) : trashApi.purgeTrashedProject(client, workspaceId, entry.project.id))
    },
    onMutate: async ({ entry }) => {
      await queryClient.cancelQueries({ queryKey: trashKey })
      const previous = queryClient.getQueryData<trashApi.TaskTrash>(trashKey)
      if (previous) queryClient.setQueryData(trashKey, trashApi.withoutEntry(previous, entry.kind, entry.kind === 'task' ? entry.task.id : entry.project.id))
      return previous
    },
    onError: (error, { action }, previous) => {
      if (previous) queryClient.setQueryData(trashKey, previous)
      Alert.alert(action === 'restore' ? 'Could not restore' : 'Could not delete forever', errorMessage(error))
    },
    onSettled: () => {
      void queryClient.invalidateQueries({ queryKey: keys.tasks(workspaceId) })
      void queryClient.invalidateQueries({ queryKey: keys.projects(workspaceId) })
      void queryClient.invalidateQueries({ queryKey: keys.statuses(workspaceId) })
    },
  })
  const empty = useMutation({
    mutationFn: () => trashApi.emptyTrash(client, workspaceId),
    onSuccess: () => queryClient.setQueryData<trashApi.TaskTrash>(trashKey, { projects: [], tasks: [] }),
    onError: (error) => Alert.alert('Could not empty the trash', errorMessage(error)),
    onSettled: () => void queryClient.invalidateQueries({ queryKey: trashKey }),
  })

  const keyOf = (projectId: string) => projects.data?.find((project) => project.id === projectId)?.key
  const nameOf = (entry: Entry) => (entry.kind === 'project' ? entry.project.name : entry.task.title || 'Untitled task')
  const purge = (entry: Entry) =>
    setConfirm({
      title: `Delete “${nameOf(entry)}” forever?`,
      description:
        entry.kind === 'project'
          ? 'The project and all its tasks are deleted permanently. They can’t be restored.'
          : (entry.task.trashed_descendant_count ?? 0) > 0
            ? `The task and its ${trashApi.trashedSubIssuesLabel(entry.task.trashed_descendant_count)?.slice(1)} are deleted permanently. They can’t be restored.`
            : 'The task is deleted permanently. It can’t be restored.',
      confirmLabel: 'Delete forever',
      danger: true,
      onConfirm: () => write.mutate({ entry, action: 'purge' }),
    })
  const entryMenu = (entry: Entry) =>
    setMenu({
      title: nameOf(entry),
      actions: [
        { label: entry.kind === 'project' ? 'Restore project' : 'Restore', icon: 'rotateLeft', onPress: () => write.mutate({ entry, action: 'restore' }) },
        ...(canPurge ? [{ label: 'Delete forever', icon: 'trash' as const, destructive: true, onPress: () => purge(entry) }] : []),
      ],
    })
  const requestEmpty = () => {
    if (!trash.data) return
    const { projects: trashedProjects, tasks } = trash.data
    const parts = [
      trashedProjects.length > 0 ? `${trashedProjects.length} ${trashedProjects.length === 1 ? 'project' : 'projects'} with all their tasks` : null,
      tasks.length > 0 ? `${tasks.length} ${tasks.length === 1 ? 'task' : 'tasks'}` : null,
    ].filter(Boolean)
    setConfirm({ title: 'Empty trash?', description: `${parts.join(' and ')} will be deleted forever. They can’t be restored.`, confirmLabel: 'Empty trash', danger: true, onConfirm: () => empty.mutate() })
  }

  const total = (trash.data?.projects.length ?? 0) + (trash.data?.tasks.length ?? 0)
  const shown = trash.data ? filterTrash(trash.data.projects, trash.data.tasks, search, keyOf) : { projects: [], tasks: [] }
  const entries: Entry[] = [...shown.projects.map((project): Entry => ({ kind: 'project', project })), ...shown.tasks.map((task): Entry => ({ kind: 'task', task }))]

  return (
    <SafeAreaView edges={['top', 'left', 'right']} className="flex-1 bg-background">
      <PaneHeader className="flex-wrap">
        <IconButton icon="back" label="Back to tasks" color={muted} onPress={() => (router.canGoBack() ? router.back() : router.navigate('/tasks'))} />
        <Icon name="trash" size={15} color={colors.mutedForeground} />
        <Text className="font-noto-semibold text-[13px] flex-1">Trash</Text>
        {canPurge && total > 0 ? <Button label="Empty trash" size="sm" variant="ghost" disabled={empty.isPending} onPress={requestEmpty} /> : null}
        {total > 0 ? (
          <View className="h-8.5 mt-1 basis-full flex-row items-center border border-input rounded-lg bg-input/30">
            <View className="w-5.5 items-end">
              <Icon name="search" size={14} color={colors.mutedForeground} />
            </View>
            <TextInput
              accessibilityLabel="Search trash"
              placeholder="Search trash"
              placeholderTextColor={colors.mutedForeground}
              selectionColor={colors.primary}
              value={search}
              onChangeText={setSearch}
              className="flex-1 h-8 pl-2 pr-2.5 font-noto text-[16px] text-foreground"
            />
          </View>
        ) : null}
      </PaneHeader>
      {!trash.data ? (
        trash.error ? (
          <ErrorView error={trash.error} onRetry={() => void trash.refetch()} />
        ) : (
          <LoadingView />
        )
      ) : total === 0 ? (
        <EmptyState icon="trash" title="Trash is empty" description="Deleted projects and tasks show here for 30 days, until you restore them." />
      ) : entries.length === 0 ? (
        <EmptyState icon="search" title="No results" description="No deleted project or task matches your search." />
      ) : (
        <ScrollView contentContainerClassName="pb-6" keyboardShouldPersistTaps="handled" refreshControl={<RefreshControl refreshing={trash.isRefetching} onRefresh={() => void trash.refetch()} tintColor={colors.mutedForeground} />}>
          {entries.map((entry) => {
            const deletedAt = entry.kind === 'project' ? entry.project.deleted_at : entry.task.deleted_at
            const left = deletedAt ? daysUntilPurge(deletedAt) : null
            const detail = [
              entry.kind === 'project' ? 'Project with all its tasks' : (trashIdentifier(entry.task, keyOf) ?? null),
              entry.kind === 'task' ? trashApi.trashedSubIssuesLabel(entry.task.trashed_descendant_count) : null,
              left === null ? null : left === 0 ? 'deleted today' : `deleted forever in ${left} ${left === 1 ? 'day' : 'days'}`,
            ]
              .filter(Boolean)
              .join(' · ')
            return (
              <SwipeRow
                key={`${entry.kind}:${entry.kind === 'task' ? entry.task.id : entry.project.id}`}
                left={{ label: 'Restore', icon: 'rotateLeft', tone: 'primary', onSwipe: () => write.mutate({ entry, action: 'restore' }) }}
                right={canPurge ? { label: 'Delete', icon: 'trash', tone: 'destructive', onSwipe: () => purge(entry) } : undefined}
              >
                <Pressable
                  accessibilityRole="button"
                  accessibilityLabel={`${entry.kind === 'project' ? 'Project ' : ''}${nameOf(entry)}`}
                  accessibilityHint="Opens restore and delete"
                  onPress={() => entryMenu(entry)}
                  onLongPress={() => entryMenu(entry)}
                  className="min-h-14 flex-row items-center gap-3 px-4 py-2 border-b border-border active:bg-muted"
                >
                  <Icon name={entry.kind === 'project' ? 'folder' : 'tasks'} size={16} color={entry.kind === 'project' ? entry.project.color : colors.mutedForeground} />
                  <View className="flex-1 min-w-0">
                    <Text numberOfLines={1} className="font-noto-medium text-[14px]">
                      {nameOf(entry)}
                    </Text>
                    <Text numberOfLines={1} className="text-[12px] text-muted-foreground">
                      {detail}
                    </Text>
                  </View>
                  <IconButton icon="rotateLeft" label={`Restore ${nameOf(entry)}`} size={32} color={colors.mutedForeground} onPress={() => write.mutate({ entry, action: 'restore' })} />
                </Pressable>
              </SwipeRow>
            )
          })}
        </ScrollView>
      )}
      <ActionSheet visible={menu !== null} title={menu?.title} actions={menu?.actions ?? []} onClose={() => setMenu(null)} />
      <ConfirmDialog request={confirm} onClose={() => setConfirm(null)} />
    </SafeAreaView>
  )
}
