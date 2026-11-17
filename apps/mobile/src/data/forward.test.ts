import { expect, test } from 'bun:test'

import { FORWARD_LIMIT, forwardDestinations, forwardToEach, messageLink, taskTitleFromMessage, toggleDestination } from './forward'
import type { ConversationRecord } from './orbit'

const conversation = (id: string, extra: Partial<ConversationRecord>) =>
  ({ id, name: id, kind: 'public', member_ids: ['me'], archived: false, created_at: '2026-11-01T00:00:00Z', last_message_at: null, ...extra }) as ConversationRecord

test('destinations: joined and not archived, newest activity first, search ignores case and #', () => {
  const list = [
    conversation('general', { last_message_at: '2026-11-02T00:00:00Z' }),
    conversation('random', { last_message_at: '2026-11-05T00:00:00Z' }),
    conversation('old', { archived: true }),
    conversation('other', { member_ids: ['ada'] }),
  ]
  const title = (item: ConversationRecord) => `#${item.name}`
  expect(forwardDestinations(list, 'me', title).map((item) => item.id)).toEqual(['random', 'general'])
  expect(forwardDestinations(list, 'me', title, '#GEN').map((item) => item.id)).toEqual(['general'])
})

test('a sixth destination is not added; a selected one comes out', () => {
  const five = ['a', 'b', 'c', 'd', 'e']
  expect(FORWARD_LIMIT).toBe(5)
  expect(toggleDestination(five, 'f')).toEqual(five)
  expect(toggleDestination(five, 'c')).toEqual(['a', 'b', 'd', 'e'])
})

test('forwardToEach stops at the first failure and sends each comment after its forward', async () => {
  const calls: string[] = []
  const result = await forwardToEach(
    ['a', 'b', 'c'],
    async (id) => {
      calls.push(`forward ${id}`)
      if (id === 'b') throw new Error('refused')
    },
    async (id) => void calls.push(`comment ${id}`),
  )
  expect(result.sentIds).toEqual(['a'])
  expect((result.error as Error).message).toBe('refused')
  expect(calls).toEqual(['forward a', 'comment a', 'forward b'])
})

test('a failed comment does not stop the forwards', async () => {
  const result = await forwardToEach(['a', 'b'], async () => undefined, async () => Promise.reject(new Error('no')))
  expect(result).toEqual({ sentIds: ['a', 'b'] })
})

test('links and task titles match the web', () => {
  expect(messageLink('https://orbit.test/', { id: 'm1', conversation_id: 'c1', thread_root_id: null })).toBe('https://orbit.test/chat/c1?m=m1')
  expect(messageLink('https://orbit.test', { id: 'm2', conversation_id: 'c1', thread_root_id: 'm1' })).toBe('https://orbit.test/chat/c1/thread/m1?m=m2')
  expect(taskTitleFromMessage('\n  Fix the login  \nmore')).toBe('Fix the login')
  expect(taskTitleFromMessage(`${'word '.repeat(30)}`, 20)).toBe('word word word word…')
})
