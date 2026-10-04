// Orbit's service worker: shows pushed notifications, opens Orbit when one is clicked, and keeps the images of chat
// messages so a reload does not read them from the server again. Every other request goes to the network as if the
// worker were not there.

self.addEventListener('install', () => self.skipWaiting())
self.addEventListener('activate', (event) => event.waitUntil(self.clients.claim()))

/** An app path only: a pushed `url` never opens another site. */
function appPath(url) {
  if (typeof url !== 'string' || !url.startsWith('/')) return '/'
  try {
    // The parser decides: `/\\host` and a path with a tab in it name another site.
    const parsed = new URL(url, self.location.origin)
    return parsed.origin === self.location.origin ? parsed.pathname + parsed.search + parsed.hash : '/'
  } catch {
    return '/'
  }
}

/** The picture of who caused the notice (a path of the app), or the app's icon. */
function noticeIcon(icon) {
  return typeof icon === 'string' && icon.startsWith('/api/v1/users/') ? icon : '/icon-192.png'
}

self.addEventListener('push', (event) => {
  let notice = {}
  try {
    notice = event.data ? event.data.json() : {}
  } catch {
    // A push that is not JSON still shows a notification: browsers take the permission away after silent pushes.
  }
  const title = typeof notice.title === 'string' && notice.title ? notice.title : 'Orbit'
  event.waitUntil(
    // A visible Orbit window got the same notice and plays Orbit's sound: the system's sound would be a second one.
    // A hidden or frozen one may play nothing, so the system's sound stays.
    self.clients.matchAll({ type: 'window' }).then((windows) =>
      self.registration.showNotification(title, {
        body: typeof notice.body === 'string' ? notice.body : '',
        tag: typeof notice.tag === 'string' ? notice.tag : undefined,
        data: { url: appPath(notice.url) },
        icon: noticeIcon(notice.icon),
        badge: '/icon-192.png',
        silent: windows.some((client) => client.visibilityState === 'visible'),
      }),
    ),
  )
})

const OUTSIDE_APP = /^\/(login|setup|recovery|accept-invitation|oauth)(\/|$)/

self.addEventListener('notificationclick', (event) => {
  event.notification.close()
  const url = appPath(event.notification.data && event.notification.data.url)
  event.waitUntil(
    self.clients.matchAll({ type: 'window', includeUncontrolled: true }).then(async (windows) => {
      // A window on a page outside the app (sign-in, setup, an invitation) has nobody who takes the message.
      const inApp = windows.filter((client) => !OUTSIDE_APP.test(new URL(client.url).pathname))
      const open = inApp.find((client) => client.focused) || inApp.find((client) => client.visibilityState === 'visible') || inApp[0]
      if (!open) return self.clients.openWindow(url)
      open.postMessage({ type: 'orbit:navigate', url })
      try {
        await open.focus()
      } catch {
        // Some browsers refuse the focus; the page has the message and navigates anyway.
      }
    }),
  )
})

// The same name and header as in `src/lib/localCache.ts`, which limits and clears this cache.
const MEDIA_CACHE = 'orbit-media-v1'
const SIZE_HEADER = 'x-orbit-size'
const MAX_CACHED_FILE_BYTES = 10_000_000
// A chat file has an id of its own and never changes, so a copy of it never goes out of date.
const CHAT_FILE = /^\/api\/v1\/workspaces\/[^/]+\/chat\/files\/[^/]+$/

/** An image that may be kept. Other files (video, documents) are not: they are large and read in ranges. */
function keepable(response) {
  if (response.status !== 200 || !(response.headers.get('content-type') || '').startsWith('image/')) return false
  // Without a length the size is known only after the read (`keepImage` checks it again).
  return Number(response.headers.get('content-length') || 0) <= MAX_CACHED_FILE_BYTES
}

async function keepImage(request, response) {
  const blob = await response.blob()
  if (blob.size > MAX_CACHED_FILE_BYTES) return
  const headers = new Headers(response.headers)
  headers.set(SIZE_HEADER, String(blob.size))
  const cache = await caches.open(MEDIA_CACHE)
  await cache.put(request.url, new Response(blob, { status: 200, headers }))
}

self.addEventListener('fetch', (event) => {
  const { request } = event
  if (request.method !== 'GET' || request.headers.has('range')) return
  const url = new URL(request.url)
  if (url.origin !== self.location.origin || !CHAT_FILE.test(url.pathname)) return
  event.respondWith(
    caches
      .open(MEDIA_CACHE)
      .then((cache) => cache.match(request.url))
      .catch(() => undefined)
      .then((cached) => {
        if (cached) return cached
        return fetch(request).then((response) => {
          // A full cache or a blocked one must not break the image: the copy is a bonus.
          // Only a response that is kept is read a second time: a clone of a large download would fill the memory.
          if (keepable(response)) event.waitUntil(keepImage(request, response.clone()).catch(() => {}))
          return response
        })
      }),
  )
})
