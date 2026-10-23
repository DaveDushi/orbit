import { expect, test } from 'bun:test'

import { buildMessageRows, chatSections, chatUnreadCount, conversationTitle, decodeMentions, type MessagePages, removeMessage, upsertMessage } from './chat'
import type { ConversationRecord, ConversationStateRecord, MessageRecord } from './orbit'

const conversation = (id: string, fields: Partial<ConversationRecord>): ConversationRecord => ({
  id,
  name: id,
  kind: 'public',
  archived: false,
  category_id: null,
  created_at: '2026-10-01T00:00:00Z',
  created_by: 'me',
  is_default: false,
  last_message_at: null,
  member_ids: ['me'],
  position: 0,
  self_dm: false,
  topic: '',
  ...fields,
})

const state = (conversation_id: string, fields: Partial<ConversationStateRecord>): ConversationStateRecord => ({
  conversation_id,
  favorite: false,
  last_read_message_id: null,
  mention_count: 0,
  notify: 'all',
  unread_count: 0,
  ...fields,
})

const message = (id: string, fields: Partial<MessageRecord>): MessageRecord =>
  ({
    id,
    author_id: 'ada',
    body: id,
    conversation_id: 'c1',
    created_at: '2026-10-23T10:00:00Z',
    kind: 'message',
    deleted: false,
    also_in_channel: false,
    forwarded: null,
    reply_to_id: null,
    thread_root_id: null,
    nonce: null,
    ...fields,
  }) as MessageRecord

const CONVERSATIONS = [
  conversation('general', { position: 1 }),
  conversation('random', { position: 0 }),
  conversation('design', { category_id: 'product' }),
  conversation('left', { member_ids: ['you'] }),
  conversation('old', { archived: true }),
  conversation('dm-ada', { kind: 'dm', name: '', member_ids: ['me', 'ada'], last_message_at: '2026-10-20T00:00:00Z' }),
  conversation('dm-me', { kind: 'dm', name: '', member_ids: ['me'], self_dm: true }),
  conversation('dm-bob', { kind: 'dm', name: '', member_ids: ['me', 'bob'], last_message_at: '2026-10-22T00:00:00Z' }),
  conversation('starred', { position: 5 }),
]

test('sections follow the web sidebar: favorites, categories, channels, then DMs with your own first', () => {
  const sections = chatSections(CONVERSATIONS, [{ id: 'product', name: 'Product', position: 0 }, { id: 'empty', name: 'Empty', position: 1 }], [state('starred', { favorite: true })], 'me')
  expect(sections.map((section) => [section.title, section.conversations.map((item) => item.id)])).toEqual([
    ['Favorites', ['starred']],
    ['Product', ['design']],
    ['Channels', ['random', 'general']],
    ['Direct messages', ['dm-me', 'dm-bob', 'dm-ada']],
  ])
})

test('unread count: all unread where notify is all, mentions elsewhere, nothing from left conversations', () => {
  const states = [
    state('general', { notify: 'all', unread_count: 4, mention_count: 1 }),
    state('random', { notify: 'mentions', unread_count: 9, mention_count: 2 }),
    state('design', { notify: 'muted', unread_count: 7, mention_count: 1 }),
    state('left', { notify: 'all', unread_count: 5 }),
  ]
  expect(chatUnreadCount(CONVERSATIONS, states, 'me')).toBe(7)
})

test('DM titles and mention tokens use names', () => {
  const people = [
    { id: 'me', name: 'Orbit Developer' },
    { id: 'ada', name: 'Ada Lovelace' },
  ]
  expect(conversationTitle(CONVERSATIONS[5]!, people, 'me')).toBe('Ada Lovelace')
  expect(conversationTitle(CONVERSATIONS[6]!, people, 'me')).toBe('Orbit Developer (you)')
  expect(decodeMentions('hi <@ada> see <#general>, <!here> <@zed>', people, [{ id: 'general', name: 'general' }])).toBe('hi @Ada Lovelace see #general, @here @unknown')
})

test('rows: day separators, and a group continues for the same author within five minutes', () => {
  const rows = buildMessageRows([
    message('m1', { created_at: '2026-10-22T10:00:00Z' }),
    message('m2', { created_at: '2026-10-23T10:00:00Z' }),
    message('m3', { created_at: '2026-10-23T10:04:00Z' }),
    message('m4', { created_at: '2026-10-23T10:05:00Z', author_id: 'bob' }),
    message('m5', { created_at: '2026-10-23T10:06:00Z', author_id: 'bob', reply_to_id: 'm4' }),
  ])
  expect(rows.map((row) => (row.type === 'day' ? 'day' : `${row.message.id}:${row.groupStart ? 'start' : 'cont'}`))).toEqual([
    'day',
    'm1:start',
    'day',
    'm2:start',
    'm3:cont',
    'm4:start',
    'm5:start',
  ])
})

const pages = (...items: string[][]): MessagePages => ({
  pages: items.map((ids, index) => ({ items: ids.map((id) => message(id, {})), before: index === 0 ? null : 'x', after: index === items.length - 1 ? null : 'y' })),
  pageParams: items.map(() => undefined),
})
const ids = (data: MessagePages) => data.pages.map((page) => page.items.map((item) => `${item.id}:${item.body}`))

test('a new message joins the newest page; an edit replaces its copy; a thread reply stays out', () => {
  let data = pages(['m1'], ['m2'])
  data = upsertMessage(data, message('m3', {}))
  data = upsertMessage(data, message('m1', { body: 'edited' }))
  data = upsertMessage(data, message('r1', { thread_root_id: 'm1' }))
  expect(ids(data)).toEqual([['m1:edited'], ['m2:m2', 'm3:m3']])
  expect(ids(removeMessage(data, 'm2'))).toEqual([['m1:edited'], ['m3:m3']])
})

test('a new message is not appended when the newest page is not loaded', () => {
  const data: MessagePages = { pages: [{ items: [message('m1', {})], before: null, after: 'm2' }], pageParams: [undefined] }
  expect(ids(upsertMessage(data, message('m9', {})))).toEqual([['m1:m1']])
})
