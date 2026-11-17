/** The person's scheduled messages and reminders: the list, Send later, and changes that show at once. */
import { deleteScheduled, listScheduled, scheduleMessage, updateScheduled } from '@orbit/api/scheduled'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'

import type { ScheduledMessage } from '../data/scheduled'
import { useOrbit } from '../orbit-context'
import { showError } from './chat'

/** The web's key, so both apps name the same list. */
const scheduledKey = (workspaceId: string) => ['workspace', workspaceId, 'scheduled-messages'] as const

/** A sent one leaves the list, so it refreshes each minute while there are any. */
export function useScheduledMessages(enabled = true) {
  const { client, workspaceId } = useOrbit()
  return useQuery({
    queryKey: scheduledKey(workspaceId),
    queryFn: () => listScheduled(client, workspaceId),
    enabled,
    refetchInterval: (query) => ((query.state.data?.length ?? 0) > 0 ? 60_000 : false),
  })
}

export function useScheduleMessage() {
  const { client, workspaceId } = useOrbit()
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: ({ conversationId, body, sendAt }: { conversationId: string; body: string; sendAt: Date }) => scheduleMessage(client, workspaceId, conversationId, body, sendAt.toISOString()),
    onSuccess: (message) => queryClient.setQueryData<ScheduledMessage[]>(scheduledKey(workspaceId), (list) => (list ? [...list.filter((item) => item.id !== message.id), message] : list)),
    onSettled: () => void queryClient.invalidateQueries({ queryKey: scheduledKey(workspaceId) }),
  })
}

/**
 * Edits and cancels show in the list at once and come back when the server refuses them. A change of the time makes
 * a failed message wait again.
 */
function useOptimistic<T>(write: (input: T) => Promise<unknown>, change: (list: ScheduledMessage[], input: T) => ScheduledMessage[], failure: string) {
  const { workspaceId } = useOrbit()
  const queryClient = useQueryClient()
  const key = scheduledKey(workspaceId)
  return useMutation({
    mutationFn: write,
    onMutate: async (input: T) => {
      await queryClient.cancelQueries({ queryKey: key })
      const before = queryClient.getQueryData<ScheduledMessage[]>(key)
      if (before) queryClient.setQueryData(key, change(before, input))
      return { before }
    },
    onError: (error, _input, context) => {
      if (context?.before) queryClient.setQueryData(key, context.before)
      showError(failure, error)
    },
    onSettled: () => void queryClient.invalidateQueries({ queryKey: key }),
  })
}

export function useUpdateScheduled() {
  const { client, workspaceId } = useOrbit()
  return useOptimistic(
    ({ id, body, sendAt }: { id: string; body?: string; sendAt?: Date }) => updateScheduled(client, workspaceId, id, { body, send_at: sendAt?.toISOString() }),
    (list, { id, body, sendAt }) =>
      list.map((item) => (item.id === id ? { ...item, ...(body !== undefined ? { body } : {}), ...(sendAt ? { send_at: sendAt.getTime(), status: 'scheduled' as const, error: null } : {}) } : item)),
    'Could not change the message',
  )
}

export function useDeleteScheduled() {
  const { client, workspaceId } = useOrbit()
  return useOptimistic((id: string) => deleteScheduled(client, workspaceId, id), (list, id) => list.filter((item) => item.id !== id), 'Could not cancel the message')
}
