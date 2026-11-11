import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import {
  deleteRoutine,
  deleteScheduled,
  deleteScript,
  listRoutineRuns,
  listRoutines,
  listScheduled,
  listScripts,
  scheduleMessage,
  testRoutine,
  updateRoutine,
  updateScheduled,
  updateScript,
  type RoutineChange,
  type ScriptChange,
} from '@orbit/api/scheduled'
import { apiClient } from '@/api/client'

const routinesKey = (workspaceId: string) => ['workspace', workspaceId, 'routines'] as const
const runsKey = (workspaceId: string, routineId: string) => ['workspace', workspaceId, 'routines', routineId, 'runs'] as const
const scriptsKey = (workspaceId: string) => ['workspace', workspaceId, 'scripts'] as const
const scheduledKey = (workspaceId: string) => ['workspace', workspaceId, 'scheduled-messages'] as const

/** The person's routines; every routine of the workspace for owners and admins. A run moves the times, so it refreshes each minute. */
export function useRoutines(workspaceId: string) {
  return useQuery({ queryKey: routinesKey(workspaceId), queryFn: () => listRoutines(apiClient, workspaceId), refetchInterval: 60_000 })
}

export function useUpdateRoutine(workspaceId: string) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: ({ id, change }: { id: string; change: RoutineChange }) => updateRoutine(apiClient, workspaceId, id, change),
    onSettled: () => {
      void queryClient.invalidateQueries({ queryKey: routinesKey(workspaceId) })
      void queryClient.invalidateQueries({ queryKey: scriptsKey(workspaceId) })
    },
  })
}

export function useDeleteRoutine(workspaceId: string) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (id: string) => deleteRoutine(apiClient, workspaceId, id),
    onSettled: () => void queryClient.invalidateQueries({ queryKey: routinesKey(workspaceId) }),
  })
}

/** A routine's last runs. A run that is going ends on its own, so the list refreshes every 5 seconds while one is. */
export function useRoutineRuns(workspaceId: string, routineId: string, enabled: boolean) {
  return useQuery({
    queryKey: runsKey(workspaceId, routineId),
    queryFn: () => listRoutineRuns(apiClient, workspaceId, routineId),
    enabled,
    refetchInterval: (query) => (query.state.data?.some((run) => run.status === 'started') ? 5_000 : false),
  })
}

export function useTestRoutine(workspaceId: string) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (id: string) => testRoutine(apiClient, workspaceId, id),
    onSettled: (_run, _error, id) => void queryClient.invalidateQueries({ queryKey: runsKey(workspaceId, id) }),
  })
}

/** The person's saved scripts; every script of the workspace for owners and admins. */
export function useScripts(workspaceId: string) {
  return useQuery({ queryKey: scriptsKey(workspaceId), queryFn: () => listScripts(apiClient, workspaceId) })
}

/** A new script changes which routines may write: both lists refresh. */
export function useUpdateScript(workspaceId: string) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: ({ id, change }: { id: string; change: ScriptChange }) => updateScript(apiClient, workspaceId, id, change),
    onSettled: () => {
      void queryClient.invalidateQueries({ queryKey: scriptsKey(workspaceId) })
      void queryClient.invalidateQueries({ queryKey: routinesKey(workspaceId), exact: true })
    },
  })
}

export function useDeleteScript(workspaceId: string) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (id: string) => deleteScript(apiClient, workspaceId, id),
    onSettled: () => {
      void queryClient.invalidateQueries({ queryKey: scriptsKey(workspaceId) })
      void queryClient.invalidateQueries({ queryKey: routinesKey(workspaceId), exact: true })
    },
  })
}

/** The person's scheduled messages and reminders. A sent one leaves the list, so it refreshes each minute while there are any. */
export function useScheduledMessages(workspaceId: string, enabled = true) {
  return useQuery({
    queryKey: scheduledKey(workspaceId),
    queryFn: () => listScheduled(apiClient, workspaceId),
    enabled,
    refetchInterval: (query) => ((query.state.data?.length ?? 0) > 0 ? 60_000 : false),
  })
}

export function useScheduleMessage(workspaceId: string) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: ({ conversationId, body, sendAt }: { conversationId: string; body: string; sendAt: Date }) =>
      scheduleMessage(apiClient, workspaceId, conversationId, body, sendAt.toISOString()),
    onSettled: () => void queryClient.invalidateQueries({ queryKey: scheduledKey(workspaceId) }),
  })
}

export function useUpdateScheduled(workspaceId: string) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: ({ id, body, sendAt }: { id: string; body?: string; sendAt?: Date }) =>
      updateScheduled(apiClient, workspaceId, id, { body, send_at: sendAt?.toISOString() }),
    onSettled: () => void queryClient.invalidateQueries({ queryKey: scheduledKey(workspaceId) }),
  })
}

export function useDeleteScheduled(workspaceId: string) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (id: string) => deleteScheduled(apiClient, workspaceId, id),
    onSettled: () => void queryClient.invalidateQueries({ queryKey: scheduledKey(workspaceId) }),
  })
}
