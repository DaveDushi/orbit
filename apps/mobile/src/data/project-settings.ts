/**
 * Project settings without React (the web's `ProjectSettingsPage`): the API calls and the rules that turn the form
 * into the API's bodies, so Bun tests check them.
 */
import {
  addProjectMember,
  createLabel,
  createStatus,
  deleteLabel,
  deleteProject,
  deleteStatus,
  listProjectMembers,
  removeProjectMember,
  reorderStatuses,
  updateLabel,
  updateProject,
  updateStatus,
} from '@orbit/api/generated/sdk.gen'
import type { LabelRecord, ProjectMemberRecord, ProjectRecord, ProjectUpdateBody, ReorderItem, StatusRecord } from '@orbit/api/generated/types.gen'

import type { OrbitClient } from './orbit'

export const CATEGORY_ORDER = ['unstarted', 'started', 'completed', 'cancelled', 'duplicate'] as const
export const CATEGORY_LABEL: Record<string, string> = { unstarted: 'Unstarted', started: 'Started', completed: 'Completed', cancelled: 'Cancelled', duplicate: 'Duplicate' }
export const STATUS_COLORS = ['#8b8f98', '#5e6ad2', '#26b5ce', '#4cb782', '#f2c94c', '#f2994a', '#f7c8c1', '#eb5757', '#a78bfa']
export const PROJECT_COLORS = ['#8b5cf6', '#6366f1', '#0ea5e9', '#06b6d4', '#10b981', '#22c55e', '#84cc16', '#eab308', '#f59e0b', '#f97316', '#ef4444', '#ec4899', '#d946ef', '#64748b', '#78716c']

/** The tag field keeps upper-case letters and digits, at most five. */
export const projectKey = (text: string) => text.toUpperCase().replace(/[^A-Z0-9]/g, '').slice(0, 5)

export interface ProjectDraft {
  name: string
  key: string
  color: string
}

export const projectDraftOf = (project: ProjectRecord): ProjectDraft => ({ name: project.name, key: project.key, color: project.color })

export const draftChanged = (project: ProjectRecord, draft: ProjectDraft) => draft.name !== project.name || draft.key !== project.key || draft.color !== project.color

export const draftValid = (draft: ProjectDraft) => draft.name.trim().length > 0 && draft.key.length > 0

/**
 * The PATCH body. It always repeats the name, tag and colour (the server wants them) and the version the change is
 * based on; `change` adds the visibility or a sub-issue switch.
 */
export function projectUpdate(project: ProjectRecord, change: Partial<Pick<ProjectUpdateBody, 'name' | 'key' | 'color' | 'private' | 'auto_close_parent' | 'auto_close_sub_issues'>> = {}): ProjectUpdateBody {
  return { name: project.name, key: project.key, color: project.color, expected_version: project.version, ...change, ...(change.name !== undefined ? { name: change.name.trim() } : {}) }
}

/** A project's statuses of one category, in their order. */
export const categoryStatuses = (statuses: readonly StatusRecord[], projectId: string, category: string) =>
  statuses.filter((status) => status.project_id === projectId && status.category === category).sort((a, b) => a.position - b.position)

/**
 * The reorder body that moves a status one place up (-1) or down (+1) inside its category; `null` at the end of the
 * list. Every status of the category gets its new position and the version it is based on.
 */
export function moveStatus(own: readonly StatusRecord[], statusId: string, step: -1 | 1): ReorderItem[] | null {
  const index = own.findIndex((status) => status.id === statusId)
  const target = index + step
  if (index < 0 || target < 0 || target >= own.length) return null
  const ordered = [...own]
  ;[ordered[index], ordered[target]] = [ordered[target], ordered[index]]
  return ordered.map((status, position) => ({ id: status.id, expected_version: status.version, position }))
}

/** The Duplicate status is the system's: renamed and recoloured, never deleted. The last other status stays too. */
export function canDeleteStatus(status: StatusRecord, statuses: readonly StatusRecord[]): boolean {
  if (status.category === 'duplicate') return false
  return statuses.filter((item) => item.project_id === status.project_id && item.category !== 'duplicate').length > 1
}

export async function saveProject(client: OrbitClient, workspaceId: string, projectId: string, body: ProjectUpdateBody): Promise<ProjectRecord> {
  return (await updateProject({ client, path: { workspace_id: workspaceId, project_id: projectId }, body, throwOnError: true })).data
}

export async function removeProject(client: OrbitClient, workspaceId: string, project: Pick<ProjectRecord, 'id' | 'version'>): Promise<void> {
  await deleteProject({ client, path: { workspace_id: workspaceId, project_id: project.id }, query: { expected_version: project.version }, throwOnError: true })
}

export interface StatusValues {
  name: string
  description: string
  color: string
}

export async function addStatus(client: OrbitClient, workspaceId: string, projectId: string, category: string, values: StatusValues): Promise<StatusRecord> {
  return (await createStatus({ client, path: { workspace_id: workspaceId, project_id: projectId }, body: { ...values, category }, throwOnError: true })).data
}

export async function saveStatus(client: OrbitClient, workspaceId: string, status: StatusRecord, values: StatusValues): Promise<StatusRecord> {
  const body = { ...values, category: status.category, position: status.position, expected_version: status.version }
  return (await updateStatus({ client, path: { workspace_id: workspaceId, project_id: status.project_id, status_id: status.id }, body, throwOnError: true })).data
}

export async function removeStatus(client: OrbitClient, workspaceId: string, status: StatusRecord): Promise<void> {
  await deleteStatus({ client, path: { workspace_id: workspaceId, project_id: status.project_id, status_id: status.id }, query: { expected_version: status.version }, throwOnError: true })
}

export async function reorder(client: OrbitClient, workspaceId: string, projectId: string, items: ReorderItem[]): Promise<void> {
  await reorderStatuses({ client, path: { workspace_id: workspaceId, project_id: projectId }, body: { items }, throwOnError: true })
}

export async function members(client: OrbitClient, workspaceId: string, projectId: string): Promise<ProjectMemberRecord[]> {
  return (await listProjectMembers({ client, path: { workspace_id: workspaceId, project_id: projectId }, throwOnError: true })).data
}

export async function addMember(client: OrbitClient, workspaceId: string, projectId: string, userId: string): Promise<void> {
  await addProjectMember({ client, path: { workspace_id: workspaceId, project_id: projectId }, body: { user_id: userId }, throwOnError: true })
}

export async function removeMember(client: OrbitClient, workspaceId: string, projectId: string, userId: string): Promise<void> {
  await removeProjectMember({ client, path: { workspace_id: workspaceId, project_id: projectId, user_id: userId }, throwOnError: true })
}

export async function newLabel(client: OrbitClient, workspaceId: string, projectId: string, body: { name: string; color: string }): Promise<LabelRecord> {
  return (await createLabel({ client, path: { workspace_id: workspaceId, project_id: projectId }, body, throwOnError: true })).data
}

export async function saveLabel(client: OrbitClient, workspaceId: string, label: LabelRecord, body: { name: string; color: string }): Promise<LabelRecord> {
  return (await updateLabel({ client, path: { workspace_id: workspaceId, project_id: label.project_id, label_id: label.id }, body: { ...body, expected_version: label.version }, throwOnError: true })).data
}

export async function removeLabel(client: OrbitClient, workspaceId: string, label: LabelRecord): Promise<void> {
  await deleteLabel({ client, path: { workspace_id: workspaceId, project_id: label.project_id, label_id: label.id }, query: { expected_version: label.version }, throwOnError: true })
}
