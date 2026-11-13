import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { getHomeLayout, putHomeLayout } from '@orbit/api/generated/sdk.gen'
import type { HomeLayout } from '@orbit/api/generated/types.gen'
import { apiClient } from '@/api/client'
import type { HomeCardEntry } from './layout'

const homeLayoutKey = (workspaceId: string) => ['workspace', workspaceId, 'home-layout'] as const
const saveKey = ['home-layout', 'save'] as const

/** The person's Home cards (one layout per user, shared with the iPhone app), as the server filters them for this workspace. */
export function useHomeLayout(workspaceId: string) {
  return useQuery({
    queryKey: homeLayoutKey(workspaceId),
    queryFn: async () => {
      const { data } = await getHomeLayout({ client: apiClient, path: { workspace_id: workspaceId }, throwOnError: true })
      if (!data) throw new Error('Home layout response was empty.')
      return data
    },
  })
}

/** Saves the cards at once: Home shows the change before the answer and rolls back on error. */
export function useSaveHomeLayout(workspaceId: string) {
  const queryClient = useQueryClient()
  const key = homeLayoutKey(workspaceId)
  return useMutation<HomeLayout, Error, HomeCardEntry[], { previous?: HomeLayout }>({
    mutationKey: saveKey,
    mutationFn: async (cards) => {
      const { data } = await putHomeLayout({ client: apiClient, path: { workspace_id: workspaceId }, body: { cards }, throwOnError: true })
      if (!data) throw new Error('Home layout response was empty.')
      return data
    },
    onMutate: async (cards) => {
      await queryClient.cancelQueries({ queryKey: key, exact: true })
      const previous = queryClient.getQueryData<HomeLayout>(key)
      if (previous) queryClient.setQueryData<HomeLayout>(key, { ...previous, cards, customized: true })
      return { previous }
    },
    onError: (_error, _cards, snapshot) => queryClient.setQueryData(key, snapshot?.previous),
    // only the last of several quick saves refetches, so an older answer cannot undo a newer change
    onSettled: () => {
      if (queryClient.isMutating({ mutationKey: saveKey }) <= 1) void queryClient.invalidateQueries({ queryKey: key, exact: true })
    },
  })
}
