import { expect, test } from 'bun:test'

import type { ConversationRecord, PageSummary, ProjectRecord, TaskRecord } from './orbit'
import { searchAll, searchWords } from './search'

const projects = [{ id: 'p1', key: 'GEN' }] as ProjectRecord[]
const task = (id: string, fields: Partial<TaskRecord>): TaskRecord => ({ id, title: id, number: 1, project_id: 'p1', updated_at: '2026-10-01T00:00:00Z', ...fields }) as TaskRecord
const page = (id: string, title: string, updated_at = '2026-10-01T00:00:00Z'): PageSummary => ({ id, title, updated_at }) as PageSummary
const chat = (id: string, title: string, archived = false) => ({ conversation: { id, archived } as ConversationRecord, title })
const input = (fields: Partial<Parameters<typeof searchAll>[1]>) => ({ tasks: [], projects, pages: [], conversations: [], ...fields })

test('a blank query finds nothing', () => {
  expect(searchWords('   ')).toEqual([])
  expect(searchAll('  ', input({ tasks: [task('a', { title: 'Report' })], pages: [page('p', 'Report')] }))).toEqual({ tasks: [], pages: [], conversations: [] })
})

test('every word must match, in any order and case', () => {
  const tasks = [task('a', { title: 'Prepare the Q4 board report' }), task('b', { title: 'Board games night' })]
  expect(searchAll('REPORT board', input({ tasks })).tasks.map((item) => item.id)).toEqual(['a'])
})

test('a task matches by its identifier, and trashed tasks are left out', () => {
  const tasks = [task('a', { number: 12 }), task('b', { number: 3 }), task('c', { number: 12, project_id: 'p1', deleted_at: '2026-10-02T00:00:00Z' })]
  expect(searchAll('gen-12', input({ tasks })).tasks.map((item) => item.id)).toEqual(['a'])
})

test('the last changed tasks and pages come first, up to the limit', () => {
  const pages = [page('old', 'Notes', '2026-01-01T00:00:00Z'), page('new', 'Meeting notes', '2026-11-01T00:00:00Z'), page('mid', 'Notes 2', '2026-06-01T00:00:00Z')]
  expect(searchAll('notes', input({ pages }), 2).pages.map((item) => item.id)).toEqual(['new', 'mid'])
})

test('chats match by their shown title; archived ones are left out', () => {
  const conversations = [chat('c1', 'general'), chat('c2', 'Ada Lovelace'), chat('c3', 'general-old', true)]
  expect(searchAll('ada', input({ conversations })).conversations.map((item) => item.conversation.id)).toEqual(['c2'])
  expect(searchAll('general', input({ conversations })).conversations.map((item) => item.conversation.id)).toEqual(['c1'])
})
