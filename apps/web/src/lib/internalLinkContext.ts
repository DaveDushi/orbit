import { createContext, type ReactNode } from 'react'

/**
 * Lets a host draw the bare links to this app in rendered markdown its own way (chat: a task or page chip, or nothing
 * when a card stands in for the link). It gets the URL and `plain`, the default link, to fall back to.
 */
export const InternalLinkContext = createContext<((url: string, plain: ReactNode) => ReactNode) | null>(null)
