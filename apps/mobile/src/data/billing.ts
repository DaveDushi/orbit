import { getBilling } from '@orbit/api/generated/sdk.gen'
import type { BillingView } from '@orbit/api/generated/types.gen'

import type { OrbitClient } from './orbit'

const DAY_MILLIS = 24 * 60 * 60 * 1000

export const billingKey = (workspaceId: string) => ['billing', workspaceId] as const

export async function workspaceBilling(client: OrbitClient, workspaceId: string): Promise<BillingView> {
  return (await getBilling({ client, path: { workspace_id: workspaceId }, throwOnError: true })).data
}

export type BillingBanner =
  | { kind: 'trial'; text: string; action: string }
  | { kind: 'read_only'; text: string; action: string }

/** The web's `ReadOnlyBanner`: the trial's days left (at least 1), or the read-only line; nothing with full access. */
export function billingBanner(view: BillingView | undefined, now: number): BillingBanner | null {
  if (!view) return null
  const action = view.can_manage ? 'Choose a plan' : 'View billing'
  if (view.access === 'trial' && view.trial) {
    const days = Math.max(1, Math.ceil((Date.parse(view.trial.ends_at) - now) / DAY_MILLIS))
    return { kind: 'trial', text: `Trial: ${days} ${days === 1 ? 'day' : 'days'} left`, action }
  }
  if (view.access === 'read_only') return { kind: 'read_only', text: 'This workspace is read-only because it has no active subscription.', action }
  return null
}

const STATUS_LABELS: Record<NonNullable<BillingView['status']>, string> = {
  active: 'Active',
  trialing: 'Trial',
  past_due: 'Payment failed, retrying',
  incomplete: 'Waiting for payment',
  incomplete_expired: 'Payment expired',
  canceled: 'Cancelled',
  unpaid: 'Unpaid',
  paused: 'Paused',
}

const ACCESS_LABELS: Record<BillingView['access'], string> = { full: 'Full access', trial: 'Trial', read_only: 'Read-only' }

const INTERVAL_LABELS = { month: 'Monthly', year: 'Annual' } as const

/** The web's Billing card as label and value rows: plan, status, access, the trial and the renewal. */
export function billingRows(view: BillingView, date: (iso: string) => string): { label: string; value: string }[] {
  const plan = view.plans.find((candidate) => candidate.key === view.plan)
  const planText = view.plan ? (plan ? (plan.interval ? `${plan.name} · ${INTERVAL_LABELS[plan.interval]}` : plan.name) : view.plan) : view.status ? 'Other plan' : view.exempt ? 'Included' : 'No plan'
  const rows = [
    { label: 'Plan', value: planText },
    { label: 'Status', value: view.status ? STATUS_LABELS[view.status] : view.exempt ? 'No subscription needed' : 'Not subscribed' },
    { label: 'Access', value: ACCESS_LABELS[view.access] },
  ]
  if (view.trial) {
    const trial = view.trial
    rows.push({ label: 'Trial ends', value: date(trial.ends_at) })
    rows.push({ label: 'Trial usage', value: (['tasks', 'pages', 'members'] as const).map((noun) => `${trial[noun].used} of ${trial[noun].limit} ${noun}`).join(', ') })
  }
  if (view.current_period_end && view.access === 'full') rows.push({ label: view.cancel_at_period_end ? 'Ends' : 'Renews', value: `${view.cancel_at_period_end ? 'Ends' : 'Renews'} on ${date(view.current_period_end)}` })
  return rows
}

export function billingDescription(view: Pick<BillingView, 'exempt' | 'trial_days'>): string {
  if (view.exempt) return 'This workspace is included with Orbit and needs no plan.'
  if (view.trial_days > 0) return `Each workspace has its own plan. A new workspace starts with a ${view.trial_days}-day trial. Without an active plan after it, the workspace is read-only: everything stays readable and exportable.`
  return 'Each workspace has its own plan. Without an active plan, the workspace is read-only: everything stays readable and exportable.'
}
