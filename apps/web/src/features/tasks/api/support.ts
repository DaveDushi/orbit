import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { apiClient } from '@/api/client'
import {
  createSupportMailbox, createSupportReply, deleteSupportMailbox, getSupportMailbox, getTaskSupport, retrySupportMessage, rotateSupportMailbox,
  testSupportMailbox, updateSupportMailbox,
} from '@orbit/api/generated/sdk.gen'
import type { SupportMailboxBody, SupportMailboxResponse, SupportMessage, SupportSmtpTest, SupportTicket } from '@orbit/api/generated/types.gen'
import { queryKeys } from '@/api/queryKeys'

export type { SupportMailbox, SupportMessage, SupportTicket } from '@orbit/api/generated/types.gen'

// Under the workspace key: every write (an email in, a reply sent) refreshes them through the change feed.
const ticketKey = (workspaceId: string, taskId: string) => [...queryKeys.workspace(workspaceId), 'support', 'task', taskId] as const
const mailboxKey = (workspaceId: string, projectId: string) => [...queryKeys.workspace(workspaceId), 'support', 'mailbox', projectId] as const

/** The emails of a task, and its customer when it is a support ticket. */
export function useTaskSupport(workspaceId: string, taskId: string | undefined) {
  return useQuery({
    queryKey: ticketKey(workspaceId, taskId ?? ''),
    enabled: Boolean(taskId),
    queryFn: async (): Promise<SupportTicket> => {
      const { data } = await getTaskSupport({ client: apiClient, path: { workspace_id: workspaceId, task_id: taskId! }, throwOnError: true })
      if (!data) throw new Error('Support response was empty.')
      return data
    },
  })
}

/** Emails a reply to the ticket's customer (plain text). */
export function useSendSupportReply(workspaceId: string, taskId: string) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async (body: string): Promise<SupportMessage> => {
      const { data } = await createSupportReply({ client: apiClient, path: { workspace_id: workspaceId, task_id: taskId }, body: { body }, throwOnError: true })
      if (!data) throw new Error('Reply response was empty.')
      return data
    },
    onSuccess: (message) => {
      queryClient.setQueryData<SupportTicket>(ticketKey(workspaceId, taskId), (current) =>
        current ? { ...current, messages: [...current.messages, message] } : current)
      // the reply may have moved the ticket to Pending
      void queryClient.invalidateQueries({ queryKey: queryKeys.workspace(workspaceId) })
    },
  })
}

export function useRetrySupportMessage(workspaceId: string, taskId: string) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async (messageId: string): Promise<SupportMessage> => {
      const { data } = await retrySupportMessage({ client: apiClient, path: { workspace_id: workspaceId, message_id: messageId }, throwOnError: true })
      if (!data) throw new Error('Retry response was empty.')
      return data
    },
    onSuccess: (message) =>
      queryClient.setQueryData<SupportTicket>(ticketKey(workspaceId, taskId), (current) =>
        current ? { ...current, messages: current.messages.map((item) => (item.id === message.id ? message : item)) } : current),
  })
}

/** The project's mailbox (owners and admins). */
export function useSupportMailbox(workspaceId: string, projectId: string, enabled = true) {
  return useQuery({
    queryKey: mailboxKey(workspaceId, projectId),
    enabled,
    queryFn: async (): Promise<SupportMailboxResponse> => {
      const { data } = await getSupportMailbox({ client: apiClient, path: { workspace_id: workspaceId, project_id: projectId }, throwOnError: true })
      if (!data) throw new Error('Mailbox response was empty.')
      return data
    },
  })
}

/** Saves the mailbox: creates it when the project has none. A create or rotate answers with the inbound URL, shown once. */
export function useSaveSupportMailbox(workspaceId: string, projectId: string, exists: boolean) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async (body: SupportMailboxBody): Promise<SupportMailboxResponse> => {
      const path = { workspace_id: workspaceId, project_id: projectId }
      const { data } = exists
        ? await updateSupportMailbox({ client: apiClient, path, body, throwOnError: true })
        : await createSupportMailbox({ client: apiClient, path, body, throwOnError: true })
      if (!data) throw new Error('Mailbox response was empty.')
      return data
    },
    onSuccess: (data) => {
      queryClient.setQueryData<SupportMailboxResponse>(mailboxKey(workspaceId, projectId), { mailbox: data.mailbox })
      // a new mailbox can add the support statuses to the project
      void queryClient.invalidateQueries({ queryKey: queryKeys.workspace(workspaceId) })
    },
  })
}

export function useRotateSupportMailbox(workspaceId: string, projectId: string) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async (): Promise<SupportMailboxResponse> => {
      const { data } = await rotateSupportMailbox({ client: apiClient, path: { workspace_id: workspaceId, project_id: projectId }, throwOnError: true })
      if (!data) throw new Error('Mailbox response was empty.')
      return data
    },
    onSuccess: (data) => queryClient.setQueryData<SupportMailboxResponse>(mailboxKey(workspaceId, projectId), { mailbox: data.mailbox }),
  })
}

export function useDeleteSupportMailbox(workspaceId: string, projectId: string) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async () => {
      await deleteSupportMailbox({ client: apiClient, path: { workspace_id: workspaceId, project_id: projectId }, throwOnError: true })
    },
    onSuccess: () => queryClient.setQueryData<SupportMailboxResponse>(mailboxKey(workspaceId, projectId), { mailbox: null }),
  })
}

/** Signs in to the mailbox's saved SMTP server without sending: `ok`, or what failed and how to fix it. */
export function useTestSupportMailbox(workspaceId: string, projectId: string) {
  return useMutation({
    mutationFn: async (): Promise<SupportSmtpTest> => {
      const { data } = await testSupportMailbox({ client: apiClient, path: { workspace_id: workspaceId, project_id: projectId }, throwOnError: true })
      if (!data) throw new Error('Test response was empty.')
      return data
    },
  })
}
