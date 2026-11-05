// Web bundle sizes from apps/web/dist: gzip (level 9) and brotli (quality 11) of the entry JS/CSS (what index.html
// loads, plus its static imports) and of every lazy chunk. Which chunks each page really loads is measured by
// pageload.ts. Writes $EVALS_WORK/parts/bundle.json.
import { readdirSync, readFileSync } from 'node:fs'
import { join } from 'node:path'
import { brotliCompressSync, constants, gzipSync } from 'node:zlib'
import { round, writePart } from './common'

const dist = join(import.meta.dir, '../../apps/web/dist')
const assets = join(dist, 'assets')
const html = readFileSync(join(dist, 'index.html'), 'utf8')
const kb = (bytes: number) => round(bytes / 1024)

type Size = { file: string; raw_kb: number; gzip_kb: number; brotli_kb: number }
const sizes = new Map<string, Size>()
const size = (file: string): Size => {
  let known = sizes.get(file)
  if (!known) {
    const content = readFileSync(join(assets, file))
    known = {
      file,
      raw_kb: kb(content.length),
      gzip_kb: kb(gzipSync(content, { level: 9 }).length),
      brotli_kb: kb(brotliCompressSync(content, { params: { [constants.BROTLI_PARAM_QUALITY]: 11 } }).length),
    }
    sizes.set(file, known)
  }
  return known
}

// Static imports (`import{a}from"./x.js"`, `import"./x.js"`) are loaded with the importer; `import("./x.js")` is lazy.
const staticImports = (file: string) =>
  [...readFileSync(join(assets, file), 'utf8').matchAll(/\bimport\s*(?:[\w$*{}\s,]+?\s*from\s*)?["']\.\/([^"']+\.js)["']/g)].map((match) => match[1])

const entryScripts = [...html.matchAll(/<(?:script[^>]+src|link[^>]+rel="modulepreload"[^>]+href)="\/assets\/([^"]+\.js)"/g)].map((match) => match[1])
const entryCss = [...html.matchAll(/<link[^>]+rel="stylesheet"[^>]+href="\/assets\/([^"]+\.css)"/g)].map((match) => match[1])
const initial = new Set<string>()
const queue = [...entryScripts]
while (queue.length) {
  const file = queue.pop()!
  if (initial.has(file)) continue
  initial.add(file)
  queue.push(...staticImports(file))
}

const all = readdirSync(assets).filter((file) => /\.(js|css)$/.test(file))
const total = (files: string[]) => ({
  raw_kb: round(files.reduce((sum, file) => sum + size(file).raw_kb, 0)),
  gzip_kb: round(files.reduce((sum, file) => sum + size(file).gzip_kb, 0)),
  brotli_kb: round(files.reduce((sum, file) => sum + size(file).brotli_kb, 0)),
})
const lazy = all.filter((file) => file.endsWith('.js') && !initial.has(file)).map(size).sort((x, y) => y.gzip_kb - x.gzip_kb)
// Chunk names without the content hash, so results compare across builds. A generic name (`src.js`) gets a hint of
// the package inside when one is clear, and a name that is still shared keeps its hash.
const hints = ['mediabunny', 'mermaid', 'pdfjs', 'cytoscape', 'katex', 'blocknote', 'yjs', 'elkjs']
const baseName = (file: string) => {
  const short = file.replace(/-[\w-]{8}\.(js|css)$/, '.$1')
  if (!/^(src|index|chunk-[\w]+)\.js$/.test(short)) return short
  const content = readFileSync(join(assets, file), 'utf8')
  const hint = hints.find((package_) => content.includes(package_))
  return hint ? short.replace(/\.js$/, `(${hint}).js`) : short
}
const counts = new Map<string, number>()
for (const file of all) counts.set(baseName(file), (counts.get(baseName(file)) ?? 0) + 1)
const name = (file: string) => (counts.get(baseName(file))! > 1 ? file : baseName(file))

const result = {
  entry_js: { files: [...initial].map(name), ...total([...initial]) },
  entry_css: { files: entryCss.map(name), ...total(entryCss) },
  all_js: total(all.filter((file) => file.endsWith('.js'))),
  all_css: total(all.filter((file) => file.endsWith('.css'))),
  chunk_count: all.length,
  largest_lazy_gzip_kb: lazy[0]?.gzip_kb ?? 0,
  lazy_over_100kb_gzip: lazy.filter((chunk) => chunk.gzip_kb > 100).map((chunk) => ({ ...chunk, file: name(chunk.file) })),
  top_lazy: lazy.slice(0, 10).map((chunk) => ({ ...chunk, file: name(chunk.file) })),
}
writePart('bundle', result)
console.log(`bundle: entry JS ${result.entry_js.gzip_kb} kB gzip, entry CSS ${result.entry_css.gzip_kb} kB gzip, ${result.lazy_over_100kb_gzip.length} lazy chunks over 100 kB gzip`)
