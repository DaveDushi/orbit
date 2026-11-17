/** The task trash (the web's `/tasks-trash`): trashed projects and tasks, restore, delete forever, empty. */
import {
  emptyTaskTrash,
  listProjectTrash,
  listTaskTrash,
  purgeProject,
  purgeTask,
  restoreProject,
  restoreTask,
} from '@orbit/api/generated/sdk.gen'
import type { TaskTrashEmptied } from '@orbit/api/generated/types.gen'

import { allPages, type OrbitClient, type ProjectRecord, type TaskRecord } from './orbit'

export interface TaskTrash {
  projects: ProjectRecord[]
  tasks: TaskRecord[]
}

/** Trashed projects (with all their tasks) and tasks trashed on their own, newest first as the server lists them. */
export async function taskTrash(client: OrbitClient, workspaceId: string): Promise<TaskTrash> {
  const path = { workspace_id: workspaceId }
  const [projects, tasks] = await Promise.all([
    allPages(async (cursor) => (await listProjectTrash({ client, path, query: { cursor, limit: 100 }, throwOnError: true })).data),
    allPages(async (cursor) => (await listTaskTrash({ client, path, query: { cursor, limit: 100 }, throwOnError: true })).data as { items: TaskRecord[]; next_cursor?: string | null }),
  ])
  return { projects, tasks }
}

export async function restoreTrashedTask(client: OrbitClient, workspaceId: string, task: Pick<TaskRecord, 'id' | 'version'>): Promise<void> {
  await restoreTask({ client, path: { workspace_id: workspaceId, task_id: task.id }, body: { expected_version: task.version }, throwOnError: true })
}

export async function purgeTrashedTask(client: OrbitClient, workspaceId: string, taskId: string): Promise<void> {
  await purgeTask({ client, path: { workspace_id: workspaceId, task_id: taskId }, throwOnError: true })
}

export async function restoreTrashedProject(client: OrbitClient, workspaceId: string, project: Pick<ProjectRecord, 'id' | 'version'>): Promise<void> {
  await restoreProject({ client, path: { workspace_id: workspaceId, project_id: project.id }, body: { expected_version: project.version }, throwOnError: true })
}

export async function purgeTrashedProject(client: OrbitClient, workspaceId: string, projectId: string): Promise<void> {
  await purgeProject({ client, path: { workspace_id: workspaceId, project_id: projectId }, throwOnError: true })
}

export async function emptyTrash(client: OrbitClient, workspaceId: string): Promise<TaskTrashEmptied> {
  return (await emptyTaskTrash({ client, path: { workspace_id: workspaceId }, throwOnError: true })).data
}

/** "+3 sub-issues": what goes with a trashed task. */
export const trashedSubIssuesLabel = (count: number | null | undefined): string | null => (count && count > 0 ? `+${count} sub-issue${count === 1 ? '' : 's'}` : null)

/** The trash without the restored or deleted entries, for the optimistic list. */
export function withoutEntry(trash: TaskTrash, kind: 'task' | 'project', id: string): TaskTrash {
  return kind === 'task' ? { ...trash, tasks: trash.tasks.filter((task) => task.id !== id) } : { ...trash, projects: trash.projects.filter((project) => project.id !== id) }
}
