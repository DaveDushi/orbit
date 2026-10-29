import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { disconnectChatgpt, readAgentIdentity, readAgentStatus, readTeamInstructions, saveAgentName, saveTeamInstructions, startChatgptLogin } from '@orbit/api/agent'
import { apiClient } from '@/api/client'

const key = ['agent'] as const

/** The agent's state for the signed-in person. Polls while a ChatGPT sign-in waits for its code to be entered. */
export function useAgentStatus() {
  return useQuery({
    queryKey: key,
    queryFn: () => readAgentStatus(apiClient),
    refetchInterval: (query) => {
      const data = query.state.data
      return data?.enabled && data.login?.state === 'pending' && !data.chatgpt.connected ? 3_000 : false
    },
  })
}

export function useStartChatgptLogin() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: () => startChatgptLogin(apiClient),
    onSettled: () => void queryClient.invalidateQueries({ queryKey: key }),
  })
}

export function useDisconnectChatgpt() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: () => disconnectChatgpt(apiClient),
    onSettled: () => void queryClient.invalidateQueries({ queryKey: key }),
  })
}

const identityKey = ['agent', 'identity'] as const

/** Admin → AI agent: the agent's name. Installation admins only. */
export function useAgentIdentity() {
  return useQuery({ queryKey: identityKey, queryFn: () => readAgentIdentity(apiClient) })
}

/** Renames the agent; the status (its name on the account page) and member lists show the new name. */
export function useSaveAgentName() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (name: string) => saveAgentName(apiClient, name),
    onSuccess: (identity) => queryClient.setQueryData(identityKey, identity),
    onSettled: () => void queryClient.invalidateQueries({ queryKey: key }),
  })
}

const teamKey = (workspaceId: string) => ['agent', 'team', workspaceId] as const

/** Workspace settings → AI agent: the team instructions. Every member reads them. */
export function useTeamInstructions(workspaceId: string) {
  return useQuery({ queryKey: teamKey(workspaceId), queryFn: () => readTeamInstructions(apiClient, workspaceId) })
}

export function useSaveTeamInstructions(workspaceId: string) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (instructions: string) => saveTeamInstructions(apiClient, workspaceId, instructions),
    onSuccess: (saved) => queryClient.setQueryData(teamKey(workspaceId), saved),
  })
}
