/** "Suggest a change" (`@orbit/api/improvements`): the Admin switch, the requests and sending one. Installation admins only. */
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { type ImprovementContext, listImprovements, readImprovementSettings, saveImprovementSettings, sendImprovement, withdrawImprovement } from '@orbit/api/improvements'

import { useOrbit } from '../orbit-context'

const improvementsKey = ['admin', 'improvements'] as const
const settingsKey = [...improvementsKey, 'settings'] as const
const listKey = [...improvementsKey, 'list'] as const

/** Members get 403, so only installation admins ask. */
export function useImprovementSettings() {
  const { client, user } = useOrbit()
  return useQuery({ queryKey: settingsKey, queryFn: () => readImprovementSettings(client), enabled: user.installation_admin, staleTime: 60_000 })
}

/** An installation admin, with the switch on: they see Suggest a change and Feedback & fixes. */
export function useImprovementsEnabled(): boolean {
  const admin = useOrbit().user.installation_admin
  return admin && (useImprovementSettings().data?.enabled ?? false)
}

export function useSaveImprovementSettings() {
  const { client } = useOrbit()
  const queryClient = useQueryClient()
  return useMutation({ mutationFn: (enabled: boolean) => saveImprovementSettings(client, enabled), onSuccess: (settings) => queryClient.setQueryData(settingsKey, settings) })
}

export function useImprovements(enabled: boolean) {
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

export function useWithdrawImprovement() {
  const { client } = useOrbit()
  const queryClient = useQueryClient()
  return useMutation({ mutationFn: (id: string) => withdrawImprovement(client, id), onSettled: () => void queryClient.invalidateQueries({ queryKey: listKey }) })
}
