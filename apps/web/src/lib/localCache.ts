import { useSyncExternalStore } from 'react'

/**
 * What Orbit keeps on this device so a reload does not read everything from the server again: chat data (IndexedDB)
 * and the images of chat messages (the service worker's cache, see `public/sw.js`). It belongs to one signed-in user:
 * sign-out, an expired session and another user clear it. Without the browser APIs (a private window, a test) every
 * function does nothing.
 */

const DATABASE = 'orbit-cache'
const STORE = 'entries'
/** The same name as in `public/sw.js`. */
export const MEDIA_CACHE = 'orbit-media-v1'
/** The response header in which the service worker notes the size of a cached file. */
const SIZE_HEADER = 'x-orbit-size'
const OWNER_KEY = 'orbit:cache:owner'

export interface CacheEntry<T = unknown> {
  key: string
  data: T
  savedAt: number
  /** The length of the stored JSON: what the settings show as the size. */
  size: number
}

let database: Promise<IDBDatabase | null> | null = null

function open(): Promise<IDBDatabase | null> {
  database ??= new Promise((resolve) => {
    if (typeof indexedDB === 'undefined') return resolve(null)
    try {
      const request = indexedDB.open(DATABASE, 1)
      request.onupgradeneeded = () => request.result.createObjectStore(STORE, { keyPath: 'key' })
      request.onsuccess = () => resolve(request.result)
      // Storage is blocked: Orbit works without the cache.
      request.onerror = () => resolve(null)
      request.onblocked = () => resolve(null)
    } catch {
      resolve(null)
    }
  })
  return database
}

async function run<T, F>(mode: IDBTransactionMode, work: (store: IDBObjectStore) => IDBRequest<T>, fallback: F): Promise<T | F> {
  const db = await open()
  if (!db) return fallback
  return new Promise((resolve) => {
    try {
      const request = work(db.transaction(STORE, mode).objectStore(STORE))
      request.onsuccess = () => resolve(request.result)
      request.onerror = () => resolve(fallback)
    } catch {
      resolve(fallback)
    }
  })
}

export function readEntries(): Promise<CacheEntry[]> {
  return run<CacheEntry[], CacheEntry[]>('readonly', (store) => store.getAll(), [])
}

export async function writeEntry(key: string, data: unknown) {
  const size = JSON.stringify(data).length
  await run('readwrite', (store) => store.put({ key, data, savedAt: Date.now(), size } satisfies CacheEntry), undefined)
}

export async function deleteEntries(keys: readonly string[]) {
  for (const key of keys) await run('readwrite', (store) => store.delete(key), undefined)
}

async function clearEntries() {
  await run('readwrite', (store) => store.clear(), undefined)
}

/** The choices for how much space the cached images may take, in megabytes. */
export const MEDIA_LIMITS_MB = [100, 250, 500, 1000] as const
export const DEFAULT_MEDIA_LIMIT_MB = 250
const LIMIT_KEY = 'orbit:cache:media_limit_mb'

export function parseMediaLimit(raw: string | null): number {
  const limit = Number(raw)
  return (MEDIA_LIMITS_MB as readonly number[]).includes(limit) ? limit : DEFAULT_MEDIA_LIMIT_MB
}

const limitListeners = new Set<() => void>()

/** The limit belongs to the device (it is about its disk), so it is in `localStorage`. */
export function getMediaLimit(): number {
  try {
    return parseMediaLimit(window.localStorage.getItem(LIMIT_KEY))
  } catch {
    return DEFAULT_MEDIA_LIMIT_MB
  }
}

export function setMediaLimit(megabytes: number) {
  try {
    window.localStorage.setItem(LIMIT_KEY, String(parseMediaLimit(String(megabytes))))
  } catch {
    // Storage is full or blocked: the default stays.
  }
  for (const listener of [...limitListeners]) listener()
  void trimMedia()
}

export function useMediaLimit(): [number, (megabytes: number) => void] {
  const limit = useSyncExternalStore(
    (listener) => {
      limitListeners.add(listener)
      return () => limitListeners.delete(listener)
    },
    getMediaLimit,
  )
  return [limit, setMediaLimit]
}

interface MediaFile {
  request: Request
  size: number
}

/** The cached files, oldest first: a cache lists its entries in the order they were put in. */
async function mediaFiles(): Promise<MediaFile[]> {
  if (typeof caches === 'undefined') return []
  try {
    const cache = await caches.open(MEDIA_CACHE)
    const files: MediaFile[] = []
    for (const request of await cache.keys()) {
      const size = Number((await cache.match(request))?.headers.get(SIZE_HEADER))
      files.push({ request, size: Number.isFinite(size) ? size : 0 })
    }
    return files
  } catch {
    return []
  }
}

/** The oldest files to remove so that the rest fits in `limit` bytes. */
export function overLimit<T extends { size: number }>(files: readonly T[], limit: number): T[] {
  let total = files.reduce((sum, file) => sum + file.size, 0)
  const remove: T[] = []
  for (const file of files) {
    if (total <= limit) break
    remove.push(file)
    total -= file.size
  }
  return remove
}

/** Removes the oldest cached images until the cache is within the limit. */
export async function trimMedia() {
  const remove = overLimit(await mediaFiles(), getMediaLimit() * 1_000_000)
  if (remove.length === 0) return
  const cache = await caches.open(MEDIA_CACHE)
  for (const file of remove) await cache.delete(file.request)
}

export interface CacheUsage {
  /** Cached chat lists (conversations, states, one message list for each conversation). */
  entries: number
  entryBytes: number
  mediaFiles: number
  mediaBytes: number
}

export async function cacheUsage(): Promise<CacheUsage> {
  const [entries, files] = await Promise.all([readEntries(), mediaFiles()])
  return {
    entries: entries.length,
    entryBytes: entries.reduce((sum, entry) => sum + entry.size, 0),
    mediaFiles: files.length,
    mediaBytes: files.reduce((sum, file) => sum + file.size, 0),
  }
}

/** `work`, or nothing after `ms`: a browser whose storage never answers must not hold up sign-out or chat. */
export function withinTime(work: Promise<unknown>, ms = 2000): Promise<void> {
  return new Promise((resolve) => {
    const timer = setTimeout(resolve, ms)
    void work.catch(() => {}).finally(() => {
      clearTimeout(timer)
      resolve()
    })
  })
}

/** Removes everything Orbit cached on this device. Settings (theme, drafts, this limit) are not a cache and stay. */
export async function clearLocalCache() {
  await clearEntries()
  if (typeof caches !== 'undefined') await caches.delete(MEDIA_CACHE).catch(() => false)
}

/**
 * Makes the cache the cache of `userId`: what another user left on this device is removed first. Resolves when the
 * cache may be read.
 */
export async function claimLocalCache(userId: string) {
  let owner: string | null = null
  try {
    owner = window.localStorage.getItem(OWNER_KEY)
  } catch {
    // Storage is blocked: there is no cache to claim.
    return
  }
  if (owner === userId) return
  await clearLocalCache()
  try {
    window.localStorage.setItem(OWNER_KEY, userId)
  } catch {
    // The next start clears again: nothing is read from an unclaimed cache.
  }
}
