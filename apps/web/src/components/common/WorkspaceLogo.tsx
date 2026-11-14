import { Avatar, AvatarFallback, AvatarImage } from '@/components/ui/avatar'
import { cn } from 'cn'

/** The workspace's logo, or its first letter on a muted tile while it has none (or the image does not load). Decorative: the
    workspace name always stands beside it. */
export function WorkspaceLogo({ workspace, size = 20, className }: { workspace: { name: string; logo_url?: string | null }; size?: number; className?: string }) {
  const large = size >= 40
  return (
    <Avatar data-slot="workspace-logo" aria-hidden className={cn('after:border-transparent', large ? 'rounded-xl after:rounded-xl' : 'rounded-md after:rounded-md', className)} style={{ width: size, height: size }}>
      {workspace.logo_url ? <AvatarImage src={workspace.logo_url} alt="" className={large ? 'rounded-xl' : 'rounded-md'} /> : null}
      <AvatarFallback className={cn('font-semibold', large ? 'rounded-xl' : 'rounded-md')} style={{ fontSize: Math.max(10, Math.round(size * 0.55)) }}>
        {workspace.name.charAt(0).toUpperCase()}
      </AvatarFallback>
    </Avatar>
  )
}
