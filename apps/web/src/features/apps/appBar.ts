import type { AppFailure } from './appReady'

/**
 * Two choices that this browser remembers per workspace and app: the person closed the bar of an app that sends no
 * ready signal, and the app sent the ready signal here before (so its silence is a failure, see `appReady.ts`).
 */

type Host = { localStorage: Pick<Storage, 'getItem' | 'setItem'> }

const closedKey = (workspaceId: string, slug: string) => `orbit:app-bar-closed:${workspaceId}:${slug}`
const knownReadyKey = (workspaceId: string, slug: string) => `orbit:app-ready-seen:${workspaceId}:${slug}`

function readFlag(host: Host, key: string): boolean {
  try {
    return host.localStorage.getItem(key) === '1'
  } catch {
    return false
  }
}

function rememberFlag(host: Host, key: string): void {
  try {
    host.localStorage.setItem(key, '1')
  } catch {
    // Storage full or disabled: the choice holds until the page loads again.
  }
}

export const readAppBarClosed = (host: Host, workspaceId: string, slug: string) => readFlag(host, closedKey(workspaceId, slug))
export const rememberAppBarClosed = (host: Host, workspaceId: string, slug: string) => rememberFlag(host, closedKey(workspaceId, slug))
export const readAppKnownReady = (host: Host, workspaceId: string, slug: string) => readFlag(host, knownReadyKey(workspaceId, slug))
export const rememberAppKnownReady = (host: Host, workspaceId: string, slug: string) => rememberFlag(host, knownReadyKey(workspaceId, slug))

/** Why the card "Sign in to <app>" shows. */
export type AppCard = Exclude<AppFailure, 'timeout'>

/** `ready`: did the app say it loaded? `null`: not yet, and the time to say so has not ended. */
export type AppFrameState = { ready: boolean | null; card: AppCard | null; signingIn: boolean; popupBlocked: boolean }

export type AppFrameEvent =
  | { type: 'ready' }
  | { type: 'failed'; failure: AppFailure }
  | { type: 'reload' }
  | { type: 'sign-in'; opened: boolean }
  | { type: 'show-app' }

export const initialAppFrame: AppFrameState = { ready: null, card: null, signingIn: false, popupBlocked: false }

/**
 * The signal hides the card at once. Reload starts the rule again. Sign in hides the card while the popup is open,
 * and the frame reloads after it; a blocked popup keeps the card, which says so. Show the app hides the card until
 * the next failure.
 */
export function nextAppFrame(state: AppFrameState, event: AppFrameEvent): AppFrameState {
  switch (event.type) {
    case 'ready':
      return { ...state, ready: true, card: null }
    case 'failed':
      return { ...state, ready: false, card: event.failure === 'timeout' ? state.card : event.failure }
    case 'reload':
      return initialAppFrame
    case 'sign-in':
      return event.opened ? { ...state, signingIn: true, popupBlocked: false, card: null } : { ...state, signingIn: false, popupBlocked: true }
    case 'show-app':
      return { ...state, card: null }
  }
}

type AppBarState = AppFrameState & { embedded: boolean; closed: boolean }

/**
 * The bar is for when something did not load. Without a signal in time it shows, so no blank page lacks a way out,
 * unless the person closed it: then the corner menu has the actions. The card shows over the frame also with the bar
 * closed, and replaces the help line; a blocked popup says so in the card, or else above the frame.
 */
export function appBarView({ embedded, ready, card, signingIn, popupBlocked, closed }: AppBarState): { bar: boolean; notice: 'popup-blocked' | 'help' | null; card: AppCard | null } {
  const bar = !embedded || (!closed && (ready === false || signingIn || popupBlocked))
  if (!embedded) return { bar, notice: null, card: null }
  if (card) return { bar, notice: null, card }
  return { bar, notice: popupBlocked ? 'popup-blocked' : bar ? 'help' : null, card: null }
}
