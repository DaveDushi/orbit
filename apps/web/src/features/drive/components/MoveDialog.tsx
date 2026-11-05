import { useState } from 'react'
import { Modal } from '@/components/common/Modal'
import { Button } from '@/components/ui/button'
import { FolderTree } from './FolderTree'

interface MoveDialogProps {
  workspaceId: string
  count: number
  /** The folders being moved: they and their subfolders cannot be the target. */
  movingFolders: ReadonlySet<string>
  currentFolder: string | null
  onMove: (folderId: string | null) => Promise<unknown>
  onClose: () => void
}

export function MoveDialog({ workspaceId, count, movingFolders, currentFolder, onMove, onClose }: MoveDialogProps) {
  const [target, setTarget] = useState<string | null>(currentFolder)
  const [busy, setBusy] = useState(false)
  return (
    <Modal title={`Move ${count === 1 ? '1 item' : `${count} items`}`} description="Pick the folder to move to." onClose={onClose} className="sm:max-w-md" dismissible={!busy}>
      <div className="max-h-80 overflow-y-auto rounded-md border p-1">
        <FolderTree workspaceId={workspaceId} selected={target} onSelect={setTarget} disabled={movingFolders} />
      </div>
      <div className="flex justify-end gap-2">
        <Button type="button" variant="outline" onClick={onClose} disabled={busy}>Cancel</Button>
        <Button
          type="button"
          disabled={busy || target === currentFolder}
          onClick={() => {
            setBusy(true)
            void onMove(target).finally(onClose)
          }}
        >
          Move here
        </Button>
      </div>
    </Modal>
  )
}
