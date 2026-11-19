import { useState } from 'react'
import { Link, useParams } from 'react-router'
import { ArrowLeft } from 'reicon-react'
import { toast } from 'sonner'
import { formatBytes, type EgressLogRow, type EgressPending, type EgressRule } from '@orbit/api/agentSandbox'
import { ApiProblem } from '@orbit/api/problem'
import { SettingsCard } from '@/components/common/SettingsCard'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Field, FieldLabel } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { SettingsRow } from '@/features/settings/components/SettingsParts'
import { useWorkspace } from '@/features/workspaces/workspaceContext'
import { relativeTime } from '@/lib/format'
import { useAgentProfiles } from './api'
import { sandboxOff, useAddEgressRule, useAgentSandbox, useAnswerEgress, useDeleteEgressRule } from './sandboxApi'

const ACTIONS = [
  { value: 'allow', label: 'Allow' },
  { value: 'deny', label: 'Deny' },
]

const ago = (millis: number) => relativeTime(new Date(millis).toISOString())
const problemText = (error: unknown, fallback: string) => (error instanceof ApiProblem ? error.detail : fallback)

/**
 * Settings → Agentic → Agents → an agent → Internet access (owners and admins): the hosts its sandbox may reach (rules),
 * the requests that wait for an answer, and the latest connections. Opened from the agent's row and its card.
 */
export function InternetAccessPage() {
  const { workspace } = useWorkspace()
  const agentId = useParams().agentId ?? ''
  const agent = useAgentProfiles(workspace.id).data?.agents.find((profile) => profile.id === agentId)
  const sandbox = useAgentSandbox(workspace.id, agentId)
  const name = agent?.name ?? 'Agent'
  const back = (
    <Link to="/settings/agentic/agents" className="inline-flex items-center gap-1 self-start text-xs text-muted-foreground hover:text-foreground">
      <ArrowLeft className="size-3.5" aria-hidden="true" />
      Agents
    </Link>
  )
  if (sandbox.isPending) return back
  if (!sandbox.data) {
    const detail = sandboxOff(sandbox.error) ? 'Agent sandboxes are off on this server.' : problemText(sandbox.error, 'The Internet access settings could not be loaded.')
    return (
      <>
        {back}
        <SettingsCard title={`${name}: Internet access`}>
          <p role="alert" className="text-sm text-muted-foreground">{detail}</p>
        </SettingsCard>
      </>
    )
  }
  const data = sandbox.data
  return (
    <>
      {back}
      <RulesCard agentId={agentId} name={name} rules={data.rules} enabled={data.settings.enabled} />
      <PendingCard requests={data.pending} />
      <LogCard log={data.log} />
    </>
  )
}

function RulesCard({ agentId, name, rules, enabled }: { agentId: string; name: string; rules: EgressRule[]; enabled: boolean }) {
  const { workspace } = useWorkspace()
  const add = useAddEgressRule(workspace.id, agentId)
  const remove = useDeleteEgressRule(workspace.id, agentId)
  const [host, setHost] = useState('')
  const [port, setPort] = useState('443')
  const [action, setAction] = useState<'allow' | 'deny'>('allow')

  function submit(event: React.FormEvent) {
    event.preventDefault()
    add.mutate({ host: host.trim(), port: Number(port), action }, { onSuccess: () => { setHost(''); setPort('443') } })
  }

  return (
    <SettingsCard
      title={`${name}: Internet access`}
      description={`Code in ${name}'s sandbox reaches the internet over HTTPS only, through Orbit. An allowed host works in every thread; any other host asks the person in the thread first (Allow once, Allow always, Deny). Deny rules win. Some hosts (paste sites, tunnels, file storage) always ask.${enabled ? '' : ' The sandbox is off: turn it on in the agent’s card.'}`}
      flush
    >
      <form onSubmit={submit} className="flex flex-col gap-3 border-b p-4">
        <div className="grid grid-cols-1 gap-3 min-[700px]:grid-cols-[minmax(0,1fr)_6rem_8rem_auto] min-[700px]:items-end">
          <Field>
            <FieldLabel htmlFor="egress-rule-host">Host</FieldLabel>
            <Input id="egress-rule-host" required placeholder="api.example.com or *.example.com" value={host} onChange={(event) => { add.reset(); setHost(event.target.value) }} />
          </Field>
          <Field>
            <FieldLabel htmlFor="egress-rule-port">Port</FieldLabel>
            <Input id="egress-rule-port" type="number" inputMode="numeric" min={1} max={65535} required value={port} onChange={(event) => setPort(event.target.value)} />
          </Field>
          <Field>
            <FieldLabel htmlFor="egress-rule-action">Rule</FieldLabel>
            <Select items={ACTIONS} value={action} onValueChange={(value) => setAction((value as 'allow' | 'deny' | null) ?? 'allow')}>
              <SelectTrigger id="egress-rule-action"><SelectValue /></SelectTrigger>
              <SelectContent>
                {ACTIONS.map((option) => <SelectItem key={option.value} value={option.value}>{option.label}</SelectItem>)}
              </SelectContent>
            </Select>
          </Field>
          <Button type="submit" disabled={add.isPending || host.trim() === ''}>Add rule</Button>
        </div>
        <p className="text-xs text-muted-foreground">*.example.com matches every subdomain, not example.com itself. No IP addresses, and no wildcard on a public suffix such as *.com or *.github.io.</p>
        {add.isError ? <p role="alert" className="text-sm text-destructive">{problemText(add.error, 'The rule could not be added.')}</p> : null}
      </form>
      {rules.length === 0 ? (
        <p className="px-4 py-3 text-sm text-muted-foreground">No rules yet: every host asks first.</p>
      ) : (
        <ul className="flex flex-col divide-y">
          {rules.map((rule) => (
            <li key={rule.id}><SettingsRow data-slot="egress-rule">
              <div className="flex min-w-0 flex-1 flex-col gap-0.5">
                <div className="flex min-w-0 items-center gap-2">
                  <span className="truncate font-mono text-[13px] text-foreground">{rule.host}{rule.port === 443 ? '' : `:${rule.port}`}</span>
                  <Badge variant={rule.action === 'allow' ? 'secondary' : 'destructive'}>{rule.action === 'allow' ? 'Allow' : 'Deny'}</Badge>
                </div>
                <p className="text-xs text-muted-foreground">
                  Added {ago(rule.created_at)}{rule.created_by_name ? ` by ${rule.created_by_name}` : ''} · {rule.last_used_at ? `last used ${ago(rule.last_used_at)}` : 'not used yet'}
                </p>
              </div>
              <Button
                size="sm"
                variant="ghost"
                className="text-destructive"
                disabled={remove.isPending}
                onClick={() => remove.mutate(rule.id, { onError: (error) => toast.error(problemText(error, 'The rule could not be removed.')) })}
              >
                Remove
              </Button>
            </SettingsRow></li>
          ))}
        </ul>
      )}
    </SettingsCard>
  )
}

/** Requests that wait for an answer in their threads; owners and admins can answer here too. */
function PendingCard({ requests }: { requests: EgressPending[] }) {
  const { workspace } = useWorkspace()
  const answer = useAnswerEgress(workspace.id)
  const buttons = (request: EgressPending) =>
    [
      ['allow_once', 'Allow once', request.can_allow_once],
      ['allow_always', 'Allow always', request.can_allow_always],
      ['deny', 'Deny', request.can_deny],
    ] as const
  return (
    <SettingsCard title="Pending requests" description="Hosts a sandbox asked for in the last 30 minutes. Allow once is for the person in the thread only." flush>
      {requests.length === 0 ? (
        <p className="px-4 py-3 text-sm text-muted-foreground">Nothing waits for an answer.</p>
      ) : (
        <ul className="flex flex-col divide-y">
          {requests.map((request) => (
            <li key={request.id}><SettingsRow data-slot="egress-pending">
              <div className="flex min-w-0 flex-1 flex-col gap-0.5">
                <span className="truncate font-mono text-[13px] text-foreground">{request.host}{request.port === 443 ? '' : `:${request.port}`}</span>
                <p className="text-xs text-muted-foreground">For {request.driver_name} · {ago(request.created_at)}{request.risky ? ' · always asks' : ''}</p>
              </div>
              <div className="flex flex-wrap gap-1">
                {buttons(request)
                  .filter(([, , may]) => may)
                  .map(([value, label]) => (
                    <Button
                      key={value}
                      size="sm"
                      variant="outline"
                      className={value === 'deny' ? 'text-destructive' : undefined}
                      disabled={answer.isPending}
                      onClick={() => answer.mutate({ requestId: request.id, answer: value }, { onError: (error) => toast.error(problemText(error, 'The answer could not be sent.')) })}
                    >
                      {label}
                    </Button>
                  ))}
              </div>
            </SettingsRow></li>
          ))}
        </ul>
      )}
    </SettingsCard>
  )
}

const DECISIONS: Record<EgressLogRow['decision'], { label: string; variant: 'secondary' | 'outline' | 'destructive' }> = {
  allow: { label: 'Allowed', variant: 'secondary' },
  ask: { label: 'Asked', variant: 'outline' },
  deny: { label: 'Denied', variant: 'destructive' },
  blocked: { label: 'Blocked', variant: 'destructive' },
}

/** The latest 100 connections: host and port only (Orbit cannot see URLs or content). Kept 90 days. */
function LogCard({ log }: { log: EgressLogRow[] }) {
  return (
    <SettingsCard title="Connection log" description="The latest 100 connection attempts: host, answer, traffic, for whom and when. Orbit sees no addresses within a site and no content." flush>
      {log.length === 0 ? (
        <p className="px-4 py-3 text-sm text-muted-foreground">No connections yet.</p>
      ) : (
        <div className="overflow-x-auto">
          <table data-slot="egress-log" className="w-full min-w-[640px] text-left text-sm">
            <thead className="text-xs text-muted-foreground">
              <tr className="border-b">
                <th className="px-4 py-2 font-medium">Host</th>
                <th className="px-2 py-2 font-medium">Answer</th>
                <th className="px-2 py-2 text-right font-medium">Sent</th>
                <th className="px-2 py-2 text-right font-medium">Received</th>
                <th className="px-2 py-2 font-medium">For</th>
                <th className="px-4 py-2 text-right font-medium">When</th>
              </tr>
            </thead>
            <tbody className="divide-y">
              {log.map((row) => (
                <tr key={row.id} title={row.reason}>
                  <td className="max-w-64 truncate px-4 py-2 font-mono text-[13px] text-foreground">{row.host}{row.port === 443 ? '' : `:${row.port}`}</td>
                  <td className="px-2 py-2"><Badge variant={DECISIONS[row.decision].variant}>{DECISIONS[row.decision].label}</Badge></td>
                  <td className="px-2 py-2 text-right text-muted-foreground tabular-nums">{row.decision === 'allow' ? formatBytes(row.bytes_up) : '—'}</td>
                  <td className="px-2 py-2 text-right text-muted-foreground tabular-nums">{row.decision === 'allow' ? formatBytes(row.bytes_down) : '—'}</td>
                  <td className="px-2 py-2 text-muted-foreground">{row.driver_name ?? '—'}</td>
                  <td className="px-4 py-2 text-right text-muted-foreground">{ago(row.created_at)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </SettingsCard>
  )
}
