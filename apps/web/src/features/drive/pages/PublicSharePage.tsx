import { useState } from 'react'
import { useParams, useSearchParams } from 'react-router'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { ArrowRight2, DocumentDownload } from 'reicon-react'
import type { PublicDriveNode } from '@orbit/api/generated/types.gen'
import { ApiProblem } from '@orbit/api/problem'
import { buttonVariants } from '@/components/ui/button'
import { Spinner } from '@/components/ui/spinner'
import { cn } from 'cn'
import { AuthBrand, AuthCard, AuthForm, AuthInput } from '@/features/auth/components/AuthForm'
import { AuthMessage } from '@/features/auth/components/AuthMessage'
import { formatBytes } from '@/lib/formatBytes'
import { NodeIcon } from '../components/NodeIcon'
import { openShare, shareDownloadHref, shareFolder, unlockShare } from '../shares'

const NOT_AVAILABLE = { title: 'Link not available', detail: 'This link does not exist, was turned off or has expired.' }

/** `/s/:token`: a public Drive link, for anyone who has it (no Orbit account). */
export function PublicSharePage() {
  const token = useParams().token ?? ''
  const queryClient = useQueryClient()
  const share = useQuery({ queryKey: ['public-share', token], queryFn: () => openShare(token), retry: false })

  if (share.isError) return <AuthMessage {...NOT_AVAILABLE} />
  if (!share.data) {
    return (
      <AuthCard>
        <div className="flex justify-center p-6"><Spinner className="text-muted-foreground" /></div>
      </AuthCard>
    )
  }
  if (share.data.locked || !share.data.node) {
    return <PasswordForm token={token} onUnlocked={() => void queryClient.invalidateQueries({ queryKey: ['public-share', token] })} />
  }
  const node = share.data.node
  return node.kind === 'folder' ? <SharedFolder token={token} root={node} /> : <SharedFile token={token} node={node} expiresAt={share.data.expires_at ?? null} />
}

function PasswordForm({ token, onUnlocked }: { token: string; onUnlocked: () => void }) {
  const [password, setPassword] = useState('')
  const [error, setError] = useState<Error | null>(null)
  const [pending, setPending] = useState(false)
  return (
    <AuthForm
      title="This link has a password"
      description="Enter the password you got with the link."
      error={error}
      pending={pending}
      submitLabel="Open"
      onSubmit={async () => {
        setPending(true)
        setError(null)
        try {
          await unlockShare(token, password)
          onUnlocked()
        } catch (failure) {
          setError(failure instanceof ApiProblem ? failure : new Error('failed'))
        } finally {
          setPending(false)
        }
      }}
    >
      <AuthInput label="Password" type="password" value={password} onChange={setPassword} />
    </AuthForm>
  )
}

function SharedFile({ token, node, expiresAt }: { token: string; node: PublicDriveNode; expiresAt: string | null }) {
  return (
    <AuthCard className="w-[min(100%,520px)]">
      <div className="mb-4"><AuthBrand /></div>
      <div className="flex aspect-[4/3] items-center justify-center overflow-hidden rounded-lg border bg-muted/40">
        {node.thumbnail_url ? <img src={node.thumbnail_url} alt="" className="size-full object-contain" data-share-thumbnail /> : <NodeIcon node={node} className="size-16" />}
      </div>
      <h1 className="mt-4 truncate text-lg font-semibold" title={node.name}>{node.name}</h1>
      <p className="text-[13px] text-muted-foreground">
        {node.size != null ? formatBytes(node.size) : ''}
        {expiresAt ? ` · link works until ${new Date(expiresAt).toLocaleString()}` : ''}
      </p>
      <div className="mt-4 flex gap-2">
        <a className={cn(buttonVariants({ size: 'lg' }), 'flex-1')} href={shareDownloadHref(token, null)}>
          <DocumentDownload className="size-4" /> Download
        </a>
        {node.mime?.startsWith('image/') || node.mime?.startsWith('video/') || node.mime === 'application/pdf' ? (
          <a className={buttonVariants({ size: 'lg', variant: 'outline' })} href={shareDownloadHref(token, null, true)} target="_blank" rel="noopener noreferrer">
            Open
          </a>
        ) : null}
      </div>
    </AuthCard>
  )
}

function SharedFolder({ token, root }: { token: string; root: PublicDriveNode }) {
  const [params, setParams] = useSearchParams()
  const folderId = params.get('folder')
  const folder = useQuery({ queryKey: ['public-share', token, 'folder', folderId], queryFn: () => shareFolder(token, folderId), retry: false })
  const open = (id: string | null) => setParams(id && id !== root.id ? { folder: id } : {})

  let body
  if (folder.isError) {
    body = <p className="p-6 text-[13px] text-muted-foreground">This folder is not part of the link.</p>
  } else if (!folder.data) {
    body = <div className="flex justify-center p-6"><Spinner className="text-muted-foreground" /></div>
  } else if (folder.data.items.length === 0) {
    body = <p className="p-6 text-[13px] text-muted-foreground">This folder is empty.</p>
  } else {
    body = (
      <ul aria-label="Files">
        {folder.data.items.map((item) => (
          <li key={item.id} className="flex h-11 items-center gap-3 border-b px-4 text-[13px] last:border-b-0">
            {item.thumbnail_url ? <img src={item.thumbnail_url} alt="" className="size-6 rounded-sm object-cover" /> : <NodeIcon node={item} className="size-5" />}
            {item.kind === 'folder' ? (
              <button type="button" className="min-w-0 flex-1 truncate text-left font-medium hover:underline" onClick={() => open(item.id)}>
                {item.name}
              </button>
            ) : (
              <span className="min-w-0 flex-1 truncate">{item.name}</span>
            )}
            <span className="w-20 text-right text-xs text-muted-foreground tabular-nums">{item.size != null ? formatBytes(item.size) : ''}</span>
            {item.kind === 'file' ? (
              <a className={buttonVariants({ size: 'sm', variant: 'outline' })} href={shareDownloadHref(token, item.id)} aria-label={`Download ${item.name}`}>
                <DocumentDownload className="size-3.5" /> Download
              </a>
            ) : (
              <span className="w-[92px]" />
            )}
          </li>
        ))}
      </ul>
    )
  }

  return (
    <AuthCard className="w-[min(100%,820px)] p-0">
      <header className="flex flex-col gap-3 px-5 pt-5 pb-4">
        <AuthBrand />
        <nav aria-label="Folders" className="flex min-w-0 flex-wrap items-center gap-1 text-[13px]">
          {(folder.data?.path ?? [{ id: root.id, name: root.name }]).map((crumb, index, path) => (
            <span key={crumb.id} className="flex min-w-0 items-center gap-1">
              {index > 0 ? <ArrowRight2 className="size-3 shrink-0 text-muted-foreground" /> : null}
              <button
                type="button"
                className={cn('truncate rounded px-1 py-0.5 hover:bg-black/4 dark:hover:bg-white/5', index === path.length - 1 ? 'text-base font-semibold' : 'text-muted-foreground')}
                onClick={() => open(crumb.id)}
              >
                {crumb.name}
              </button>
            </span>
          ))}
        </nav>
      </header>
      <div className="border-t">{body}</div>
    </AuthCard>
  )
}
