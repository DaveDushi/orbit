/**
 * The installation Admin calls the app offers (users, audit log, backups, background job limits) over the generated
 * SDK, with the rules of the Users menu. No React. The server refuses every admin request from other accounts.
 */
import {
  createRecoveryLink,
  getJobLimits,
  listAdminUsers,
  listBackups,
  listGlobalAudit,
  resetTwoFactor,
  setAccountSuspension,
  setInstanceAdmin,
} from '@orbit/api/generated/sdk.gen'
import type { AdminUser, BackupSummary, JobLimitView, PageAuditEvent, RecoveryLink } from '@orbit/api/generated/types.gen'

import type { OrbitClient } from './orbit'

export type { AdminUser, BackupSummary, JobLimitView, RecoveryLink }
export type AuditEvent = PageAuditEvent['items'][number]

export interface Page<T> {
  items: T[]
  next: string | null
}

export const adminKeys = {
  users: (search: string) => ['admin', 'users', search] as const,
  audit: ['admin', 'audit'] as const,
  backups: ['admin', 'backups'] as const,
  jobs: ['admin', 'jobs'] as const,
}

export async function adminUsers(client: OrbitClient, search: string, cursor?: string): Promise<Page<AdminUser>> {
  const { data } = await listAdminUsers({ client, query: { q: search || undefined, cursor, limit: 50 }, throwOnError: true })
  return { items: data.items, next: data.next_cursor ?? null }
}

export async function auditEvents(client: OrbitClient, cursor?: string): Promise<Page<AuditEvent>> {
  const { data } = await listGlobalAudit({ client, query: { cursor, limit: 50 }, throwOnError: true })
  return { items: data.items, next: data.next_cursor ?? null }
}

export async function backups(client: OrbitClient): Promise<BackupSummary[]> {
  return (await listBackups({ client, throwOnError: true })).data.items
}

export async function jobLimits(client: OrbitClient): Promise<JobLimitView[]> {
  return (await getJobLimits({ client, throwOnError: true })).data.kinds
}

export async function setSuspended(client: OrbitClient, userId: string, suspended: boolean): Promise<void> {
  await setAccountSuspension({ client, path: { user_id: userId }, body: { suspended }, throwOnError: true })
}

export async function setAdmin(client: OrbitClient, userId: string, admin: boolean): Promise<void> {
  await setInstanceAdmin({ client, path: { user_id: userId }, body: { admin }, throwOnError: true })
}

export async function recoveryLink(client: OrbitClient, userId: string): Promise<RecoveryLink> {
  return (await createRecoveryLink({ client, path: { user_id: userId }, throwOnError: true })).data
}

export async function clearTwoFactor(client: OrbitClient, userId: string): Promise<void> {
  await resetTwoFactor({ client, path: { user_id: userId }, throwOnError: true })
}

export type AdminAction = 'make-admin' | 'remove-admin' | 'recovery-link' | 'reset-two-factor' | 'suspend' | 'reinstate'

/**
 * The web's Manage menu: nobody manages the root account or their own; only the root user manages instance admins.
 * A suspended account can only be reinstated.
 */
export function adminActions(user: AdminUser, viewer: { id: string; root: boolean }): AdminAction[] {
  if (user.root || user.id === viewer.id || (user.admin && !viewer.root)) return []
  const admin: AdminAction[] = viewer.root ? (user.admin ? ['remove-admin'] : user.suspended ? [] : ['make-admin']) : []
  if (user.suspended) return [...admin, 'reinstate']
  return [...admin, 'recovery-link', ...(user.two_factor ? (['reset-two-factor'] as const) : []), 'suspend']
}

/** `Root`, `Admin`, `2FA`, `Suspended`: the badges beside a user's name. */
export function adminBadges(user: AdminUser): string[] {
  return [user.root ? 'Root' : user.admin ? 'Admin' : null, user.two_factor ? '2FA' : null, user.suspended ? 'Suspended' : null].filter((badge): badge is string => badge !== null)
}

/** Who did it: the account's name, or `System` for the server's own events. */
export const auditActor = (event: Pick<AuditEvent, 'actor_id' | 'actor_name'>) => (event.actor_id ? (event.actor_name ?? 'Unknown account') : 'System')

/** The web's job names; an unknown kind shows as it is. */
const JOB_LABELS: Record<string, string> = { 'notion.scan': 'Notion scans', 'notion.import': 'Notion imports' }
export const jobLabel = (kind: string) => JOB_LABELS[kind] ?? kind
export const limitText = (limit: number | null | undefined) => (limit == null ? 'no limit' : String(limit))
