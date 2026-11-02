import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import {
  listImprovements,
  readImprovementSettings,
  saveImprovementSettings,
  sendImprovement,
  withdrawImprovement,
  type ImprovementContext,
} from '@orbit/api/improvements'
import { apiClient } from '@/api/client'
import { queryKeys } from '@/api/queryKeys'
import { useIsInstallationAdmin } from '@/features/workspaces/permissions'

export const improvementsKey = ['admin', 'improvements'] as const
export const improvementSettingsKey = [...improvementsKey, 'settings'] as const

/** The "Suggest a change" switch. Only installation admins ask: members get 403. */
export function useImprovementSettings() {
  const admin = useIsInstallationAdmin()
  return useQuery({
    queryKey: improvementSettingsKey,
    queryFn: () => readImprovementSettings(apiClient),
    enabled: admin,
    staleTime: 60_000,
  })
}

/** Whether this person sees "Suggest a change" and "Feedback & fixes": an installation admin, with the switch on. */
export function useImprovementsEnabled(): boolean {
  const admin = useIsInstallationAdmin()
  const enabled = useImprovementSettings().data?.enabled ?? false
  return admin && enabled
}

export function useSaveImprovementSettings() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (enabled: boolean) => saveImprovementSettings(apiClient, enabled),
    onSuccess: (settings) => queryClient.setQueryData(improvementSettingsKey, settings),
    onSettled: () => queryClient.invalidateQueries({ queryKey: queryKeys.adminAudit }),
  })
}

export function useImprovements() {
  return useQuery({ queryKey: [...improvementsKey, 'list'], queryFn: () => listImprovements(apiClient) })
}

/** What the app knows about where the request comes from. */
export function currentContext(revision: string): ImprovementContext {
  return {
    path: window.location.pathname,
    viewport: `${Math.round(window.innerWidth)}x${Math.round(window.innerHeight)}`,
    revision,
  }
}

export function useSendImprovement() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: ({ text, context }: { text: string; context: ImprovementContext }) => sendImprovement(apiClient, text, context),
    onSettled: () => queryClient.invalidateQueries({ queryKey: [...improvementsKey, 'list'] }),
  })
}

export function useWithdrawImprovement() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (id: string) => withdrawImprovement(apiClient, id),
    onSettled: () => queryClient.invalidateQueries({ queryKey: [...improvementsKey, 'list'] }),
  })
}
