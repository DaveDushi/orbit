import { describe, expect, test } from 'bun:test'
import type { AgentStatus } from '@orbit/api/agent'

import { askDonnaAgent, clearPageItem, currentPageItem, pageAreaOf, pageContextFor, setPageItem } from './ask-donna'

const connection = (connected: boolean) => ({ connected, invalid: false, account_id: null, connected_at: null })
const status = (overrides: Partial<Extract<AgentStatus, { enabled: true }>> = {}): AgentStatus => ({
  enabled: true,
  online: true,
  can_connect: true,
  bot_user_id: 'bot-1',
  name: 'Donna',
  chatgpt: connection(true),
  login: null,
  ...overrides,
})

describe('askDonnaAgent', () => {
  test('a member whose ChatGPT is connected can ask the agent', () => {
    expect(askDonnaAgent(status(), false)).toEqual({ name: 'Donna', botId: 'bot-1' })
  })

  test('a guest never sees it, also with a connection', () => {
    expect(askDonnaAgent(status(), true)).toBeNull()
  })

  test('without the connection, the agent or its user it is hidden', () => {
    expect(askDonnaAgent(status({ chatgpt: connection(false) }), false)).toBeNull()
    expect(askDonnaAgent(status({ bot_user_id: null }), false)).toBeNull()
    expect(askDonnaAgent({ enabled: false }, false)).toBeNull()
    // the status is not loaded yet
    expect(askDonnaAgent(undefined, false)).toBeNull()
  })
})

describe('page context', () => {
  test('each tab is its area, and Chat has none', () => {
    expect(pageAreaOf('/')).toBe('home')
    expect(pageAreaOf('/tasks/abc')).toBe('tasks')
    expect(pageAreaOf('/docs')).toBe('docs')
    expect(pageAreaOf('/activity')).toBe('activity')
    expect(pageAreaOf('/settings')).toBe('settings')
    expect(pageAreaOf('/settings/members')).toBe('settings')
    expect(pageAreaOf('/settings/profile')).toBe('account')
    expect(pageAreaOf('/settings/admin/users')).toBe('admin')
    expect(pageAreaOf('/chat')).toBeNull()
    expect(pageAreaOf('/chat/abc/members')).toBeNull()
    // a prefix is not an area
    expect(pageAreaOf('/tasksx')).toBe('other')
  })

  test('the open item is sent only on its own area', () => {
    const task = { kind: 'task' as const, id: 't1', title: 'Ship it' }
    expect(pageContextFor('/tasks/t1', task)).toEqual({ page: 'tasks', item: task })
    expect(pageContextFor('/docs', task)).toEqual({ page: 'docs', item: null })
    expect(pageContextFor('/chat', task)).toBeNull()
  })

  test('a screen that goes away clears only its own item', () => {
    const first = { kind: 'doc' as const, id: 'p1', title: 'Plan' }
    const second = { kind: 'doc' as const, id: 'p2', title: 'Notes' }
    setPageItem(first)
    setPageItem(second)
    clearPageItem(first)
    expect(currentPageItem()).toBe(second)
    clearPageItem(second)
    expect(currentPageItem()).toBeNull()
  })
})
