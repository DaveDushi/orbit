import { useEffect, useState } from 'react'
import { Navigate, useSearchParams } from 'react-router'
import { toast } from 'sonner'
import { Button } from '@/components/ui/button'
import { SettingsCard } from '@/components/common/SettingsCard'
import type { BillingView, SubscriptionStatus } from '@orbit/api/generated/types.gen'
import { useBillingCheckout, useBillingEnabled, useBillingPortal, useWorkspaceBilling } from '@/features/workspaces/billing'
import { useAuthOptions } from '@/features/auth/api'
import { fullDate } from '@/lib/format'
import { loadFailed } from '@/lib/connection'

const STATUS_LABELS: Record<SubscriptionStatus, string> = {
  active: 'Active',
  trialing: 'Trial',
  past_due: 'Payment failed, retrying',
  incomplete: 'Waiting for payment',
  incomplete_expired: 'Payment expired',
  canceled: 'Cancelled',
  unpaid: 'Unpaid',
  paused: 'Paused',
}

/** Plans have no display names yet: the key, capitalized. */
function planName(key: string) {
  return key.charAt(0).toUpperCase() + key.slice(1)
}

function renewal(billing: BillingView) {
  if (!billing.current_period_end || billing.access !== 'full') return null
  const date = fullDate(billing.current_period_end)
  return billing.cancel_at_period_end ? `Ends on ${date}` : `Renews on ${date}`
}

/** Orbit Cloud: the workspace's plan. The owner subscribes through Stripe Checkout and manages it in the Billing Portal. */
export function BillingPage() {
  const options = useAuthOptions()
  const enabled = useBillingEnabled()
  const query = useWorkspaceBilling()
  const checkout = useBillingCheckout()
  const portal = useBillingPortal()
  const [params, setParams] = useSearchParams()
  const returned = params.get('checkout')
  // Back from Stripe Checkout: keep watching for the subscription after the query parameter is gone.
  const [confirming] = useState(() => returned === 'success')

  useEffect(() => {
    if (!returned) return
    if (returned === 'success') toast.success('Thanks! Your subscription starts as soon as Stripe confirms the payment.')
    setParams({}, { replace: true })
  }, [returned, setParams])

  // Stripe confirms through the webhook a moment after the redirect: look again until access arrives.
  const waiting = confirming && query.data?.access !== 'full'
  const { refetch } = query
  useEffect(() => {
    if (!waiting) return
    const timer = window.setInterval(() => void refetch(), 3000)
    const stop = window.setTimeout(() => window.clearInterval(timer), 60_000)
    return () => {
      window.clearInterval(timer)
      window.clearTimeout(stop)
    }
  }, [waiting, refetch])

  if (options.data && !enabled) return <Navigate to="/settings" replace />
  if (loadFailed(query)) return <SettingsCard title="Billing">Billing is not available right now.</SettingsCard>
  if (query.data === undefined) return <SettingsCard title="Billing">Loading billing.</SettingsCard>
  const billing = query.data
  const error = checkout.error ?? portal.error
  const busy = checkout.isPending || portal.isPending

  return (
    <>
      <SettingsCard
        title="Billing"
        description="Each workspace has its own plan. Without an active plan the workspace is read-only: everything stays readable and exportable."
        actions={
          billing.can_manage && billing.has_customer ? (
            <Button variant="outline" disabled={busy} onClick={() => portal.mutate()}>
              Manage billing
            </Button>
          ) : null
        }
      >
        <dl className="grid gap-3 text-sm sm:grid-cols-[160px_minmax(0,1fr)]">
          <dt className="text-muted-foreground">Plan</dt>
          <dd>{billing.plan ? planName(billing.plan) : billing.status ? 'Other plan' : 'No plan'}</dd>
          <dt className="text-muted-foreground">Status</dt>
          <dd>{billing.status ? STATUS_LABELS[billing.status] : 'Not subscribed'}</dd>
          <dt className="text-muted-foreground">Access</dt>
          <dd data-access={billing.access}>{billing.access === 'full' ? 'Full access' : 'Read-only'}</dd>
          {renewal(billing) ? (
            <>
              <dt className="text-muted-foreground">{billing.cancel_at_period_end ? 'Ends' : 'Renews'}</dt>
              <dd>{renewal(billing)}</dd>
            </>
          ) : null}
        </dl>
        {error ? (
          <p role="alert" className="mt-3 text-xs text-destructive">
            {error.message}
          </p>
        ) : null}
      </SettingsCard>
      {billing.access === 'read_only' ? (
        <SettingsCard title="Choose a plan" description={billing.can_manage ? 'You pay through Stripe. You can change or cancel the plan at any time.' : undefined}>
          {billing.can_manage ? (
            <div className="flex flex-wrap gap-2">
              {billing.plans.map((plan) => (
                <Button key={plan} disabled={busy || waiting} onClick={() => checkout.mutate(plan)}>
                  {planName(plan)}
                </Button>
              ))}
            </div>
          ) : (
            <p className="text-sm text-muted-foreground">Only the workspace owner can choose a plan.</p>
          )}
          {waiting ? <p className="mt-3 text-xs text-muted-foreground">Waiting for Stripe to confirm the payment.</p> : null}
        </SettingsCard>
      ) : null}
    </>
  )
}
