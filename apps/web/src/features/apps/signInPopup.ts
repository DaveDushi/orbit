/**
 * The way out for an app whose login refuses frames. A page cannot reliably detect a frame that the browser refused,
 * so the control is always there and the person chooses it. The popup holds the app's first-party sign-in; the frame
 * reloads after it.
 */

/** Must run in the click, or the browser blocks it. `null`: it was blocked. */
export function openSignInPopup(host: Pick<Window, 'open'>, url: string): Window | null {
  // Blank first, so Orbit keeps a handle to the popup while the pages in it cannot reach Orbit through `opener`.
  const popup = host.open('about:blank', 'orbit-app-sign-in', 'popup,width=520,height=720')
  if (!popup) return null
  popup.opener = null
  popup.location.href = url
  return popup
}

const POLL_MILLIS = 500

/**
 * Calls `reload` when the popup closes, and again when the person comes back to this page. A login page with
 * `Cross-Origin-Opener-Policy` makes `closed` true while the popup is still open, so the first reload can be too early;
 * the return to this page is what catches that case. Stops after that return. The Reload button stays as the manual way out.
 */
export function watchSignInPopup(host: Pick<Window, 'addEventListener' | 'removeEventListener' | 'setInterval' | 'clearInterval'>, popup: Pick<Window, 'closed'>, reload: () => void): () => void {
  let closed = false
  const stop = () => {
    host.clearInterval(timer)
    host.removeEventListener('focus', onFocus)
  }
  const onFocus = () => {
    if (!closed && !popup.closed) return // only a look at this page: the sign-in goes on
    stop()
    reload()
  }
  const timer = host.setInterval(() => {
    if (closed || !popup.closed) return
    closed = true
    reload()
  }, POLL_MILLIS)
  host.addEventListener('focus', onFocus)
  return stop
}
