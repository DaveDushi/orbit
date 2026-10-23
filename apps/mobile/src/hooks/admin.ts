/** The Admin screens' queries and writes, over `data/admin-api.ts`. */
import { useInfiniteQuery, useMutation, useQuery, useQueryClient } from '@tanstack/react-query'

import * as admin from '../data/admin-api'
import { useOrbit } from '../orbit-context'

export function useAdminUsers(search: string) {
  const { client } = useOrbit()
  return useInfiniteQuery({
    queryKey: admin.adminKeys.users(search),
    queryFn: ({ pageParam }) => admin.adminUsers(client, search, pageParam),
    initialPageParam: undefined as string | undefined,
    getNextPageParam: (last) => last.next ?? undefined,
  })
}

export function useAuditEvents() {
  const { client } = useOrbit()
  return useInfiniteQuery({
    queryKey: admin.adminKeys.audit,
    queryFn: ({ pageParam }) => admin.auditEvents(client, pageParam),
    initialPageParam: undefined as string | undefined,
    getNextPageParam: (last) => last.next ?? undefined,
  })
}

export function useBackups() {
  const { client } = useOrbit()
  return useQuery({ queryKey: admin.adminKeys.backups, queryFn: () => admin.backups(client) })
}

export function useJobLimits() {
  const { client } = useOrbit()
  return useQuery({ queryKey: admin.adminKeys.jobs, queryFn: () => admin.jobLimits(client) })
}

/** The Users menu's writes. Each refreshes every users list. */
export function useAdminUserWrites() {
  const { client } = useOrbit()
  const queryClient = useQueryClient()
  const onSettled = () => void queryClient.invalidateQueries({ queryKey: ['admin', 'users'] })
  return {
    suspend: useMutation({ mutationFn: ({ userId, suspended }: { userId: string; suspended: boolean }) => admin.setSuspended(client, userId, suspended), onSettled }),
    setAdmin: useMutation({ mutationFn: ({ userId, isAdmin }: { userId: string; isAdmin: boolean }) => admin.setAdmin(client, userId, isAdmin), onSettled }),
    resetTwoFactor: useMutation({ mutationFn: (userId: string) => admin.clearTwoFactor(client, userId), onSettled }),
    recoveryLink: useMutation({ mutationFn: (userId: string) => admin.recoveryLink(client, userId) }),
  }
}
