/**
 * Workspace calls the settings need besides the members (`account-api.ts`): create one, join one with an invitation
 * link, export and delete it. With the pure rules the screens share. No React.
 */
import {
  acceptInvitation as acceptInvitationRequest,
  createWorkspace as createWorkspaceRequest,
  createWorkspaceExport,
  deleteWorkspace as deleteWorkspaceRequest,
  listWorkspaceExports,
  previewInvitation as previewInvitationRequest,
} from '@orbit/api/generated/sdk.gen'
import type { AcceptanceRecord, InvitationPreview, WorkspaceExport } from '@orbit/api/generated/types.gen'

import type { OrbitClient, WorkspaceRecord } from './orbit'

export type { InvitationPreview, WorkspaceExport }

export async function createWorkspace(client: OrbitClient, name: string): Promise<WorkspaceRecord> {
  return (await createWorkspaceRequest({ client, body: { name }, throwOnError: true })).data
}

/**
 * The token of an invitation link (`https://orbit.example.com/accept-invitation?token=…`) for this server. Answers
 * with the problem to show instead: the app can only accept an invitation of the server it is signed in to. A bare
 * token (no link) is taken as it is.
 */
export function invitationToken(input: string, server: string): { token: string } | { problem: string } {
  const text = input.trim()
  if (!text) return { problem: 'Paste the invitation link.' }
  if (!/^https?:\/\//i.test(text)) return /^[\w-]{16,}$/.test(text) ? { token: text } : { problem: 'This is not an invitation link.' }
  let url: URL
  try {
    url = new URL(text)
  } catch {
    return { problem: 'This is not an invitation link.' }
  }
  const token = url.searchParams.get('token')
  if (!url.pathname.replace(/\/+$/, '').endsWith('/accept-invitation') || !token) return { problem: 'This is not an invitation link.' }
  if (url.origin.toLowerCase() !== new URL(server).origin.toLowerCase()) return { problem: `This invitation is for ${url.host}. Sign in to that server to accept it.` }
  return { token }
}

export async function previewInvitation(client: OrbitClient, token: string): Promise<InvitationPreview> {
  return (await previewInvitationRequest({ client, body: { token }, throwOnError: true })).data
}

/** Accepts for the signed-in account; the server refuses when the invitation is for another email. */
export async function acceptInvitation(client: OrbitClient, token: string): Promise<AcceptanceRecord> {
  return (await acceptInvitationRequest({ client, body: { token }, throwOnError: true })).data
}

/** The invitation's email is not the signed-in account's: accepting it here would fail. */
export const invitationForOther = (preview: Pick<InvitationPreview, 'email'>, email: string) => preview.email.trim().toLowerCase() !== email.trim().toLowerCase()

// ---- export

export async function workspaceExports(client: OrbitClient, workspaceId: string): Promise<WorkspaceExport[]> {
  return (await listWorkspaceExports({ client, path: { workspace_id: workspaceId }, throwOnError: true })).data.items
}

export async function startExport(client: OrbitClient, workspaceId: string): Promise<WorkspaceExport> {
  return (await createWorkspaceExport({ client, path: { workspace_id: workspaceId }, throwOnError: true })).data
}

export const isActiveExport = (item: Pick<WorkspaceExport, 'status'>) => item.status === 'queued' || item.status === 'running'

/** Polls every 3 s while an export is being prepared, as the web does. */
export const exportPollInterval = (items: readonly Pick<WorkspaceExport, 'status'>[] | undefined): number | false => (items?.some(isActiveExport) ? 3000 : false)

export const exportDownloadPath = (workspaceId: string, exportId: string) => `/api/v1/workspaces/${encodeURIComponent(workspaceId)}/exports/${encodeURIComponent(exportId)}/download`

// ---- delete

export async function deleteWorkspace(client: OrbitClient, workspace: Pick<WorkspaceRecord, 'id' | 'version'>): Promise<void> {
  await deleteWorkspaceRequest({ client, path: { workspace_id: workspace.id }, query: { expected_version: workspace.version }, throwOnError: true })
}
