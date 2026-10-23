import { ApiProblem } from '@orbit/api/problem'

import type { ProjectRecord, StatusRecord, TaskRecord } from './orbit'

export type StatusCategory = 'unstarted' | 'started' | 'completed' | 'cancelled' | 'duplicate'
export type Priority = 'none' | 'low' | 'medium' | 'high' | 'urgent'

/** The workflow order of the web: a status sorts by its category, then by its position. */
const CATEGORY_ORDER: readonly string[] = ['unstarted', 'started', 'completed', 'cancelled', 'duplicate']
const CLOSED: readonly string[] = ['completed', 'cancelled', 'duplicate']

export const PRIORITY_LABEL: Record<Priority, string> = { none: 'No priority', low: 'Low', medium: 'Medium', high: 'High', urgent: 'Urgent' }
const PRIORITY_ORDER: readonly string[] = ['urgent', 'high', 'medium', 'low', 'none']

export const priorityOf = (task: Pick<TaskRecord, 'priority'>): Priority =>
  PRIORITY_ORDER.includes(task.priority) ? (task.priority as Priority) : 'none'

export function sortStatuses(statuses: readonly StatusRecord[]): StatusRecord[] {
  return [...statuses].sort((a, b) => CATEGORY_ORDER.indexOf(a.category) - CATEGORY_ORDER.indexOf(b.category) || a.position - b.position)
}

/** Same-named statuses of different projects share a key, so a list across projects shows one "Todo" group. */
export const statusKeyOf = (status: Pick<StatusRecord, 'category' | 'name'>) =>
  `${status.category}:${status.name.trim().replace(/[A-Z]+/g, (letters) => letters.toLowerCase())}`

export const isClosed = (status: Pick<StatusRecord, 'category'> | undefined) => status !== undefined && CLOSED.includes(status.category)

/** `GEN-12`. Just the number while the projects load. */
export function identifier(task: Pick<TaskRecord, 'project_id' | 'number'>, projects: readonly ProjectRecord[]): string {
  const project = projects.find((item) => item.id === task.project_id)
  return project ? `${project.key}-${task.number}` : `#${task.number}`
}

/** The statuses of one project in workflow order. */
export const projectStatuses = (statuses: readonly StatusRecord[], projectId: string) => sortStatuses(statuses.filter((status) => status.project_id === projectId))

/** Where a new task lands: the first unstarted status of the project, else its first status. */
export function defaultStatusOf(statuses: readonly StatusRecord[], projectId: string): StatusRecord | undefined {
  const own = projectStatuses(statuses, projectId)
  return own.find((status) => status.category === 'unstarted') ?? own[0]
}

export interface StatusGroup {
  key: string
  name: string
  /** The first status of the group: its category and color draw the icon. `undefined` for tasks with an unknown status. */
  status: StatusRecord | undefined
  tasks: TaskRecord[]
}

const byPriorityThenRecent = (a: TaskRecord, b: TaskRecord) =>
  PRIORITY_ORDER.indexOf(priorityOf(a)) - PRIORITY_ORDER.indexOf(priorityOf(b)) || b.updated_at.localeCompare(a.updated_at)

/**
 * Tasks in status groups, in workflow order (Backlog, Todo, In Progress, Done …). Empty groups are left out. A task
 * whose status is not loaded goes to a last "No status" group.
 */
export function groupTasksByStatus(tasks: readonly TaskRecord[], statuses: readonly StatusRecord[]): StatusGroup[] {
  const keyById = new Map(statuses.map((status) => [status.id, statusKeyOf(status)]))
  const groups: StatusGroup[] = []
  for (const status of sortStatuses(statuses)) {
    const key = statusKeyOf(status)
    if (!groups.some((group) => group.key === key)) groups.push({ key, name: status.name, status, tasks: [] })
  }
  const unknown: StatusGroup = { key: 'none', name: 'No status', status: undefined, tasks: [] }
  for (const task of tasks) {
    const key = keyById.get(task.status_id)
    ;(groups.find((group) => group.key === key) ?? unknown).tasks.push(task)
  }
  return [...groups, unknown].filter((group) => group.tasks.length > 0).map((group) => ({ ...group, tasks: group.tasks.sort(byPriorityThenRecent) }))
}

/** Open tasks assigned to the user: due soonest first (no due date last), then by priority. */
export function myOpenTasks(tasks: readonly TaskRecord[], statuses: readonly StatusRecord[], userId: string, limit: number): TaskRecord[] {
  const statusById = new Map(statuses.map((status) => [status.id, status]))
  const due = (task: TaskRecord) => (task.due_at ? Date.parse(task.due_at) : Number.POSITIVE_INFINITY)
  return tasks
    .filter((task) => task.assignee_ids.includes(userId) && !isClosed(statusById.get(task.status_id)))
    .sort((a, b) => due(a) - due(b) || byPriorityThenRecent(a, b))
    .slice(0, limit)
}

/** `Overdue`, `Today`, `Tomorrow`, or the weekday and date (`Fri, Oct 30`), as the web Home card. */
export function dueLabel(dueAt: string, now: Date, locale?: string): string {
  const day = (date: Date) => new Date(date.getFullYear(), date.getMonth(), date.getDate()).getTime()
  const days = Math.round((day(new Date(dueAt)) - day(now)) / 86_400_000)
  if (days < 0) return 'Overdue'
  if (days === 0) return 'Today'
  if (days === 1) return 'Tomorrow'
  return new Date(dueAt).toLocaleDateString(locale, { weekday: 'short', month: 'short', day: 'numeric' })
}

/** The list with `task` in place of its old copy. */
export const replaceTask = (list: readonly TaskRecord[], task: TaskRecord) => list.map((item) => (item.id === task.id ? task : item))

/** The task changed since it was loaded: `expected_version` no longer matches. */
export const isVersionConflict = (error: unknown) => error instanceof ApiProblem && error.status === 409 && ['task_conflict', 'conflict'].includes(error.code)
