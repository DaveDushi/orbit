// "Insert from Drive": search the workspace's Drive or walk its folders, and pick a file. The page then shows the file
// by reference (a `driveFile` block), never a copy. Built like PageLinkDialog on the shadcn Command (cmdk).
import { useState } from 'react'
import { ArrowLeft2 } from 'reicon-react'
import type { DriveNode } from '@orbit/api/generated/types.gen'
import { Command, CommandEmpty, CommandInput, CommandItem, CommandList } from '@/components/ui/command'
import { Modal } from '@/components/common/Modal'
import { Spinner } from '@/components/ui/spinner'
import { formatBytes } from '@/lib/formatBytes'
import { useDriveChildren, useDriveNode, useDriveSearch } from '@/features/drive/api'
import { NodeIcon } from '@/features/drive/components/NodeIcon'

export function DriveFilePicker({ workspaceId, onPick, onClose }: { workspaceId: string; onPick: (file: DriveNode) => void; onClose: () => void }) {
  const [query, setQuery] = useState('')
  const [folderId, setFolderId] = useState<string | null>(null)
  const searching = query.trim() !== ''
  const children = useDriveChildren(workspaceId, folderId, 'name', 'asc')
  const search = useDriveSearch(workspaceId, { q: query.trim(), kind: 'file' }, searching)
  const folder = useDriveNode(workspaceId, folderId)
  const active = searching ? search : children
  const items: { node: DriveNode; path?: string }[] = searching
    ? (search.data?.pages.flatMap((page) => page.items) ?? []).map((entry) => ({ node: entry.node, path: entry.folder_path }))
    : (children.data?.pages.flatMap((page) => page.items) ?? []).map((node) => ({ node }))
  const crumbs = folderId ? [...(folder.data?.path ?? []), ...(folder.data ? [{ id: folder.data.node.id, name: folder.data.node.name }] : [])] : []

  return (
    <Modal title="Insert from Drive" onClose={onClose}>
      <Command shouldFilter={false} label="Insert from Drive" className="gap-2 bg-transparent p-0">
        <CommandInput autoFocus placeholder="Search Drive files…" value={query} onValueChange={setQuery} />
        {!searching ? (
          <div className="flex min-h-7 min-w-0 items-center gap-1 px-1 text-xs text-muted-foreground" data-slot="drive-picker-path">
            {folderId ? (
              <button
                type="button"
                aria-label="Back"
                className="flex size-6 shrink-0 items-center justify-center rounded-md hover:bg-foreground/[0.06]"
                onClick={() => setFolderId(crumbs.at(-2)?.id ?? null)}
              >
                <ArrowLeft2 className="size-3.5" />
              </button>
            ) : null}
            <button type="button" className="shrink-0 rounded px-1 hover:text-foreground" onClick={() => setFolderId(null)}>
              Drive
            </button>
            {crumbs.map((crumb) => (
              <span key={crumb.id} className="flex min-w-0 items-center gap-1">
                <span aria-hidden>/</span>
                <button type="button" className="truncate rounded px-1 hover:text-foreground" onClick={() => setFolderId(crumb.id)}>
                  {crumb.name}
                </button>
              </span>
            ))}
          </div>
        ) : null}
        <CommandList className="max-h-[360px]">
          {active.isPending ? (
            <div className="flex justify-center py-7">
              <Spinner className="text-muted-foreground" />
            </div>
          ) : (
            <CommandEmpty className="px-3 py-7 text-[13px] text-muted-foreground/70">
              {active.isError ? 'Drive could not load.' : searching ? 'No files found' : 'This folder is empty'}
            </CommandEmpty>
          )}
          {items.map(({ node, path }) => (
            <CommandItem
              key={node.id}
              value={node.id}
              className="gap-[9px] rounded-[7px] px-2.5 py-2 text-foreground"
              onSelect={() => {
                if (node.kind === 'folder') {
                  setFolderId(node.id)
                } else {
                  onPick(node)
                }
              }}
            >
              <NodeIcon node={node} className="size-4" />
              <span className="flex min-w-0 flex-1 flex-col">
                <span className="truncate">{node.name}</span>
                {path !== undefined ? <span className="truncate text-[11px] text-muted-foreground/70">{path || 'Drive'}</span> : null}
              </span>
              {node.kind === 'file' && node.size != null ? <span className="shrink-0 text-[11px] text-muted-foreground/70 tabular-nums">{formatBytes(node.size)}</span> : null}
            </CommandItem>
          ))}
        </CommandList>
      </Command>
    </Modal>
  )
}
