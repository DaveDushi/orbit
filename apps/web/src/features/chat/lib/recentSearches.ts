import { useSyncExternalStore } from 'react'

/** Recent searches live in `localStorage` only: one list for each workspace and person, newest first. */
export const MAX_RECENT_SEARCHES = 10

const storageKey = (workspaceId: string, userId: string) => `orbit:chat:recent-searches:${workspaceId}:${userId}`
const EMPTY: readonly string[] = []

const listeners = new Set<() => void>()
/** Parsed lists, so reads during render do not parse JSON and snapshots stay stable. */
const cache = new Map<string, readonly string[]>()

function storage(): Storage | null {
  try {
    return globalThis.localStorage ?? null
  } catch {
    return null
  }
}

/** `query` first; an older entry that differs only in case or spaces goes, and the list keeps `max` entries. */
export function withRecentSearch(list: readonly string[], query: string, max = MAX_RECENT_SEARCHES): string[] {
  const text = query.trim().replace(/\s+/g, ' ')
  if (!text) return [...list]
  const key = text.toLowerCase()
  return [text, ...list.filter((entry) => entry.toLowerCase() !== key)].slice(0, max)
}

function read(key: string): readonly string[] {
  const cached = cache.get(key)
  if (cached) return cached
  let list: readonly string[] = EMPTY
  try {
    const parsed: unknown = JSON.parse(storage()?.getItem(key) ?? '[]')
    if (Array.isArray(parsed)) list = parsed.filter((entry): entry is string => typeof entry === 'string').slice(0, MAX_RECENT_SEARCHES)
  } catch {
    // Unreadable storage or JSON: start without recent searches.
  }
  cache.set(key, list)
  return list
}

function write(key: string, list: readonly string[]) {
  cache.set(key, list)
  try {
    if (list.length === 0) storage()?.removeItem(key)
    else storage()?.setItem(key, JSON.stringify(list))
  } catch {
    // Storage is full or blocked: the list still lives in memory for this page.
  }
  for (const listener of listeners) listener()
}

export function getRecentSearches(workspaceId: string, userId: string): readonly string[] {
  return read(storageKey(workspaceId, userId))
}

export function addRecentSearch(workspaceId: string, userId: string, query: string) {
  const key = storageKey(workspaceId, userId)
  const current = read(key)
  const next = withRecentSearch(current, query)
  if (next.length !== current.length || next.some((entry, index) => entry !== current[index])) write(key, next)
}

export function clearRecentSearches(workspaceId: string, userId: string) {
  write(storageKey(workspaceId, userId), EMPTY)
}

function subscribe(listener: () => void) {
  listeners.add(listener)
  return () => listeners.delete(listener)
}

export function useRecentSearches(workspaceId: string, userId: string | null): readonly string[] {
  return useSyncExternalStore(subscribe, () => (userId ? getRecentSearches(workspaceId, userId) : EMPTY))
}
