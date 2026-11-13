import { useCallback, useState, type ReactNode } from 'react'
import { keepPreviousData, useInfiniteQuery } from '@tanstack/react-query'
import type { ChatFileContent } from '@orbit/api/generated/types.gen'
import { CONTRACT_ID } from '@/api/client'
import { Spinner } from '@/components/ui/spinner'
import { Tabs, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { FileTable } from '@/features/docs/components/FileTable'

/** Rows asked for at a time, as in the Docs file panel. */
const ROWS_PER_PART = 200

type SheetPart = Pick<ChatFileContent, 'sheets' | 'sheet' | 'rows' | 'row_numbers' | 'next_offset'>

async function readPart(contentUrl: string, sheet: string | undefined, offset: number): Promise<SheetPart> {
  const query = new URLSearchParams({ offset: String(offset), limit: String(ROWS_PER_PART) })
  if (sheet !== undefined) query.set('sheet', sheet)
  const response = await fetch(`${contentUrl}?${query}`, { credentials: 'include', headers: { 'X-Orbit-Contract': CONTRACT_ID } })
  if (!response.ok) throw new Error(`content: ${response.status}`)
  return (await response.json()) as SheetPart
}

/**
 * A workbook, csv or tsv in the viewer: a read-only table with a tab per sheet, loaded in parts as the reader
 * scrolls. `contentUrl` is the file's content route (chat or Drive). When the server cannot read the file (damaged,
 * protected, too large), `fallback` shows instead: the server's PDF of it.
 */
export function SheetPreview({ contentUrl, fallback }: { contentUrl: string; fallback: ReactNode }) {
  const [sheet, setSheet] = useState<string | undefined>(undefined)
  const parts = useInfiniteQuery({
    queryKey: ['file-content', contentUrl, sheet ?? null],
    queryFn: ({ pageParam }) => readPart(contentUrl, sheet, pageParam),
    initialPageParam: 0,
    getNextPageParam: (last: SheetPart) => last.next_offset ?? undefined,
    // The tabs stay while another sheet loads.
    placeholderData: keepPreviousData,
    // Limits and damaged files do not change by asking again.
    retry: false,
  })
  const { hasNextPage, isFetchingNextPage, fetchNextPage } = parts
  const reachEnd = useCallback(() => {
    if (hasNextPage && !isFetchingNextPage) void fetchNextPage()
  }, [hasNextPage, isFetchingNextPage, fetchNextPage])

  // A later part that fails keeps the rows on show: scrolling asks for it again.
  if (parts.data === undefined) return parts.isError ? fallback : <Spinner className="text-white/70" />
  const first = parts.data.pages[0]
  const rows = parts.data.pages.flatMap((part) => part.rows ?? [])
  const rowNumbers = parts.data.pages.flatMap((part) => part.row_numbers ?? [])

  return (
    <div className="flex h-full w-full max-w-6xl flex-col overflow-hidden rounded-md bg-background text-foreground" data-drive-preview="sheet">
      {parts.isPlaceholderData ? (
        <div className="flex flex-1 items-center justify-center">
          <Spinner />
        </div>
      ) : rows.length === 0 ? (
        <p className="flex flex-1 items-center justify-center text-sm text-muted-foreground">This sheet is empty.</p>
      ) : (
        <FileTable rows={rows} rowNumbers={rowNumbers} onReachEnd={reachEnd} />
      )}
      {first.sheets.length > 1 ? (
        <Tabs className="shrink-0 gap-0 overflow-x-auto border-t border-border px-2 py-1.5" value={sheet ?? first.sheet ?? ''} onValueChange={(value) => setSheet(value as string)}>
          <TabsList>
            {first.sheets.map((name) => (
              <TabsTrigger key={name} value={name}>
                {name}
              </TabsTrigger>
            ))}
          </TabsList>
        </Tabs>
      ) : null}
    </div>
  )
}
