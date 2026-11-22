/**
 * The web's API tokens page without React: the calls, the access rows (`ScopeAccess`) and the form's rules, so Bun
 * tests check them.
 */
import { createApiToken, listApiTokens, revokeApiToken } from '@orbit/api/generated/sdk.gen'
import type { ApiTokenRecord, ApiTokenScope } from '@orbit/api/generated/types.gen'

import type { OrbitClient } from './orbit'

export type { ApiTokenRecord, ApiTokenScope }

export const EXPIRATION_OPTIONS = [
  { value: 'never', label: 'Never expires' },
  { value: '7', label: '7 days' },
  { value: '30', label: '30 days' },
  { value: '90', label: '90 days' },
  { value: '365', label: '1 year' },
] as const

export type Expiration = (typeof EXPIRATION_OPTIONS)[number]['value']

export type AccessLevel = 'none' | 'read' | 'write' | 'manage'

/** One access row: a level without a scope is unavailable, except `read` with no read scope, which is always given. */
export interface ScopeResource {
  name: string
  hint: string
  read?: ApiTokenScope
  write?: ApiTokenScope
  /** A level above write; the other rows do not show it. */
  manage?: ApiTokenScope
  disabled?: boolean
}

/** A service account has task access only. */
export function tokenResources(serviceAccount: boolean): ScopeResource[] {
  return [
    { name: 'Tasks', hint: 'In the selected projects. Write can trash tasks; Manage adds other deletes and project setup.', read: 'tasks:read', write: 'tasks:write', manage: serviceAccount ? undefined : 'tasks:manage' },
    { name: 'Docs', hint: 'Including your private docs. Manage adds doc locks and deletes of comments and, over MCP, moving and trashing docs.', read: 'docs:read', write: 'docs:write', manage: 'docs:manage', disabled: serviceAccount },
    { name: 'Chat', hint: 'Including your private channels and DMs.', read: 'chat:read', disabled: serviceAccount },
    { name: 'Drive', hint: 'MCP only. Write can move, rename and trash files, never delete forever.', read: 'drive:read', write: 'drive:write', disabled: serviceAccount },
  ]
}

export function accessLevel(resource: ScopeResource, scopes: readonly ApiTokenScope[]): AccessLevel {
  if (resource.manage && scopes.includes(resource.manage)) return 'manage'
  if (resource.write && scopes.includes(resource.write)) return 'write'
  return !resource.read || scopes.includes(resource.read) ? 'read' : 'none'
}

export function withAccessLevel(resource: ScopeResource, scopes: readonly ApiTokenScope[], level: AccessLevel): ApiTokenScope[] {
  return [
    ...scopes.filter((scope) => scope !== resource.read && scope !== resource.write && scope !== resource.manage),
    ...(level !== 'none' && resource.read ? [resource.read] : []),
    ...((level === 'write' || level === 'manage') && resource.write ? [resource.write] : []),
    ...(level === 'manage' && resource.manage ? [resource.manage] : []),
  ]
}

/** Whether a level button of a row can be chosen. */
export function levelAvailable(resource: ScopeResource, level: AccessLevel): boolean {
  if (resource.disabled) return level === 'none'
  if (level === 'none') return !!resource.read
  if (level === 'manage') return !!resource.manage
  return level === 'write' ? !!resource.write : true
}

/** Selecting a service account drops the docs, chat, drive and manage scopes; tasks:read stays as the least. */
export function serviceAccountScopes(scopes: readonly ApiTokenScope[]): ApiTokenScope[] {
  const tasks = scopes.filter((scope) => scope === 'tasks:read' || scope === 'tasks:write')
  return tasks.length ? tasks : ['tasks:read']
}

export interface TokenForm {
  name: string
  scopes: ApiTokenScope[]
  projectIds: string[]
  allProjects: boolean
  serviceAccount: boolean
  expiration: Expiration
}

/** Projects only limit task access, so a token without a task scope needs none. */
export const hasTaskScope = (scopes: readonly ApiTokenScope[]) => scopes.some((scope) => scope.startsWith('tasks:'))

export const tokenFormReady = (form: TokenForm) => form.name.trim() !== '' && form.scopes.length > 0 && (!hasTaskScope(form.scopes) || form.allProjects || form.projectIds.length > 0)

export function tokenBody(form: TokenForm) {
  const tasks = hasTaskScope(form.scopes)
  return {
    name: form.name.trim(),
    project_ids: tasks && !form.allProjects ? form.projectIds : [],
    all_projects: tasks && form.allProjects,
    scopes: form.scopes,
    service_account: form.serviceAccount,
    expires_in_days: form.expiration === 'never' ? null : Number(form.expiration),
  }
}

export const isExpired = (expiresAt: string | null | undefined, now = Date.now()) => expiresAt != null && new Date(expiresAt).getTime() <= now

export async function apiTokens(client: OrbitClient, workspaceId: string): Promise<ApiTokenRecord[]> {
  return (await listApiTokens({ client, path: { workspace_id: workspaceId }, throwOnError: true })).data
}

/** Answers with the secret, which the server shows only this once. */
export async function createToken(client: OrbitClient, workspaceId: string, form: TokenForm): Promise<string> {
  return (await createApiToken({ client, path: { workspace_id: workspaceId }, body: tokenBody(form), throwOnError: true })).data.token
}

export async function revokeToken(client: OrbitClient, workspaceId: string, tokenId: string): Promise<void> {
  await revokeApiToken({ client, path: { workspace_id: workspaceId, token_id: tokenId }, throwOnError: true })
}
