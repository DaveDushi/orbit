import { expect, test } from 'bun:test'

import { githubLocked, newTaskBody } from './task-api'

test('newTaskBody trims the text, leaves an empty description out, and sends every property', () => {
  expect(
    newTaskBody({
      title: '  Ship it ',
      description: '   ',
      projectId: 'p1',
      statusId: 's1',
      priority: 'high',
      assigneeIds: ['u1'],
      labelIds: ['l1', 'l2'],
      dueAt: '2026-10-25T07:00:00.000Z',
      dueStartAt: null,
      parentId: 't0',
      milestoneId: 'm1',
      cycleId: null,
      estimate: 3,
    }),
  ).toEqual({
    title: 'Ship it',
    project_id: 'p1',
    status_id: 's1',
    priority: 'high',
    assignee_ids: ['u1'],
    label_ids: ['l1', 'l2'],
    due_at: '2026-10-25T07:00:00.000Z',
    due_start_at: null,
    parent_task_id: 't0',
    milestone_id: 'm1',
    cycle_id: null,
    estimate: 3,
  })
  expect(newTaskBody({ title: 'x', description: ' Notes ', projectId: 'p', statusId: 's', priority: 'none', assigneeIds: [], labelIds: [], dueAt: null, dueStartAt: null, parentId: null, milestoneId: null, cycleId: null, estimate: null }).description).toBe('Notes')
})

test('the title and description lock while the task syncs from GitHub, and until its links are known', () => {
  const pull = { kind: 'pull_request', source: false, state: 'open', title: 'Fix', url: 'https://github.com/o/r/pull/1' }
  expect(githubLocked(undefined)).toBe(true)
  expect(githubLocked([])).toBe(false)
  expect(githubLocked([pull])).toBe(false)
  expect(githubLocked([pull, { ...pull, kind: 'issue', source: true }])).toBe(true)
})
