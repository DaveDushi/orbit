/**
 * The web preview has no share extension: a test puts the share in `sessionStorage['orbit-share']` (resolved payloads
 * as JSON) to see the Share screen.
 */
import { useState } from 'react'

import type { SharedPayload } from './data/share'
import type { IncomingShare } from './share-intent'

const KEY = 'orbit-share'

function stored(): SharedPayload[] {
  try {
    const value: unknown = JSON.parse(sessionStorage.getItem(KEY) ?? '[]')
    return Array.isArray(value) ? (value as SharedPayload[]) : []
  } catch {
    return []
  }
}

export function useIncomingShareItems(): IncomingShare {
  const [payloads, setPayloads] = useState(stored)
  return {
    payloads,
    resolving: false,
    error: null,
    clear: () => {
      sessionStorage.removeItem(KEY)
      setPayloads([])
    },
  }
}

export function hasPendingShare(): boolean {
  return stored().length > 0
}
