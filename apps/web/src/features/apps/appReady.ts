/**
 * The app's "I have loaded" signal. A parent page cannot tell a working frame from a refused one, so the app says so
 * with one `postMessage` to `window.top` (see docs/apps.md). Orbit's wrapper document between them has no scripts, so
 * it relays nothing. The signal only hides the bar: it has no other effect.
 */

export const APP_READY_MESSAGE = 'orbit:app-ready'

/** Long enough for a sign-in that passes through a login proxy and Orbit on a phone. A late signal still hides the bar. */
export const APP_READY_TIMEOUT_MILLIS = 8_000

/** Only the app's frame, from the app's origin, with this message. */
export function isAppReadyMessage(event: Pick<MessageEvent, 'data' | 'origin' | 'source'>, appOrigin: string, frame: unknown): boolean {
  return frame != null && event.source === frame && event.origin === appOrigin && (event.data as { type?: unknown } | null)?.type === APP_READY_MESSAGE
}

type Host = {
  addEventListener: (type: 'message', listener: (event: MessageEvent) => void) => void
  removeEventListener: (type: 'message', listener: (event: MessageEvent) => void) => void
  setTimeout: (callback: () => void, millis: number) => number
  clearTimeout: (timer: number) => void
}

/** Calls `onTimeout` if no signal comes in time, and `onReady` at each signal, also a late one. Returns the stop. */
export function watchAppReady(host: Host, appOrigin: string, frame: () => unknown, onReady: () => void, onTimeout: () => void): () => void {
  const timer = host.setTimeout(onTimeout, APP_READY_TIMEOUT_MILLIS)
  const listener = (event: MessageEvent) => {
    if (!isAppReadyMessage(event, appOrigin, frame())) return
    host.clearTimeout(timer)
    onReady()
  }
  host.addEventListener('message', listener)
  return () => {
    host.clearTimeout(timer)
    host.removeEventListener('message', listener)
  }
}
