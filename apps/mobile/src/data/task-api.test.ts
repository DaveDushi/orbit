import { expect, test } from 'bun:test'
import type { GithubLink } from '@orbit/api/generated/types.gen'

import { githubLocked, githubSourceIssue, newTaskBody } from './task-api'

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

test('the GitHub issue line comes only from a valid stored source issue', () => {
  const issue: GithubLink = { kind: 'issue', title: 'acme/api#12', url: 'https://github.com/acme/api/issues/12', state: 'active', source: true, repository: 'acme/api', number: 12, project_repositories: 1 }
  const pull: GithubLink = { kind: 'pull_request', title: 'PR', url: 'https://github.com/acme/api/pull/3', state: 'open', source: false }
  expect(githubSourceIssue([pull, issue])).toEqual({ label: 'GitHub issue #12', url: 'https://github.com/acme/api/issues/12' })
  // More than one repository in the project: the line names it.
  expect(githubSourceIssue([{ ...issue, project_repositories: 2 }])?.label).toBe('GitHub issue acme/api#12')
  // A task synced from a pull request, a mention or no links: no line.
  expect(githubSourceIssue([{ ...issue, kind: 'pull_request' }])).toBeNull()
  expect(githubSourceIssue([pull])).toBeNull()
  expect(githubSourceIssue(undefined)).toBeNull()
  // The URL is built from the stored values only, never from a bad one (or the link's own `url`).
  for (const repository of ['acme', 'acme/api/x', '../api', 'acme/..', 'evil.com/x?y', '-acme/api']) expect(githubSourceIssue([{ ...issue, repository }])).toBeNull()
  for (const number of [0, -1, 1.5, null]) expect(githubSourceIssue([{ ...issue, number }])).toBeNull()
  expect(githubSourceIssue([{ ...issue, url: 'https://evil.example/' }])?.url).toBe('https://github.com/acme/api/issues/12')
})
