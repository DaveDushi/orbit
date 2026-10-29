import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import {
  createAgentIntegration,
  deleteAgentIntegration,
  readAgentIntegrations,
  testAgentIntegration,
  updateAgentIntegration,
  type AgentIntegrationInput,
} from '@orbit/api/agentIntegrations'
import { apiClient } from '@/api/client'

const key = (workspaceId: string) => ['agent-integrations', workspaceId] as const

export function useAgentIntegrations(workspaceId: string, enabled: boolean) {
  return useQuery({ queryKey: key(workspaceId), queryFn: () => readAgentIntegrations(apiClient, workspaceId), enabled })
}

export function useCreateAgentIntegration(workspaceId: string) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (input: AgentIntegrationInput) => createAgentIntegration(apiClient, workspaceId, input),
    onSettled: () => void queryClient.invalidateQueries({ queryKey: key(workspaceId) }),
  })
}

export function useUpdateAgentIntegration(workspaceId: string) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: ({ id, input }: { id: string; input: AgentIntegrationInput }) => updateAgentIntegration(apiClient, workspaceId, id, input),
    onSettled: () => void queryClient.invalidateQueries({ queryKey: key(workspaceId) }),
  })
}

export function useDeleteAgentIntegration(workspaceId: string) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (id: string) => deleteAgentIntegration(apiClient, workspaceId, id),
    onSettled: () => void queryClient.invalidateQueries({ queryKey: key(workspaceId) }),
  })
}

export function useTestAgentIntegration(workspaceId: string) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (id: string) => testAgentIntegration(apiClient, workspaceId, id),
    onSettled: () => void queryClient.invalidateQueries({ queryKey: key(workspaceId) }),
  })
}
