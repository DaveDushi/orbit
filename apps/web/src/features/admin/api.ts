import { keepPreviousData, useInfiniteQuery, useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { apiClient } from '@/api/client'
import { queryKeys } from '@/api/queryKeys'
import { createBackup, createRecoveryLink, getInstanceSettings, listAdminUsers, listBackups, listGlobalAudit, removeSmtp, setInstanceAdmin, saveSmtp, sendTestEmail, setAccountSuspension, setRegistration } from '@/api/generated/sdk.gen'
import type { InstanceSettingsView, SmtpBody } from '@/api/generated/types.gen'

/** Accounts on this instance in cursor pages; `search` matches part of the name or email. Admins only. */
export function useAdminUsers(search: string) {
  return useInfiniteQuery({
    queryKey: [...queryKeys.adminUsers, search],
    initialPageParam: undefined as string | undefined,
    queryFn: async ({ pageParam }) => {
      const { data } = await listAdminUsers({ client: apiClient, throwOnError: true, query: { q: search || undefined, cursor: pageParam } })
      return data
    },
    getNextPageParam: (last) => last.next_cursor ?? undefined,
    select: (data) => data.pages.flatMap((page) => page.items),
    placeholderData: keepPreviousData,
  })
}

export function useSetSuspension() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async ({ userId, suspended }: { userId: string; suspended: boolean }) => {
      await setAccountSuspension({ client: apiClient, throwOnError: true, path: { user_id: userId }, body: { suspended } })
    },
    onSettled: () => Promise.all([
      queryClient.invalidateQueries({ queryKey: queryKeys.adminUsers }),
      queryClient.invalidateQueries({ queryKey: queryKeys.adminAudit }),
    ]),
  })
}

export function useSetInstanceAdmin() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async ({ userId, admin }: { userId: string; admin: boolean }) => {
      await setInstanceAdmin({ client: apiClient, throwOnError: true, path: { user_id: userId }, body: { admin } })
    },
    onSettled: () => Promise.all([
      queryClient.invalidateQueries({ queryKey: queryKeys.adminUsers }),
      queryClient.invalidateQueries({ queryKey: queryKeys.adminAudit }),
    ]),
  })
}

export function useCreateRecoveryLink() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async (userId: string) => {
      const { data } = await createRecoveryLink({ client: apiClient, throwOnError: true, path: { user_id: userId } })
      return data
    },
    onSettled: () => queryClient.invalidateQueries({ queryKey: queryKeys.adminAudit }),
  })
}

/** Security events of every workspace, newest first, in cursor pages. */
export function useGlobalAudit() {
  return useInfiniteQuery({
    queryKey: queryKeys.adminAudit,
    initialPageParam: undefined as string | undefined,
    queryFn: async ({ pageParam }) => {
      const { data } = await listGlobalAudit({ client: apiClient, throwOnError: true, query: { cursor: pageParam } })
      return data
    },
    getNextPageParam: (last) => last.next_cursor ?? undefined,
    select: (data) => data.pages.flatMap((page) => page.items),
  })
}

export function useBackups() {
  return useQuery({
    queryKey: queryKeys.backups,
    queryFn: async () => {
      const { data } = await listBackups({ client: apiClient, throwOnError: true })
      return data.items
    },
  })
}

export function useCreateBackup() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async () => {
      const { data } = await createBackup({ client: apiClient, throwOnError: true })
      if (!data) throw new Error('Backup response was empty.')
      return data
    },
    onSettled: () => queryClient.invalidateQueries({ queryKey: queryKeys.backups }),
  })
}

export function useInstanceSettings() {
  return useQuery({
    queryKey: queryKeys.adminSettings,
    queryFn: async () => {
      const { data } = await getInstanceSettings({ client: apiClient, throwOnError: true })
      return data
    },
  })
}

/** Each settings write answers with the whole settings; the sign-in options follow from them. */
function useSettingsWrite<T>(write: (input: T) => Promise<InstanceSettingsView>) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: write,
    onSuccess: (settings) => {
      queryClient.setQueryData(queryKeys.adminSettings, settings)
      void queryClient.invalidateQueries({ queryKey: queryKeys.authOptions })
    },
  })
}

export function useSetRegistration() {
  return useSettingsWrite(async (open: boolean) => {
    const { data } = await setRegistration({ client: apiClient, throwOnError: true, body: { open } })
    return data
  })
}

export function useSaveSmtp() {
  return useSettingsWrite(async (body: SmtpBody) => {
    const { data } = await saveSmtp({ client: apiClient, throwOnError: true, body })
    return data
  })
}

export function useRemoveSmtp() {
  return useSettingsWrite(async () => {
    const { data } = await removeSmtp({ client: apiClient, throwOnError: true })
    return data
  })
}

export function useSendTestEmail() {
  return useMutation({
    mutationFn: async () => {
      await sendTestEmail({ client: apiClient, throwOnError: true })
    },
  })
}
