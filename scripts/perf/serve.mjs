#!/usr/bin/env node
// Serves a web build (`apps/web/dist`) in front of a running Orbit server, so the perf scripts can measure a web build
// without a new server binary (the server embeds its own build at compile time). Static files get the server's cache
// headers (`/assets` immutable, `index.html` no-store) and its security headers; everything else, WebSockets too,
// goes to the server unchanged.
//
//   node scripts/perf/serve.mjs <dist directory> <listen port> <orbit port>
import { createReadStream, existsSync, statSync } from 'node:fs'
import { createServer, request } from 'node:http'
import { connect } from 'node:net'
import { extname, join, normalize } from 'node:path'

const [dist, listenPort, orbitPort] = process.argv.slice(2)
const types = { '.js': 'text/javascript; charset=utf-8', '.css': 'text/css; charset=utf-8', '.html': 'text/html; charset=utf-8', '.json': 'application/json', '.svg': 'image/svg+xml', '.png': 'image/png', '.woff2': 'font/woff2', '.webmanifest': 'application/manifest+json', '.mjs': 'text/javascript; charset=utf-8', '.wasm': 'application/wasm' }
const forwarded = (path) => path === '/api' || path.startsWith('/api/') || path.startsWith('/mcp') || path.startsWith('/oauth/') || path.startsWith('/.well-known/')

/** The app frame document of a server with the app-signin-card change, as main's server writes it (`src`, not `data-src`). */
const appFrame = /^\/api\/v1\/workspaces\/[^/]+\/apps\/[^/]+\/frame$/

function proxy(req, res) {
  const upstream = request({ host: '127.0.0.1', port: orbitPort, method: req.method, path: req.url, headers: req.headers }, (answer) => {
    if (appFrame.test(new URL(req.url, 'http://x').pathname) && answer.statusCode === 200) {
      const chunks = []
      answer.on('data', (chunk) => chunks.push(chunk))
      answer.on('end', () => {
        const body = Buffer.concat(chunks).toString().replace('<iframe data-src=', '<iframe src=')
        res.writeHead(200, { ...answer.headers, 'content-length': Buffer.byteLength(body) })
        res.end(body)
      })
      return
    }
    res.writeHead(answer.statusCode, answer.headers)
    answer.pipe(res)
  })
  upstream.on('error', () => res.destroy())
  req.pipe(upstream)
}

/** The security headers (CSP and others) of the server's own index page. */
function securityHeaders() {
  return new Promise((resolve) => {
    request({ host: '127.0.0.1', port: orbitPort, path: '/' }, (answer) => {
      answer.resume()
      const headers = { ...answer.headers }
      for (const name of ['content-length', 'content-type', 'cache-control', 'date', 'connection', 'keep-alive', 'transfer-encoding', 'etag', 'last-modified']) delete headers[name]
      resolve(headers)
    }).on('error', () => resolve({})).end()
  })
}

createServer(async (req, res) => {
  const path = decodeURIComponent(new URL(req.url, 'http://x').pathname)
  if (forwarded(path) || !['GET', 'HEAD'].includes(req.method)) return proxy(req, res)
  let file = normalize(join(dist, path))
  if (!file.startsWith(normalize(dist))) return proxy(req, res)
  if (!existsSync(file) || statSync(file).isDirectory()) {
    // A path with a file extension that is not in the build is the server's 404; any other path is the app.
    if (extname(path)) return proxy(req, res)
    file = join(dist, 'index.html')
  }
  const name = file.slice(normalize(dist).length + 1)
  const cache = ['index.html', 'orbit-build.json', 'manifest.webmanifest'].includes(name) ? 'no-store' : name.startsWith('assets/') ? 'public, max-age=31536000, immutable' : 'public, max-age=3600'
  const headers = name === 'index.html' ? await securityHeaders() : {}
  res.writeHead(200, { ...headers, 'content-type': types[extname(file)] ?? 'application/octet-stream', 'cache-control': cache, 'content-length': statSync(file).size })
  if (req.method === 'HEAD') return res.end()
  createReadStream(file).pipe(res)
})
  .on('upgrade', (req, socket, head) => {
    // WebSockets (realtime events, the docs editor): pass the raw upgrade through.
    const upstream = connect(Number(orbitPort), '127.0.0.1', () => {
      upstream.write(`${req.method} ${req.url} HTTP/1.1\r\n${Object.entries(req.headers).map(([key, value]) => `${key}: ${value}`).join('\r\n')}\r\n\r\n`)
      upstream.write(head)
      socket.pipe(upstream).pipe(socket)
    })
    upstream.on('error', () => socket.destroy())
    socket.on('error', () => upstream.destroy())
  })
  .listen(Number(listenPort), '127.0.0.1', () => console.log(`serving ${dist} on 127.0.0.1:${listenPort}, API on ${orbitPort}`))
