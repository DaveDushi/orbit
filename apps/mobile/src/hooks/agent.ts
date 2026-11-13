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
  createAgentProfile,
  deleteAgentProfile,
  updateAgentProfile,
  updateDefaultAgent,
  type AgentMemoryScope,
  type AgentProfileInput,
  type DefaultAgentInput,
} from '@orbit/api/agent'
import { deleteRoutine, deleteScript, listRoutineRuns, listRoutines, listScripts, testRoutine, updateRoutine, updateScript, type RoutineChange, type ScriptChange } from '@orbit/api/scheduled'

import { setAgentAvatar } from '../data/agentic'
import { keys } from '../data/keys'
import { useOrbit } from '../orbit-context'

const key = ['agent'] as const

/** Polls while a ChatGPT sign-in waits for its code to be entered. `enabled: false` skips it (a guest's Ask Donna). */
export function useAgentStatus(enabled = true) {
  const { client } = useOrbit()
  return useQuery({
    queryKey: key,
    enabled,
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
  return useQuery({ queryKey: profilesKey(workspaceId), queryFn: () => readAgentProfiles(client, workspaceId) })
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

const profilesKey = (workspaceId: string) => ['agent', 'profiles', workspaceId] as const

/** An agent's name and avatar are in the member lists too. */
function useRefreshAgents() {
  const { workspaceId } = useOrbit()
  const queryClient = useQueryClient()
  return () => {
    void queryClient.invalidateQueries({ queryKey: profilesKey(workspaceId) })
    void queryClient.invalidateQueries({ queryKey: keys.members(workspaceId) })
  }
}

/** Creates (`id` null) or changes a custom agent; `avatar` undefined keeps the picture, null removes it. */
export function useSaveAgentProfile() {
  const { client, workspaceId } = useOrbit()
  const refresh = useRefreshAgents()
  return useMutation({
    mutationFn: async ({ id, input, avatar }: { id: string | null; input: AgentProfileInput; avatar?: Blob | null }) => {
      const saved = id ? await updateAgentProfile(client, workspaceId, id, input) : await createAgentProfile(client, workspaceId, input)
      if (avatar !== undefined) await setAgentAvatar(client, workspaceId, saved.id, avatar)
      return saved
    },
    onSettled: refresh,
  })
}

/** The default agent's card. Its name is also in the agent status (Ask Donna). */
export function useSaveDefaultAgent() {
  const { client, workspaceId } = useOrbit()
  const queryClient = useQueryClient()
  const refresh = useRefreshAgents()
  return useMutation({
    mutationFn: async ({ input, avatar }: { input: DefaultAgentInput; avatar?: Blob | null }) => {
      const saved = await updateDefaultAgent(client, workspaceId, input)
      if (avatar !== undefined) await setAgentAvatar(client, workspaceId, 'default', avatar)
      return saved
    },
    onSettled: () => {
      refresh()
      void queryClient.invalidateQueries({ queryKey: key, exact: true })
    },
  })
}

export function useDeleteAgentProfile() {
  const { client, workspaceId } = useOrbit()
  const refresh = useRefreshAgents()
  return useMutation({ mutationFn: (id: string) => deleteAgentProfile(client, workspaceId, id), onSettled: refresh })
}

const routinesKey = (workspaceId: string) => ['workspace', workspaceId, 'routines'] as const
const runsKey = (workspaceId: string, routineId: string) => ['workspace', workspaceId, 'routines', routineId, 'runs'] as const
const scriptsKey = (workspaceId: string) => ['workspace', workspaceId, 'scripts'] as const

/** The person's routines; every routine of the workspace for owners and admins. A run moves the times, so it refreshes each minute. */
export function useRoutines() {
  const { client, workspaceId } = useOrbit()
  return useQuery({ queryKey: routinesKey(workspaceId), queryFn: () => listRoutines(client, workspaceId), refetchInterval: 60_000 })
}

export function useUpdateRoutine() {
  const { client, workspaceId } = useOrbit()
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: ({ id, change }: { id: string; change: RoutineChange }) => updateRoutine(client, workspaceId, id, change),
    onSettled: () => {
      void queryClient.invalidateQueries({ queryKey: routinesKey(workspaceId) })
      void queryClient.invalidateQueries({ queryKey: scriptsKey(workspaceId) })
    },
  })
}

export function useDeleteRoutine() {
  const { client, workspaceId } = useOrbit()
  const queryClient = useQueryClient()
  return useMutation({ mutationFn: (id: string) => deleteRoutine(client, workspaceId, id), onSettled: () => void queryClient.invalidateQueries({ queryKey: routinesKey(workspaceId) }) })
}

/** A routine's last runs; while one runs the list refreshes every 5 seconds. */
export function useRoutineRuns(routineId: string) {
  const { client, workspaceId } = useOrbit()
  return useQuery({
    queryKey: runsKey(workspaceId, routineId),
    queryFn: () => listRoutineRuns(client, workspaceId, routineId),
    refetchInterval: (query) => (query.state.data?.some((run) => run.status === 'started') ? 5_000 : false),
  })
}

export function useTestRoutine() {
  const { client, workspaceId } = useOrbit()
  const queryClient = useQueryClient()
  return useMutation({ mutationFn: (id: string) => testRoutine(client, workspaceId, id), onSettled: (_run, _error, id) => void queryClient.invalidateQueries({ queryKey: runsKey(workspaceId, id) }) })
}

/** The person's saved scripts; every script of the workspace for owners and admins. */
export function useScripts() {
  const { client, workspaceId } = useOrbit()
  return useQuery({ queryKey: scriptsKey(workspaceId), queryFn: () => listScripts(client, workspaceId) })
}

/** A new script changes which routines may write: both lists refresh. */
export function useUpdateScript() {
  const { client, workspaceId } = useOrbit()
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: ({ id, change }: { id: string; change: ScriptChange }) => updateScript(client, workspaceId, id, change),
    onSettled: () => {
      void queryClient.invalidateQueries({ queryKey: scriptsKey(workspaceId) })
      void queryClient.invalidateQueries({ queryKey: routinesKey(workspaceId), exact: true })
    },
  })
}

export function useDeleteScript() {
  const { client, workspaceId } = useOrbit()
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (id: string) => deleteScript(client, workspaceId, id),
    onSettled: () => {
      void queryClient.invalidateQueries({ queryKey: scriptsKey(workspaceId) })
      void queryClient.invalidateQueries({ queryKey: routinesKey(workspaceId), exact: true })
    },
  })
}
