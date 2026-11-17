/** "Suggest a change" and Admin → Feedback & fixes, for installation admins (others get 403, so they never ask). */
import { listImprovements, readImprovementSettings, saveImprovementSettings, sendImprovement, withdrawImprovement, type ImprovementContext, type ImprovementRequest } from '@orbit/api/improvements'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'

import { useOrbit } from '../orbit-context'

const improvementsKey = ['admin', 'improvements'] as const
const settingsKey = [...improvementsKey, 'settings'] as const
const listKey = [...improvementsKey, 'list'] as const

export function useImprovementSettings() {
  const { client, user } = useOrbit()
  return useQuery({ queryKey: settingsKey, queryFn: () => readImprovementSettings(client), enabled: user.installation_admin, staleTime: 60_000 })
}

/** Whether this person sees "Suggest a change" and the requests: an installation admin, with the switch on. */
export function useImprovementsEnabled(): boolean {
  const { user } = useOrbit()
  return user.installation_admin && (useImprovementSettings().data?.enabled ?? false)
}

export function useSaveImprovementSettings() {
  const { client } = useOrbit()
  const queryClient = useQueryClient()
  return useMutation({ mutationFn: (enabled: boolean) => saveImprovementSettings(client, enabled), onSuccess: (settings) => queryClient.setQueryData(settingsKey, settings) })
}

export function useImprovements(enabled = true) {
  const { client } = useOrbit()
  return useQuery({ queryKey: listKey, queryFn: () => listImprovements(client), enabled })
}

export function useSendImprovement() {
  const { client } = useOrbit()
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: ({ text, context }: { text: string; context: ImprovementContext }) => sendImprovement(client, text, context),
    onSettled: () => void queryClient.invalidateQueries({ queryKey: listKey }),
  })
}

/** Shows as declined at once; the server's answer replaces it, a refusal puts the request back. */
export function useWithdrawImprovement() {
  const { client } = useOrbit()
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (id: string) => withdrawImprovement(client, id),
    onMutate: async (id: string) => {
      await queryClient.cancelQueries({ queryKey: listKey })
      const before = queryClient.getQueryData<ImprovementRequest[]>(listKey)
      if (before) queryClient.setQueryData(listKey, before.map((item) => (item.id === id ? { ...item, status: 'declined' as const, note: 'Withdrawn' } : item)))
      return { before }
    },
    onError: (_error, _id, context) => {
      if (context?.before) queryClient.setQueryData(listKey, context.before)
    },
    onSettled: () => void queryClient.invalidateQueries({ queryKey: listKey }),
  })
}
