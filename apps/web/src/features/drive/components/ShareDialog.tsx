import { useState } from 'react'
import { toast } from 'sonner'
import { Copy, Link, Lock } from 'reicon-react'
import type { DriveNode, DriveShare } from '@orbit/api/generated/types.gen'
import { ApiProblem } from '@orbit/api/problem'
import { Modal } from '@/components/common/Modal'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { PasswordInput } from '@/components/ui/password-input'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { fullDate, relativeTime } from '@/lib/format'
import { useCreateDriveShare, useDriveShares, useRevokeDriveShare } from '../shares'

const EXPIRY: { value: string; label: string; days: number | null }[] = [
  { value: 'never', label: 'Never', days: null },
  { value: '1', label: 'In 1 day', days: 1 },
  { value: '7', label: 'In 7 days', days: 7 },
  { value: '30', label: 'In 30 days', days: 30 },
]

function shareState(share: DriveShare): { label: string; active: boolean } {
  if (share.revoked_at) return { label: 'Off', active: false }
  if (share.expires_at && new Date(share.expires_at).getTime() <= Date.now()) return { label: 'Expired', active: false }
  return { label: 'Active', active: true }
}

/** Public links of a file or folder: make one (the address shows once), see them, turn them off. */
export function ShareDialog({ workspaceId, node, onClose }: { workspaceId: string; node: DriveNode; onClose: () => void }) {
  const shares = useDriveShares(workspaceId, node.id)
  const create = useCreateDriveShare(workspaceId, node.id)
  const revoke = useRevokeDriveShare(workspaceId, node.id)
  const [expiry, setExpiry] = useState('never')
  const [password, setPassword] = useState('')
  const [created, setCreated] = useState<string | null>(null)

  const submit = () => {
    const days = EXPIRY.find((option) => option.value === expiry)?.days ?? null
    create.mutate(
      { expiresAt: days === null ? null : new Date(Date.now() + days * 86_400_000).toISOString(), password: password || null },
      {
        onSuccess: (result) => {
          setCreated(result.url)
          setPassword('')
          void navigator.clipboard?.writeText(result.url).then(() => toast.success('Link copied'), () => undefined)
        },
      },
    )
  }

  return (
    <Modal title={`Share “${node.name}”`} description="Anyone with the link can open it, without an Orbit account." onClose={onClose} className="sm:max-w-lg">
      <div className="flex flex-col gap-4">
        {created ? (
          <div className="flex flex-col gap-2 rounded-md border border-primary/40 bg-primary/5 p-3">
            <span className="text-[13px] font-medium">New link: copy it now, Orbit shows it only once.</span>
            <div className="flex gap-2">
              <Input readOnly value={created} aria-label="New link" onFocus={(event) => event.currentTarget.select()} />
              <Button
                type="button"
                variant="outline"
                onClick={() => void navigator.clipboard?.writeText(created).then(() => toast.success('Link copied'), () => toast.error('Could not copy the link.'))}
              >
                <Copy className="size-4" /> Copy
              </Button>
            </div>
          </div>
        ) : null}
        <form
          className="grid grid-cols-[1fr_1fr] gap-3 max-sm:grid-cols-1"
          onSubmit={(event) => {
            event.preventDefault()
            submit()
          }}
        >
          <div className="grid gap-1.5">
            <Label htmlFor="drive-share-expiry" className="text-muted-foreground">Expires</Label>
            <Select items={EXPIRY} value={expiry} onValueChange={(value) => setExpiry(String(value))}>
              <SelectTrigger id="drive-share-expiry" aria-label="Expires" className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {EXPIRY.map((option) => <SelectItem key={option.value} value={option.value}>{option.label}</SelectItem>)}
              </SelectContent>
            </Select>
          </div>
          <div className="grid gap-1.5">
            <Label htmlFor="drive-share-password" className="text-muted-foreground">Password (optional, 12+ characters)</Label>
            <PasswordInput id="drive-share-password" value={password} autoComplete="new-password" onChange={(event) => setPassword(event.target.value)} />
          </div>
          {create.error ? (
            <p className="col-span-full text-[13px] text-destructive" role="alert">
              {create.error instanceof ApiProblem ? create.error.detail : 'Could not make the link.'}
            </p>
          ) : null}
          <div className="col-span-full flex justify-end">
            <Button type="submit" disabled={create.isPending}>
              <Link className="size-4" /> Create link
            </Button>
          </div>
        </form>
        <div className="flex flex-col gap-1">
          <span className="text-xs font-medium text-muted-foreground">Links</span>
          {shares.data?.length === 0 ? <p className="text-[13px] text-muted-foreground">No links yet.</p> : null}
          <ul className="flex max-h-56 flex-col overflow-y-auto rounded-md border" aria-label="Links">
            {(shares.data ?? []).map((share) => {
              const state = shareState(share)
              return (
                <li key={share.id} className="flex items-center gap-2 border-b px-3 py-2 text-[13px] last:border-b-0" data-share-state={state.label.toLowerCase()}>
                  <Badge variant={state.active ? 'default' : 'secondary'}>{state.label}</Badge>
                  {share.has_password ? <Lock className="size-3.5 text-muted-foreground" aria-label="Password" /> : null}
                  <span className="min-w-0 flex-1 truncate text-muted-foreground" title={fullDate(share.created_at)}>
                    Made {relativeTime(share.created_at)}
                    {share.expires_at ? ` · expires ${fullDate(share.expires_at)}` : ''}
                    {` · ${share.download_count} ${share.download_count === 1 ? 'download' : 'downloads'}`}
                  </span>
                  {state.active && share.can_revoke ? (
                    <Button
                      type="button"
                      variant="ghost"
                      size="xs"
                      className="text-destructive hover:text-destructive"
                      disabled={revoke.isPending}
                      onClick={() => revoke.mutate(share.id, { onSuccess: () => toast.success('Link turned off') })}
                    >
                      Turn off
                    </Button>
                  ) : null}
                </li>
              )
            })}
          </ul>
        </div>
      </div>
    </Modal>
  )
}
