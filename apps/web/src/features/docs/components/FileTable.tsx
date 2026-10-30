import { useEffect, useRef } from 'react'
import { useVirtualizer } from '@tanstack/react-virtual'
import { Button } from '@/components/ui/button'

const ROW_HEIGHT = 30
const COLUMN_WIDTH = 160
const NUMBER_WIDTH = 56

/** Column letters as in a spreadsheet: A … Z, AA … */
export function columnName(index: number): string {
  let name = ''
  for (let left = index + 1; left > 0; left = Math.floor((left - 1) / 26)) name = String.fromCharCode(65 + ((left - 1) % 26)) + name
  return name
}

/**
 * A table of a file's rows that draws only the visible ones, so a large sheet stays fast. With `onChange` the cells are
 * inputs. `onReachEnd` asks for the next part.
 */
export function FileTable({
  rows,
  rowNumbers,
  onChange,
  onRemoveRow,
  onReachEnd,
}: {
  rows: string[][]
  rowNumbers?: number[]
  onChange?: (row: number, column: number, value: string) => void
  onRemoveRow?: (row: number) => void
  onReachEnd?: () => void
}) {
  const scrollRef = useRef<HTMLDivElement>(null)
  const columns = rows.reduce((most, row) => Math.max(most, row.length), 1)
  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => ROW_HEIGHT,
    overscan: 12,
  })
  const items = virtualizer.getVirtualItems()
  const lastIndex = items.at(-1)?.index ?? 0

  useEffect(() => {
    if (rows.length > 0 && lastIndex >= rows.length - 20) onReachEnd?.()
  }, [lastIndex, rows.length, onReachEnd])

  return (
    <div ref={scrollRef} data-virtual-scroller="" data-file-table="" className="min-h-0 flex-1 overflow-auto text-[13px]">
      <div style={{ width: NUMBER_WIDTH + columns * COLUMN_WIDTH + (onRemoveRow ? 32 : 0) }}>
        <div className="sticky top-0 z-10 flex h-[30px] border-b border-border bg-muted text-xs font-medium text-muted-foreground">
          <span className="sticky left-0 shrink-0 border-r border-border bg-muted" style={{ width: NUMBER_WIDTH }} />
          {Array.from({ length: columns }, (_, column) => (
            <span key={column} className="flex shrink-0 items-center justify-center border-r border-border" style={{ width: COLUMN_WIDTH }}>
              {columnName(column)}
            </span>
          ))}
        </div>
        <div className="relative" style={{ height: virtualizer.getTotalSize() }}>
          {items.map((item) => {
            const row = rows[item.index]
            return (
              <div
                key={item.index}
                data-index={item.index}
                className="absolute left-0 flex w-full border-b border-border"
                style={{ top: item.start, height: ROW_HEIGHT }}
              >
                <span
                  className="sticky left-0 z-[1] flex shrink-0 items-center justify-end border-r border-border bg-muted px-2 text-xs text-muted-foreground tabular-nums"
                  style={{ width: NUMBER_WIDTH }}
                >
                  {rowNumbers?.[item.index] ?? item.index + 1}
                </span>
                {Array.from({ length: columns }, (_, column) =>
                  onChange ? (
                    <input
                      key={column}
                      aria-label={`${columnName(column)}${item.index + 1}`}
                      className="h-full shrink-0 border-r border-border bg-transparent px-2 outline-none focus:bg-accent"
                      style={{ width: COLUMN_WIDTH }}
                      value={row[column] ?? ''}
                      onChange={(event) => onChange(item.index, column, event.target.value)}
                    />
                  ) : (
                    <span
                      key={column}
                      className="flex shrink-0 items-center truncate border-r border-border px-2"
                      style={{ width: COLUMN_WIDTH }}
                      title={row[column] || undefined}
                    >
                      <span className="truncate">{row[column] ?? ''}</span>
                    </span>
                  ),
                )}
                {onRemoveRow ? (
                  <Button type="button" variant="ghost" size="icon-xs" className="my-auto shrink-0 text-muted-foreground" aria-label={`Remove row ${item.index + 1}`} onClick={() => onRemoveRow(item.index)}>
                    ×
                  </Button>
                ) : null}
              </div>
            )
          })}
        </div>
      </div>
    </div>
  )
}
