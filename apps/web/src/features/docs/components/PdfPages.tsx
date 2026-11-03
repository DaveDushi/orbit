import { useEffect, useRef, useState } from 'react'
import { getDocument, GlobalWorkerOptions, type PDFDocumentProxy } from 'pdfjs-dist'
import { Spinner } from '@/components/ui/spinner'

/** The worker is bundled and same-origin, so the CSP stays `script-src 'self'`. */
function ensureWorker() {
  GlobalWorkerOptions.workerPort ??= new Worker(new URL('pdfjs-dist/build/pdf.worker.min.mjs', import.meta.url), { type: 'module' })
}

/**
 * One page, drawn while it is within one panel height of the visible part of the panel. A page that scrolls farther
 * away gives its bitmap back (its size stays from CSS), so a long PDF holds only a few pages in memory.
 */
function PdfPage({ pdf, number, width }: { pdf: PDFDocumentProxy; number: number; width: number }) {
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const [near, setNear] = useState(false)
  const [ratio, setRatio] = useState(1.414)
  /** The canvas holds a page bitmap (not the empty default one). */
  const drawn = useRef(false)

  useEffect(() => {
    const canvas = canvasRef.current
    if (!canvas) return
    // The root is the scrolling panel: the margin of the default root (the window) cannot reach past the panel's clip.
    const observer = new IntersectionObserver(([entry]) => setNear(entry.isIntersecting), {
      root: canvas.closest('[data-pdf-pages]')?.parentElement,
      rootMargin: '100% 0px',
    })
    observer.observe(canvas)
    return () => observer.disconnect()
  }, [])

  useEffect(() => {
    if (!near) {
      const canvas = canvasRef.current
      if (canvas && drawn.current) {
        canvas.width = 0
        canvas.height = 0
        drawn.current = false
      }
      return
    }
    if (width <= 0) return
    let render: ReturnType<Awaited<ReturnType<PDFDocumentProxy['getPage']>>['render']> | undefined
    let stopped = false
    void pdf.getPage(number).then((page) => {
      const canvas = canvasRef.current
      if (stopped || !canvas) return
      const plain = page.getViewport({ scale: 1 })
      setRatio(plain.height / plain.width)
      const pixels = window.devicePixelRatio || 1
      const viewport = page.getViewport({ scale: (width / plain.width) * pixels })
      canvas.width = Math.floor(viewport.width)
      canvas.height = Math.floor(viewport.height)
      drawn.current = true
      render = page.render({ canvas, viewport })
      render.promise.catch(() => {})
    })
    return () => {
      stopped = true
      render?.cancel()
    }
  }, [pdf, number, width, near])

  return (
    <canvas
      ref={canvasRef}
      data-pdf-page={number}
      className="block w-full border border-border bg-white"
      style={{ aspectRatio: `1 / ${ratio}` }}
    />
  )
}

/** The real pages of a PDF, drawn in the browser from the file's bytes. */
export function PdfPages({ url, onError }: { url: string; onError: () => void }) {
  const [pdf, setPdf] = useState<PDFDocumentProxy | null>(null)
  const [width, setWidth] = useState(0)
  const containerRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    let stopped = false
    let loading: ReturnType<typeof getDocument> | null = null
    ensureWorker()
    void (async () => {
      try {
        const response = await fetch(url)
        if (!response.ok) throw new Error(`The file answered ${response.status}.`)
        const data = await response.arrayBuffer()
        if (stopped) return
        loading = getDocument({ data })
        const document = await loading.promise
        if (!stopped) setPdf(document)
      } catch {
        if (!stopped) onError()
      }
    })()
    return () => {
      stopped = true
      void loading?.destroy()
    }
  }, [url, onError])

  useEffect(() => {
    const container = containerRef.current
    if (!container) return
    // The canvases scale with CSS at once; they draw again only when a drag of the divider rests.
    let timer: ReturnType<typeof setTimeout> | undefined
    const observer = new ResizeObserver(([entry]) => {
      const next = Math.floor(entry.contentRect.width)
      clearTimeout(timer)
      timer = setTimeout(() => setWidth(next), 150)
    })
    observer.observe(container)
    return () => {
      clearTimeout(timer)
      observer.disconnect()
    }
  }, [])

  return (
    <div ref={containerRef} data-pdf-pages="" className="flex flex-col gap-3 p-3">
      {pdf ? (
        Array.from({ length: pdf.numPages }, (_, index) => <PdfPage key={index} pdf={pdf} number={index + 1} width={width} />)
      ) : (
        <div className="flex justify-center py-10">
          <Spinner className="text-muted-foreground" />
        </div>
      )}
    </div>
  )
}

export default PdfPages
