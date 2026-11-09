// The desktop bridge: runs before the page's own scripts, in the main frame of the main window,
// on the server's own pages only. It changes no Orbit code; it gives the web app what a browser
// would give it, and hands the result to the app through `window.ipc` (orbit-desktop/src/bridge.rs):
// - Notifications: `Notification` and `ServiceWorkerRegistration.showNotification` become OS
//   notifications. The permission is "granted": the app is the person's own install.
// - The unread badge: `navigator.setAppBadge` / `clearAppBadge` (the Badging API).
// - Links: a click on a link to another site goes to the app, which opens the default browser.
// - The update check: `/downloads/desktop/latest.json`, read with the page's own gateway cookies.
(() => {
  const ORIGIN = __ORBIT_ORIGIN__
  if (location.origin !== ORIGIN || location.pathname.startsWith('/__exe.dev') || window.__orbitDesktop) return
  const post = (message) => {
    try {
      window.ipc.postMessage(JSON.stringify(message))
    } catch {}
  }
  const shown = new Map()
  let next = 1

  const notify = (title, options, notification) => {
    const id = next++
    const data = options && options.data
    const url = data && typeof data.url === 'string' ? data.url : ''
    shown.set(id, { notification, url })
    if (shown.size > 50) shown.delete(shown.keys().next().value)
    post({ kind: 'notify', id, title: String(title), body: String((options && options.body) ?? ''), tag: String((options && options.tag) ?? ''), url })
  }

  class DesktopNotification extends EventTarget {
    static get permission() {
      return 'granted'
    }
    static requestPermission(callback) {
      if (typeof callback === 'function') callback('granted')
      return Promise.resolve('granted')
    }
    constructor(title, options = {}) {
      super()
      Object.assign(this, { title: String(title), body: String(options.body ?? ''), tag: String(options.tag ?? ''), data: options.data ?? null })
      this.onclick = this.onclose = this.onshow = this.onerror = null
      notify(title, options, this)
    }
    close() {}
  }
  window.Notification = DesktopNotification
  if (window.ServiceWorkerRegistration) {
    ServiceWorkerRegistration.prototype.showNotification = function (title, options = {}) {
      notify(title, options, null)
      return Promise.resolve()
    }
  }

  // The OS notification was clicked: do what the web app's own click handlers do.
  const clicked = (id) => {
    const entry = shown.get(id)
    if (!entry) return
    const { notification, url } = entry
    if (notification && typeof notification.onclick === 'function') {
      notification.onclick(new Event('click'))
    } else if (url && navigator.serviceWorker) {
      // The page follows its worker's "open this page" message (features/realtime/push.ts).
      navigator.serviceWorker.dispatchEvent(new MessageEvent('message', { data: { type: 'orbit:navigate', url } }))
    } else if (url) {
      location.assign(url)
    }
  }
  Object.defineProperty(window, '__orbitDesktop', { value: Object.freeze({ version: 1, clicked }) })

  navigator.setAppBadge = (count) => {
    post({ kind: 'badge', count: count === undefined ? null : Number(count) })
    return Promise.resolve()
  }
  navigator.clearAppBadge = () => {
    post({ kind: 'badge', count: 0 })
    return Promise.resolve()
  }

  document.addEventListener(
    'click',
    (event) => {
      if (event.button !== 0) return
      const link = event.target instanceof Element ? event.target.closest('a[href]') : null
      if (!link || link.hasAttribute('download')) return
      const url = link.href
      if (url === ORIGIN || url.startsWith(ORIGIN + '/') || !/^(https?|mailto|tel):/i.test(url)) return
      event.preventDefault()
      post({ kind: 'open', url })
    },
    true,
  )

  fetch('/downloads/desktop/latest.json', { credentials: 'same-origin', cache: 'no-store' })
    .then((response) => (response.ok ? response.text() : null))
    .then((json) => json && post({ kind: 'latest', json }))
    .catch(() => {})
})()
