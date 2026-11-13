/**
 * The task calls the detail page and the New task dialog make beyond `orbit.ts`: relations, sub-issues, task search,
 * labels, attachments, and a create that takes every property. Plain functions over the generated SDK, so the
 * screens and `scripts/api-check-tasks.ts` share them.
 */
import {
  createCycle,
  createLabel,
  createMilestone,
  createTask,
  createTaskRelation,
  deleteCycle,
  deleteLabel,
  deleteMilestone,
  deleteTaskAttachment,
  deleteTaskRelation,
  followTask,
  listCommentAttachments,
  listCycles,
  listMilestones,
  listTaskAttachments,
  listTaskFollowers,
  listTaskRelations,
  listTasks,
  moveUnfinishedCycleTasks,
  reorderMilestones,
  unfollowTask,
  updateCycle,
  updateMilestone,
  uploadTaskAttachments,
} from '@orbit/api/generated/sdk.gen'
import type {
  AttachmentRecord,
  CreateTaskBody,
  NewTaskRelationType,
  TaskCycle,
  TaskCycleBody,
  TaskCycleMoveOutcome,
  TaskCycleUpdateBody,
  TaskFollowers,
  TaskMilestone,
  TaskMilestoneBody,
  TaskMilestoneUpdateBody,
  TaskRelationRecord,
} from '@orbit/api/generated/types.gen'

import type { LabelRecord, OrbitClient, TaskRecord } from './orbit'
import type { PickedFile } from './tasks'

export type {
  AttachmentRecord,
  CreateTaskBody,
  NewTaskRelationType,
  TaskCycle,
  TaskCycleBody,
  TaskCycleMoveOutcome,
  TaskCycleUpdateBody,
  TaskFollowers,
  TaskMilestone,
  TaskMilestoneBody,
  TaskMilestoneUpdateBody,
  TaskRelationRecord,
}

const task = (client: OrbitClient, workspaceId: string, taskId: string) => ({ client, path: { workspace_id: workspaceId, task_id: taskId }, throwOnError: true as const })

/** Follows `next_cursor` until the server has no more. */
async function allPages<T>(page: (cursor: string | undefined) => Promise<{ items: T[]; next_cursor?: string | null }>): Promise<T[]> {
  const items: T[] = []
  let cursor: string | undefined
  do {
    const next = await page(cursor)
    items.push(...next.items)
    cursor = next.next_cursor ?? undefined
  } while (cursor)
  return items
}

export async function createTaskWith(client: OrbitClient, workspaceId: string, body: CreateTaskBody): Promise<TaskRecord> {
  return (await createTask({ client, path: { workspace_id: workspaceId }, body, throwOnError: true })).data
}

/** The draft of the New task dialog as the create body: trimmed text, and no description when it is empty. */
export function newTaskBody(draft: {
  title: string
  description: string
  projectId: string
  statusId: string
  priority: string
  assigneeIds: readonly string[]
  labelIds: readonly string[]
  dueAt: string | null
  dueStartAt: string | null
  parentId: string | null
  milestoneId: string | null
  cycleId: string | null
  estimate: number | null
}): CreateTaskBody {
  const description = draft.description.trim()
  return {
    title: draft.title.trim(),
    ...(description ? { description } : {}),
    project_id: draft.projectId,
    status_id: draft.statusId,
    priority: draft.priority,
    assignee_ids: [...draft.assigneeIds],
    label_ids: [...draft.labelIds],
    due_at: draft.dueAt,
    due_start_at: draft.dueStartAt,
    parent_task_id: draft.parentId,
    milestone_id: draft.milestoneId,
    cycle_id: draft.cycleId,
    estimate: draft.estimate,
  }
}

/** Recently updated tasks, or the server's search over titles, descriptions and identifiers (`ENG-12`). */
export async function searchTasks(client: OrbitClient, workspaceId: string, search: string): Promise<TaskRecord[]> {
  const query = { sort: 'updated_at', order: 'desc', limit: 100, search: search.trim() || undefined }
  return (await listTasks({ client, path: { workspace_id: workspaceId }, query, throwOnError: true })).data.items
}

/** Direct sub-issues, oldest first. */
export function subIssues(client: OrbitClient, workspaceId: string, parentId: string): Promise<TaskRecord[]> {
  return allPages(
    async (cursor) =>
      (await listTasks({ client, path: { workspace_id: workspaceId }, query: { parent_task_id: parentId, sort: 'created_at', order: 'asc', limit: 100, cursor }, throwOnError: true }))
        .data,
  )
}

export async function relations(client: OrbitClient, workspaceId: string, taskId: string): Promise<TaskRelationRecord[]> {
  return (await listTaskRelations(task(client, workspaceId, taskId))).data
}

export async function addRelation(client: OrbitClient, workspaceId: string, taskId: string, body: { type: NewTaskRelationType; task_id: string }): Promise<TaskRelationRecord> {
  return (await createTaskRelation({ ...task(client, workspaceId, taskId), body })).data
}

export async function removeRelation(client: OrbitClient, workspaceId: string, taskId: string, relationId: string): Promise<void> {
  await deleteTaskRelation({ client, path: { workspace_id: workspaceId, task_id: taskId, relation_id: relationId }, throwOnError: true })
}

export async function newLabel(client: OrbitClient, workspaceId: string, projectId: string, body: { name: string; color: string }): Promise<LabelRecord> {
  return (await createLabel({ client, path: { workspace_id: workspaceId, project_id: projectId }, body, throwOnError: true })).data
}

export async function removeLabel(client: OrbitClient, workspaceId: string, label: Pick<LabelRecord, 'id' | 'project_id' | 'version'>): Promise<void> {
  await deleteLabel({ client, path: { workspace_id: workspaceId, project_id: label.project_id, label_id: label.id }, query: { expected_version: label.version }, throwOnError: true })
}

// ---- milestones and cycles of a project

const inProject = (client: OrbitClient, workspaceId: string, projectId: string) => ({ client, path: { workspace_id: workspaceId, project_id: projectId }, throwOnError: true as const })

/** Every milestone of the project, with its task counts. The server returns them in one page. */
export async function milestones(client: OrbitClient, workspaceId: string, projectId: string): Promise<TaskMilestone[]> {
  return (await listMilestones(inProject(client, workspaceId, projectId))).data.items
}

export async function newMilestone(client: OrbitClient, workspaceId: string, projectId: string, body: TaskMilestoneBody): Promise<TaskMilestone> {
  return (await createMilestone({ ...inProject(client, workspaceId, projectId), body })).data
}

export async function changeMilestone(client: OrbitClient, workspaceId: string, milestone: Pick<TaskMilestone, 'id' | 'project_id'>, body: TaskMilestoneUpdateBody): Promise<TaskMilestone> {
  return (await updateMilestone({ client, path: { workspace_id: workspaceId, project_id: milestone.project_id, milestone_id: milestone.id }, body, throwOnError: true })).data
}

export async function removeMilestone(client: OrbitClient, workspaceId: string, milestone: Pick<TaskMilestone, 'id' | 'project_id' | 'version'>): Promise<void> {
  await deleteMilestone({ client, path: { workspace_id: workspaceId, project_id: milestone.project_id, milestone_id: milestone.id }, query: { expected_version: milestone.version }, throwOnError: true })
}

/** Gives the milestones the positions of their order in `ordered`; returns them in the new order. */
export async function reorderProjectMilestones(client: OrbitClient, workspaceId: string, projectId: string, ordered: readonly Pick<TaskMilestone, 'id' | 'version'>[]): Promise<TaskMilestone[]> {
  const items = ordered.map((milestone, position) => ({ id: milestone.id, expected_version: milestone.version, position }))
  return (await reorderMilestones({ ...inProject(client, workspaceId, projectId), body: { items } })).data.items
}

/** Every cycle of the project, with its task counts. */
export async function cycles(client: OrbitClient, workspaceId: string, projectId: string): Promise<TaskCycle[]> {
  return (await listCycles(inProject(client, workspaceId, projectId))).data.items
}

export async function newCycle(client: OrbitClient, workspaceId: string, projectId: string, body: TaskCycleBody): Promise<TaskCycle> {
  return (await createCycle({ ...inProject(client, workspaceId, projectId), body })).data
}

export async function changeCycle(client: OrbitClient, workspaceId: string, cycle: Pick<TaskCycle, 'id' | 'project_id'>, body: TaskCycleUpdateBody): Promise<TaskCycle> {
  return (await updateCycle({ client, path: { workspace_id: workspaceId, project_id: cycle.project_id, cycle_id: cycle.id }, body, throwOnError: true })).data
}

export async function removeCycle(client: OrbitClient, workspaceId: string, cycle: Pick<TaskCycle, 'id' | 'project_id' | 'version'>): Promise<void> {
  await deleteCycle({ client, path: { workspace_id: workspaceId, project_id: cycle.project_id, cycle_id: cycle.id }, query: { expected_version: cycle.version }, throwOnError: true })
}

/** Moves the cycle's tasks that are not done or cancelled to the next cycle by date; 409 `no_next_cycle` when there is none. */
export async function moveUnfinished(client: OrbitClient, workspaceId: string, cycle: Pick<TaskCycle, 'id' | 'project_id'>): Promise<TaskCycleMoveOutcome> {
  return (await moveUnfinishedCycleTasks({ client, path: { workspace_id: workspaceId, project_id: cycle.project_id, cycle_id: cycle.id }, throwOnError: true })).data
}

/** Files attached to the task itself (before rich text). Inline files of the description and comments are not in it. */
export function taskAttachments(client: OrbitClient, workspaceId: string, taskId: string): Promise<AttachmentRecord[]> {
  return allPages(async (cursor) => (await listTaskAttachments({ ...task(client, workspaceId, taskId), query: { limit: 100, cursor } })).data)
}

/** Files attached to a comment (before rich text). */
export function commentAttachments(client: OrbitClient, workspaceId: string, taskId: string, commentId: string): Promise<AttachmentRecord[]> {
  return allPages(
    async (cursor) =>
      (await listCommentAttachments({ client, path: { workspace_id: workspaceId, task_id: taskId, comment_id: commentId }, query: { limit: 100, cursor }, throwOnError: true })).data,
  )
}

/**
 * Uploads a file for a comment's or the description's text (`inline`), as the web's paperclip does; the text then
 * links it by its download path. The app's part is an expo-file-system `File`, which the SDK's serializer would turn
 * into JSON, so this one builds the form itself.
 */
export function uploadInline(client: OrbitClient, workspaceId: string, taskId: string, file: PickedFile): Promise<AttachmentRecord> {
  return uploadTaskFile(client, workspaceId, taskId, file, true)
}

/** Uploads a file to the task's attachment list (`inline`: for a comment's or the description's text instead). */
export async function uploadTaskFile(client: OrbitClient, workspaceId: string, taskId: string, file: Pick<PickedFile, 'name' | 'blob'>, inline = false): Promise<AttachmentRecord> {
  const { data } = await uploadTaskAttachments({
    ...task(client, workspaceId, taskId),
    query: inline ? { inline: '1' } : undefined,
    body: { file: file.blob },
    bodySerializer: () => {
      const form = new FormData()
      if (file.blob instanceof Blob) form.append('file', file.blob, file.name)
      else form.append('file', file.blob)
      return form
    },
  })
  return data
}

export async function removeAttachment(client: OrbitClient, workspaceId: string, taskId: string, attachmentId: string): Promise<void> {
  await deleteTaskAttachment({ client, path: { workspace_id: workspaceId, task_id: taskId, attachment_id: attachmentId }, throwOnError: true })
}

/** Who follows the task (gets its new comments and status changes), and whether the caller does. */
export async function followers(client: OrbitClient, workspaceId: string, taskId: string): Promise<TaskFollowers> {
  return (await listTaskFollowers(task(client, workspaceId, taskId))).data
}

/** Follows (`true`) or unfollows (`false`) the task for the signed-in member. */
export async function setFollowing(client: OrbitClient, workspaceId: string, taskId: string, follow: boolean): Promise<TaskFollowers> {
  return (await (follow ? followTask : unfollowTask)(task(client, workspaceId, taskId))).data
}
