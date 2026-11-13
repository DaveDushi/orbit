import { expect, test } from 'bun:test'

import type { ConversationRecord } from './orbit'
import { MAX_SHARED_FILES, shareConversations, sharedContent, shareTargets, shareText, shareToNewTask, taskTitleOf } from './share'

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

test('a guest can share only into chat', () => {
  expect(shareTargets(true)).toEqual(['chat'])
  expect(shareTargets(false)).toEqual(['chat', 'drive', 'task'])
})

test('a share splits into files and text, links as text, at most 10 files', () => {
  const content = sharedContent([
    { contentType: 'image', contentUri: 'file:///group/IMG%201.jpg', contentMimeType: 'image/jpeg', contentSize: 10, originalName: null },
    { contentType: 'website', contentUri: 'https://example.com/a', value: 'https://example.com/a' },
    { contentType: 'text', contentUri: null, value: '  Look at this  ' },
    { contentType: 'file', contentUri: 'file:///group/x', contentMimeType: null, originalName: 'Quote.pdf' },
    { contentType: 'text', contentUri: null, value: '   ' },
  ])
  expect(content.files).toEqual([
    { uri: 'file:///group/IMG%201.jpg', name: 'IMG 1.jpg', mimeType: 'image/jpeg', size: 10 },
    { uri: 'file:///group/x', name: 'Quote.pdf', mimeType: 'application/octet-stream', size: null },
  ])
  expect(content.texts).toEqual(['https://example.com/a', 'Look at this'])
  const many = sharedContent(Array.from({ length: 12 }, (_, index) => ({ contentType: 'image' as const, contentUri: `file:///p${index}.png` })))
  expect(many.files).toHaveLength(MAX_SHARED_FILES)
})

test('the note comes before the shared text, and the title from the text or the first file', () => {
  expect(shareText(' Hi ', ['https://a.b'])).toBe('Hi\nhttps://a.b')
  expect(shareText('', [])).toBe('')
  expect(taskTitleOf({ files: [], texts: ['Fix the door\nmore'] })).toBe('Fix the door')
  expect(taskTitleOf({ files: [{ uri: 'u', name: 'Invoice 12.pdf', mimeType: 'application/pdf', size: 1 }], texts: [] })).toBe('Invoice 12')
  expect(taskTitleOf({ files: [], texts: [] })).toBe('')
})

test('only conversations the person is in and that are not archived, the agent DM first', () => {
  const list = shareConversations(
    [
      conversation('old', { last_message_at: '2026-10-01T00:00:00Z' }),
      conversation('new', { last_message_at: '2026-11-01T00:00:00Z' }),
      conversation('not-mine', { member_ids: ['other'] }),
      conversation('archived', { archived: true }),
      conversation('donna', { kind: 'dm', member_ids: ['me', 'bot'] }),
    ],
    'me',
    'bot',
  )
  expect(list.map((item) => item.id)).toEqual(['donna', 'new', 'old'])
})

test('a new task is made first, then every file goes to its id; a failed file does not stop the others', async () => {
  const calls: string[] = []
  const files = ['a.png', 'b.pdf', 'c.txt'].map((name) => ({ uri: `file:///${name}`, name, mimeType: 'x/y', size: 1 }))
  const result = await shareToNewTask(
    {
      createTask: async () => {
        calls.push('create')
        return { id: 'task-7' }
      },
      attach: async (taskId, file) => {
        calls.push(`${taskId}:${file.name}`)
        if (file.name === 'b.pdf') throw new Error('too large')
      },
    },
    files,
  )
  expect(calls).toEqual(['create', 'task-7:a.png', 'task-7:b.pdf', 'task-7:c.txt'])
  expect(result).toEqual({ task: { id: 'task-7' }, failed: ['b.pdf'] })

  // no task, no uploads
  const none: string[] = []
  await expect(shareToNewTask({ createTask: async () => Promise.reject(new Error('refused')), attach: async () => none.push('x') }, files)).rejects.toThrow('refused')
  expect(none).toEqual([])
})
