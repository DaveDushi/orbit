import { describe, expect, test } from 'bun:test'
import { pageAreaOf, pageContextFor, type PageItem } from './pageContext'

const task: PageItem = { kind: 'task', id: '0199c3a0-0000-7000-8000-00000000000a', title: 'GEN-12 Fix login' }
const folder: PageItem = { kind: 'drive_folder', id: '0199c3a0-0000-7000-8000-00000000000b', title: 'Invoices' }

describe('pageAreaOf', () => {
  test('names the area of each page, and none in Chat', () => {
    expect(pageAreaOf('/')).toBe('home')
    expect(pageAreaOf('/tasks')).toBe('tasks')
    expect(pageAreaOf('/tasks/GEN-12')).toBe('tasks')
    expect(pageAreaOf('/views/abc/GEN-12')).toBe('tasks')
    expect(pageAreaOf('/tasks-trash')).toBe('tasks')
    expect(pageAreaOf('/docs/abc')).toBe('docs')
    expect(pageAreaOf('/drive/abc')).toBe('drive')
    expect(pageAreaOf('/activity')).toBe('activity')
    expect(pageAreaOf('/settings/members')).toBe('settings')
    expect(pageAreaOf('/profile/general')).toBe('account')
    expect(pageAreaOf('/admin/users')).toBe('admin')
    expect(pageAreaOf('/apps/shop')).toBe('apps')
    expect(pageAreaOf('/somewhere')).toBe('other')
    // a prefix is a whole segment: `/docsx` is not Docs
    expect(pageAreaOf('/docsx')).toBe('other')
    for (const chat of ['/chat', '/chat/abc', '/chat/abc/thread/def', '/chat/unreads', '/dm/abc']) expect(pageAreaOf(chat)).toBeNull()
  })
})

describe('pageContextFor', () => {
  test('sends the open item with its page', () => {
    expect(pageContextFor('/tasks/GEN-12', task)).toEqual({ page: 'tasks', item: task })
    expect(pageContextFor('/drive/x', folder)).toEqual({ page: 'drive', item: folder })
  })

  test('drops an item of another area: a page that is gone cannot name it', () => {
    expect(pageContextFor('/docs/abc', task)).toEqual({ page: 'docs', item: null })
    expect(pageContextFor('/settings', folder)).toEqual({ page: 'settings', item: null })
    expect(pageContextFor('/activity', null)).toEqual({ page: 'activity', item: null })
  })

  test('is nothing in Chat', () => {
    expect(pageContextFor('/chat/abc', task)).toBeNull()
  })
})
