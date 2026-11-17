/**
 * Which queries the app keeps on disk, so the lists open at once on the next start. Only the workspace's content is
 * kept: never a token, a setting with a secret (storage buckets, mail servers, API keys) or an admin screen.
 */
import { createAsyncStoragePersister } from '@tanstack/query-async-storage-persister'
import type { Query, QueryClient } from '@tanstack/react-query'
import { type AsyncStorage, persistQueryClientRestore, persistQueryClientSubscribe } from '@tanstack/react-query-persist-client'

/** The first key part of each query that is kept. Everything else stays in memory only. */
const KEPT = new Set([
  'workspaces',
  'app-badge',
  'tasks',
  'task',
  'comments',
  'task-activity',
  'projects',
  'statuses',
  'labels',
  'milestones',
  'cycles',
  'members',
  'conversations',
  'chat-categories',
  'chat-states',
  'messages',
  'notifications',
  'pages',
  'teamspaces',
  'drive',
  'home-layout',
])

/** A query to keep: a kept resource, loaded without an error. Invitations are under `members` but hold invite links. */
export function shouldPersistQuery(query: Pick<Query, 'queryKey' | 'state'>): boolean {
  const [resource, , part] = query.queryKey
  return query.state.status === 'success' && typeof resource === 'string' && KEPT.has(resource) && !(resource === 'members' && part === 'invitations')
}

/** How long a kept cache is good for. The queries live as long in memory, so a list left a while ago is kept too. */
export const CACHE_MAX_AGE = 3 * 24 * 60 * 60 * 1000

/** A cache saved for another server or person is thrown away, not shown. Raise `v1` when the cached shapes change. */
export const cacheBuster = (server: string, userId: string) => `v1|${server}|${userId}`

/** The queries of one workspace: they go when the user switches to another one. */
export const isWorkspaceQuery = (queryKey: readonly unknown[], workspaceId: string) => queryKey.includes(workspaceId)

/**
 * The cache on disk of the signed-in session. `restore` loads it (a cache of another server or person is deleted, not
 * shown), `keep` saves every change from then on (at most every 2 seconds), and `forget` deletes it: no save that was
 * still waiting writes it again, until the next `restore`.
 */
export function createQueryStore(storage: AsyncStorage<string>, throttleTime = 2000) {
  let writable = false
  let writing: Promise<unknown> = Promise.resolve()
  const persister = createAsyncStoragePersister({
    key: 'orbit-query-cache',
    throttleTime,
    storage: {
      getItem: (key) => storage.getItem(key),
      removeItem: (key) => storage.removeItem(key),
      setItem: (key, value) => {
        if (!writable) return
        writing = Promise.resolve(storage.setItem(key, value))
        return writing
      },
    },
  })
  return {
    async restore(queryClient: QueryClient, buster: string) {
      writable = true
      await persistQueryClientRestore({ queryClient, persister, buster, maxAge: CACHE_MAX_AGE })
    },
    keep: (queryClient: QueryClient, buster: string) =>
      persistQueryClientSubscribe({
        queryClient,
        persister,
        buster,
        dehydrateOptions: { shouldDehydrateQuery: shouldPersistQuery, shouldDehydrateMutation: () => false },
      }),
    async forget() {
      writable = false
      await writing.catch(() => undefined)
      await persister.removeClient()
    },
  }
}

export type QueryStore = ReturnType<typeof createQueryStore>
