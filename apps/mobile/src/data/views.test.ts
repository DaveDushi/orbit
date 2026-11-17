import { expect, test } from 'bun:test'

import type { LabelRecord, ProjectRecord, StatusRecord, TaskRecord } from './orbit'
import { EMPTY_QUICK, type FilterGroup, filterFromQuick, groupViewTasks, labelNameChoices, normalizeViewState, PRESET_FILTERS, quickCount, quickFromFilter, type SavedView, toggleLabelChoice, viewSections, withFavorite } from './views'

const status = (id: string, name: string, category: string, position: number, project_id = 'p1') =>
  ({ id, name, category, position, project_id, color: '#888', description: '', version: 1, workspace_id: 'w' }) as StatusRecord

const task = (id: string, fields: Partial<TaskRecord> = {}) =>
  ({ id, title: id, number: 1, project_id: 'p1', status_id: 'todo', priority: 'none', assignee_ids: [], label_ids: [], due_at: null, updated_at: '2026-10-01T00:00:00Z', ...fields }) as TaskRecord

const view = (id: string, fields: Partial<SavedView> = {}) =>
  ({ id, name: id, description: '', visibility: 'personal', is_favorite: false, favorite_position: null, state: null, ...fields }) as SavedView

test('the phone rows round-trip through a filter tree', () => {
  const quick = { ...EMPTY_QUICK, statusCategory: ['started'], assignee: ['me'], priority: ['urgent', 'high'], project: ['p1'], label: ['l1'], due: 'this_week' as const, text: ' bug ' }
  const filter = filterFromQuick(quick)
  expect(filter.op).toBe('and')
  expect(filter.children).toHaveLength(7)
  const back = quickFromFilter(filter)
  expect(back.extra).toEqual([])
  expect(back.quick).toEqual({ ...quick, text: 'bug' })
  expect(quickCount(back.quick)).toBe(6)
})

test('conditions the phone cannot edit stay as they are, after the rows', () => {
  const nested: FilterGroup = { op: 'or', children: [{ field: 'priority', operator: 'is', value: ['low'] }] }
  const filter: FilterGroup = {
    op: 'and',
    children: [
      { field: 'assignee', operator: 'is_not', value: ['u2'] },
      nested,
      { field: 'created_at', operator: 'after', value: { relative: 'today', offset_days: -7 } },
      { field: 'priority', operator: 'is', value: ['high'] },
    ],
  }
  const { quick, extra } = quickFromFilter(filter)
  expect(quick.priority).toEqual(['high'])
  expect(quick.assignee).toEqual([])
  expect(extra).toHaveLength(3)
  // a change to the rows keeps every kept condition
  const next = filterFromQuick({ ...quick, priority: [] }, extra)
  expect(next.children).toEqual(extra)
})

test('an OR filter is kept whole', () => {
  const filter: FilterGroup = { op: 'or', children: [{ field: 'priority', operator: 'is', value: ['low'] }, { field: 'label', operator: 'is_empty' }] }
  const { quick, extra } = quickFromFilter(filter)
  expect(quick).toEqual(EMPTY_QUICK)
  expect(extra).toEqual([filter])
  expect(filterFromQuick(quick, extra)).toEqual({ op: 'and', children: [filter] })
})

test('the presets read as due rows plus kept conditions', () => {
  const { quick, extra } = quickFromFilter({ op: 'and', children: PRESET_FILTERS.overdue })
  expect(quick.due).toBe('overdue')
  expect(extra).toEqual([{ field: 'status_category', operator: 'is_not', value: ['completed', 'cancelled', 'duplicate'] }])
  expect(quickFromFilter({ op: 'and', children: PRESET_FILTERS.mine }).quick.assignee).toEqual(['me'])
})

test('a stored state without display keys takes the defaults', () => {
  const state = normalizeViewState({ filter: { op: 'and', children: [] }, display: { group_by: 'priority' } as never })
  expect(state.display.group_by).toBe('priority')
  expect(state.display.order_by).toBe('manual')
  expect(normalizeViewState(null).filter).toEqual({ op: 'and', children: [] })
})

const ctx = {
  statuses: [status('done', 'Done', 'completed', 3), status('todo', 'Todo', 'unstarted', 1), status('todo2', 'todo', 'unstarted', 1, 'p2')],
  people: [
    { id: 'u2', name: 'Bea' },
    { id: 'u1', name: 'Ana' },
    { id: 'me', name: 'Zoe' },
  ],
  labels: [
    { id: 'l1', name: 'Bug', color: '#f00', project_id: 'p1' },
    { id: 'l2', name: 'bug', color: '#f00', project_id: 'p2' },
  ] as LabelRecord[],
  projects: [{ id: 'p1', name: 'Web', color: '#00f' }] as ProjectRecord[],
  userId: 'me',
}

test('status groups merge same-named statuses and keep the server order inside a group', () => {
  const sections = groupViewTasks([task('b', { status_id: 'todo2' }), task('a'), task('c', { status_id: 'done' }), task('x', { status_id: 'gone' })], 'status', ctx)
  expect(sections.map((section) => [section.key, section.tasks.map((item) => item.id)])).toEqual([
    ['unstarted:todo', ['b', 'a']],
    ['completed:done', ['c']],
    ['none', ['x']],
  ])
})

test('assignee groups put me first and a task with two assignees in both', () => {
  const sections = groupViewTasks([task('a', { assignee_ids: ['u1', 'me'] }), task('b'), task('c', { assignee_ids: ['left'] })], 'assignee', ctx)
  expect(sections.map((section) => [section.label, section.tasks.map((item) => item.id)])).toEqual([
    ['Zoe', ['a']],
    ['Ana', ['a']],
    ['No assignee', ['b', 'c']],
  ])
})

test('label groups join same-named labels of two projects', () => {
  const sections = groupViewTasks([task('a', { label_ids: ['l1'] }), task('b', { label_ids: ['l2'] }), task('c')], 'label', ctx)
  expect(sections.map((section) => [section.label, section.tasks.length])).toEqual([
    ['Bug', 2],
    ['No label', 1],
  ])
})

test('no grouping is one list, and no tasks is no group', () => {
  expect(groupViewTasks([task('a')], 'none', ctx).map((section) => section.key)).toEqual(['all'])
  expect(groupViewTasks([], 'none', ctx)).toEqual([])
  expect(groupViewTasks([], 'priority', ctx)).toEqual([])
})

test('the views page lists favorites in order, then own, then workspace views, and searches', () => {
  const list = [
    view('w', { visibility: 'workspace' }),
    view('f2', { is_favorite: true, favorite_position: 2 }),
    view('mine', { description: 'sprint board' }),
    view('f1', { is_favorite: true, favorite_position: 1, visibility: 'workspace' }),
  ]
  expect(viewSections(list, '').map((section) => [section.key, section.views.map((item) => item.id)])).toEqual([
    ['favorites', ['f1', 'f2']],
    ['mine', ['mine']],
    ['workspace', ['w']],
  ])
  expect(viewSections(list, 'SPRINT').map((section) => section.views.map((item) => item.id))).toEqual([['mine']])
})

test('a new favorite goes last and an unstarred view loses its place', () => {
  const list = [view('a', { is_favorite: true, favorite_position: 4 }), view('b')]
  expect(withFavorite(list, 'b', true).find((item) => item.id === 'b')?.favorite_position).toBe(5)
  expect(withFavorite(list, 'a', false)[0]).toMatchObject({ is_favorite: false, favorite_position: null })
})

test('a label name stands for the same-named labels of every project', () => {
  const labels = [
    { id: 'l1', name: 'Bug', color: '#f00', project_id: 'p1' },
    { id: 'l3', name: 'UI', color: '#0f0', project_id: 'p1' },
    { id: 'l2', name: 'bug ', color: '#f00', project_id: 'p2' },
  ] as LabelRecord[]
  const choices = labelNameChoices(labels)
  expect(choices.map((choice) => [choice.name, choice.ids])).toEqual([
    ['Bug', ['l1', 'l2']],
    ['UI', ['l3']],
  ])
  expect(toggleLabelChoice([], choices[0].ids)).toEqual(['l1', 'l2'])
  // a view saved on the web with one of them: the tap takes it out
  expect(toggleLabelChoice(['l2', 'l3'], choices[0].ids)).toEqual(['l3'])
})
