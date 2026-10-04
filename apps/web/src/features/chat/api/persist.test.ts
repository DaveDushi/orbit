import { expect, test } from 'bun:test'
import type { CacheEntry } from '@/lib/localCache'
import { chatKeys } from './keys'
import { MAX_MESSAGE_LISTS, persistedData, usableEntries } from './persist'

const page = (after: string | null, id: string) => ({ items: [{ id }], before: null, after })

test('the copy keeps the lists and only the newest page of a main message list', () => {
  const conversations = [{ id: 'c1' }]
  expect(persistedData(chatKeys.conversations('w1'), conversations)).toBe(conversations)
  expect(persistedData(chatKeys.states('w1'), [])).toEqual([])

  const scrolledToEnd = { pages: [page('m50', 'm1'), page(null, 'm51')], pageParams: [{ before: 'm51' }, {}] }
  expect(persistedData(chatKeys.messages('w1', 'c1'), scrolledToEnd)).toEqual({ pages: [page(null, 'm51')], pageParams: [{}] })

  // A list that is scrolled back, a window around a message, and queries without a copy.
  expect(persistedData(chatKeys.messages('w1', 'c1'), { pages: [page('m50', 'm1')], pageParams: [{}] })).toBeUndefined()
  expect(persistedData(chatKeys.messages('w1', 'c1', 'm7'), { pages: [page(null, 'm7')], pageParams: [{}] })).toBeUndefined()
  expect(persistedData(chatKeys.pins('w1', 'c1'), [])).toBeUndefined()
  expect(persistedData(chatKeys.search('w1', { query: 'a' }), { pages: [] })).toBeUndefined()
  expect(persistedData(chatKeys.linkPreview('w1', 'https://example.com'), null)).toBeUndefined()
})

test('old copies and message lists over the limit are removed; other workspaces stay', () => {
  const now = 100 * 24 * 60 * 60_000
  const entry = (key: readonly unknown[], savedAt: number): CacheEntry => ({ key: JSON.stringify(key), data: null, savedAt, size: 4 })
  const lists = Array.from({ length: MAX_MESSAGE_LISTS + 2 }, (_, index) => entry(chatKeys.messages('w1', `c${index}`), now - index))
  const entries = [
    ...lists,
    entry(chatKeys.conversations('w1'), now - 5),
    entry(chatKeys.conversations('w2'), now - 5),
    entry(chatKeys.states('w1'), now - 31 * 24 * 60 * 60_000),
    { key: 'not json', data: null, savedAt: now, size: 4 },
  ]
  const { keep, remove } = usableEntries(entries, 'w1', now)
  expect(keep).toHaveLength(MAX_MESSAGE_LISTS + 1)
  expect(keep.every((kept) => JSON.parse(kept.key)[1] === 'w1')).toBe(true)
  // The two lists saved the longest time ago, the old states, and the entry that cannot be read.
  expect(remove.sort()).toEqual([lists.at(-1)!.key, lists.at(-2)!.key, JSON.stringify(chatKeys.states('w1')), 'not json'].sort())
})
