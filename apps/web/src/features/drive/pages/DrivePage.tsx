import { useEffect, useMemo, useRef, useState, type DragEvent, type MouseEvent } from 'react'
import { useNavigate, useParams, useSearchParams } from 'react-router'
import { toast } from 'sonner'
import {
  Add,
  ArrowRight2,
  Category,
  CloudAdd,
  DocumentDownload,
  Edit2,
  Folder,
  FolderAdd,
  Link,
  More,
  RowVertical,
  Search4,
  Trash,
} from 'reicon-react'
import type { DriveEntry, DriveNode } from '@orbit/api/generated/types.gen'
import { needsThumbnail } from '@orbit/api/drive'
import { ApiProblem } from '@orbit/api/problem'
import { cn } from 'cn'
import { EmptyState } from '@/components/common/EmptyState'
import { Pane, PaneHeader, PaneTitle } from '@/components/common/Pane'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { Input } from '@/components/ui/input'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { Spinner } from '@/components/ui/spinner'
import { relativeTime } from '@/lib/format'
import { formatBytes } from '@/lib/formatBytes'
import { useWorkspace } from '@/features/workspaces/workspaceContext'
import { InAppSidebar, SidebarListHeader } from '@/app/shell/InAppSidebar'
import { useInAppSidebar } from '@/app/shell/sidebarSlot'
import {
  createFolder,
  moveNode,
  trashNode,
  useDriveChildren,
  useDriveNode,
  useDriveSearch,
  useRefreshDrive,
  useRenameNode,
  type DriveSearchInput,
  type DriveSort,
  type SortOrder,
} from '../api'
import { DRIVE_DRAG_TYPE, FolderTree } from '../components/FolderTree'
import { MoveDialog } from '../components/MoveDialog'
import { NameDialog } from '../components/NameDialog'
import { NodeIcon } from '../components/NodeIcon'
import { ShareDialog } from '../components/ShareDialog'
import { UploadPanel } from '../components/UploadPanel'
import { DriveTrash } from '../components/DriveTrash'
import { DriveViewer } from '../components/DriveViewer'
import { queueThumbnails } from '../media'
import { downloadFile, downloadZip } from '../download'
import { draggingFiles, droppedFiles, pickedFolder } from '../dropped'
import { uploadQueue, useUploads } from '../uploads'
import { driveClick } from '../click'

type Layout = 'list' | 'grid'
type Dialog = { kind: 'new-folder' } | { kind: 'rename'; node: DriveNode } | { kind: 'move'; nodes: DriveNode[] } | { kind: 'share'; node: DriveNode } | null

const LAYOUT_KEY = 'orbit.drive.layout'
const SORTS: { value: string; label: string }[] = [
  { value: 'name:asc', label: 'Name A–Z' },
  { value: 'name:desc', label: 'Name Z–A' },
  { value: 'updated_at:desc', label: 'Newest first' },
  { value: 'updated_at:asc', label: 'Oldest first' },
  { value: 'size:desc', label: 'Largest first' },
  { value: 'size:asc', label: 'Smallest first' },
]
const TYPES: { value: string; label: string; search: Pick<DriveSearchInput, 'kind' | 'mime'> }[] = [
  { value: 'any', label: 'Any type', search: {} },
  { value: 'folder', label: 'Folders', search: { kind: 'folder' } },
  { value: 'image', label: 'Images', search: { kind: 'file', mime: 'image/' } },
  { value: 'video', label: 'Videos', search: { kind: 'file', mime: 'video/' } },
  { value: 'pdf', label: 'PDFs', search: { kind: 'file', mime: 'application/pdf' } },
  { value: 'audio', label: 'Audio', search: { kind: 'file', mime: 'audio/' } },
]
const DATES: { value: string; label: string; days: number | null }[] = [
  { value: 'any', label: 'Any time', days: null },
  { value: '7', label: 'Last 7 days', days: 7 },
  { value: '30', label: 'Last 30 days', days: 30 },
  { value: '365', label: 'Last year', days: 365 },
]

function problemText(error: unknown, fallback: string) {
  return error instanceof ApiProblem ? error.detail : fallback
}

/** Drive: `/drive` (top level), `/drive/:folderId`, `/drive/trash`; `?q=`, `?type=`, `?date=` search the whole drive. */
export function DrivePage({ view }: { view?: 'trash' }) {
  const { workspace } = useWorkspace()
  const workspaceId = workspace.id
  const navigate = useNavigate()
  const folderId = useParams().folderId ?? null
  const [params, setParams] = useSearchParams()
  const query = params.get('q') ?? ''
  const type = params.get('type') ?? 'any'
  const date = params.get('date') ?? 'any'
  const searching = query !== '' || type !== 'any' || date !== 'any'
  const [sortValue, setSortValue] = useState('name:asc')
  const [sort, order] = sortValue.split(':') as [DriveSort, SortOrder]
  const [layout, setLayout] = useState<Layout>(() => (localStorage.getItem(LAYOUT_KEY) === 'grid' ? 'grid' : 'list'))
  const [selected, setSelected] = useState<ReadonlySet<string>>(new Set())
  const [anchor, setAnchor] = useState<string | null>(null)
  const [dialog, setDialog] = useState<Dialog>(null)
  const [viewing, setViewing] = useState<string | null>(null)
  const [dropping, setDropping] = useState(false)
  const [busy, setBusy] = useState(false)
  const fileInput = useRef<HTMLInputElement>(null)
  const folderInput = useRef<HTMLInputElement>(null)
  const refresh = useRefreshDrive(workspaceId)
  const rename = useRenameNode(workspaceId)

  const folder = useDriveNode(workspaceId, folderId)
  const children = useDriveChildren(workspaceId, folderId, sort, order)
  const days = DATES.find((option) => option.value === date)?.days ?? null
  const search = useMemo<DriveSearchInput>(
    () => ({
      q: query,
      ...(TYPES.find((option) => option.value === type)?.search ?? {}),
      updatedAfter: days === null ? undefined : new Date(Date.now() - days * 86_400_000).toISOString(),
    }),
    [query, type, days],
  )
  const results = useDriveSearch(workspaceId, search, searching)

  const entries: DriveEntry[] = useMemo(() => {
    if (searching) return results.data?.pages.flatMap((page) => page.items) ?? []
    return children.data?.pages.flatMap((page) => page.items.map((node) => ({ node, folder_path: '' }))) ?? []
  }, [searching, results.data, children.data])
  const nodes = entries.map((entry) => entry.node)
  const list = searching ? results : children
  const selectedNodes = nodes.filter((node) => selected.has(node.id))

  // Finished uploads show up in the folder.
  const doneUploads = useUploads().filter((item) => item.status === 'done').length
  useEffect(() => {
    if (doneUploads > 0) refresh()
    // eslint-disable-next-line react-hooks/exhaustive-deps -- only a newly finished upload refreshes
  }, [doneUploads])

  useEffect(() => {
    setSelected(new Set())
    setAnchor(null)
  }, [folderId, searching, query, type, date])

  useEffect(() => {
    localStorage.setItem(LAYOUT_KEY, layout)
  }, [layout])

  // Older images and videos get their thumbnail the first time someone sees them in the grid.
  const missingThumbnails = layout === 'grid' ? nodes.filter(needsThumbnail) : []
  const missingKey = missingThumbnails.map((node) => node.id).join(',')
  useEffect(() => {
    if (missingThumbnails.length > 0) queueThumbnails(workspaceId, missingThumbnails, refresh)
    // eslint-disable-next-line react-hooks/exhaustive-deps -- the ids decide; the nodes are new objects on each refetch
  }, [workspaceId, missingKey])

  if (view === 'trash') {
    return (
      <div className="flex h-full min-h-0 min-w-0 flex-1">
        <DriveFolders workspaceId={workspaceId} selected="" trash onSelect={(id) => navigate(id === null ? '/drive' : `/drive/${id}`)} />
        <DriveTrash workspaceId={workspaceId} />
      </div>
    )
  }

  const openFolder = (id: string | null) => navigate(id === null ? '/drive' : `/drive/${id}`)
  const open = (node: DriveNode) => {
    if (node.kind === 'folder') openFolder(node.id)
    else setViewing(node.id)
  }

  // A plain click or tap opens the item; Ctrl/Cmd+click and Shift+click select, as the checkbox does (`driveClick`).
  const click = (node: DriveNode, event: MouseEvent) => {
    const from = anchor === null ? -1 : nodes.findIndex((candidate) => candidate.id === anchor)
    const action = driveClick(event, from !== -1)
    if (action === 'open') return open(node)
    if (action === 'toggle') return toggle(node.id)
    const to = nodes.findIndex((candidate) => candidate.id === node.id)
    const [start, end] = from < to ? [from, to] : [to, from]
    setSelected(new Set(nodes.slice(start, end + 1).map((candidate) => candidate.id)))
  }
  const toggle = (id: string) => {
    setSelected((current) => {
      const next = new Set(current)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })
    setAnchor(id)
  }

  const moveNodes = async (ids: string[], target: string | null) => {
    const moving = ids.filter((id) => id !== target)
    if (moving.length === 0) return
    setBusy(true)
    let moved = 0
    const failures: string[] = []
    for (const id of moving) {
      try {
        await moveNode(workspaceId, id, target)
        moved += 1
      } catch (error) {
        failures.push(problemText(error, 'Could not move an item.'))
      }
    }
    setBusy(false)
    setSelected(new Set())
    refresh()
    if (moved > 0) toast.success(moved === 1 ? 'Moved 1 item' : `Moved ${moved} items`)
    if (failures.length > 0) toast.error(failures[0], { description: failures.length > 1 ? `${failures.length} items were not moved.` : undefined })
  }

  const trashNodes = async (targets: DriveNode[]) => {
    setBusy(true)
    const trashed: DriveNode[] = []
    for (const node of targets) {
      try {
        await trashNode(workspaceId, node.id)
        trashed.push(node)
      } catch (error) {
        toast.error(problemText(error, `Could not move “${node.name}” to the trash.`))
      }
    }
    setBusy(false)
    setSelected(new Set())
    refresh()
    if (trashed.length > 0) {
      toast.success(trashed.length === 1 ? `Moved “${trashed[0]?.name}” to the trash` : `Moved ${trashed.length} items to the trash`, {
        action: { label: 'Open trash', onClick: () => navigate('/drive/trash') },
      })
    }
  }

  const download = async (targets: DriveNode[]) => {
    const [only] = targets
    if (targets.length === 1 && only?.kind === 'file') {
      await downloadFile(workspaceId, only.id).catch((error: unknown) => toast.error(problemText(error, 'Could not download the file.')))
      return
    }
    const name = targets.length === 1 && only ? only.name : folder.data?.node.name ?? 'Drive'
    const progress = toast.loading('Preparing the ZIP…')
    try {
      const count = await downloadZip(workspaceId, targets, name)
      toast.dismiss(progress)
      if (count !== null) toast.success(`Downloaded ${count} ${count === 1 ? 'file' : 'files'} as ${name}.zip`)
    } catch (error) {
      toast.dismiss(progress)
      toast.error(error instanceof Error ? error.message : 'Could not build the ZIP.')
    }
  }

  const upload = (picked: Parameters<typeof uploadQueue.add>[2]) => {
    if (picked.length === 0) return
    void uploadQueue.add(workspaceId, folderId, picked).catch((error: unknown) => toast.error(problemText(error, 'Could not start the upload.')))
  }

  const dragNodes = (event: DragEvent, node: DriveNode) => {
    const ids = selected.has(node.id) ? [...selected] : [node.id]
    event.dataTransfer.setData(DRIVE_DRAG_TYPE, JSON.stringify(ids))
    event.dataTransfer.effectAllowed = 'move'
  }

  const crumbs = folderId === null ? [] : [...(folder.data?.path ?? []), ...(folder.data ? [{ id: folder.data.node.id, name: folder.data.node.name }] : [])]
  const title = searching ? 'Search results' : folder.data?.node.name ?? (folderId === null ? 'My Drive' : '')

  let body
  if (folderId !== null && folder.isError) {
    body = (
      <div className="p-6">
        <EmptyState icon={Folder} title="Folder not found" description="It was moved to the trash or deleted." action={<Button type="button" variant="outline" onClick={() => openFolder(null)}>Go to My Drive</Button>} />
      </div>
    )
  } else if (list.isError) {
    body = (
      <div className="p-6">
        <EmptyState
          icon={Folder}
          title="Drive unavailable"
          description={problemText(list.error, 'The server could not load the files.')}
          action={<Button type="button" variant="outline" onClick={() => void list.refetch()}>Retry</Button>}
        />
      </div>
    )
  } else if (list.data === undefined) {
    body = (
      <div className="flex flex-1 items-center justify-center p-10">
        <Spinner className="text-muted-foreground" />
      </div>
    )
  } else if (entries.length === 0) {
    body = (
      <div className="p-6">
        {searching ? (
          <EmptyState icon={Search4} title="Nothing found" description="No file or folder matches the search." />
        ) : (
          <EmptyState icon={CloudAdd} title="This folder is empty" description="Drop files or folders here, or use New." />
        )}
      </div>
    )
  } else if (layout === 'grid') {
    body = (
      <div className="grid grid-cols-[repeat(auto-fill,minmax(150px,1fr))] gap-3 p-4" role="grid" aria-label="Files">
        {entries.map((entry) => (
          <GridCard
            key={entry.node.id}
            entry={entry}
            searching={searching}
            selected={selected.has(entry.node.id)}
            onClick={click}
            onToggle={toggle}
            onOpen={open}
            onDragStart={dragNodes}
            onDropNodes={moveNodes}
            onThumbnailExpired={refresh}
          />
        ))}
      </div>
    )
  } else {
    body = (
      <div role="grid" aria-label="Files" aria-multiselectable="true" className="min-w-0">
        <div role="row" className="sticky top-0 z-10 flex h-8 items-center gap-3 border-b bg-background px-3 text-xs text-muted-foreground">
          <span className="w-4" />
          <span className="flex-1">Name</span>
          {searching ? <span className="w-48 max-[899px]:hidden">Folder</span> : null}
          <span className="w-24 text-right max-[699px]:hidden">Size</span>
          <span className="w-32 max-[699px]:hidden">Modified</span>
          <span className="w-7" />
        </div>
        {entries.map((entry) => (
          <ListRow
            key={entry.node.id}
            entry={entry}
            searching={searching}
            selected={selected.has(entry.node.id)}
            onClick={click}
            onToggle={toggle}
            onOpen={open}
            onDragStart={dragNodes}
            onDropNodes={moveNodes}
            onRename={(node) => setDialog({ kind: 'rename', node })}
            onShare={(node) => setDialog({ kind: 'share', node })}
            onMove={(node) => setDialog({ kind: 'move', nodes: [node] })}
            onTrash={(node) => void trashNodes([node])}
            onDownload={(node) => void download([node])}
          />
        ))}
      </div>
    )
  }

  return (
    <div className="flex h-full min-h-0 min-w-0 flex-1">
      <DriveFolders workspaceId={workspaceId} selected={searching ? '' : folderId} onSelect={openFolder} onDropNodes={(target, ids) => void moveNodes(ids, target)} />
      <Pane
        className={cn('relative', dropping && 'ring-2 ring-primary ring-inset')}
        onDragOver={(event) => {
          if (!draggingFiles(event.dataTransfer) || searching) return
          event.preventDefault()
          setDropping(true)
        }}
        onDragLeave={(event) => {
          if (event.currentTarget.contains(event.relatedTarget as Node | null)) return
          setDropping(false)
        }}
        onDrop={(event) => {
          setDropping(false)
          if (!draggingFiles(event.dataTransfer) || searching) return
          event.preventDefault()
          void droppedFiles(event.dataTransfer).then(upload)
        }}
      >
        <PaneHeader className="flex-wrap">
          <nav aria-label="Breadcrumbs" className="flex min-w-0 flex-1 items-center gap-1 text-[13px]">
            {searching ? (
              <PaneTitle render={<h1 />}>{title}</PaneTitle>
            ) : (
              <>
                <Crumb label="My Drive" current={folderId === null} onOpen={() => openFolder(null)} onDropNodes={(ids) => void moveNodes(ids, null)} />
                {crumbs.map((crumb, index) => (
                  <span key={crumb.id} className="flex min-w-0 items-center gap-1">
                    <ArrowRight2 className="size-3 shrink-0 text-muted-foreground" />
                    <Crumb label={crumb.name} current={index === crumbs.length - 1} onOpen={() => openFolder(crumb.id)} onDropNodes={(ids) => void moveNodes(ids, crumb.id)} />
                  </span>
                ))}
              </>
            )}
          </nav>
          <form
            role="search"
            className="relative w-64 max-[899px]:w-full"
            onSubmit={(event) => {
              event.preventDefault()
              const value = new FormData(event.currentTarget).get('q')?.toString().trim() ?? ''
              setParams((current) => {
                const next = new URLSearchParams(current)
                if (value) next.set('q', value)
                else next.delete('q')
                return next
              })
            }}
          >
            <Search4 className="pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2 text-muted-foreground" />
            <Input key={query} name="q" defaultValue={query} placeholder="Search Drive" aria-label="Search Drive" className="h-8 pl-8" />
          </form>
        </PaneHeader>
        <div className="flex h-11 shrink-0 items-center gap-2 overflow-x-auto border-b px-3 whitespace-nowrap">
          {selectedNodes.length > 0 ? (
            <>
              <span className="shrink-0 text-[13px] font-medium">{selectedNodes.length} selected</span>
              <Button type="button" variant="outline" size="sm" disabled={busy} onClick={() => void download(selectedNodes)}>
                <DocumentDownload className="size-3.5" /> Download
              </Button>
              <Button type="button" variant="outline" size="sm" disabled={busy} onClick={() => setDialog({ kind: 'move', nodes: selectedNodes })}>
                Move
              </Button>
              {selectedNodes.length === 1 && selectedNodes[0] ? (
                <Button type="button" variant="outline" size="sm" disabled={busy} onClick={() => selectedNodes[0] && setDialog({ kind: 'rename', node: selectedNodes[0] })}>
                  <Edit2 className="size-3.5" /> Rename
                </Button>
              ) : null}
              {selectedNodes.length === 1 && selectedNodes[0] ? (
                <Button type="button" variant="outline" size="sm" disabled={busy} onClick={() => selectedNodes[0] && setDialog({ kind: 'share', node: selectedNodes[0] })}>
                  <Link className="size-3.5" /> Share
                </Button>
              ) : null}
              <Button type="button" variant="outline" size="sm" disabled={busy} className="text-destructive hover:text-destructive" onClick={() => void trashNodes(selectedNodes)}>
                <Trash className="size-3.5" /> Move to trash
              </Button>
              <Button type="button" variant="ghost" size="sm" onClick={() => setSelected(new Set())}>Clear</Button>
            </>
          ) : (
            <>
              {searching ? null : (
                <DropdownMenu>
                  <DropdownMenuTrigger render={<Button type="button" size="sm" />}>
                    <Add className="size-3.5" /> New
                  </DropdownMenuTrigger>
                  <DropdownMenuContent align="start" className="w-48">
                    <DropdownMenuItem onClick={() => setDialog({ kind: 'new-folder' })}>
                      <FolderAdd /> New folder
                    </DropdownMenuItem>
                    <DropdownMenuSeparator />
                    <DropdownMenuItem onClick={() => fileInput.current?.click()}>
                      <CloudAdd /> Upload files
                    </DropdownMenuItem>
                    <DropdownMenuItem onClick={() => folderInput.current?.click()}>
                      <Folder /> Upload folder
                    </DropdownMenuItem>
                  </DropdownMenuContent>
                </DropdownMenu>
              )}
              {/* A filter searches the whole drive. */}
              <FilterSelect label="Type" value={type} options={TYPES} onChange={(value) => setParam(setParams, 'type', value)} />
              <FilterSelect label="Modified" value={date} options={DATES} onChange={(value) => setParam(setParams, 'date', value)} />
              {searching ? (
                <Button type="button" variant="ghost" size="sm" onClick={() => setParams(new URLSearchParams())}>Clear search</Button>
              ) : null}
            </>
          )}
          <span className="flex-1" />
          {searching ? null : <FilterSelect label="Sort" value={sortValue} options={SORTS} onChange={setSortValue} />}
          <div className="flex shrink-0 items-center rounded-md border" role="group" aria-label="Layout">
            <Button type="button" variant="ghost" size="icon-sm" aria-label="List" aria-pressed={layout === 'list'} className={cn(layout === 'list' && 'bg-accent')} onClick={() => setLayout('list')}>
              <RowVertical className="size-4" />
            </Button>
            <Button type="button" variant="ghost" size="icon-sm" aria-label="Grid" aria-pressed={layout === 'grid'} className={cn(layout === 'grid' && 'bg-accent')} onClick={() => setLayout('grid')}>
              <Category className="size-4" />
            </Button>
          </div>
        </div>
        <div className="min-h-0 flex-1 overflow-y-auto">
          {body}
          {list.hasNextPage ? (
            <div className="flex justify-center p-3">
              <Button type="button" variant="outline" size="sm" disabled={list.isFetchingNextPage} onClick={() => void list.fetchNextPage()}>
                {list.isFetchingNextPage ? 'Loading…' : 'Load more'}
              </Button>
            </div>
          ) : null}
        </div>
        <input
          ref={fileInput}
          type="file"
          multiple
          hidden
          aria-label="Files to upload"
          onChange={(event) => {
            const files = [...(event.target.files ?? [])].map((file) => ({ file }))
            event.target.value = ''
            upload(files)
          }}
        />
        <input
          ref={folderInput}
          type="file"
          hidden
          aria-label="Folder to upload"
          {...{ webkitdirectory: '' }}
          onChange={(event) => {
            const files = event.target.files ? pickedFolder(event.target.files) : []
            event.target.value = ''
            upload(files)
          }}
        />
      </Pane>
      <UploadPanel workspaceId={workspaceId} />
      {viewing ? (
        <DriveViewer
          workspaceId={workspaceId}
          files={nodes.filter((node) => node.kind === 'file')}
          startId={viewing}
          onDownload={(node) => void download([node])}
          onClose={() => setViewing(null)}
        />
      ) : null}
      {dialog?.kind === 'share' ? <ShareDialog workspaceId={workspaceId} node={dialog.node} onClose={() => setDialog(null)} /> : null}
      {dialog?.kind === 'new-folder' ? (
        <NameDialog
          title="New folder"
          label="Folder name"
          initial="Untitled folder"
          submitLabel="Create"
          onSubmit={async (name) => {
            try {
              await createFolder(workspaceId, folderId, name)
            } catch (error) {
              throw new Error(problemText(error, 'Could not create the folder.'))
            }
            refresh()
          }}
          onClose={() => setDialog(null)}
        />
      ) : null}
      {dialog?.kind === 'rename' ? (
        <NameDialog
          title={`Rename ${dialog.node.kind}`}
          label="Name"
          initial={dialog.node.name}
          submitLabel="Rename"
          onSubmit={async (name) => {
            try {
              await rename.mutateAsync({ nodeId: dialog.node.id, name })
            } catch (error) {
              throw new Error(problemText(error, 'Could not rename.'))
            }
          }}
          onClose={() => setDialog(null)}
        />
      ) : null}
      {dialog?.kind === 'move' ? (
        <MoveDialog
          workspaceId={workspaceId}
          count={dialog.nodes.length}
          movingFolders={new Set(dialog.nodes.filter((node) => node.kind === 'folder').map((node) => node.id))}
          currentFolder={searching ? null : folderId}
          onMove={(target) => moveNodes(dialog.nodes.map((node) => node.id), target)}
          onClose={() => setDialog(null)}
        />
      ) : null}
    </div>
  )
}

function setParam(setParams: ReturnType<typeof useSearchParams>[1], name: string, value: string) {
  setParams((current) => {
    const next = new URLSearchParams(current)
    if (value === 'any') next.delete(name)
    else next.set(name, value)
    return next
  })
}

function FilterSelect({ label, value, options, onChange }: { label: string; value: string; options: { value: string; label: string }[]; onChange: (value: string) => void }) {
  return (
    <Select items={options} value={value} onValueChange={(next) => onChange(String(next))}>
      <SelectTrigger size="sm" aria-label={label} className="h-8 w-36 shrink-0">
        <SelectValue />
      </SelectTrigger>
      <SelectContent>
        {options.map((option) => <SelectItem key={option.value} value={option.value}>{option.label}</SelectItem>)}
      </SelectContent>
    </Select>
  )
}

/** A breadcrumb; Drive items dropped on it move into that folder. */
/** The folder tree and Trash: in the app sidebar when it has room (as the Docs page tree), otherwise beside the files. */
function DriveFolders({ workspaceId, selected, trash = false, onSelect, onDropNodes }: {
  workspaceId: string
  selected: string | null
  trash?: boolean
  onSelect: (folderId: string | null) => void
  onDropNodes?: (folderId: string | null, nodeIds: string[]) => void
}) {
  const navigate = useNavigate()
  const inAppSidebar = useInAppSidebar()
  return (
    <InAppSidebar>
      <aside className={cn('flex min-h-0 flex-col', inAppSidebar ? 'flex-1' : 'w-60 shrink-0 border-r max-[899px]:hidden')} aria-label="Drive folders">
        {inAppSidebar ? (
          <SidebarListHeader label="Folders" />
        ) : (
          <PaneHeader>
            <PaneTitle>Drive</PaneTitle>
          </PaneHeader>
        )}
        <div className="min-h-0 flex-1 overflow-y-auto px-2">
          <FolderTree workspaceId={workspaceId} selected={selected} onSelect={onSelect} onDropNodes={onDropNodes} />
        </div>
        <div className="shrink-0 border-t p-2">
          <button
            type="button"
            aria-current={trash ? 'page' : undefined}
            className={cn(
              'flex h-8 w-full min-w-0 items-center gap-2 rounded-md px-2 text-[13px] font-medium text-muted-foreground transition-colors hover:bg-sidebar-accent/50 hover:text-foreground',
              trash && 'bg-sidebar-accent text-sidebar-accent-foreground',
            )}
            onClick={() => navigate('/drive/trash')}
          >
            <Trash className="size-[15px] shrink-0" /> Trash
          </button>
        </div>
      </aside>
    </InAppSidebar>
  )
}

function Crumb({ label, current, onOpen, onDropNodes }: { label: string; current: boolean; onOpen: () => void; onDropNodes: (ids: string[]) => void }) {
  const [over, setOver] = useState(false)
  return (
    <button
      type="button"
      aria-current={current ? 'page' : undefined}
      className={cn(
        'min-w-0 truncate rounded px-1.5 py-0.5 hover:bg-black/4 dark:hover:bg-white/5',
        current ? 'font-semibold text-foreground' : 'text-muted-foreground',
        over && 'ring-2 ring-primary/60',
      )}
      onClick={onOpen}
      onDragOver={(event) => {
        if (!event.dataTransfer.types.includes(DRIVE_DRAG_TYPE)) return
        event.preventDefault()
        setOver(true)
      }}
      onDragLeave={() => setOver(false)}
      onDrop={(event) => {
        setOver(false)
        if (!event.dataTransfer.types.includes(DRIVE_DRAG_TYPE)) return
        event.preventDefault()
        onDropNodes(JSON.parse(event.dataTransfer.getData(DRIVE_DRAG_TYPE)) as string[])
      }}
    >
      {label}
    </button>
  )
}

interface ItemProps {
  entry: DriveEntry
  searching: boolean
  selected: boolean
  onClick: (node: DriveNode, event: MouseEvent) => void
  onToggle: (id: string) => void
  onOpen: (node: DriveNode) => void
  onDragStart: (event: DragEvent, node: DriveNode) => void
  onDropNodes: (ids: string[], target: string) => Promise<void>
}

/** Drop handling of a folder row or card: Drive items move into the folder. */
function useFolderDrop(node: DriveNode, onDropNodes: ItemProps['onDropNodes']) {
  const [over, setOver] = useState(false)
  if (node.kind !== 'folder') return { over: false, handlers: {} }
  return {
    over,
    handlers: {
      onDragOver: (event: DragEvent) => {
        if (!event.dataTransfer.types.includes(DRIVE_DRAG_TYPE)) return
        event.preventDefault()
        event.stopPropagation()
        setOver(true)
      },
      onDragLeave: () => setOver(false),
      onDrop: (event: DragEvent) => {
        setOver(false)
        if (!event.dataTransfer.types.includes(DRIVE_DRAG_TYPE)) return
        event.preventDefault()
        event.stopPropagation()
        const ids = (JSON.parse(event.dataTransfer.getData(DRIVE_DRAG_TYPE)) as string[]).filter((id) => id !== node.id)
        void onDropNodes(ids, node.id)
      },
    },
  }
}

function ListRow({
  entry,
  searching,
  selected,
  onClick,
  onToggle,
  onOpen,
  onDragStart,
  onDropNodes,
  onRename,
  onShare,
  onMove,
  onTrash,
  onDownload,
}: ItemProps & {
  onRename: (node: DriveNode) => void
  onShare: (node: DriveNode) => void
  onMove: (node: DriveNode) => void
  onTrash: (node: DriveNode) => void
  onDownload: (node: DriveNode) => void
}) {
  const { node } = entry
  const drop = useFolderDrop(node, onDropNodes)
  return (
    <div
      role="row"
      aria-selected={selected}
      tabIndex={0}
      draggable
      data-node-kind={node.kind}
      className={cn(
        'group/row flex h-10 cursor-default items-center gap-3 border-b px-3 text-[13px] outline-none select-none hover:bg-black/3 focus-visible:bg-black/4 dark:hover:bg-white/4',
        selected && 'bg-primary/8 hover:bg-primary/10 dark:bg-primary/15',
        drop.over && 'ring-2 ring-primary/60 ring-inset',
      )}
      onClick={(event) => onClick(node, event)}
      onKeyDown={(event) => {
        if (event.key === 'Enter') onOpen(node)
        if (event.key === ' ') {
          event.preventDefault()
          onToggle(node.id)
        }
      }}
      onDragStart={(event) => onDragStart(event, node)}
      {...drop.handlers}
    >
      <Checkbox
        checked={selected}
        aria-label={`Select ${node.name}`}
        onClick={(event) => event.stopPropagation()}
        onCheckedChange={() => onToggle(node.id)}
      />
      <span className="flex min-w-0 flex-1 items-center gap-2.5">
        {node.thumbnail_url ? (
          <img src={node.thumbnail_url} alt="" loading="lazy" draggable={false} className="size-[18px] shrink-0 rounded-sm object-cover" />
        ) : (
          <NodeIcon node={node} className="size-[18px]" />
        )}
        <span className="truncate">{node.name}</span>
      </span>
      {searching ? <span className="w-48 truncate text-xs text-muted-foreground max-[899px]:hidden">{entry.folder_path || 'My Drive'}</span> : null}
      <span className="w-24 text-right text-xs text-muted-foreground tabular-nums max-[699px]:hidden">{node.size == null ? '—' : formatBytes(node.size)}</span>
      <span className="w-32 truncate text-xs text-muted-foreground max-[699px]:hidden">{relativeTime(node.updated_at)}</span>
      <DropdownMenu modal={false}>
        <DropdownMenuTrigger
          render={<Button type="button" variant="ghost" size="icon-xs" aria-label={`Actions for ${node.name}`} className="opacity-0 group-hover/row:opacity-100 group-focus-within/row:opacity-100 max-[899px]:opacity-100" />}
          onClick={(event) => event.stopPropagation()}
        >
          <More className="size-4" />
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end" className="w-44">
          <DropdownMenuItem onClick={() => onOpen(node)}>{node.kind === 'folder' ? <Folder /> : <DocumentDownload />} {node.kind === 'folder' ? 'Open' : 'Download'}</DropdownMenuItem>
          {node.kind === 'folder' ? <DropdownMenuItem onClick={() => onDownload(node)}><DocumentDownload /> Download ZIP</DropdownMenuItem> : null}
          <DropdownMenuItem onClick={() => onRename(node)}><Edit2 /> Rename</DropdownMenuItem>
          <DropdownMenuItem onClick={() => onShare(node)}><Link /> Share link…</DropdownMenuItem>
          <DropdownMenuItem onClick={() => onMove(node)}><ArrowRight2 /> Move</DropdownMenuItem>
          <DropdownMenuSeparator />
          <DropdownMenuItem variant="destructive" onClick={() => onTrash(node)}><Trash /> Move to trash</DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
    </div>
  )
}

function GridCard({ entry, searching, selected, onClick, onToggle, onOpen, onDragStart, onDropNodes, onThumbnailExpired }: ItemProps & { onThumbnailExpired: () => void }) {
  const { node } = entry
  const drop = useFolderDrop(node, onDropNodes)
  const retried = useRef(false)
  return (
    <div
      role="gridcell"
      aria-selected={selected}
      tabIndex={0}
      draggable
      className={cn(
        'group/card flex cursor-default flex-col overflow-hidden rounded-lg border bg-card outline-none select-none hover:border-primary/40 focus-visible:ring-2 focus-visible:ring-ring/50',
        selected && 'border-primary ring-1 ring-primary',
        drop.over && 'ring-2 ring-primary/60',
      )}
      onClick={(event) => onClick(node, event)}
      onKeyDown={(event) => {
        if (event.key === 'Enter') onOpen(node)
        if (event.key === ' ') {
          event.preventDefault()
          onToggle(node.id)
        }
      }}
      onDragStart={(event) => onDragStart(event, node)}
      {...drop.handlers}
    >
      <div className="relative flex aspect-[4/3] items-center justify-center overflow-hidden bg-muted/40">
        {/* the card opens on a click: this box selects it (shown on hover, always on touch screens and once selected) */}
        <Checkbox
          checked={selected}
          aria-label={`Select ${node.name}`}
          className={cn('absolute top-2 left-2 z-10 bg-background/80', !selected && 'hover-fine:opacity-0 hover-fine:group-hover/card:opacity-100 hover-fine:group-focus-within/card:opacity-100')}
          onClick={(event) => event.stopPropagation()}
          onCheckedChange={() => onToggle(node.id)}
        />
        {node.thumbnail_url ? (
          <img
            src={node.thumbnail_url}
            alt=""
            loading="lazy"
            draggable={false}
            className="size-full object-cover"
            data-drive-thumbnail
            // The URL lives 5 minutes: a later load asks for the list again (once, so a missing object cannot loop).
            onError={() => {
              if (retried.current) return
              retried.current = true
              onThumbnailExpired()
            }}
          />
        ) : (
          <NodeIcon node={node} className="size-10" />
        )}
        {node.media?.duration_ms != null ? (
          <span className="absolute right-1.5 bottom-1.5 rounded bg-black/70 px-1.5 py-0.5 text-[11px] font-medium text-white tabular-nums">
            {duration(node.media.duration_ms)}
          </span>
        ) : null}
      </div>
      <div className="flex flex-col gap-0.5 px-2.5 py-2">
        <span className="truncate text-[13px] font-medium">{node.name}</span>
        <span className="truncate text-xs text-muted-foreground">
          {searching ? entry.folder_path || 'My Drive' : node.size == null ? 'Folder' : formatBytes(node.size)}
        </span>
      </div>
    </div>
  )
}

/** `1:05`, `1:02:03`. */
function duration(milliseconds: number) {
  const seconds = Math.round(milliseconds / 1000)
  const [hours, minutes, rest] = [Math.floor(seconds / 3600), Math.floor((seconds % 3600) / 60), seconds % 60]
  const pad = (value: number) => value.toString().padStart(2, '0')
  return hours > 0 ? `${hours}:${pad(minutes)}:${pad(rest)}` : `${minutes}:${pad(rest)}`
}
