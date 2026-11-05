import { defineConfig } from 'vite'
import react, { reactCompilerPreset } from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'
import babel from '@rolldown/plugin-babel'
import openapi from '@orbit/api/generated/openapi.json' with { type: 'json' }
import { createHash } from 'node:crypto'
import { cpSync, existsSync, mkdirSync, readdirSync, readFileSync, writeFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import { basename, dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { brotliCompressSync, constants as zlibConstants, gzipSync } from 'node:zlib'

const buildRevision = process.env.ORBIT_BUILD_REVISION ?? 'development'
const sourceHtml = readFileSync(new URL('./index.html', import.meta.url), 'utf8')
const bootstrapScript = sourceHtml.match(/<script>([\s\S]*?)<\/script>/)?.[1]
if (!bootstrapScript) throw new Error('theme bootstrap script is missing')
const cspScriptHash = `sha256-${createHash('sha256').update(bootstrapScript).digest('base64')}`

/**
 * Emoji files that Orbit serves itself (the production CSP has no CDN): the Twemoji images at `/assets/twemoji-<version>/<code>.svg`
 * and the emoji data at `/assets/emojibase-<version>/en/…`. The version in the path makes them immutable: the server
 * caches `/assets` for a year, so a browser downloads each file once. They stay in `node_modules`: the dev server
 * reads them from there and the build copies them into `dist`. The versions are those of the installed packages, as in
 * `src/lib/twemoji.ts`. The images are the `jdecked/twemoji` repository: its npm package of images stops at 15.0.0.
 */
const packageDir = (name: string): string => dirname(createRequire(import.meta.url).resolve(`${name}/package.json`))
const packageVersion = (name: string): string =>
  JSON.parse(readFileSync(join(packageDir(name), 'package.json'), 'utf8')).version
const twemojiVersion = packageVersion('twemoji-assets')
const emojibaseVersion = packageVersion('emojibase-data')
const emojiAssets = [
  { url: `/assets/twemoji-${twemojiVersion}/`, dir: join(packageDir('twemoji-assets'), 'assets/svg/'), files: /^[0-9a-f-]+\.svg$/, type: 'image/svg+xml' },
  { url: `/assets/emojibase-${emojibaseVersion}/en/shortcodes/`, dir: join(packageDir('emojibase-data'), 'en/shortcodes/'), files: /^github\.json$/, type: 'application/json' },
  { url: `/assets/emojibase-${emojibaseVersion}/en/`, dir: join(packageDir('emojibase-data'), 'en/'), files: /^(data|messages)\.json$/, type: 'application/json' },
]

// https://vite.dev/config/
export default defineConfig({
  define: {
    __ORBIT_BUILD_REVISION__: JSON.stringify(buildRevision),
  },
  resolve: {
    alias: {
      '@': fileURLToPath(new URL('./src', import.meta.url)),
    },
  },
  plugins: [
    react(),
    tailwindcss(),
    babel({ presets: [reactCompilerPreset()] }),
    {
      name: 'orbit-emoji-assets',
      configureServer(server) {
        server.middlewares.use((request, response, next) => {
          const path = (request.url ?? '').split('?')[0]
          const asset = emojiAssets.find((candidate) => path.startsWith(candidate.url))
          const name = asset ? basename(path) : ''
          if (!asset || !asset.files.test(name) || !existsSync(asset.dir + name)) return next()
          response.setHeader('Content-Type', asset.type)
          response.setHeader('Cache-Control', 'public, max-age=31536000, immutable')
          response.setHeader('ETag', `"${asset.url}"`)
          response.end(readFileSync(asset.dir + name))
        })
      },
      writeBundle(options) {
        for (const asset of emojiAssets) {
          const target = `${options.dir}${asset.url}`
          mkdirSync(target, { recursive: true })
          cpSync(asset.dir, target, { recursive: true, filter: (source) => source === asset.dir || asset.files.test(basename(source)) })
        }
      },
    },
    {
      // Sonner adds its CSS as a runtime <style> tag, which the production CSP (`style-src 'self'`) blocks and reports
      // as a console error. `src/components/ui/sonner.tsx` bundles the same CSS from `sonner/dist/styles.css`.
      name: 'orbit-sonner-no-runtime-css',
      apply: 'build',
      transform(code, id) {
        if (!/[\\/]sonner[\\/]dist[\\/]index\.m?js$/.test(id)) return
        const injector = 'function __insertCSS(code) {'
        if (!code.includes(injector)) throw new Error('sonner no longer defines __insertCSS: update the orbit-sonner-no-runtime-css plugin')
        return code.replace(injector, `${injector} return;`)
      },
    },
    {
      name: 'orbit-build-manifest',
      generateBundle() {
        this.emitFile({
          type: 'asset',
          fileName: 'orbit-build.json',
          source: `${JSON.stringify({ contract: openapi.info.version, revision: buildRevision, csp_script_hash: cspScriptHash }, null, 2)}\n`,
        })
      },
    },
    {
      // The server embeds `dist` and sends `<file>.br` or `<file>.gz` when the browser accepts it
      // (`apps/server/src/static_assets.rs`). Only text files of 1 kB or more, and only when the result is smaller.
      name: 'orbit-precompress',
      apply: 'build',
      closeBundle() {
        const dist = fileURLToPath(new URL('./dist/', import.meta.url))
        for (const entry of readdirSync(dist, { recursive: true, withFileTypes: true })) {
          if (!entry.isFile() || !/\.(js|mjs|css|html|json|svg|webmanifest|txt|wasm)$/.test(entry.name)) continue
          const path = join(entry.parentPath, entry.name)
          const source = readFileSync(path)
          if (source.length < 1024) continue
          const brotli = brotliCompressSync(source, { params: { [zlibConstants.BROTLI_PARAM_QUALITY]: 11, [zlibConstants.BROTLI_PARAM_SIZE_HINT]: source.length } })
          if (brotli.length < source.length * 0.9) writeFileSync(`${path}.br`, brotli)
          const gzip = gzipSync(source, { level: 9 })
          if (gzip.length < source.length * 0.9) writeFileSync(`${path}.gz`, gzip)
        }
      },
    },
  ],
  server: {
    port: 8888,
    allowedHosts: ['.ts.net'],
    proxy: {
      '/.well-known/oauth-': {
        target: process.env.VITE_API_PROXY ?? 'http://127.0.0.1:8080',
      },
      '^/oauth/(authorize|token|register|revoke)(\\?|$)': {
        target: process.env.VITE_API_PROXY ?? 'http://127.0.0.1:8080',
      },
      '/mcp': {
        target: process.env.VITE_API_PROXY ?? 'http://127.0.0.1:8080',
      },
      '/api': {
        target: process.env.VITE_API_PROXY ?? 'http://127.0.0.1:8080',
        ws: true,
      },
    },
  },
})
