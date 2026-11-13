/** The Home cards' layout and data (the web's `features/home/api.ts` and its cards' queries). */
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { getHomeLayout, putHomeLayout, queryTasks, searchDrive } from '@orbit/api/generated/sdk.gen'
import { useCallback, useMemo, useState } from 'react'

import { latestSaver, taskCardQuery, type HomeCard, type HomeLayout } from '../data/home'
import { keys } from '../data/keys'
import * as api from '../data/orbit'
import { useOrbit } from '../orbit-context'
import { showError } from './chat'

const layoutKey = (workspaceId: string) => ['home-layout', workspaceId] as const

/** The person's Home cards (one layout per user, shared with the web), as the server filters them for this workspace. */
export function useHomeLayout() {
  const { client, workspaceId } = useOrbit()
  return useQuery({
    queryKey: layoutKey(workspaceId),
    queryFn: async () => (await getHomeLayout({ client, path: { workspace_id: workspaceId }, throwOnError: true })).data,
  })
}

/**
 * Saves the whole card list at once: Home shows it before the answer. Saves go one at a time, newest only
 * (`latestSaver`), and the server's answer replaces the list once nothing newer waits. A failure reloads the saved list.
 */
export function useSaveHomeLayout() {
  const { client, workspaceId } = useOrbit()
  const queryClient = useQueryClient()
  const saver = useMemo(
    () => latestSaver(async (cards: HomeCard[]) => (await putHomeLayout({ client, path: { workspace_id: workspaceId }, body: { cards }, throwOnError: true })).data),
    [client, workspaceId],
  )
  return useCallback(
    async (cards: HomeCard[]) => {
      const key = layoutKey(workspaceId)
      void queryClient.cancelQueries({ queryKey: key, exact: true })
      queryClient.setQueryData<HomeLayout>(key, (previous) => previous && { ...previous, cards, customized: true })
      const outcome = await saver.save(cards)
      if (outcome.kind === 'superseded' || saver.pending()) return
      if (outcome.kind === 'saved') queryClient.setQueryData(key, outcome.result)
      else {
        void queryClient.invalidateQueries({ queryKey: key, exact: true })
        showError('Could not save your Home', outcome.error)
      }
    },
    [queryClient, saver, workspaceId],
  )
}

/** A task card's open tasks, under the `tasks` prefix so a task change refreshes it like a task list. */
export function useTaskCard(card: string, limit: number) {
  const { client, workspaceId } = useOrbit()
  const body = taskCardQuery(card, limit)
  return useQuery({
    queryKey: keys.taskView(workspaceId, `home-${card}-${limit}`),
    enabled: body !== null,
    queryFn: async () => (body ? (await queryTasks({ client, path: { workspace_id: workspaceId }, body, throwOnError: true })).data.items : []),
  })
}

/** Files changed in the last 30 days (the web card's Drive search), under the `drive` prefix. */
export function useRecentDriveFiles() {
  const { client, workspaceId } = useOrbit()
  // a whole day, so the key stays the same between renders
  const [since] = useState(() => {
    const today = new Date()
    return new Date(today.getFullYear(), today.getMonth(), today.getDate() - 30).toISOString()
  })
  return useQuery({
    queryKey: [...keys.drive(workspaceId), 'recent', since],
    queryFn: async () =>
      (await searchDrive({ client, path: { workspace_id: workspaceId }, query: { kind: 'file', updated_after: since, limit: 200 }, throwOnError: true })).data.items.map((entry) => entry.node),
  })
}

/** The newest notifications, read or not (the web's Activity card). */
export function useLatestNotifications(limit: number) {
  const { client, workspaceId } = useOrbit()
  return useQuery({
    queryKey: [...keys.notifications(workspaceId), 'latest', String(limit)],
    queryFn: () => api.latestNotifications(client, workspaceId, limit),
  })
}
