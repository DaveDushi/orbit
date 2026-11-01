import { lazy, Suspense, useCallback, useState } from 'react'
import { Download, History, Xmark as X } from 'reicon-react'
import { cn } from 'cn'
import { ApiProblem } from '@orbit/api/problem'
import type { PageFileContent } from '@orbit/api/generated/types.gen'
import { Button, buttonVariants } from '@/components/ui/button'
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover'
import { Spinner } from '@/components/ui/spinner'
import { Tabs, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { Textarea } from '@/components/ui/textarea'
import { Tip } from '@/components/common/Tip'
import { dayLabel, timeOfDay } from '@/lib/format'
import {
  fetchAllRows,
  fetchAllText,
  fileUrl,
  useFileContent,
  useFileParts,
  useFileVersions,
  useRestoreFileVersion,
  useSaveFile,
  type FileTarget,
} from '@/features/docs/api/pageFiles'
import { columnIndex, columnName, FileTable } from './FileTable'

// pdf.js is large: its chunk and worker load only when a PDF opens.
const PdfPages = lazy(() => import('./PdfPages'))

function problemText(error: unknown, fallback: string): string {
  return error instanceof ApiProblem ? error.detail : fallback
}

const isConflict = (error: unknown) => error instanceof ApiProblem && error.status === 409

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`
}

function Centered({ children }: { children: React.ReactNode }) {
  return <div className="flex flex-1 flex-col items-center justify-center gap-3 p-6 text-center text-[13px] text-muted-foreground">{children}</div>
}

function DownloadLink({ url, children }: { url: string; children: React.ReactNode }) {
  return (
    <a href={url} download className={cn(buttonVariants({ variant: 'outline', size: 'sm' }), 'no-underline')}>
      {children}
    </a>
  )
}

/** Save and cancel over an editor, and what a stale save tells the person. */
function EditBar({ dirty, pending, conflict, error, onSave, onCancel, onReload, children }: {
  dirty: boolean
  pending: boolean
  conflict: boolean
  error: string | null
  onSave: () => void
  onCancel: () => void
  onReload: () => void
  children?: React.ReactNode
}) {
  return (
    <div className="flex shrink-0 flex-wrap items-center gap-2 border-b border-border px-3 py-2" data-file-edit-bar="">
      <Button type="button" size="sm" disabled={!dirty || pending || conflict} onClick={onSave}>
        {pending ? 'Saving…' : 'Save'}
      </Button>
      <Button type="button" size="sm" variant="ghost" onClick={onCancel}>
        Cancel
      </Button>
      {children}
      {conflict ? (
        <span className="flex items-center gap-2 text-xs text-destructive" role="alert">
          Someone changed this file. Nothing was saved.
          <Button type="button" size="xs" variant="outline" onClick={onReload}>
            Load their version
          </Button>
        </span>
      ) : error ? (
        <span className="text-xs text-destructive" role="alert">
          {error}
        </span>
      ) : null}
    </div>
  )
}

/**
 * A workbook as a table. With `can_edit`, cells become inputs and a save sends only the changed ones: the server changes
 * those cells and leaves the rest of the file as it is. Formula cells stay read-only.
 */
function SheetView({ target, first, onDirty }: { target: FileTarget; first: PageFileContent; onDirty: (dirty: boolean) => void }) {
  const [sheet, setSheet] = useState<string | undefined>(first.sheet ?? undefined)
  const [draft, setDraft] = useState<{ edits: Map<string, string>; version: number; addedRows: number } | null>(null)
  const parts = useFileParts(target, sheet, true)
  const save = useSaveFile(target)
  const loaded = parts.data?.pages.flatMap((part) => part.rows ?? []) ?? []
  const loadedNumbers = parts.data?.pages.flatMap((part) => part.row_numbers ?? []) ?? []
  const formulas = new Set(parts.data?.pages.flatMap((part) => part.formula_cells ?? []) ?? [])
  const { hasNextPage, isFetchingNextPage, fetchNextPage } = parts
  const reachEnd = useCallback(() => {
    if (hasNextPage && !isFetchingNextPage) void fetchNextPage()
  }, [hasNextPage, isFetchingNextPage, fetchNextPage])

  const lastNumber = loadedNumbers.at(-1) ?? 0
  const added = draft?.addedRows ?? 0
  const rowNumbers = [...loadedNumbers, ...Array.from({ length: added }, (_, index) => lastNumber + index + 1)]
  const rows = [...loaded, ...Array.from({ length: added }, () => [] as string[])].map((row, index) => {
    if (!draft) return row
    const next = [...row]
    for (const [cell, value] of draft.edits) {
      const match = /^([A-Z]+)(\d+)$/.exec(cell)
      if (!match || Number(match[2]) !== rowNumbers[index]) continue
      const column = columnIndex(match[1])
      while (next.length <= column) next.push('')
      next[column] = value
    }
    return next
  })
  const cellAt = (row: number, column: number) => `${columnName(column)}${rowNumbers[row]}`
  const stop = () => {
    setDraft(null)
    onDirty(false)
    save.reset()
  }
  const notice = first.file.can_edit ? (
    <p className="shrink-0 border-b border-border bg-muted/40 px-3 py-2 text-xs text-muted-foreground" data-formula-notice="">
      Formula cells are read-only. Formula results shown here can be old: they update when the file opens in Excel.
    </p>
  ) : null

  return (
    <>
      {notice}
      {first.sheets.length > 1 ? (
        <Tabs className="shrink-0 gap-0 overflow-x-auto border-b border-border px-2 py-1.5" value={sheet ?? ''} onValueChange={(value) => setSheet(value as string)}>
          <TabsList>
            {first.sheets.map((name) => (
              <TabsTrigger key={name} value={name} disabled={Boolean(draft)}>
                {name}
              </TabsTrigger>
            ))}
          </TabsList>
        </Tabs>
      ) : null}
      {draft ? (
        <EditBar
          dirty={draft.edits.size > 0}
          pending={save.isPending}
          conflict={isConflict(save.error)}
          error={save.error && !isConflict(save.error) ? problemText(save.error, 'The file could not be saved.') : null}
          onSave={() =>
            save.mutate(
              { expected_version: draft.version, sheet, edits: [...draft.edits].map(([cell, value]) => ({ cell, value })) },
              { onSuccess: stop },
            )
          }
          onCancel={stop}
          onReload={stop}
        >
          <Button type="button" size="sm" variant="outline" disabled={hasNextPage} onClick={() => setDraft({ ...draft, addedRows: draft.addedRows + 1 })}>
            Add row
          </Button>
        </EditBar>
      ) : first.file.can_edit ? (
        <div className="flex shrink-0 items-center gap-2 border-b border-border px-3 py-2">
          <Button type="button" size="sm" variant="outline" onClick={() => setDraft({ edits: new Map(), version: first.file.version, addedRows: 0 })}>
            Edit
          </Button>
        </div>
      ) : null}
      {parts.isError ? (
        <Centered>
          {problemText(parts.error, 'This sheet could not be read.')}
          <DownloadLink url={fileUrl(target)}>Download</DownloadLink>
        </Centered>
      ) : parts.isPending ? (
        <Centered>
          <Spinner />
        </Centered>
      ) : rows.length === 0 && !draft ? (
        <Centered>This sheet is empty.</Centered>
      ) : (
        <FileTable
          rows={rows}
          rowNumbers={rowNumbers}
          onReachEnd={reachEnd}
          readOnlyCell={(row, column) => formulas.has(cellAt(row, column))}
          onChange={
            draft
              ? (row, column, value) => {
                  const edits = new Map(draft.edits)
                  edits.set(cellAt(row, column), value)
                  setDraft({ ...draft, edits })
                  onDirty(true)
                }
              : undefined
          }
        />
      )}
    </>
  )
}

function DelimitedView({ target, first, onDirty }: { target: FileTarget; first: PageFileContent; onDirty: (dirty: boolean) => void }) {
  const [draft, setDraft] = useState<{ rows: string[][]; version: number; dirty: boolean } | null>(null)
  const [loadError, setLoadError] = useState<string | null>(null)
  const save = useSaveFile(target)
  const parts = useFileParts(target, undefined, draft === null)
  const { hasNextPage, isFetchingNextPage, fetchNextPage } = parts
  const reachEnd = useCallback(() => {
    if (hasNextPage && !isFetchingNextPage) void fetchNextPage()
  }, [hasNextPage, isFetchingNextPage, fetchNextPage])
  const rows = parts.data?.pages.flatMap((part) => part.rows ?? []) ?? first.rows ?? []

  const startEdit = () => {
    setLoadError(null)
    save.reset()
    fetchAllRows(target).then(
      (all) => setDraft({ ...all, dirty: false }),
      (error: unknown) => setLoadError(problemText(error, 'The file could not be loaded for editing.')),
    )
  }
  const change = (next: (rows: string[][]) => string[][]) => {
    setDraft((current) => (current ? { ...current, rows: next(current.rows), dirty: true } : current))
    onDirty(true)
  }
  const stop = () => {
    setDraft(null)
    onDirty(false)
    save.reset()
  }

  if (!draft) {
    return (
      <>
        {first.file.can_edit ? (
          <div className="flex shrink-0 items-center gap-2 border-b border-border px-3 py-2">
            <Button type="button" size="sm" variant="outline" onClick={startEdit}>
              Edit
            </Button>
            {loadError ? <span className="text-xs text-destructive">{loadError}</span> : null}
          </div>
        ) : null}
        {rows.length === 0 ? <Centered>This file is empty.</Centered> : <FileTable rows={rows} onReachEnd={reachEnd} />}
      </>
    )
  }
  const columns = Math.max(1, ...draft.rows.map((row) => row.length))
  return (
    <>
      <EditBar
        dirty={draft.dirty}
        pending={save.isPending}
        conflict={isConflict(save.error)}
        error={save.error && !isConflict(save.error) ? problemText(save.error, 'The file could not be saved.') : null}
        onSave={() => save.mutate({ expected_version: draft.version, rows: draft.rows }, { onSuccess: stop })}
        onCancel={stop}
        onReload={() => {
          onDirty(false)
          startEdit()
        }}
      >
        <Button type="button" size="sm" variant="outline" onClick={() => change((all) => [...all, Array.from({ length: columns }, () => '')])}>
          Add row
        </Button>
        <Button type="button" size="sm" variant="outline" onClick={() => change((all) => all.map((row) => [...row, ...Array.from({ length: Math.max(0, columns - row.length) }, () => ''), '']))}>
          Add column
        </Button>
      </EditBar>
      <FileTable
        rows={draft.rows}
        onChange={(row, column, value) =>
          change((all) =>
            all.map((cells, index) => {
              if (index !== row) return cells
              const next = [...cells]
              while (next.length <= column) next.push('')
              next[column] = value
              return next
            }),
          )
        }
        onRemoveRow={(row) => change((all) => all.filter((_, index) => index !== row))}
      />
    </>
  )
}

function TextView({ target, first, onDirty }: { target: FileTarget; first: PageFileContent; onDirty: (dirty: boolean) => void }) {
  const [draft, setDraft] = useState<{ text: string; version: number; dirty: boolean } | null>(null)
  const [loadError, setLoadError] = useState<string | null>(null)
  const save = useSaveFile(target)
  const parts = useFileParts(target, undefined, draft === null)
  const text = parts.data?.pages.map((part) => part.text ?? '').join('') ?? first.text ?? ''

  const startEdit = () => {
    setLoadError(null)
    save.reset()
    fetchAllText(target).then(
      (all) => setDraft({ ...all, dirty: false }),
      (error: unknown) => setLoadError(problemText(error, 'The file could not be loaded for editing.')),
    )
  }
  const stop = () => {
    setDraft(null)
    onDirty(false)
    save.reset()
  }

  if (!draft) {
    return (
      <>
        {first.file.can_edit ? (
          <div className="flex shrink-0 items-center gap-2 border-b border-border px-3 py-2">
            <Button type="button" size="sm" variant="outline" onClick={startEdit}>
              Edit
            </Button>
            {loadError ? <span className="text-xs text-destructive">{loadError}</span> : null}
          </div>
        ) : null}
        <div className="min-h-0 flex-1 overflow-auto p-3">
          {first.no_text ? (
            <Centered>No text was found in this file.</Centered>
          ) : (
            <pre className="font-mono text-[13px] break-words whitespace-pre-wrap" data-file-text="">
              {text}
            </pre>
          )}
          {parts.hasNextPage ? (
            <Button type="button" variant="outline" size="sm" className="mt-3" disabled={parts.isFetchingNextPage} onClick={() => void parts.fetchNextPage()}>
              {parts.isFetchingNextPage ? 'Loading…' : 'Load more'}
            </Button>
          ) : null}
        </div>
      </>
    )
  }
  return (
    <>
      <EditBar
        dirty={draft.dirty}
        pending={save.isPending}
        conflict={isConflict(save.error)}
        error={save.error && !isConflict(save.error) ? problemText(save.error, 'The file could not be saved.') : null}
        onSave={() => save.mutate({ expected_version: draft.version, text: draft.text }, { onSuccess: stop })}
        onCancel={stop}
        onReload={() => {
          onDirty(false)
          startEdit()
        }}
      />
      <Textarea
        aria-label="File text"
        className="min-h-0 flex-1 resize-none rounded-none border-0 font-mono text-[13px] shadow-none focus-visible:ring-0"
        value={draft.text}
        onChange={(event) => {
          setDraft({ ...draft, text: event.target.value, dirty: true })
          onDirty(true)
        }}
      />
    </>
  )
}

function VersionsButton({ target, current }: { target: FileTarget; current: number }) {
  const [open, setOpen] = useState(false)
  const versions = useFileVersions(target, open)
  const restore = useRestoreFileVersion(target)
  return (
    <Popover open={open} onOpenChange={setOpen} modal={false}>
      <Tip label="Versions" side="bottom">
        <PopoverTrigger render={<Button type="button" variant="ghost" size="icon-sm" className="text-muted-foreground" aria-label="Versions" />}>
          <History className="size-4" />
        </PopoverTrigger>
      </Tip>
      <PopoverContent align="end" className="max-h-80 w-80 gap-1 overflow-y-auto p-2" data-file-versions="">
        {versions.isPending ? <Spinner className="mx-auto my-2" /> : null}
        {(versions.data ?? []).map((version) => {
          return (
            <div key={version.version} className="flex items-center gap-2 rounded-md px-2 py-1.5 text-[13px]" data-file-version={version.version}>
              <span className="min-w-0 flex-1">
                <span className="font-medium">Version {version.version}</span>
                {version.current ? <span className="text-muted-foreground"> · current</span> : null}
                <span className="block truncate text-xs text-muted-foreground">
                  {version.created_by_name ?? 'Unknown'} · {dayLabel(version.created_at)} {timeOfDay(version.created_at)} · {formatSize(version.size_bytes)}
                </span>
              </span>
              {version.current ? null : (
                <Button
                  type="button"
                  size="xs"
                  variant="outline"
                  disabled={restore.isPending}
                  onClick={() =>
                    restore.mutate(
                      { version: version.version, expectedVersion: current },
                      { onSuccess: () => setOpen(false) },
                    )
                  }
                >
                  Restore
                </Button>
              )}
            </div>
          )
        })}
        {restore.error ? (
          <p className="px-2 py-1 text-xs text-destructive" role="alert">
            {problemText(restore.error, 'The version could not be restored.')}
          </p>
        ) : null}
      </PopoverContent>
    </Popover>
  )
}

/** A file of the page in a panel beside it: its pages, table or text, and a download button. */
export function FilePanel({
  target,
  readOnly,
  onDirty,
  onClose,
}: {
  target: FileTarget
  readOnly: boolean
  /** Whether the panel holds changes that are not saved; the page asks before it leaves them. */
  onDirty: (dirty: boolean) => void
  onClose: () => void
}) {
  const content = useFileContent(target)
  const [pdfFailed, setPdfFailed] = useState(false)
  const onPdfError = useCallback(() => setPdfFailed(true), [])
  const url = fileUrl(target)
  const data = content.data
  const file = data ? { ...data.file, can_edit: data.file.can_edit && !readOnly } : null

  const shown = data && file ? { ...data, file } : null

  return (
    <aside
      aria-label="File"
      data-file-panel=""
      data-print-hide=""
      className="flex w-[calc(var(--file-share)*100%)] shrink-0 animate-in flex-col border-l border-border bg-background duration-150 ease-out fade-in max-[1099px]:absolute max-[1099px]:inset-y-0 max-[1099px]:right-0 max-[1099px]:z-20 max-[1099px]:w-[min(100%,640px)] max-[1099px]:min-w-[360px] max-[1099px]:shadow-xl max-[599px]:w-full max-[599px]:min-w-0 motion-reduce:animate-none min-[1100px]:group-data-[split=file]/split:border-l-0 min-[1100px]:group-data-[split=page]/split:invisible min-[1100px]:group-data-[split=page]/split:absolute min-[1100px]:group-data-[split=page]/split:inset-y-0 min-[1100px]:group-data-[split=page]/split:right-0 min-[1100px]:group-data-[split=page]/split:w-1/2"
    >
      <div className="flex h-12 shrink-0 items-center gap-1 border-b border-border pr-2 pl-3">
        <span className="min-w-0 flex-1 truncate text-[13px] font-semibold text-foreground" title={file?.file_name}>
          {file?.file_name ?? 'File'}
        </span>
        {file ? <span className="shrink-0 pr-1 text-xs text-muted-foreground">{formatSize(file.size_bytes)}</span> : null}
        {file?.kind === 'delimited' || file?.kind === 'text' || file?.kind === 'sheet' ? <VersionsButton target={target} current={file.version} /> : null}
        <Tip label="Download" side="bottom">
          <a href={url} download aria-label="Download" className={cn(buttonVariants({ variant: 'ghost', size: 'icon-sm' }), 'text-muted-foreground')}>
            <Download className="size-4" />
          </a>
        </Tip>
        <Tip label="Close" side="bottom">
          <Button type="button" variant="ghost" size="icon-sm" className="text-muted-foreground" aria-label="Close file" onClick={onClose}>
            <X className="size-4" />
          </Button>
        </Tip>
      </div>
      {content.isPending ? (
        <Centered>
          <Spinner />
        </Centered>
      ) : content.isError || !shown ? (
        <Centered>
          {problemText(content.error, 'This file could not be opened.')}
          <DownloadLink url={url}>Download</DownloadLink>
        </Centered>
      ) : shown.file.kind === 'pdf' ? (
        pdfFailed ? (
          <Centered>
            This PDF could not be shown. It may be damaged or protected.
            <DownloadLink url={url}>Download</DownloadLink>
          </Centered>
        ) : (
          <div className="min-h-0 flex-1 overflow-y-auto bg-muted/40">
            <Suspense fallback={<Centered><Spinner /></Centered>}>
              <PdfPages url={url} onError={onPdfError} />
            </Suspense>
          </div>
        )
      ) : shown.file.kind === 'image' ? (
        <div className="flex min-h-0 flex-1 items-start justify-center overflow-auto p-3">
          <img src={url} alt={shown.file.file_name} className="max-w-full" />
        </div>
      ) : shown.file.kind === 'sheet' || shown.file.kind === 'legacy_sheet' ? (
        <SheetView key={shown.file.version} target={target} first={shown} onDirty={onDirty} />
      ) : shown.file.kind === 'delimited' ? (
        <DelimitedView target={target} first={shown} onDirty={onDirty} />
      ) : shown.file.kind === 'text' || shown.file.kind === 'docx' ? (
        <TextView target={target} first={shown} onDirty={onDirty} />
      ) : (
        <Centered>
          This file cannot be shown here.
          <DownloadLink url={url}>Download</DownloadLink>
        </Centered>
      )}
    </aside>
  )
}
