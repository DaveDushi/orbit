import { useEffect } from 'react'
import { Link } from 'react-router'
import { Lock } from 'reicon-react'
import { READ_ONLY_EVENT } from '@/api/client'
import { useWorkspaceBilling } from '@/features/workspaces/billing'

/**
 * Orbit Cloud: one line above the top bar while the workspace has no subscription that grants access. Its data stays
 * readable; the link opens the billing page. A refused write fetches the billing state again, so the line appears as
 * soon as a subscription lapses.
 */
export function ReadOnlyBanner() {
  const billing = useWorkspaceBilling()
  const { refetch } = billing

  useEffect(() => {
    const refused = () => void refetch()
    window.addEventListener(READ_ONLY_EVENT, refused)
    return () => window.removeEventListener(READ_ONLY_EVENT, refused)
  }, [refetch])

  if (billing.data?.access !== 'read_only') return null
  return (
    <div
      data-slot="read-only-banner"
      role="status"
      className="flex min-h-8 shrink-0 items-center justify-center gap-2 border-b border-border bg-muted px-3 py-1 text-xs text-muted-foreground"
    >
      <Lock className="size-3.5 shrink-0" />
      <span className="min-w-0 truncate">This workspace is read-only because it has no active subscription.</span>
      <Link to="/settings/billing" className="shrink-0 font-medium text-foreground underline-offset-4 hover:underline">
        {billing.data.can_manage ? 'Choose a plan' : 'View billing'}
      </Link>
    </div>
  )
}
