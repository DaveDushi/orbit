import { createContext, useContext, useImperativeHandle, useLayoutEffect, useMemo, useRef, useState, type Ref } from 'react'
import { cn } from 'cn'

const STORAGE_KEY = 'orbit:file_panel_share'
/** The size to go back to from the full screen button. It stays over a reload, as full screen does. */
const BEFORE_FULL_KEY = 'orbit:file_panel_share_before_full'
/** Below this size a side is of no use, so the divider snaps to the edge and hides it. */
export const FILE_SPLIT_MIN = 240
const KEY_STEP = 16

export type FileSplitHandle = { reveal: () => void }

/** The saved share of the row for the file: 0 hides the file, 1 hides the page, null is the default size. */
export function parseFileShare(saved: string | null): number | null {
  if (!saved) return null
  const share = Number(saved)
  return Number.isFinite(share) && share >= 0 && share <= 1 ? share : null
}

/** The panel's width before anyone moves the divider: `min(52%, 760px)`, and at least 360px. */
export function defaultFileShare(width: number): number {
  return width > 0 ? Math.min(1, Math.max(360, Math.min(width * 0.52, 760)) / width) : 0.52
}

/** Where a dragged divider lands: near an edge it snaps there. */
export function snapFileShare(share: number, width: number): number {
  const file = share * width
  const page = width - file
  if (file < FILE_SPLIT_MIN && page < FILE_SPLIT_MIN) return file < page ? 0 : 1
  if (file < FILE_SPLIT_MIN) return 0
  if (page < FILE_SPLIT_MIN) return 1
  return share
}

/**
 * The share to show at this width. The saved value stays as it is, so a window that gets small and then big again
 * returns to the same split. Only a saved 0 or 1 hides a side: a small window does not hide a file that was clicked.
 */
export function fitFileShare(share: number | null, width: number): number {
  const wanted = share ?? defaultFileShare(width)
  if (wanted === 0 || wanted === 1 || width <= 0) return wanted
  if (width < FILE_SPLIT_MIN * 2) return 1
  return Math.min(1 - FILE_SPLIT_MIN / width, Math.max(FILE_SPLIT_MIN / width, wanted))
}

/** One arrow key press. From a hidden side it goes to that side's smallest useful size, not back into the snap. */
export function stepFileShare(share: number, width: number, direction: -1 | 1): number {
  if (width <= 0) return share
  if (share === 0 && direction === 1) return snapFileShare(FILE_SPLIT_MIN / width, width)
  if (share === 1 && direction === -1) return snapFileShare(1 - FILE_SPLIT_MIN / width, width)
  return snapFileShare(Math.min(1, Math.max(0, share + (direction * KEY_STEP) / width)), width)
}

/** A click on a file must show it, so a saved size that hides the file gives way to the default. */
export function revealFileShare(share: number | null): number | null {
  return share === 0 ? null : share
}

/**
 * The full screen button, from `saved` and the size saved before full screen. Going in keeps the size; coming out
 * uses it. A drag to full width leaves no size to go back to, so that comes out at the default (null).
 */
export function toggleFullFile(saved: number | null, before: number | null): { saved: number | null; before: number | null } {
  return saved === 1 ? { saved: before, before: null } : { saved: 1, before: saved }
}

function storedShare(key = STORAGE_KEY): number | null {
  return parseFileShare(window.localStorage.getItem(key))
}

function store(key: string, share: number | null) {
  if (share === null) window.localStorage.removeItem(key)
  else window.localStorage.setItem(key, String(share))
}

const FileFullScreenContext = createContext<{ fullScreen: boolean; toggle: () => void }>({ fullScreen: false, toggle: () => {} })

/** For the panel's header. Only a change of this value renders the panel again, so a drag does not. */
export const useFileFullScreen = () => useContext(FileFullScreenContext)

/**
 * The page column and the open file, with a divider between them. From 1100px up it moves over the full width of this
 * row; below that the panel is an overlay and has no divider. A hidden side stays mounted and laid out (see the
 * `data-split` variants on both children), so edits, the editor and the scroll position stay. The state lives here and
 * reaches the children only through CSS, so a drag does not render the editor again.
 */
export function FileSplit({ open, ref, children }: { open: boolean; ref: Ref<FileSplitHandle>; children: React.ReactNode }) {
  const rowRef = useRef<HTMLDivElement>(null)
  const [saved, setSaved] = useState(storedShare)
  const [width, setWidth] = useState(0)
  const [dragging, setDragging] = useState(false)

  /** A size chosen with the divider replaces the one saved before full screen. */
  const save = (next: number | null, before: number | null = null) => {
    setSaved(next)
    store(STORAGE_KEY, next)
    store(BEFORE_FULL_KEY, before)
  }
  useImperativeHandle(ref, () => ({
    reveal: () => {
      const stored = storedShare()
      if (revealFileShare(stored) !== stored) save(null)
    },
  }), [])

  useLayoutEffect(() => {
    const row = rowRef.current
    if (!row) return
    setWidth(row.clientWidth)
    const observer = new ResizeObserver(() => setWidth(row.clientWidth))
    observer.observe(row)
    return () => observer.disconnect()
  }, [])

  const share = fitFileShare(saved, width)
  const split = !open ? undefined : share === 0 ? 'page' : share === 1 ? 'file' : 'both'
  const fullScreen = share === 1
  const fullScreenValue = useMemo(
    () => ({
      fullScreen,
      toggle: () => {
        const next = toggleFullFile(storedShare(), storedShare(BEFORE_FULL_KEY))
        save(next.saved, next.before)
      },
    }),
    [fullScreen],
  )

  function startDrag(event: React.PointerEvent<HTMLDivElement>) {
    if (event.button !== 0) return
    event.preventDefault()
    const handle = event.currentTarget
    handle.setPointerCapture(event.pointerId)
    const startX = event.clientX
    const startFile = share * width
    setDragging(true)
    const move = (moveEvent: PointerEvent) => {
      if (width > 0) save(snapFileShare(Math.min(1, Math.max(0, (startFile + startX - moveEvent.clientX) / width)), width))
    }
    const end = () => {
      setDragging(false)
      handle.removeEventListener('pointermove', move)
      handle.removeEventListener('pointerup', end)
      handle.removeEventListener('pointercancel', end)
    }
    handle.addEventListener('pointermove', move)
    handle.addEventListener('pointerup', end)
    handle.addEventListener('pointercancel', end)
  }

  return (
    <div
      ref={rowRef}
      data-file-split=""
      data-split={split}
      data-dragging={dragging || undefined}
      style={{ '--file-share': share } as React.CSSProperties}
      className="group/split relative flex min-h-0 min-w-0 flex-1 data-dragging:cursor-col-resize data-dragging:select-none min-[1100px]:data-[split=file]:pl-2 min-[1100px]:data-[split=page]:pr-2"
    >
      <FileFullScreenContext.Provider value={fullScreenValue}>{children}</FileFullScreenContext.Provider>
      {split ? (
        <div
          role="separator"
          aria-orientation="vertical"
          aria-label="Resize file panel"
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={Math.round(share * 100)}
          tabIndex={0}
          data-print-hide=""
          style={split === 'both' ? { left: `${(1 - share) * 100}%` } : undefined}
          className={cn(
            'absolute inset-y-0 z-10 flex w-2 cursor-col-resize touch-none items-center justify-center outline-none after:absolute after:inset-y-0 after:left-0 after:w-0.5 after:transition-colors hover:after:bg-primary/60 focus-visible:after:bg-primary max-[1099px]:hidden',
            split === 'file' && 'left-0 border-r border-border bg-muted',
            split === 'page' && 'right-0 border-l border-border bg-muted',
          )}
          onPointerDown={startDrag}
          onDoubleClick={() => save(null)}
          onKeyDown={(event) => {
            if (event.key === 'ArrowLeft') save(stepFileShare(share, width, 1))
            else if (event.key === 'ArrowRight') save(stepFileShare(share, width, -1))
            else return
            event.preventDefault()
          }}
        >
          {split === 'both' ? null : <span className="h-8 w-0.5 rounded-full bg-muted-foreground/60" />}
        </div>
      ) : null}
    </div>
  )
}
