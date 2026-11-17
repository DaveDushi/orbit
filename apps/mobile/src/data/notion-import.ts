/**
 * The Notion import (the web's `docs/import`): a Notion token lets Orbit read the pages it can see; the user picks
 * pages (each with everything under it) and a space, and Orbit imports them in the background. No React.
 */
import { cancelNotionImport, createNotionImport, getNotionImport, listNotionImports, startNotionImport } from '@orbit/api/generated/sdk.gen'
import type { NotionImport, NotionImportDestinationBody, NotionImportNode, NotionImportSelectionBody, NotionImportStatus } from '@orbit/api/generated/types.gen'

import type { OrbitClient } from './orbit'

export type { NotionImport, NotionImportNode, NotionImportStatus }

export const NOTION_POLL_MS = 2000

export const STATUS_LABEL: Record<NotionImportStatus, string> = {
  scanning: 'Reading your Notion pages',
  ready: 'Choose pages',
  queued: 'Waiting to start',
  importing: 'Importing',
  completed: 'Done',
  failed: 'Failed',
  cancelled: 'Cancelled',
  expired: 'Expired',
}

/** Scanning, choosing or importing: the screen polls and offers Cancel. */
export const isActiveImport = (status: NotionImportStatus) => ['scanning', 'ready', 'queued', 'importing'].includes(status)

export async function notionImports(client: OrbitClient, workspaceId: string): Promise<NotionImport[]> {
  return (await listNotionImports({ client, path: { workspace_id: workspaceId }, throwOnError: true })).data.items
}

export async function notionImport(client: OrbitClient, workspaceId: string, importId: string): Promise<NotionImport> {
  return (await getNotionImport({ client, path: { workspace_id: workspaceId, import_id: importId }, throwOnError: true })).data
}

/** Sends the token once; Orbit keeps it encrypted until the import ends and never shows it again. */
export async function newNotionImport(client: OrbitClient, workspaceId: string, token: string): Promise<NotionImport> {
  return (await createNotionImport({ client, path: { workspace_id: workspaceId }, body: { token }, throwOnError: true })).data
}

export async function startImport(client: OrbitClient, workspaceId: string, importId: string, selection: NotionImportSelectionBody, destination: NotionImportDestinationBody): Promise<NotionImport> {
  return (await startNotionImport({ client, path: { workspace_id: workspaceId, import_id: importId }, body: { selection, destination }, throwOnError: true })).data
}

export async function cancelImport(client: OrbitClient, workspaceId: string, importId: string): Promise<void> {
  await cancelNotionImport({ client, path: { workspace_id: workspaceId, import_id: importId }, throwOnError: true })
}

/** The pages to list: the top pages (no parent the token can see), or every page whose title matches `query`. */
export function importRows(nodes: readonly NotionImportNode[], query: string): NotionImportNode[] {
  const needle = query.trim().toLowerCase()
  if (needle) return nodes.filter((node) => (node.title.trim() || 'Untitled').toLowerCase().includes(needle))
  const ids = new Set(nodes.map((node) => node.notion_id))
  return nodes.filter((node) => node.parent_id === null || !ids.has(node.parent_id))
}

/** True when an ancestor of `id` is checked: the page comes with it. */
export function coveredByParent(nodes: readonly NotionImportNode[], checked: ReadonlySet<string>, id: string): boolean {
  const byId = new Map(nodes.map((node) => [node.notion_id, node]))
  const seen = new Set<string>()
  for (let parent = byId.get(id)?.parent_id ?? null; parent && !seen.has(parent); parent = byId.get(parent)?.parent_id ?? null) {
    seen.add(parent)
    if (checked.has(parent)) return true
  }
  return false
}

/** The `selection` of `start`: everything when every top page is checked, else the checked pages no checked page covers. */
export function importSelection(nodes: readonly NotionImportNode[], checked: ReadonlySet<string>): NotionImportSelectionBody | null {
  const roots = importRows(nodes, '')
  if (roots.length > 0 && roots.every((node) => checked.has(node.notion_id))) return { all: true }
  const ids = [...checked].filter((id) => nodes.some((node) => node.notion_id === id) && !coveredByParent(nodes, checked, id))
  return ids.length > 0 ? { notion_ids: ids } : null
}

/** Where the pages go: a teamspace (`teamspace:<id>`) or Private. */
export const importDestination = (space: string): NotionImportDestinationBody => (space === 'private' ? { private: true } : { teamspace_id: space.replace(/^teamspace:/, '') })
