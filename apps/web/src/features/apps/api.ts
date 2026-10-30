import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import {
  createManagedApp,
  deleteManagedApp,
  readApps,
  readManagedApps,
  rotateManagedAppSecret,
  updateManagedApp,
  type ManagedAppInput,
} from '@orbit/api/apps'
import { apiClient } from '@/api/client'

const visibleKey = (workspaceId: string) => ['apps', workspaceId] as const
const managedKey = (workspaceId: string) => ['workspace-apps', workspaceId] as const

/** The apps this member may open. */
export function useApps(workspaceId: string) {
  return useQuery({ queryKey: visibleKey(workspaceId), queryFn: () => readApps(apiClient, workspaceId), enabled: workspaceId !== '' })
}

export function useManagedApps(workspaceId: string, enabled: boolean) {
  return useQuery({ queryKey: managedKey(workspaceId), queryFn: () => readManagedApps(apiClient, workspaceId), enabled })
}

function useRefresh(workspaceId: string) {
  const queryClient = useQueryClient()
  return () => {
    void queryClient.invalidateQueries({ queryKey: managedKey(workspaceId) })
    void queryClient.invalidateQueries({ queryKey: visibleKey(workspaceId) })
  }
}

export function useCreateManagedApp(workspaceId: string) {
  const onSettled = useRefresh(workspaceId)
  return useMutation({ mutationFn: (input: ManagedAppInput) => createManagedApp(apiClient, workspaceId, input), onSettled })
}

export function useUpdateManagedApp(workspaceId: string) {
  const onSettled = useRefresh(workspaceId)
  return useMutation({ mutationFn: ({ id, input }: { id: string; input: ManagedAppInput }) => updateManagedApp(apiClient, workspaceId, id, input), onSettled })
}

export function useDeleteManagedApp(workspaceId: string) {
  const onSettled = useRefresh(workspaceId)
  return useMutation({ mutationFn: (id: string) => deleteManagedApp(apiClient, workspaceId, id), onSettled })
}

export function useRotateManagedAppSecret(workspaceId: string) {
  const onSettled = useRefresh(workspaceId)
  return useMutation({ mutationFn: (id: string) => rotateManagedAppSecret(apiClient, workspaceId, id), onSettled })
}
