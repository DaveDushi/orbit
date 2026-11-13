/**
 * What another app shared into Orbit (the iOS share extension of `expo-sharing` opens the app with `orbit://expo-sharing`;
 * `app/+native-intent.ts` turns that into `/share`). `share-intent.web.ts` replaces this in the web preview.
 */
import { getSharedPayloads, useIncomingShare } from 'expo-sharing'

import type { SharedPayload } from './data/share'

export interface IncomingShare {
  payloads: SharedPayload[]
  resolving: boolean
  error: Error | null
  clear: () => void
}

export function useIncomingShareItems(): IncomingShare {
  const share = useIncomingShare()
  return { payloads: share.resolvedSharedPayloads, resolving: share.isResolving, error: share.error, clear: share.clearSharedPayloads }
}

/** Whether a share waits (for example one that came while signed out). */
export function hasPendingShare(): boolean {
  try {
    return getSharedPayloads().length > 0
  } catch {
    return false
  }
}
