/**
 * Saved task views on the phone (the web's `features/views`): the `ViewState` contract, the filter rows the phone
 * edits, the grouping of a view's tasks and the API calls. No React, so the tests run them with Bun.
 */
import {
  createView,
  deleteView,
  favoriteView,
  getView,
  listViews,
  queryTasks,
  unfavoriteView,
  updateView,
} from '@orbit/api/generated/sdk.gen'
import type { SavedViewRecord, TaskQueryBody, ViewCreateBody, Visibility } from '@orbit/api/generated/types.gen'

import { allPages, type LabelRecord, type OrbitClient, type ProjectRecord, type StatusRecord, type TaskRecord } from './orbit'
import { PRIORITY_LABEL, type Priority, sortStatuses, statusKeyOf } from './tasks'

// ---- the ViewState contract (the web's `viewState.ts`)

export type GroupOp = 'and' | 'or'
export type FilterField = 'status' | 'status_category' | 'assignee' | 'creator' | 'label' | 'priority' | 'project' | 'parent' | 'sub_issues' | 'due_date' | 'created_at' | 'updated_at' | 'text' | 'milestone' | 'cycle' | 'estimate'
export type FilterOperator = 'is' | 'is_not' | 'is_empty' | 'is_not_empty' | 'includes_any' | 'includes_all' | 'excludes' | 'before' | 'after' | 'between' | 'contains'
export type DateValue = { absolute: string } | { relative: 'today' | 'start_of_week' | 'end_of_week'; offset_days?: number }
export type Condition = { field: FilterField; operator: FilterOperator; value?: unknown }
export type FilterGroup = { op: GroupOp; children: FilterNode[] }
export type FilterNode = FilterGroup | Condition
export type Layout = 'list' | 'board' | 'timeline'
export type GroupBy = 'status' | 'assignee' | 'priority' | 'project' | 'label' | 'milestone' | 'cycle' | 'none'
export type OrderBy = 'manual' | 'priority' | 'created' | 'updated' | 'title' | 'due_date' | 'estimate'
export type ShowCompleted = 'all' | 'past_week' | 'past_month' | 'none'
export type SubIssuesMode = 'nested' | 'flat' | 'hidden'
export interface DisplayOptions {
  layout: Layout
  group_by: GroupBy
  sub_group_by: GroupBy
  order_by: OrderBy
  order_direction: 'asc' | 'desc'
  properties: string[]
  show_completed: ShowCompleted
  show_empty_groups: boolean
  sub_issues: SubIssuesMode
}
export interface ViewState {
  filter: FilterGroup
  display: DisplayOptions
}

/** The record with the hand-written `ViewState` (the generated type describes the same JSON more loosely). */
export type SavedView = Omit<SavedViewRecord, 'state'> & { state: ViewState | null }
export type { Visibility }

export const DEFAULT_DISPLAY: DisplayOptions = {
  layout: 'list',
  group_by: 'status',
  sub_group_by: 'none',
  order_by: 'manual',
  order_direction: 'asc',
  properties: ['id', 'status', 'assignee', 'priority', 'project', 'due_date', 'labels', 'sub_issue_progress'],
  show_completed: 'all',
  show_empty_groups: false,
  sub_issues: 'nested',
}

export const emptyFilter = (): FilterGroup => ({ op: 'and', children: [] })

export const defaultViewState = (): ViewState => ({ filter: emptyFilter(), display: { ...DEFAULT_DISPLAY, properties: [...DEFAULT_DISPLAY.properties] } })

/** A stored state with every display key: missing keys take the defaults, as the web's `normalizeViewState`. */
export function normalizeViewState(state: ViewState | null | undefined): ViewState {
  if (!state) return defaultViewState()
  return { filter: state.filter ?? emptyFilter(), display: { ...DEFAULT_DISPLAY, ...state.display } }
}

export const isGroup = (node: FilterNode): node is FilterGroup => 'op' in node && Array.isArray((node as FilterGroup).children)

const NOT_DONE: Condition = { field: 'status_category', operator: 'is_not', value: ['completed', 'cancelled', 'duplicate'] }
const MINE: Condition = { field: 'assignee', operator: 'is', value: ['me'] }
const THIS_WEEK: Condition = { field: 'due_date', operator: 'between', value: [{ relative: 'start_of_week' }, { relative: 'end_of_week' }] }

/** The drawer's task pages as filter trees, as the web's `PRESET_FILTERS`. */
export const PRESET_FILTERS: Record<string, FilterNode[]> = {
  mine: [MINE],
  overdue: [{ field: 'due_date', operator: 'before', value: { relative: 'today' } }, NOT_DONE],
  due_soon: [{ field: 'due_date', operator: 'between', value: [{ relative: 'today' }, { relative: 'today', offset_days: 6 }] }, NOT_DONE],
  current_week: [THIS_WEEK, NOT_DONE],
  my_week: [THIS_WEEK, NOT_DONE, MINE],
}

/** The `POST /tasks/query` body of a state. */
export function taskQueryBody(state: ViewState): TaskQueryBody {
  const { display } = state
  return {
    filter: state.filter as unknown as TaskQueryBody['filter'],
    order_by: display.order_by,
    order_direction: display.order_direction,
    show_completed: display.show_completed,
    ...(display.sub_issues === 'hidden' ? { sub_issues: 'hidden' as const } : {}),
  }
}

// ---- the phone's filter rows

/** The due-date choices of the Filter sheet. */
export type DueFilter = 'overdue' | 'today' | 'this_week' | 'next_7_days' | 'none' | 'any'

export const DUE_LABEL: Record<DueFilter, string> = {
  overdue: 'Overdue',
  today: 'Due today',
  this_week: 'Due this week',
  next_7_days: 'Due in the next 7 days',
  none: 'No due date',
  any: 'Has a due date',
}

/**
 * The filter the phone edits: one row per field, each "is any of". A view's other conditions (a web filter with
 * "is not", nested groups, dates of its own) stay in `extra`, unchanged, and still apply.
 */
export interface QuickFilter {
  statusCategory: string[]
  assignee: string[]
  priority: string[]
  project: string[]
  label: string[]
  due: DueFilter | null
  text: string
}

export const EMPTY_QUICK: QuickFilter = { statusCategory: [], assignee: [], priority: [], project: [], label: [], due: null, text: '' }

const LIST_FIELDS = { statusCategory: 'status_category', assignee: 'assignee', priority: 'priority', project: 'project' } as const

const DUE_CONDITION: Record<DueFilter, Condition> = {
  overdue: { field: 'due_date', operator: 'before', value: { relative: 'today' } },
  today: { field: 'due_date', operator: 'between', value: [{ relative: 'today' }, { relative: 'today' }] },
  this_week: THIS_WEEK,
  next_7_days: { field: 'due_date', operator: 'between', value: [{ relative: 'today' }, { relative: 'today', offset_days: 6 }] },
  none: { field: 'due_date', operator: 'is_empty' },
  any: { field: 'due_date', operator: 'is_not_empty' },
}

const stringList = (value: unknown): string[] | null => (Array.isArray(value) && value.length > 0 && value.every((item) => typeof item === 'string') ? value : null)

const sameJson = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b)

/** The due choice a condition is, or `null` when it is another date filter. */
function dueOf(condition: Condition): DueFilter | null {
  const value = condition.value ?? undefined
  for (const [due, known] of Object.entries(DUE_CONDITION) as [DueFilter, Condition][]) {
    if (known.operator === condition.operator && sameJson(known.value, value)) return due
  }
  return null
}

/** Splits a filter tree into the phone's rows and the conditions it keeps as they are. */
export function quickFromFilter(filter: FilterGroup): { quick: QuickFilter; extra: FilterNode[] } {
  if (filter.op !== 'and') return { quick: { ...EMPTY_QUICK }, extra: filter.children.length > 0 ? [filter] : [] }
  const quick: QuickFilter = { ...EMPTY_QUICK }
  const extra: FilterNode[] = []
  for (const node of filter.children) {
    if (isGroup(node)) {
      extra.push(node)
      continue
    }
    const list = stringList(node.value)
    const slot = (Object.entries(LIST_FIELDS) as [keyof typeof LIST_FIELDS, FilterField][]).find(([, field]) => field === node.field)?.[0]
    if (slot && node.operator === 'is' && list && quick[slot].length === 0) quick[slot] = [...list]
    else if (node.field === 'label' && node.operator === 'includes_any' && list && quick.label.length === 0) quick.label = [...list]
    else if (node.field === 'text' && node.operator === 'contains' && typeof node.value === 'string' && !quick.text) quick.text = node.value
    else if (node.field === 'due_date' && quick.due === null && dueOf(node)) quick.due = dueOf(node)
    else extra.push(node)
  }
  return { quick, extra }
}

/** The filter tree of the phone's rows, with the kept conditions after them. */
export function filterFromQuick(quick: QuickFilter, extra: readonly FilterNode[] = []): FilterGroup {
  const children: FilterNode[] = []
  for (const [slot, field] of Object.entries(LIST_FIELDS) as [keyof typeof LIST_FIELDS, FilterField][]) {
    if (quick[slot].length > 0) children.push({ field, operator: 'is', value: [...quick[slot]] })
  }
  if (quick.label.length > 0) children.push({ field: 'label', operator: 'includes_any', value: [...quick.label] })
  if (quick.due) children.push(DUE_CONDITION[quick.due])
  if (quick.text.trim()) children.push({ field: 'text', operator: 'contains', value: quick.text.trim() })
  return { op: 'and', children: [...children, ...extra] }
}

/** How many rows of the Filter sheet are set: the dot on the Filter button. */
export function quickCount(quick: QuickFilter): number {
  return [quick.statusCategory, quick.assignee, quick.priority, quick.project, quick.label].filter((list) => list.length > 0).length + (quick.due ? 1 : 0)
}

// ---- grouping

export interface TaskSection {
  key: string
  label: string
  /** The status that draws a status group's icon. */
  status?: StatusRecord
  /** A label's color. */
  color?: string
  /** A project's color and icon. */
  project?: { color: string; icon_url?: string | null }
  priority?: Priority
  /** A person group's user id. */
  userId?: string
  tasks: TaskRecord[]
}

export interface GroupContext {
  statuses: readonly StatusRecord[]
  people: readonly { id: string; name: string }[]
  labels: readonly LabelRecord[]
  projects: readonly ProjectRecord[]
  userId: string
}

export const GROUP_LABEL: Record<GroupBy, string> = {
  status: 'Status',
  assignee: 'Assignee',
  priority: 'Priority',
  project: 'Project',
  label: 'Label',
  milestone: 'Milestone',
  cycle: 'Cycle',
  none: 'No grouping',
}

/** The groupings the phone offers; milestone and cycle groups need one project and show as no grouping. */
export const PHONE_GROUPS: readonly GroupBy[] = ['status', 'assignee', 'priority', 'project', 'label', 'none']

export const ORDER_LABEL: Record<OrderBy, string> = {
  manual: 'Manual',
  priority: 'Priority',
  created: 'Created',
  updated: 'Updated',
  title: 'Title',
  due_date: 'Due date',
  estimate: 'Estimate',
}

const PRIORITIES: readonly Priority[] = ['urgent', 'high', 'medium', 'low', 'none']

const labelKey = (label: Pick<LabelRecord, 'name'>) => label.name.trim().toLowerCase()

/**
 * The tasks in groups, each group in the server's order. A task with two assignees or labels is in both groups, as on
 * the web; an unknown value goes to the "No …" group. Empty groups are left out.
 */
export function groupViewTasks(tasks: readonly TaskRecord[], groupBy: GroupBy, ctx: GroupContext): TaskSection[] {
  const sections: TaskSection[] = []
  const section = (key: string, fields: Omit<TaskSection, 'key' | 'tasks'>) => {
    let found = sections.find((item) => item.key === key)
    if (!found) {
      found = { key, ...fields, tasks: [] }
      sections.push(found)
    }
    return found
  }
  switch (groupBy) {
    case 'status': {
      for (const status of sortStatuses(ctx.statuses)) section(statusKeyOf(status), { label: status.name, status })
      const keyOf = new Map(ctx.statuses.map((status) => [status.id, statusKeyOf(status)]))
      for (const task of tasks) {
        const key = keyOf.get(task.status_id)
        ;(key ? section(key, { label: '' }) : section('none', { label: 'No status' })).tasks.push(task)
      }
      break
    }
    case 'priority':
      for (const priority of PRIORITIES) section(priority, { label: PRIORITY_LABEL[priority], priority })
      for (const task of tasks) section(PRIORITIES.includes(task.priority as Priority) ? task.priority : 'none', { label: '' }).tasks.push(task)
      break
    case 'assignee': {
      const people = [...ctx.people.filter((person) => person.id === ctx.userId), ...ctx.people.filter((person) => person.id !== ctx.userId).sort((a, b) => a.name.localeCompare(b.name))]
      for (const person of people) section(person.id, { label: person.name, userId: person.id })
      section('none', { label: 'No assignee' })
      const known = new Set(people.map((person) => person.id))
      for (const task of tasks) {
        const ids = [...new Set(task.assignee_ids)].filter((id) => known.has(id))
        for (const id of ids.length > 0 ? ids : ['none']) section(id, { label: '' }).tasks.push(task)
      }
      break
    }
    case 'project': {
      for (const project of [...ctx.projects].sort((a, b) => a.name.localeCompare(b.name))) section(project.id, { label: project.name, project })
      for (const task of tasks) section(ctx.projects.some((project) => project.id === task.project_id) ? task.project_id : 'none', { label: 'No project' }).tasks.push(task)
      break
    }
    case 'label': {
      const byId = new Map(ctx.labels.map((label) => [label.id, label]))
      for (const label of [...ctx.labels].sort((a, b) => labelKey(a).localeCompare(labelKey(b)))) section(labelKey(label), { label: label.name, color: label.color })
      section('none', { label: 'No label' })
      for (const task of tasks) {
        const keys = [...new Set((task.label_ids ?? []).flatMap((id) => (byId.has(id) ? [labelKey(byId.get(id)!)] : [])))]
        for (const key of keys.length > 0 ? keys : ['none']) section(key, { label: '' }).tasks.push(task)
      }
      break
    }
    default:
      return tasks.length > 0 ? [{ key: 'all', label: 'All tasks', tasks: [...tasks] }] : []
  }
  return sections.filter((item) => item.tasks.length > 0)
}

// ---- the views list

export interface ViewSection {
  key: 'favorites' | 'mine' | 'workspace'
  label: string
  views: SavedView[]
}

/** The Views page: favorites first (in their order), then the user's own views, then the workspace's. */
export function viewSections(views: readonly SavedView[], search: string): ViewSection[] {
  const query = search.trim().toLowerCase()
  const shown = query ? views.filter((view) => `${view.name} ${view.description}`.toLowerCase().includes(query)) : views
  const favorites = shown.filter((view) => view.is_favorite).sort((a, b) => (a.favorite_position ?? 0) - (b.favorite_position ?? 0))
  const rest = shown.filter((view) => !view.is_favorite)
  const sections: ViewSection[] = [
    { key: 'favorites', label: 'Favorites', views: favorites },
    { key: 'mine', label: 'Your views', views: rest.filter((view) => view.visibility === 'personal') },
    { key: 'workspace', label: 'Workspace views', views: rest.filter((view) => view.visibility === 'workspace') },
  ]
  return sections.filter((section) => section.views.length > 0)
}

/** The favorites the drawer lists, in their order. */
export const favoriteViews = (views: readonly SavedView[]) => views.filter((view) => view.is_favorite).sort((a, b) => (a.favorite_position ?? 0) - (b.favorite_position ?? 0))

/** The list after a star tap: a new favorite goes last, as the server puts it. */
export function withFavorite(views: readonly SavedView[], viewId: string, favorite: boolean): SavedView[] {
  const positions = views.filter((view) => view.is_favorite && view.id !== viewId).map((view) => view.favorite_position ?? 0)
  const position = favorite ? (positions.length > 0 ? Math.max(...positions) + 1 : 0) : null
  return views.map((view) => (view.id === viewId ? { ...view, is_favorite: favorite, favorite_position: position } : view))
}

/** "Status, Assignee · 3 filters": the subtitle of a view row. */
export function viewSummary(view: SavedView): string {
  if (!view.state) return view.state_error ? 'This view cannot be read' : ''
  const state = normalizeViewState(view.state)
  const count = countConditions(state.filter)
  const group = state.display.group_by === 'none' ? 'No grouping' : `By ${GROUP_LABEL[state.display.group_by].toLowerCase()}`
  return `${count === 0 ? 'No filters' : `${count} ${count === 1 ? 'filter' : 'filters'}`} · ${group}`
}

export function countConditions(group: FilterGroup): number {
  return group.children.reduce((sum, node) => sum + (isGroup(node) ? countConditions(node) : 1), 0)
}

// ---- API

export async function views(client: OrbitClient, workspaceId: string): Promise<SavedView[]> {
  return (await listViews({ client, path: { workspace_id: workspaceId }, throwOnError: true })).data as unknown as SavedView[]
}

export async function view(client: OrbitClient, workspaceId: string, viewId: string): Promise<SavedView> {
  return (await getView({ client, path: { workspace_id: workspaceId, view_id: viewId }, throwOnError: true })).data as unknown as SavedView
}

export interface ViewFields {
  name: string
  description: string
  visibility: Visibility
}

export async function newView(client: OrbitClient, workspaceId: string, fields: ViewFields, state: ViewState): Promise<SavedView> {
  const body: ViewCreateBody = { ...fields, state: state as unknown as ViewCreateBody['state'] }
  return (await createView({ client, path: { workspace_id: workspaceId }, body, throwOnError: true })).data as unknown as SavedView
}

export async function saveView(client: OrbitClient, workspaceId: string, current: Pick<SavedView, 'id' | 'version'>, changes: Partial<ViewFields> & { state?: ViewState }): Promise<SavedView> {
  const { state, ...fields } = changes
  const body = { ...fields, ...(state ? { state: state as unknown as ViewCreateBody['state'] } : {}), expected_version: current.version }
  return (await updateView({ client, path: { workspace_id: workspaceId, view_id: current.id }, body, throwOnError: true })).data as unknown as SavedView
}

export async function removeView(client: OrbitClient, workspaceId: string, viewId: string): Promise<void> {
  await deleteView({ client, path: { workspace_id: workspaceId, view_id: viewId }, throwOnError: true })
}

export async function setViewFavorite(client: OrbitClient, workspaceId: string, viewId: string, favorite: boolean): Promise<void> {
  const path = { workspace_id: workspaceId, view_id: viewId }
  if (favorite) await favoriteView({ client, path, throwOnError: true })
  else await unfavoriteView({ client, path, throwOnError: true })
}

/** Every task of a filter, in the server's order (the server pages by 100). */
export function queryAllTasks(client: OrbitClient, workspaceId: string, body: TaskQueryBody): Promise<TaskRecord[]> {
  return allPages(async (cursor) => (await queryTasks({ client, path: { workspace_id: workspaceId }, body: { ...body, cursor, limit: 100 }, throwOnError: true })).data as { items: TaskRecord[]; next_cursor?: string | null })
}

/** Labels by name across projects (`Bug` of two projects is one choice), each with the ids it stands for. */
export function labelNameChoices(labels: readonly LabelRecord[]): { key: string; name: string; color: string; ids: string[] }[] {
  const choices: { key: string; name: string; color: string; ids: string[] }[] = []
  for (const label of [...labels].sort((a, b) => labelKey(a).localeCompare(labelKey(b)))) {
    const found = choices.find((choice) => choice.key === labelKey(label))
    if (found) found.ids.push(label.id)
    else choices.push({ key: labelKey(label), name: label.name, color: label.color, ids: [label.id] })
  }
  return choices
}

/** The label filter after a tap on one name: all its ids go in, or all go out when any is in. */
export function toggleLabelChoice(selected: readonly string[], ids: readonly string[]): string[] {
  return ids.some((id) => selected.includes(id)) ? selected.filter((id) => !ids.includes(id)) : [...selected, ...ids]
}
