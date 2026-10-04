import { hashKey, type QueryClient, type QueryKey } from '@tanstack/react-query'
import { deleteEntries, readEntries, writeEntry, type CacheEntry } from '@/lib/localCache'
import type { MessagePages } from './events'
import { chatKeys } from './keys'

/**
 * The copy of chat on this device (`@/lib/localCache`): the conversations, the categories, the read states and the
 * newest messages of each conversation. A reload shows the copy at once and reads the server behind it, so the copy
 * is never the last word.
 */

/** Message lists kept; the ones not saved for the longest time go first. */
export const MAX_MESSAGE_LISTS = 100
/** A copy older than this is not shown. */
const MAX_AGE_MS = 30 * 24 * 60 * 60_000
const WRITE_DELAY_MS = 1000

/** Lists that came from the copy and that the server has not confirmed yet, by query hash. */
const unverified = new Set<string>()

/** For chat queries: a list from the copy is stale until the server answers; after that only events change it. */
export const liveStaleTime = (query: { queryHash: string }) => (unverified.has(query.queryHash) ? 0 : Infinity)

/** What is kept of a chat query, or `undefined` for a query that has no copy (search, threads, pins, files). */
export function persistedData(key: QueryKey, data: unknown): unknown {
  if (key[0] !== 'chat' || data === undefined) return undefined
  if (key.length === 3 && (key[2] === 'conversations' || key[2] === 'categories' || key[2] === 'states')) return data
  // Only the main list of a conversation, not a window around an old message.
  if (key[2] !== 'messages' || (key[4] as { around?: string | null } | undefined)?.around !== null) return undefined
  const last = (data as MessagePages).pages.at(-1)
  // Only the newest page: a list that is scrolled back does not end at the present.
  if (!last || last.after !== null) return undefined
  return { pages: [last], pageParams: [{}] } satisfies MessagePages
}

/** The entries to show and the keys to remove: other workspaces stay, old and surplus message lists go. */
export function usableEntries(entries: readonly CacheEntry[], workspaceId: string, now: number): { keep: CacheEntry[]; remove: string[] } {
  const keep: CacheEntry[] = []
  const remove: string[] = []
  let lists = 0
  for (const entry of [...entries].sort((a, b) => b.savedAt - a.savedAt)) {
    let key: QueryKey
    try {
      key = JSON.parse(entry.key)
    } catch {
      remove.push(entry.key)
      continue
    }
    if (now - entry.savedAt > MAX_AGE_MS) remove.push(entry.key)
    else if (key[2] === 'messages' && ++lists > MAX_MESSAGE_LISTS) remove.push(entry.key)
    else if (key[1] === workspaceId) keep.push(entry)
  }
  return { keep, remove }
}

/** Puts the copy of this workspace's chat into the query cache. Queries that already have data keep it. */
export async function restoreChatCache(queryClient: QueryClient, workspaceId: string) {
  const { keep, remove } = usableEntries(await readEntries(), workspaceId, Date.now())
  void deleteEntries(remove)
  for (const entry of keep) {
    const key: QueryKey = JSON.parse(entry.key)
    if (queryClient.getQueryData(key) !== undefined) continue
    // The copy waits for its reader however long that takes; the number of lists is limited above.
    queryClient.setQueryDefaults(key, { gcTime: Infinity })
    // Before the data goes in: the copy must not be saved again as if it were new.
    unverified.add(hashKey(key))
    queryClient.setQueryData(key, entry.data, { updatedAt: entry.savedAt })
  }
}

/** Keeps the copy current with the query cache. Returns the function that stops it. */
export function persistChatCache(queryClient: QueryClient, workspaceId: string): () => void {
  const timers = new Map<string, ReturnType<typeof setTimeout>>()
  const unsubscribe = queryClient.getQueryCache().subscribe((event) => {
    if (event.type !== 'updated' || event.action.type !== 'success') return
    const { query } = event
    if (query.queryKey[0] !== 'chat' || query.queryKey[1] !== workspaceId) return
    // An answer of the server (not a change by an event) confirms a list from the copy.
    if (!event.action.manual) unverified.delete(query.queryHash)
    // A list that is not confirmed may have a gap before its newest messages: it is not saved again.
    if (unverified.has(query.queryHash) || persistedData(query.queryKey, query.state.data) === undefined) return
    clearTimeout(timers.get(query.queryHash))
    timers.set(
      query.queryHash,
      setTimeout(() => {
        timers.delete(query.queryHash)
        const data = persistedData(query.queryKey, query.state.data)
        if (data !== undefined && !unverified.has(query.queryHash)) void writeEntry(JSON.stringify(query.queryKey), data)
      }, WRITE_DELAY_MS),
    )
  })
  return () => {
    unsubscribe()
    for (const timer of timers.values()) clearTimeout(timer)
  }
}

/** A conversation the user left or lost: its messages do not stay on the device. */
export function forgetConversation(workspaceId: string, conversationId: string) {
  void deleteEntries([JSON.stringify(chatKeys.messages(workspaceId, conversationId))])
}
