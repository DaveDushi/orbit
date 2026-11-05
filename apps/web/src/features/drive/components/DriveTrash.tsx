import { useNavigate } from 'react-router'
import { toast } from 'sonner'
import { ArrowLeft, Refresh, Trash } from 'reicon-react'
import type { DriveNode } from '@orbit/api/generated/types.gen'
import { ApiProblem } from '@orbit/api/problem'
import { confirmAction } from '@/components/common/confirmAction'
import { EmptyState } from '@/components/common/EmptyState'
import { Pane, PaneTitle } from '@/components/common/Pane'
import { Button } from '@/components/ui/button'
import { Spinner } from '@/components/ui/spinner'
import { relativeTime } from '@/lib/format'
import { formatBytes } from '@/lib/formatBytes'
import { useCan } from '@/features/workspaces/permissions'
import { useDriveTrash, useEmptyTrash, usePurgeNode, useRestoreNode } from '../api'
import { NodeIcon } from './NodeIcon'

/** Items trashed directly (what was in a trashed folder comes back with it), newest first. Deleted forever after 30 days. */
export function DriveTrash({ workspaceId }: { workspaceId: string }) {
  const navigate = useNavigate()
  const trash = useDriveTrash(workspaceId)
  const restore = useRestoreNode(workspaceId)
  const purge = usePurgeNode(workspaceId)
  const emptyTrash = useEmptyTrash(workspaceId)
  const canPurge = useCan('drive.purge')
  const items = trash.data?.pages.flatMap((page) => page.items) ?? []
  const busy = restore.isPending || purge.isPending || emptyTrash.isPending

  const requestPurge = async (node: DriveNode) => {
    const confirmed = await confirmAction({
      title: `Delete “${node.name}” forever?`,
      description: node.kind === 'folder' ? 'The folder and everything in it are deleted permanently. This can’t be undone.' : 'The file is deleted permanently. This can’t be undone.',
      confirmLabel: 'Delete forever',
      danger: true,
    })
    if (!confirmed) return
    purge.mutate(node.id, {
      onSuccess: () => toast.success(`Deleted “${node.name}” forever`),
      onError: () => toast.error('Could not delete it.'),
    })
  }

  const requestEmpty = async () => {
    const confirmed = await confirmAction({
      title: 'Empty the Drive trash?',
      description: 'Everything in the trash is deleted permanently. This can’t be undone.',
      confirmLabel: 'Empty trash',
      danger: true,
    })
    if (!confirmed) return
    emptyTrash.mutate(undefined, {
      onSuccess: ({ deleted }) => toast.success(`Deleted ${deleted} ${deleted === 1 ? 'item' : 'items'} forever`),
      onError: () => toast.error('Could not empty the trash.'),
    })
  }

  let body
  if (trash.isError) {
    body = (
      <div className="p-6">
        <EmptyState icon={Trash} title="Trash unavailable" description="The server could not load the trash." action={<Button type="button" variant="outline" onClick={() => void trash.refetch()}>Retry</Button>} />
      </div>
    )
  } else if (trash.data === undefined) {
    body = <div className="flex flex-1 items-center justify-center p-10"><Spinner className="text-muted-foreground" /></div>
  } else if (items.length === 0) {
    body = (
      <div className="p-6">
        <EmptyState icon={Trash} title="Trash is empty" description="Files and folders you move to the trash stay here for 30 days." />
      </div>
    )
  } else {
    body = (
      <div>
        {items.map((node) => (
          <div key={node.id} className="flex min-h-10 min-w-0 items-center gap-2.5 border-b px-3 py-1.5">
            <NodeIcon node={node} className="size-[18px]" />
            <span className="min-w-0 flex-1 truncate text-[13px]">{node.name}</span>
            <span className="w-24 text-right text-xs text-muted-foreground tabular-nums max-[699px]:hidden">{node.size == null ? '—' : formatBytes(node.size)}</span>
            <span className="w-36 shrink-0 text-xs text-muted-foreground max-[899px]:hidden">Deleted {node.trashed_at ? relativeTime(node.trashed_at) : ''}</span>
            <Button
              type="button"
              variant="outline"
              size="sm"
              disabled={busy}
              onClick={() =>
                restore.mutate(node.id, {
                  onSuccess: (restored) =>
                    toast.success(`Restored “${restored.name}”`, {
                      action: { label: 'Open folder', onClick: () => navigate(restored.parent_id ? `/drive/${restored.parent_id}` : '/drive') },
                    }),
                  onError: (error) => toast.error(error instanceof ApiProblem ? error.detail : 'Could not restore it.'),
                })
              }
            >
              <Refresh className="size-3.5" /> Restore
            </Button>
            {canPurge ? (
              <Button type="button" variant="ghost" size="sm" className="text-destructive hover:text-destructive" disabled={busy} aria-label={`Delete “${node.name}” forever`} onClick={() => void requestPurge(node)}>
                <Trash className="size-3.5" />
                <span className="max-[899px]:hidden">Delete forever</span>
              </Button>
            ) : null}
          </div>
        ))}
        {trash.hasNextPage ? (
          <div className="flex justify-center p-3">
            <Button type="button" variant="outline" size="sm" disabled={trash.isFetchingNextPage} onClick={() => void trash.fetchNextPage()}>Load more</Button>
          </div>
        ) : null}
      </div>
    )
  }

  return (
    <Pane>
      <div className="flex h-12 shrink-0 items-center gap-2 border-b px-3">
        <Button type="button" variant="ghost" size="icon-sm" aria-label="Back to Drive" onClick={() => navigate('/drive')}>
          <ArrowLeft className="size-4" />
        </Button>
        <PaneTitle render={<h1 />} className="min-w-0 flex-1">Drive trash</PaneTitle>
        {canPurge && items.length > 0 ? (
          <Button type="button" variant="ghost" size="sm" className="text-destructive hover:text-destructive" disabled={busy} onClick={() => void requestEmpty()}>
            Empty trash
          </Button>
        ) : null}
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto">{body}</div>
    </Pane>
  )
}
