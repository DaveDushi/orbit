import { describe, expect, test } from 'bun:test'
import type { ProjectRecord, StatusRecord } from '@orbit/api/generated/types.gen'

import { canDeleteStatus, categoryStatuses, draftChanged, moveStatus, projectKey, projectUpdate } from './project-settings'

const project = { id: 'p', name: 'Web', key: 'WEB', color: '#8b5cf6', version: 7, private: false, auto_close_parent: true, auto_close_sub_issues: true } as ProjectRecord
const status = (id: string, category: string, position: number): StatusRecord => ({ id, category, position, project_id: 'p', version: position + 10, name: id, color: '#000', description: '', workspace_id: 'w' })

describe('projectUpdate', () => {
  test('always repeats name, tag, colour and the version', () => {
    expect(projectUpdate(project, { private: true })).toEqual({ name: 'Web', key: 'WEB', color: '#8b5cf6', expected_version: 7, private: true })
    expect(projectUpdate(project, { auto_close_parent: false })).toMatchObject({ expected_version: 7, auto_close_parent: false })
  })

  test('the name is trimmed', () => {
    expect(projectUpdate(project, { name: ' Site ', key: 'SITE', color: '#fff' }).name).toBe('Site')
  })

  test('draftChanged sees only real changes', () => {
    expect(draftChanged(project, { name: 'Web', key: 'WEB', color: '#8b5cf6' })).toBe(false)
    expect(draftChanged(project, { name: 'Web', key: 'WEBX', color: '#8b5cf6' })).toBe(true)
  })
})

test('projectKey keeps five upper-case letters and digits', () => {
  expect(projectKey('ab-c12 3x')).toBe('ABC12')
})

describe('moveStatus', () => {
  const own = [status('a', 'started', 0), status('b', 'started', 1), status('c', 'started', 2)]

  test('swaps with the neighbour and sends every position with its version', () => {
    expect(moveStatus(own, 'b', -1)).toEqual([
      { id: 'b', expected_version: 11, position: 0 },
      { id: 'a', expected_version: 10, position: 1 },
      { id: 'c', expected_version: 12, position: 2 },
    ])
  })

  test('nothing past either end', () => {
    expect(moveStatus(own, 'a', -1)).toBeNull()
    expect(moveStatus(own, 'c', 1)).toBeNull()
    expect(moveStatus(own, 'missing', 1)).toBeNull()
  })

  test('categoryStatuses sorts one category of one project', () => {
    const all = [status('x', 'started', 2), status('y', 'unstarted', 0), status('z', 'started', 1), { ...status('o', 'started', 0), project_id: 'other' }]
    expect(categoryStatuses(all, 'p', 'started').map((item) => item.id)).toEqual(['z', 'x'])
  })
})

describe('canDeleteStatus', () => {
  test('never the Duplicate status, never the last other one', () => {
    const duplicate = status('d', 'duplicate', 0)
    const only = status('o', 'unstarted', 0)
    expect(canDeleteStatus(duplicate, [duplicate, only])).toBe(false)
    expect(canDeleteStatus(only, [duplicate, only])).toBe(false)
    expect(canDeleteStatus(only, [duplicate, only, status('s', 'started', 0)])).toBe(true)
  })
})
