import { expect, test } from 'bun:test'
import { QueryClient } from '@tanstack/react-query'

import { cacheBuster, createQueryStore, isWorkspaceQuery, shouldPersistQuery } from './persist'

const wait = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms))

function memory() {
  const items = new Map<string, string>()
  return { items, getItem: async (key: string) => items.get(key) ?? null, setItem: async (key: string, value: string) => void items.set(key, value), removeItem: async (key: string) => void items.delete(key) }
}

const ok = (queryKey: unknown[]) => ({ queryKey, state: { status: 'success' } }) as Parameters<typeof shouldPersistQuery>[0]

test('only the workspace content is kept on disk: no settings with secrets, no invitations, no failed loads', () => {
  expect([['tasks', 'w'], ['messages', 'w', 'c'], ['workspaces'], ['drive', 'w', 'children', 'top']].map((key) => shouldPersistQuery(ok(key)))).toEqual([true, true, true, true])
  expect([['workspace-bucket', 'w'], ['admin', 'users'], ['members', 'w', 'invitations'], ['chat-search', 'w', {}], ['push-devices']].map((key) => shouldPersistQuery(ok(key)))).toEqual([false, false, false, false, false])
  expect(shouldPersistQuery({ queryKey: ['tasks', 'w'], state: { status: 'error' } } as Parameters<typeof shouldPersistQuery>[0])).toBe(false)
  expect(isWorkspaceQuery(['messages', 'w1', 'c'], 'w1')).toBe(true)
  expect(isWorkspaceQuery(['workspaces'], 'w1')).toBe(false)
})

test('the cache comes back for the same person, never for another, and sign-out deletes it for good', async () => {
  const storage = memory()
  const store = createQueryStore(storage, 5)
  const before = new QueryClient()
  const buster = cacheBuster('https://orbit.example.com', 'u1')
  await store.restore(before, buster)
  const stop = store.keep(before, buster)
  before.setQueryData(['tasks', 'w'], [{ id: 't1' }])
  before.setQueryData(['workspace-bucket', 'w'], { secret: 'no' })
  await wait(30)

  const after = new QueryClient()
  await store.restore(after, buster)
  expect(after.getQueryData(['tasks', 'w'])).toEqual([{ id: 't1' }])
  expect(after.getQueryData(['workspace-bucket', 'w'])).toBeUndefined()

  const other = new QueryClient()
  await store.restore(other, cacheBuster('https://orbit.example.com', 'u2'))
  expect(other.getQueryData(['tasks', 'w'])).toBeUndefined()

  // sign-out while a save still waits for its turn: nothing is written back
  await store.restore(before, buster)
  before.setQueryData(['tasks', 'w'], [{ id: 't2' }])
  await store.forget()
  before.clear()
  stop()
  await wait(30)
  expect(storage.items.size).toBe(0)
})
