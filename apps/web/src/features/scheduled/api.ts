import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import {
  deleteRoutine,
  deleteScheduled,
  listRoutines,
  listScheduled,
  scheduleMessage,
  updateRoutine,
  updateScheduled,
  type RoutineChange,
} from '@orbit/api/scheduled'
import { apiClient } from '@/api/client'

const routinesKey = (workspaceId: string) => ['workspace', workspaceId, 'routines'] as const
const scheduledKey = (workspaceId: string) => ['workspace', workspaceId, 'scheduled-messages'] as const

/** The person's routines; every routine of the workspace for owners and admins. A run moves the times, so it refreshes each minute. */
export function useRoutines(workspaceId: string) {
  return useQuery({ queryKey: routinesKey(workspaceId), queryFn: () => listRoutines(apiClient, workspaceId), refetchInterval: 60_000 })
}

export function useUpdateRoutine(workspaceId: string) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: ({ id, change }: { id: string; change: RoutineChange }) => updateRoutine(apiClient, workspaceId, id, change),
    onSettled: () => void queryClient.invalidateQueries({ queryKey: routinesKey(workspaceId) }),
  })
}

export function useDeleteRoutine(workspaceId: string) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (id: string) => deleteRoutine(apiClient, workspaceId, id),
    onSettled: () => void queryClient.invalidateQueries({ queryKey: routinesKey(workspaceId) }),
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
