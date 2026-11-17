// The desktop bridge: runs before the page's own scripts, in the main frame of the main window,
// on the server's own pages only. It changes no Orbit code; it gives the web app what a browser
// would give it, and hands the result to the app through `window.ipc` (orbit-desktop/src/bridge.rs):
// - Notifications: `Notification` and `ServiceWorkerRegistration.showNotification` become OS
//   notifications. The permission is "granted": the app is the person's own install.
// - The unread badge: `navigator.setAppBadge` / `clearAppBadge` (the Badging API).
// - Links: a click on a link to another site goes to the app, which opens the default browser.
// - The update check: `/downloads/desktop/latest.json`, read with the page's own gateway cookies, at the start and
//   every 6 hours (the app often runs for days).
// - `__orbitDesktop.app` (the app's version) and `.log(text)` (a line in the diagnostics log), for voice input reports.
// - Windows: `SpeechRecognition` records in the app and transcribes with a local Whisper model (src/speech.rs);
//   WebView2's recognizer refuses. A small status shows the model's one-time download and the transcription.
// - The app's state and menu (`getState`, `action`, the `orbit-desktop` event): the web app's account menu shows the
//   tray's items, because Windows 11 hides new tray icons. A banner says when an update is ready (no web deploy needed).
(() => {
  const ORIGIN = __ORBIT_ORIGIN__
  const APP = __ORBIT_APP_VERSION__
  const NATIVE_SPEECH = __ORBIT_NATIVE_SPEECH__
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
  // The part of the Web Speech API that the web app's voice input uses: the app sends `progress` (the model's
  // download, the transcription), the `final` text, `error` (a Web Speech error code and a text) and `end` per `id`.
  const dictations = new Map()
  let nextDictation = 1
  class DesktopSpeechRecognition {
    constructor() {
      Object.assign(this, { lang: '', continuous: false, interimResults: false, onresult: null, onerror: null, onend: null })
      this._id = 0
      this._finals = []
    }
    start() {
      if (this._id) throw new DOMException('Voice input is already running.', 'InvalidStateError')
      this._id = nextDictation++
      this._finals = []
      dictations.set(this._id, this)
      post({ kind: 'speech', action: 'start', id: this._id, lang: this.lang || navigator.language })
    }
    stop() {
      if (this._id) post({ kind: 'speech', action: 'stop', id: this._id })
    }
    abort() {
      if (!this._id) return
      post({ kind: 'speech', action: 'abort', id: this._id })
      this._end()
    }
    _end() {
      voiceStatus('')
      dictations.delete(this._id)
      this._id = 0
      if (typeof this.onend === 'function') this.onend()
    }
    _event(event) {
      if (event.type === 'progress') {
        voiceStatus(event.phase === 'download' ? `Downloading the speech model (60 MB, only once)… ${event.percent}%` : 'Transcribing…')
      } else if (event.type === 'final') {
        this._finals.push(event.text)
        const results = this._finals.map((transcript) => ({ isFinal: true, length: 1, 0: { transcript } }))
        if (typeof this.onresult === 'function') this.onresult({ resultIndex: 0, results })
      } else if (event.type === 'error') {
        if (typeof this.onerror === 'function') this.onerror({ error: event.error, message: event.message || '' })
      } else if (event.type === 'end') {
        this._end()
      }
    }
  }

  // The voice status, in a shadow root so the page's styles do not reach it; empty text hides it.
  let voiceHost = null
  const voiceStatus = (text) => {
    if (!text || !document.body) {
      if (voiceHost) voiceHost.remove()
      voiceHost = null
      return
    }
    if (!voiceHost) {
      voiceHost = document.createElement('orbit-desktop-voice')
      voiceHost.attachShadow({ mode: 'open' }).innerHTML = `<style>
        :host { all: initial; position: fixed; left: 50%; bottom: 88px; transform: translateX(-50%); z-index: 2147483647; }
        div { padding: 8px 14px; border-radius: 999px; font: 13px/1.3 system-ui, "Segoe UI", sans-serif;
          background: #18181b; color: #fafafa; box-shadow: 0 8px 24px #0004; }
      </style><div role="status" aria-live="polite"></div>`
      document.body.append(voiceHost)
    }
    voiceHost.shadowRoot.querySelector('div').textContent = text
  }
  const speech = (event) => {
    const dictation = event && dictations.get(event.id)
    if (dictation) dictation._event(event)
  }
  if (NATIVE_SPEECH) {
    window.SpeechRecognition = DesktopSpeechRecognition
    window.webkitSpeechRecognition = DesktopSpeechRecognition
  }

  // The app's state: { app, os, autostart, autostartLabel, update (a version or null), updating, install }.
  let state = null
  // "Later" holds until the app quits (sessionStorage survives a reload of the page, not a restart).
  const DISMISSED = 'orbit-desktop:update-later'
  const dismissed = () => {
    try {
      return sessionStorage.getItem(DISMISSED)
    } catch {
      return null
    }
  }
  const setState = (next) => {
    state = next
    window.dispatchEvent(new CustomEvent('orbit-desktop', { detail: state }))
    banner()
  }
  const action = (name, on) => post({ kind: 'action', name: String(name), on: Boolean(on) })

  // The update banner, in a shadow root so the page's styles do not reach it. "×" (Later) hides it for this version.
  let host = null
  const banner = () => {
    const show = state && state.update && dismissed() !== state.update
    if (!show) {
      if (host) host.remove()
      host = null
      return
    }
    if (!document.body) return document.addEventListener('DOMContentLoaded', banner, { once: true })
    if (!host) {
      host = document.createElement('orbit-desktop-update')
      const root = host.attachShadow({ mode: 'open' })
      root.innerHTML = `<style>
        :host { all: initial; position: fixed; right: 16px; bottom: 16px; z-index: 2147483647; }
        div { display: flex; align-items: center; gap: 10px; padding: 10px 10px 10px 14px; border-radius: 10px;
          font: 13px/1.3 system-ui, "Segoe UI", sans-serif; background: #18181b; color: #fafafa; box-shadow: 0 8px 24px #0004; }
        button { font: inherit; border: 0; border-radius: 7px; padding: 6px 10px; cursor: pointer; }
        .go { background: #e11d74; color: #fff; font-weight: 600; }
        .x { background: transparent; color: #a1a1aa; padding: 6px 8px; }
        .go:disabled { opacity: .6; cursor: default; }
      </style><div role="status"><span></span><button class="go"></button><button class="x" aria-label="Later" title="Later">✕</button></div>`
      root.querySelector('.go').addEventListener('click', () => action('update'))
      root.querySelector('.x').addEventListener('click', () => {
        try {
          sessionStorage.setItem(DISMISSED, state.update)
        } catch {}
        banner()
      })
      document.body.append(host)
    }
    const root = host.shadowRoot
    root.querySelector('span').textContent = state.updating ? `Downloading Orbit ${state.update}…` : `Orbit ${state.update} is available.`
    const go = root.querySelector('.go')
    go.textContent = state.install ? 'Restart to update' : 'Download'
    go.disabled = Boolean(state.updating)
  }

  Object.defineProperty(window, '__orbitDesktop', {
    value: Object.freeze({
      version: 2,
      app: APP,
      clicked,
      speech,
      log: (text) => post({ kind: 'log', text: String(text) }),
      getState: () => state,
      setState,
      action,
      checkUpdate: () => checkUpdate(),
    }),
  })

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

  // An empty `json` says the check failed (the app tells the person after a check from its menu).
  const checkUpdate = () =>
    fetch('/downloads/desktop/latest.json', { credentials: 'same-origin', cache: 'no-store' })
      .then((response) => (response.ok ? response.text() : ''))
      .catch(() => '')
      .then((json) => post({ kind: 'latest', json }))
  checkUpdate()
  setInterval(checkUpdate, 6 * 60 * 60 * 1000)
  post({ kind: 'ready' })
})()
