import { createContext, useContext } from 'react'
import type { AskDonna } from './useAskDonna'

/**
 * The shell's Ask bar for the page (the Home "Ask Donna" card). `ask` is set where the bar shows (a wide window,
 * not Chat); `enabled` is whether the person can use the agent at all (`useAskDonna` is not `null`).
 */
export const AskDonnaContext = createContext<{ ask: AskDonna | null; enabled: boolean }>({ ask: null, enabled: false })

export function useAskDonnaContext() {
  return useContext(AskDonnaContext)
}
