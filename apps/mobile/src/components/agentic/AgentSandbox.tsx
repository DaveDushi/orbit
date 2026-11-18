import { useState } from 'react'
import { View } from 'react-native'
import { formatBytes, type AgentSandbox, type EgressLogRow, type EgressPending, type SandboxDraft } from '@orbit/api/agentSandbox'

import { Badge, FieldLabel, Note, OptionHint, Segmented, SettingsCard, SettingsRow, Toggle } from '../settings/Shell'
import { relativeAgo } from '../../data/format'
import { problemDetail } from '../../data/agentic'
import { useAgentProfiles } from '../../hooks/agent'
import { sandboxOff, useAddEgressRule, useAgentSandbox, useAnswerEgress, useDeleteEgressRule } from '../../hooks/agentSandbox'
import { Button, Input, Text } from '../../ui'

/** The web's `SandboxFields`: on the agent's sheet when `sandbox_shell` is ticked. The switch, the limits, this month's use. */
export function SandboxFields({ sandbox, draft, onChange, onOpenInternet }: { sandbox: AgentSandbox; draft: SandboxDraft; onChange: (draft: SandboxDraft) => void; onOpenInternet: () => void }) {
  const set = (change: Partial<SandboxDraft>) => onChange({ ...draft, ...change })
  const usage = sandbox.usage
  return (
    <View className="gap-2">
      <FieldLabel>Sandbox</FieldLabel>
      <View className="flex-row items-center justify-between">
        <Text className="text-[14px]">Run code in a sandbox</Text>
        <Toggle label="Run code in a sandbox" value={draft.enabled} onChange={(enabled) => set({ enabled })} />
      </View>
      <OptionHint>
        Each chat thread gets its own isolated sandbox (1 CPU, 1 GB memory, 4 GB of files, 15 minutes per command). It reaches only the hosts allowed on its{' '}
        <Text className="text-[12px] text-foreground underline" onPress={onOpenInternet}>Internet access</Text> page. Off: no new commands and no internet.
      </OptionHint>
      {draft.enabled ? (
        <>
          <Input label="Stop after (minutes without commands)" keyboardType="number-pad" value={draft.idle} onChangeText={(idle) => set({ idle })} description="5 to 120. Its files stay." />
          <Input label="CPU minutes per month" keyboardType="number-pad" value={draft.cpu} onChangeText={(cpu) => set({ cpu })} description={`Used: ${Math.round(usage.cpu_ms / 60_000)} this month.`} />
          <Input label="Internet traffic per month (GB)" keyboardType="decimal-pad" value={draft.trafficGb} onChangeText={(trafficGb) => set({ trafficGb })} description={`Used: ${formatBytes(usage.egress_bytes)} this month.`} />
          <Input label="Model tokens per month (millions)" keyboardType="decimal-pad" value={draft.tokensM} onChangeText={(tokensM) => set({ tokensM })} description="For all runs that use the sandbox." />
          <OptionHint>When the CPU or traffic limit is spent, the sandbox refuses new commands and internet access until next month.</OptionHint>
        </>
      ) : null}
    </View>
  )
}

const ACTIONS = [
  { value: 'allow', label: 'Allow' },
  { value: 'deny', label: 'Deny' },
] as const

const ago = (millis: number) => relativeAgo(new Date(millis).toISOString())
const hostPort = (host: string, port: number) => (port === 443 ? host : `${host}:${port}`)

/**
 * The web's Settings → Agentic → Agents → an agent → Internet access (owners and admins), on the Agents tab: the
 * rules, the requests that wait, the latest connections.
 */
export function InternetAccess({ agentId, onBack }: { agentId: string; onBack: () => void }) {
  const sandbox = useAgentSandbox(agentId)
  const name = useAgentProfiles().data?.agents.find((profile) => profile.id === agentId)?.name ?? 'Agent'
  const back = (
    <View className="flex-row">
      <Button label="Agents" icon="chevronLeft" variant="ghost" size="sm" onPress={onBack} />
    </View>
  )
  if (!sandbox.data) {
    return (
      <>
        {back}
        <SettingsCard title={`${name}: Internet access`}>
          {sandbox.isPending ? (
            <Note loading>Loading…</Note>
          ) : (
            <View className="gap-2 items-start">
              <Note error={!sandboxOff(sandbox.error)}>{sandboxOff(sandbox.error) ? 'Agent sandboxes are off on this server.' : problemDetail(sandbox.error, 'The Internet access settings could not be loaded.')}</Note>
              {sandboxOff(sandbox.error) ? null : <Button label="Retry" variant="ghost" onPress={() => void sandbox.refetch()} />}
            </View>
          )}
        </SettingsCard>
      </>
    )
  }
  const data = sandbox.data
  return (
    <>
      {back}
      <Rules agentId={agentId} name={name} sandbox={data} />
      <Pending requests={data.pending} />
      <Log log={data.log} />
    </>
  )
}

function Rules({ agentId, name, sandbox }: { agentId: string; name: string; sandbox: AgentSandbox }) {
  const add = useAddEgressRule(agentId)
  const remove = useDeleteEgressRule(agentId)
  const [host, setHost] = useState('')
  const [port, setPort] = useState('443')
  const [action, setAction] = useState<'allow' | 'deny'>('allow')
  const submit = () => add.mutate({ host: host.trim(), port: Number(port), action }, { onSuccess: () => { setHost(''); setPort('443') } })
  return (
    <SettingsCard
      title={`${name}: Internet access`}
      description={`Code in ${name}'s sandbox reaches the internet over HTTPS only, through Orbit. An allowed host works in every thread; any other host asks the person in the thread first (Allow once, Allow always, Deny). Deny rules win. Some hosts (paste sites, tunnels, file storage) always ask.${sandbox.settings.enabled ? '' : ' The sandbox is off: turn it on in the agent’s card.'}`}
      flush
    >
      <View className="gap-3 p-4">
        <Input label="Host" autoCapitalize="none" autoCorrect={false} keyboardType="url" placeholder="api.example.com or *.example.com" value={host} onChangeText={(next) => { add.reset(); setHost(next) }} />
        <Input label="Port" keyboardType="number-pad" value={port} onChangeText={setPort} />
        <Segmented<'allow' | 'deny'> label="Rule" value={action} options={ACTIONS} onChange={setAction} />
        <OptionHint>*.example.com matches every subdomain, not example.com itself. No IP addresses, and no wildcard on a public suffix such as *.com or *.github.io.</OptionHint>
        {add.isError ? <Note error>{problemDetail(add.error, 'The rule could not be added.')}</Note> : null}
        <View className="flex-row">
          <Button label="Add rule" size="sm" disabled={host.trim() === '' || add.isPending} onPress={submit} />
        </View>
      </View>
      {sandbox.rules.length === 0 ? (
        <SettingsRow>
          <Note>No rules yet: every host asks first.</Note>
        </SettingsRow>
      ) : (
        sandbox.rules.map((rule) => (
          <SettingsRow key={rule.id}>
            <View className="flex-1 min-w-0 gap-0.5">
              <View className="flex-row items-center gap-1.5">
                <Text numberOfLines={1} className="shrink font-mono text-[13px] text-foreground">{hostPort(rule.host, rule.port)}</Text>
                <Badge label={rule.action === 'allow' ? 'Allow' : 'Deny'} tone={rule.action === 'allow' ? 'secondary' : 'destructive'} />
              </View>
              <Text className="text-[12px] text-muted-foreground">
                Added {ago(rule.created_at)}{rule.created_by_name ? ` by ${rule.created_by_name}` : ''} · {rule.last_used_at ? `last used ${ago(rule.last_used_at)}` : 'not used yet'}
              </Text>
            </View>
            <Button label="Remove" variant="ghost" size="sm" disabled={remove.isPending} onPress={() => remove.mutate(rule.id)} />
          </SettingsRow>
        ))
      )}
      {remove.isError ? <View className="p-4"><Note error>{problemDetail(remove.error, 'The rule could not be removed.')}</Note></View> : null}
    </SettingsCard>
  )
}

function Pending({ requests }: { requests: EgressPending[] }) {
  const answer = useAnswerEgress()
  return (
    <SettingsCard title="Pending requests" description="Hosts a sandbox asked for in the last 30 minutes. Allow once is for the person in the thread only." flush>
      {requests.length === 0 ? (
        <SettingsRow>
          <Note>Nothing waits for an answer.</Note>
        </SettingsRow>
      ) : (
        requests.map((request, index) => (
          <View key={request.id} className={`gap-2 border-border px-4 py-3 ${index === 0 ? '' : 'border-t'}`}>
            <Text numberOfLines={1} className="font-mono text-[13px] text-foreground">{hostPort(request.host, request.port)}</Text>
            <Text className="text-[12px] text-muted-foreground">For {request.driver_name} · {ago(request.created_at)}{request.risky ? ' · always asks' : ''}</Text>
            <View className="flex-row flex-wrap gap-2">
              {request.can_allow_once ? <Button label="Allow once" size="sm" variant="outline" disabled={answer.isPending} onPress={() => answer.mutate({ requestId: request.id, answer: 'allow_once' })} /> : null}
              {request.can_allow_always ? <Button label="Allow always" size="sm" variant="outline" disabled={answer.isPending} onPress={() => answer.mutate({ requestId: request.id, answer: 'allow_always' })} /> : null}
              {request.can_deny ? <Button label="Deny" size="sm" variant="destructive" disabled={answer.isPending} onPress={() => answer.mutate({ requestId: request.id, answer: 'deny' })} /> : null}
            </View>
          </View>
        ))
      )}
      {answer.isError ? <View className="p-4"><Note error>{problemDetail(answer.error, 'The answer could not be sent.')}</Note></View> : null}
    </SettingsCard>
  )
}

const DECISIONS: Record<EgressLogRow['decision'], { label: string; tone: 'secondary' | 'outline' | 'destructive' }> = {
  allow: { label: 'Allowed', tone: 'secondary' },
  ask: { label: 'Asked', tone: 'outline' },
  deny: { label: 'Denied', tone: 'destructive' },
  blocked: { label: 'Blocked', tone: 'destructive' },
}

function Log({ log }: { log: EgressLogRow[] }) {
  return (
    <SettingsCard title="Connection log" description="The latest 100 connection attempts: host, answer, traffic, for whom and when. Orbit sees no addresses within a site and no content." flush>
      {log.length === 0 ? (
        <SettingsRow>
          <Note>No connections yet.</Note>
        </SettingsRow>
      ) : (
        log.map((row, index) => (
          <SettingsRow key={row.id} first={index === 0}>
            <View className="flex-1 min-w-0 gap-0.5">
              <View className="flex-row items-center gap-1.5">
                <Text numberOfLines={1} className="shrink font-mono text-[13px] text-foreground">{hostPort(row.host, row.port)}</Text>
                <Badge label={DECISIONS[row.decision].label} tone={DECISIONS[row.decision].tone} />
              </View>
              <Text className="text-[12px] text-muted-foreground">
                {row.driver_name ?? '—'} · {ago(row.created_at)}{row.decision === 'allow' ? ` · ${formatBytes(row.bytes_up)} sent, ${formatBytes(row.bytes_down)} received` : ` · ${row.reason}`}
              </Text>
            </View>
          </SettingsRow>
        ))
      )}
    </SettingsCard>
  )
}
