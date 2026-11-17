import { useQueryClient } from '@tanstack/react-query'
import { type ReactNode, useEffect, useState } from 'react'

import { cacheStorage } from './cache-file'
import { cacheBuster, createQueryStore } from './data/persist'

/** The app's one query cache on disk (`data/persist.ts`). Sign-out deletes it (`session-context.tsx`). */
export const queryStore = createQueryStore(cacheStorage)

/**
 * Loads the signed-in person's saved lists before the screens render, so they open with the last data and refresh
 * in place, then keeps saving the cache while the session lasts.
 */
export function PersistedCache({ server, userId, children }: { server: string; userId: string; children: ReactNode }) {
  const queryClient = useQueryClient()
  const buster = cacheBuster(server, userId)
  const [restored, setRestored] = useState<string | null>(null)
  useEffect(() => {
    let stop: (() => void) | undefined
    let cancelled = false
    // an unreadable cache only means the screens load from the server
    void queryStore
      .restore(queryClient, buster)
      .catch(() => undefined)
      .then(() => {
        if (cancelled) return
        stop = queryStore.keep(queryClient, buster)
        setRestored(buster)
      })
    return () => {
      cancelled = true
      stop?.()
    }
  }, [buster, queryClient])
  return restored === buster ? children : null
}
