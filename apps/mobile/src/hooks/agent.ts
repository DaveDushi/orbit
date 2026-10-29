/** The team agent's queries and writes for the signed-in person (the web's `features/agent/api.ts`). */
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { disconnectChatgpt, readAgentIdentity, readAgentStatus, readTeamInstructions, saveAgentName, saveTeamInstructions, startChatgptLogin } from '@orbit/api/agent'

import { useOrbit } from '../orbit-context'

const key = ['agent'] as const

/** Polls while a ChatGPT sign-in waits for its code to be entered. */
export function useAgentStatus() {
  const { client } = useOrbit()
  return useQuery({
    queryKey: key,
    queryFn: () => readAgentStatus(client),
    refetchInterval: (query) => {
      const data = query.state.data
      return data?.enabled && data.login?.state === 'pending' && !data.chatgpt.connected ? 3_000 : false
    },
  })
}

/** The server runs the team agent: its Account settings page is shown. */
export function useAgentEnabled(): boolean {
  return useAgentStatus().data?.enabled === true
}

export function useStartChatgptLogin() {
  const { client } = useOrbit()
  const queryClient = useQueryClient()
  return useMutation({ mutationFn: () => startChatgptLogin(client), onSettled: () => void queryClient.invalidateQueries({ queryKey: key }) })
}

export function useDisconnectChatgpt() {
  const { client } = useOrbit()
  const queryClient = useQueryClient()
  return useMutation({ mutationFn: () => disconnectChatgpt(client), onSettled: () => void queryClient.invalidateQueries({ queryKey: key }) })
}

const identityKey = ['agent', 'identity'] as const

/** Admin → AI agent: the agent's name. Installation admins only. */
export function useAgentIdentity() {
  const { client } = useOrbit()
  return useQuery({ queryKey: identityKey, queryFn: () => readAgentIdentity(client) })
}

export function useSaveAgentName() {
  const { client } = useOrbit()
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (name: string) => saveAgentName(client, name),
    onSuccess: (identity) => queryClient.setQueryData(identityKey, identity),
    onSettled: () => void queryClient.invalidateQueries({ queryKey: key }),
  })
}

const teamKey = (workspaceId: string) => ['agent', 'team', workspaceId] as const

/** Workspace settings → AI agent: the team instructions. Every member reads them. */
export function useTeamInstructions() {
  const { client, workspaceId } = useOrbit()
  return useQuery({ queryKey: teamKey(workspaceId), queryFn: () => readTeamInstructions(client, workspaceId) })
}

export function useSaveTeamInstructions() {
  const { client, workspaceId } = useOrbit()
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (instructions: string) => saveTeamInstructions(client, workspaceId, instructions),
    onSuccess: (saved) => queryClient.setQueryData(teamKey(workspaceId), saved),
  })
}
