/** The team agent's queries and writes for the signed-in person (the web's `features/agent/api.ts`). */
import { useInfiniteQuery, useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import {
  deleteAgentMemory,
  disconnectChatgpt,
  forgetAgentMemorySummary,
  readAgentMemories,
  readAgentMemoryWake,
  readAgentModel,
  readAgentProfiles,
  readTeamInstructions,
  saveAgentModel,
  saveTeamInstructions,
  readAgentStatus,
  startChatgptLogin,
  type AgentMemoryScope,
} from '@orbit/api/agent'

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

const memoryKey = (workspaceId: string) => ['agent-memory', workspaceId] as const

/** The workspace's agents (the default one and the others), for the memory's agent picker. */
export function useAgentProfiles() {
  const { client, workspaceId } = useOrbit()
  return useQuery({ queryKey: ['agent', 'profiles', workspaceId], queryFn: () => readAgentProfiles(client, workspaceId) })
}

/** One agent's memory (`agent` null: the default agent) in the current workspace, newest first; older pages load with `before`. */
export function useAgentMemories(agent: string | null, scope: AgentMemoryScope, query: string) {
  const { client, workspaceId } = useOrbit()
  return useInfiniteQuery({
    queryKey: [...memoryKey(workspaceId), agent, scope, 'list', query],
    queryFn: ({ pageParam }) => readAgentMemories(client, workspaceId, scope, { query, before: pageParam, agent }),
    initialPageParam: null as number | null,
    getNextPageParam: (page) => page.next_before,
  })
}

/** What a run of the agent reads from this memory. */
export function useAgentMemoryWake(agent: string | null, scope: AgentMemoryScope) {
  const { client, workspaceId } = useOrbit()
  return useQuery({ queryKey: [...memoryKey(workspaceId), agent, scope, 'wake'], queryFn: () => readAgentMemoryWake(client, workspaceId, scope, agent) })
}

export function useDeleteAgentMemory(agent: string | null, scope: AgentMemoryScope) {
  const { client, workspaceId } = useOrbit()
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (position: number) => deleteAgentMemory(client, workspaceId, scope, position, agent),
    onSettled: () => void queryClient.invalidateQueries({ queryKey: memoryKey(workspaceId) }),
  })
}

export function useForgetAgentMemorySummary(agent: string | null, scope: AgentMemoryScope) {
  const { client, workspaceId } = useOrbit()
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: ({ lo, hi }: { lo: number; hi: number }) => forgetAgentMemorySummary(client, workspaceId, scope, lo, hi, agent),
    onSettled: () => void queryClient.invalidateQueries({ queryKey: memoryKey(workspaceId) }),
  })
}

const modelKey = ['agent', 'model'] as const

/** Admin → AI agent: the model and thinking level. Installation admins only. */
export function useAgentModel() {
  const { client } = useOrbit()
  return useQuery({ queryKey: modelKey, queryFn: () => readAgentModel(client) })
}

export function useSaveAgentModel() {
  const { client } = useOrbit()
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (choice: { model: string | null; thinking: string | null }) => saveAgentModel(client, choice),
    onSuccess: (saved) => queryClient.setQueryData(modelKey, saved),
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
