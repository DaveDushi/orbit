/** Saved views, a filter's tasks, and the task writes of a list row (long press and swipe). */
import { keepPreviousData, type QueryClient, useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Alert } from 'react-native'

import { keys } from '../data/keys'
import * as api from '../data/orbit'
import { isVersionConflict, optimisticTask, replaceTask, type TaskPatch } from '../data/tasks'
import * as views from '../data/views'
import { useOrbit } from '../orbit-context'
import { errorMessage } from '../session-context'

export function useViews() {
  const { client, workspaceId } = useOrbit()
  return useQuery({ queryKey: keys.views(workspaceId), queryFn: () => views.views(client, workspaceId) })
}

/** One view; the list's copy shows at once while it loads. */
export function useView(viewId: string) {
  const { client, workspaceId } = useOrbit()
  const queryClient = useQueryClient()
  return useQuery({
    queryKey: keys.view(workspaceId, viewId),
    queryFn: () => views.view(client, workspaceId, viewId),
    placeholderData: () => queryClient.getQueryData<views.SavedView[]>(keys.views(workspaceId))?.find((view) => view.id === viewId),
  })
}

/** Every task of a state's filter, in the server's order. The previous result stays while a changed filter loads. */
export function useViewTasks(state: views.ViewState, enabled = true) {
  const { client, workspaceId } = useOrbit()
  const body = views.taskQueryBody(state)
  return useQuery({
    queryKey: keys.taskQuery(workspaceId, JSON.stringify(body)),
    queryFn: () => views.queryAllTasks(client, workspaceId, body),
    enabled,
    placeholderData: keepPreviousData,
  })
}

/** New, save, delete and star of views. The star shows at once and goes back when the server refuses it. */
export function useViewWrites() {
  const { client, workspaceId } = useOrbit()
  const queryClient = useQueryClient()
  const listKey = keys.views(workspaceId)
  const stored = (view: views.SavedView) => {
    queryClient.setQueryData(keys.view(workspaceId, view.id), view)
    queryClient.setQueryData<views.SavedView[]>(listKey, (list) => (list ? (list.some((item) => item.id === view.id) ? list.map((item) => (item.id === view.id ? view : item)) : [...list, view]) : list))
    void queryClient.invalidateQueries({ queryKey: listKey, exact: true })
  }
  return {
    create: useMutation({
      mutationFn: ({ fields, state }: { fields: views.ViewFields; state: views.ViewState }) => views.newView(client, workspaceId, fields, state),
      onSuccess: stored,
    }),
    save: useMutation({
      mutationFn: ({ view, changes }: { view: views.SavedView; changes: Partial<views.ViewFields> & { state?: views.ViewState } }) => views.saveView(client, workspaceId, view, changes),
      onSuccess: stored,
    }),
    remove: useMutation({
      mutationFn: (viewId: string) => views.removeView(client, workspaceId, viewId),
      onMutate: async (viewId) => {
        await queryClient.cancelQueries({ queryKey: listKey, exact: true })
        const previous = queryClient.getQueryData<views.SavedView[]>(listKey)
        queryClient.setQueryData<views.SavedView[]>(listKey, (list) => list?.filter((view) => view.id !== viewId))
        return previous
      },
      onError: (error, _viewId, previous) => {
        if (previous) queryClient.setQueryData(listKey, previous)
        Alert.alert('Could not delete the view', errorMessage(error))
      },
      onSuccess: (_result, viewId) => queryClient.removeQueries({ queryKey: keys.view(workspaceId, viewId), exact: true }),
      onSettled: () => void queryClient.invalidateQueries({ queryKey: listKey, exact: true }),
    }),
    favorite: useMutation({
      mutationFn: ({ viewId, favorite }: { viewId: string; favorite: boolean }) => views.setViewFavorite(client, workspaceId, viewId, favorite),
      onMutate: async ({ viewId, favorite }) => {
        await queryClient.cancelQueries({ queryKey: listKey })
        const previous = queryClient.getQueryData<views.SavedView[]>(listKey)
        const previousView = queryClient.getQueryData<views.SavedView>(keys.view(workspaceId, viewId))
        queryClient.setQueryData<views.SavedView[]>(listKey, (list) => list && views.withFavorite(list, viewId, favorite))
        queryClient.setQueryData<views.SavedView>(keys.view(workspaceId, viewId), (view) => view && views.withFavorite([view], viewId, favorite)[0])
        return { previous, previousView }
      },
      onError: (error, { viewId }, snapshot) => {
        if (snapshot?.previous) queryClient.setQueryData(listKey, snapshot.previous)
        if (snapshot?.previousView) queryClient.setQueryData(keys.view(workspaceId, viewId), snapshot.previousView)
        Alert.alert('Could not change the favorite', errorMessage(error))
      },
      onSettled: () => void queryClient.invalidateQueries({ queryKey: listKey }),
    }),
  }
}

type Snapshot = [readonly unknown[], unknown][]

/** Puts `task` into every cached task list (all tasks, presets, filters, sub-issues), and returns what was there. */
function showTask(queryClient: QueryClient, workspaceId: string, task: api.TaskRecord): Snapshot {
  const snapshot = queryClient.getQueriesData<unknown>({ queryKey: keys.tasks(workspaceId) })
  for (const [key, data] of snapshot) {
    if (Array.isArray(data) && data.some((item: { id?: string }) => item?.id === task.id)) queryClient.setQueryData(key, replaceTask(data as api.TaskRecord[], task))
  }
  queryClient.setQueryData<api.TaskRecord>(keys.task(workspaceId, task.id), (current) => (current ? { ...current, ...task } : current))
  return snapshot
}

/**
 * A property change or a trash from a list row. The row changes at once in every list; an error puts the lists back
 * and says why (a version conflict: someone else changed the task).
 */
export function useTaskRowWrites() {
  const { client, workspaceId } = useOrbit()
  const queryClient = useQueryClient()
  const restore = (snapshot: Snapshot | undefined) => snapshot?.forEach(([key, data]) => queryClient.setQueryData(key, data))
  const refresh = () => {
    void queryClient.invalidateQueries({ queryKey: keys.tasks(workspaceId) })
    void queryClient.invalidateQueries({ queryKey: ['task', workspaceId] })
  }
  return {
    change: useMutation({
      mutationFn: ({ task, patch }: { task: api.TaskRecord; patch: TaskPatch }) => api.changeTask(client, workspaceId, task.id, { ...patch, expected_version: task.version }),
      onMutate: async ({ task, patch }) => {
        await queryClient.cancelQueries({ queryKey: keys.tasks(workspaceId) })
        return showTask(queryClient, workspaceId, optimisticTask(task, patch))
      },
      onError: (error, _input, snapshot) => {
        restore(snapshot)
        if (isVersionConflict(error)) Alert.alert('Task changed', 'Someone else changed this task. Showing the latest version; try again.')
        else Alert.alert('Task update failed', errorMessage(error))
      },
      onSuccess: (updated) => void showTask(queryClient, workspaceId, updated),
      onSettled: refresh,
    }),
    trash: useMutation({
      mutationFn: (task: api.TaskRecord) => api.trashTask(client, workspaceId, task),
      onMutate: async (task) => {
        await queryClient.cancelQueries({ queryKey: keys.tasks(workspaceId) })
        const snapshot = queryClient.getQueriesData<unknown>({ queryKey: keys.tasks(workspaceId) })
        for (const [key, data] of snapshot) {
          if (Array.isArray(data)) queryClient.setQueryData(key, (data as api.TaskRecord[]).filter((item) => item?.id !== task.id))
        }
        return snapshot
      },
      onError: (error, _task, snapshot) => {
        restore(snapshot)
        Alert.alert('Task deletion failed', errorMessage(error))
      },
      onSettled: refresh,
    }),
  }
}
