import { describe, expect, test } from 'bun:test'
import { type EgressRequest, egressCardView, sandboxInput } from './agentSandbox'

const request = (change: Partial<EgressRequest>): EgressRequest => ({
  id: 'r1',
  message_id: 'm1',
  host: 'api.example.com',
  port: 443,
  status: 'pending',
  risky: false,
  expires_at: 2_000,
  driver_id: 'u1',
  driver_name: 'Dana',
  decided_by_name: null,
  decided_at: null,
  rule: null,
  can_allow_once: false,
  can_allow_always: false,
  can_deny: false,
  ...change,
})

const labels = (change: Partial<EgressRequest>, now = 1_000) => egressCardView(request(change), now).buttons.map((button) => button.label)

describe('egress card', () => {
  test('each viewer gets only the buttons the server allows', () => {
    // the driver who is no admin
    expect(labels({ can_allow_once: true, can_deny: true })).toEqual(['Allow once', 'Deny'])
    expect(egressCardView(request({ can_allow_once: true, can_deny: true }), 1_000).note).toBe('Only owners and admins can allow always.')
    expect(egressCardView(request({ can_allow_once: true, can_deny: true }), 1_000).text).toBe('Waiting for your answer.')
    // an admin who is not the driver
    expect(labels({ can_allow_always: true, can_deny: true })).toEqual(['Allow always', 'Deny'])
    // the driver who is an admin
    expect(labels({ can_allow_once: true, can_allow_always: true, can_deny: true })).toEqual(['Allow once', 'Allow always', 'Deny'])
    // another member: no buttons, only who it waits for
    expect(labels({})).toEqual([])
    expect(egressCardView(request({}), 1_000).text).toBe('Waiting for Dana or an admin.')
  })

  test('a risky host says why it has no Allow always', () => {
    expect(egressCardView(request({ risky: true, can_allow_once: true, can_deny: true }), 1_000).note).toBe('This host always asks: it cannot be allowed always.')
  })

  test('an expired card has no buttons, also when the list is older than the expiry', () => {
    const view = egressCardView(request({ can_allow_once: true, can_deny: true }), 2_000)
    expect(view.status).toBe('expired')
    expect(view.buttons).toEqual([])
    expect(view.note).toBeNull()
  })

  test('an answered card names the answer and who gave it', () => {
    expect(egressCardView(request({ status: 'allowed', decided_by_name: 'Ada' }), 5_000).text).toBe('Allowed once by Ada, for 30 minutes in this thread.')
    expect(egressCardView(request({ status: 'allowed', decided_by_name: 'Ada', rule: 'allow' }), 5_000).text).toBe('Allowed always by Ada.')
    expect(egressCardView(request({ status: 'denied', decided_by_name: 'Dana' }), 5_000).text).toBe('Denied by Dana in this thread.')
    expect(egressCardView(request({ status: 'denied', rule: 'deny', can_deny: true }), 5_000).buttons).toEqual([])
  })
})

describe('sandbox limits', () => {
  const draft = { enabled: true, idle: '30', cpu: '600', trafficGb: '2', tokensM: '20' }
  test('the draft goes to the route in bytes and tokens', () => {
    expect(sandboxInput(draft)).toEqual({ enabled: true, idle_minutes: 30, cpu_minutes_month: 600, egress_bytes_month: 2 * 1024 ** 3, model_tokens_month: 20_000_000 })
    expect(sandboxInput({ ...draft, trafficGb: '0.5' })?.egress_bytes_month).toBe(512 * 1024 ** 2)
  })
  test('a wrong number is refused before the route', () => {
    for (const change of [{ idle: '4' }, { idle: '121' }, { idle: '7.5' }, { idle: '' }, { cpu: '-1' }, { cpu: '1.5' }, { trafficGb: 'x' }, { tokensM: '-2' }]) expect(sandboxInput({ ...draft, ...change })).toBeNull()
  })
})
