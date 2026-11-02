/**
 * The app's "I have loaded" signal. A parent page cannot tell a working frame from a refused one, so the app says so
 * with one `postMessage` to `window.top` (see docs/apps.md). Orbit's wrapper document between them has no scripts, so
 * it relays nothing. The signal hides the bar and the sign-in card: it has no other effect.
 *
 * Three failures show the card "Sign in to <app>": Orbit's own sign-in says the frame has no session (its framed
 * `login_required` page sends `orbit:app-login-required`), the wrapper's `frame-src` refuses a navigation (a login
 * page on another origin), or an app that sent the signal in this browser before is silent after its frame loaded.
 */

export const APP_READY_MESSAGE = 'orbit:app-ready'
export const APP_LOGIN_REQUIRED_MESSAGE = 'orbit:app-login-required'

/** Long enough for a sign-in that passes through a login proxy and Orbit on a phone. A late signal still hides the bar. */
export const APP_READY_TIMEOUT_MILLIS = 8_000

/** For an app known to send the signal: the time after the frame's last `load` before its silence is a failure. */
export const APP_LOADED_TIMEOUT_MILLIS = 3_000

/** `timeout`: an app never seen sending the signal is silent (the bar, no card). The others show the card. */
export type AppFailure = 'timeout' | 'silent' | 'login-required' | 'frame-blocked'

type MessageLike = Pick<MessageEvent, 'data' | 'origin' | 'source'>

const typeOf = (event: MessageLike) => (event.data as { type?: unknown } | null)?.type

/** Only the app's frame, from the app's origin, with this message. */
export function isAppReadyMessage(event: MessageLike, appOrigin: string, frame: unknown): boolean {
  return frame != null && event.source === frame && event.origin === appOrigin && typeOf(event) === APP_READY_MESSAGE
}

/** Only Orbit's own page in the app's frame: the app's origin cannot send it. */
export function isLoginRequiredMessage(event: MessageLike, orbitOrigin: string, frame: unknown): boolean {
  return frame != null && event.source === frame && event.origin === orbitOrigin && typeOf(event) === APP_LOGIN_REQUIRED_MESSAGE
}

type Host = {
  addEventListener: (type: 'message', listener: (event: MessageEvent) => void) => void
  removeEventListener: (type: 'message', listener: (event: MessageEvent) => void) => void
  setTimeout: (callback: () => void, millis: number) => number
  clearTimeout: (timer: number) => void
}

type Watch = { appOrigin: string; orbitOrigin: string; frame: () => unknown; knownReady: boolean }

export type AppWatch = {
  /** The app frame fired `load`. */
  loaded: () => void
  /** The wrapper's `frame-src` refused a navigation of the app frame. */
  blocked: () => void
  stop: () => void
}

/**
 * Calls `onFailure` once if no signal comes in time (8 s; for a known app 3 s after the frame's last `load`), or at
 * once for a detected failure, and `onReady` at each signal, also a late one. Loads after the signal or a failure do
 * not start the time again: an app may show pages without the signal after its first one.
 */
export function watchAppReady(host: Host, { appOrigin, orbitOrigin, frame, knownReady }: Watch, onReady: () => void, onFailure: (failure: AppFailure) => void): AppWatch {
  let timer: number | null = null
  const clear = () => {
    if (timer !== null) host.clearTimeout(timer)
    timer = null
  }
  const wait = (millis: number) => {
    clear()
    timer = host.setTimeout(() => {
      timer = null
      onFailure(knownReady ? 'silent' : 'timeout')
    }, millis)
  }
  const fail = (failure: AppFailure) => {
    clear()
    onFailure(failure)
  }
  const listener = (event: MessageEvent) => {
    const source = frame()
    if (isAppReadyMessage(event, appOrigin, source)) {
      clear()
      onReady()
    } else if (isLoginRequiredMessage(event, orbitOrigin, source)) fail('login-required')
  }
  wait(APP_READY_TIMEOUT_MILLIS)
  host.addEventListener('message', listener)
  return {
    loaded: () => {
      if (knownReady && timer !== null) wait(APP_LOADED_TIMEOUT_MILLIS)
    },
    blocked: () => fail('frame-blocked'),
    stop: () => {
      clear()
      host.removeEventListener('message', listener)
    },
  }
}

/**
 * Starts the app frame of the wrapper document (same origin as Orbit). The wrapper has no scripts and its frame only
 * `data-src`, so Orbit listens first and then sets `src`: no `load` or refusal comes before the listeners.
 */
export function startAppFrame(wrapper: Document | null | undefined, watch: Pick<AppWatch, 'loaded' | 'blocked'>): void {
  const frame = wrapper?.querySelector('iframe')
  const src = frame?.dataset.src
  if (!wrapper || !frame || !src || frame.getAttribute('src')) return
  wrapper.addEventListener('securitypolicyviolation', (event) => {
    if (event.effectiveDirective === 'frame-src') watch.blocked()
  }, true)
  frame.addEventListener('load', () => watch.loaded())
  frame.src = src
}
