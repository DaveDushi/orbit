import { useState, type DragEvent, type ReactNode } from 'react'
import { ArrowDown2, ArrowRight2, Folder, FolderOpen, Home2 } from 'reicon-react'
import { cn } from 'cn'
import { useDriveFolders } from '../api'

/** The MIME type of Drive items being dragged to another folder. */
export const DRIVE_DRAG_TYPE = 'application/x-orbit-drive-nodes'

interface FolderTreeProps {
  workspaceId: string
  selected: string | null
  onSelect: (folderId: string | null) => void
  /** Drive items dropped on a folder (`null`: the top level). */
  onDropNodes?: (folderId: string | null, nodeIds: string[]) => void
  /** Folders that cannot be picked (a folder cannot move into itself). */
  disabled?: ReadonlySet<string>
  rootLabel?: string
}

/** The folders of the drive; a folder's subfolders load when it opens. */
export function FolderTree({ workspaceId, selected, onSelect, onDropNodes, disabled, rootLabel = 'My Drive' }: FolderTreeProps) {
  return (
    <div role="tree" aria-label="Folders" className="flex flex-col py-1 text-[13px]">
      <TreeRow
        label={rootLabel}
        icon={<Home2 className="size-4 shrink-0 text-muted-foreground" />}
        depth={0}
        active={selected === null}
        onSelect={() => onSelect(null)}
        onDrop={onDropNodes ? (ids) => onDropNodes(null, ids) : undefined}
      />
      <Folders workspaceId={workspaceId} parentId={null} depth={1} selected={selected} onSelect={onSelect} onDropNodes={onDropNodes} disabled={disabled} />
    </div>
  )
}

function Folders({ workspaceId, parentId, depth, selected, onSelect, onDropNodes, disabled }: Omit<FolderTreeProps, 'rootLabel'> & { parentId: string | null; depth: number }) {
  const folders = useDriveFolders(workspaceId, parentId)
  const [open, setOpen] = useState<ReadonlySet<string>>(new Set())
  if (folders.data === undefined) return null
  return (
    <>
      {folders.data.map((folder) => {
        const expanded = open.has(folder.id)
        const blocked = disabled?.has(folder.id) ?? false
        return (
          <div key={folder.id} role="none">
            <TreeRow
              label={folder.name}
              depth={depth}
              active={selected === folder.id}
              blocked={blocked}
              expanded={expanded}
              onToggle={() => setOpen((current) => {
                const next = new Set(current)
                if (next.has(folder.id)) next.delete(folder.id)
                else next.add(folder.id)
                return next
              })}
              icon={expanded ? <FolderOpen className="size-4 shrink-0 text-sky-500" /> : <Folder className="size-4 shrink-0 text-sky-500" />}
              onSelect={() => onSelect(folder.id)}
              onDrop={onDropNodes && !blocked ? (ids) => onDropNodes(folder.id, ids) : undefined}
            />
            {expanded && !blocked ? (
              <Folders workspaceId={workspaceId} parentId={folder.id} depth={depth + 1} selected={selected} onSelect={onSelect} onDropNodes={onDropNodes} disabled={disabled} />
            ) : null}
          </div>
        )
      })}
    </>
  )
}

interface TreeRowProps {
  label: string
  icon: ReactNode
  depth: number
  active: boolean
  blocked?: boolean
  expanded?: boolean
  onToggle?: () => void
  onSelect: () => void
  onDrop?: (nodeIds: string[]) => void
}

function TreeRow({ label, icon, depth, active, blocked, expanded, onToggle, onSelect, onDrop }: TreeRowProps) {
  const [over, setOver] = useState(false)
  const accepts = (event: DragEvent) => onDrop !== undefined && event.dataTransfer.types.includes(DRIVE_DRAG_TYPE)
  return (
    <div
      role="treeitem"
      aria-selected={active}
      aria-expanded={onToggle ? expanded : undefined}
      aria-disabled={blocked || undefined}
      className={cn(
        'flex h-8 min-w-0 cursor-pointer items-center gap-1 rounded-md pr-2 text-sidebar-nav hover:bg-black/4 dark:hover:bg-white/5',
        active && 'bg-black/5 font-medium text-foreground dark:bg-white/6',
        blocked && 'cursor-not-allowed opacity-50',
        over && 'ring-2 ring-primary/60',
      )}
      style={{ paddingLeft: 4 + depth * 14 }}
      onClick={() => {
        if (!blocked) onSelect()
      }}
      onDragOver={(event) => {
        if (!accepts(event)) return
        event.preventDefault()
        setOver(true)
      }}
      onDragLeave={() => setOver(false)}
      onDrop={(event) => {
        setOver(false)
        if (!accepts(event)) return
        event.preventDefault()
        onDrop?.(JSON.parse(event.dataTransfer.getData(DRIVE_DRAG_TYPE)) as string[])
      }}
    >
      {onToggle ? (
        <button
          type="button"
          className="inline-flex size-5 shrink-0 items-center justify-center rounded text-muted-foreground hover:bg-black/5 dark:hover:bg-white/10"
          aria-label={expanded ? `Close ${label}` : `Open ${label}`}
          onClick={(event) => {
            event.stopPropagation()
            onToggle()
          }}
        >
          {expanded ? <ArrowDown2 className="size-3" /> : <ArrowRight2 className="size-3" />}
        </button>
      ) : (
        <span className="inline-block size-5 shrink-0" />
      )}
      {icon}
      <span className="min-w-0 truncate">{label}</span>
    </div>
  )
}
