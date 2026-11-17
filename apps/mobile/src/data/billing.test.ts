import { expect, test } from 'bun:test'
import type { BillingView } from '@orbit/api/generated/types.gen'

import { billingBanner, billingRows } from './billing'

const NOW = Date.parse('2026-10-27T12:00:00Z')
const view = (fields: Partial<BillingView>): BillingView => ({
  access: 'full',
  can_manage: true,
  cancel_at_period_end: false,
  exempt: false,
  has_customer: false,
  plans: [],
  trial_days: 14,
  ...fields,
})
const trial = (ends_at: string): BillingView['trial'] => ({ ends_at, members: { used: 1, limit: 3 }, pages: { used: 0, limit: 50 }, tasks: { used: 0, limit: 100 } })

test('a trial shows its days left, rounded up and at least one', () => {
  expect(billingBanner(view({ access: 'trial', trial: trial('2026-11-03T12:00:00Z') }), NOW)).toEqual({ kind: 'trial', text: 'Trial: 7 days left', action: 'Choose a plan' })
  expect(billingBanner(view({ access: 'trial', trial: trial('2026-10-27T13:00:00Z') }), NOW)?.text).toBe('Trial: 1 day left')
  expect(billingBanner(view({ access: 'trial', trial: trial('2026-10-27T11:00:00Z') }), NOW)?.text).toBe('Trial: 1 day left')
})

test('read-only shows the lock line; a member who cannot manage gets View billing', () => {
  expect(billingBanner(view({ access: 'read_only', can_manage: false }), NOW)).toEqual({
    kind: 'read_only',
    text: 'This workspace is read-only because it has no active subscription.',
    action: 'View billing',
  })
})

test('full access, a trial without data and no billing show nothing', () => {
  expect(billingBanner(view({}), NOW)).toBeNull()
  expect(billingBanner(view({ access: 'trial', trial: null }), NOW)).toBeNull()
  expect(billingBanner(undefined, NOW)).toBeNull()
})

test('billingRows: plan, status, access, then the trial or the renewal', () => {
  const day = (iso: string) => iso.slice(0, 10)
  expect(billingRows(view({ access: 'trial', trial: trial('2026-11-03T12:00:00Z') }), day)).toEqual([
    { label: 'Plan', value: 'No plan' },
    { label: 'Status', value: 'Not subscribed' },
    { label: 'Access', value: 'Trial' },
    { label: 'Trial ends', value: '2026-11-03' },
    { label: 'Trial usage', value: '0 of 100 tasks, 0 of 50 pages, 1 of 3 members' },
  ])
  const plans = [{ key: 'team_year', name: 'Team', interval: 'year' as const, amount: 10000, currency: 'usd' }] as BillingView['plans']
  expect(billingRows(view({ plan: 'team_year', plans, status: 'active', current_period_end: '2027-01-01T00:00:00Z', cancel_at_period_end: true }), day)).toEqual([
    { label: 'Plan', value: 'Team · Annual' },
    { label: 'Status', value: 'Active' },
    { label: 'Access', value: 'Full access' },
    { label: 'Ends', value: 'Ends on 2027-01-01' },
  ])
  expect(billingRows(view({ exempt: true }), day)[0]).toEqual({ label: 'Plan', value: 'Included' })
})
