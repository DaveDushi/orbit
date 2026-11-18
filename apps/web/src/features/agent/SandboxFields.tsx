import { Link } from 'react-router'
import { formatBytes, type AgentSandbox, type SandboxDraft } from '@orbit/api/agentSandbox'
import { Field, FieldDescription, FieldLabel } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { Switch } from '@/components/ui/switch'
import { FieldGrid } from '@/features/settings/components/SettingsParts'

/** Shown on the agent's card when `sandbox_shell` is ticked: the switch, the limits and this month's use. */
export function SandboxFields({ agentId, sandbox, draft, onChange }: { agentId: string; sandbox: AgentSandbox; draft: SandboxDraft; onChange: (draft: SandboxDraft) => void }) {
  const set = (change: Partial<SandboxDraft>) => onChange({ ...draft, ...change })
  const usage = sandbox.usage
  return (
    <fieldset data-slot="agent-sandbox" className="flex flex-col gap-3">
      <legend className="mb-1 text-sm font-medium text-foreground">Sandbox</legend>
      <label className="flex items-center gap-2 text-sm text-foreground">
        <Switch checked={draft.enabled} onCheckedChange={(enabled) => set({ enabled })} />
        Run code in a sandbox
      </label>
      <p className="-mt-1 text-xs text-muted-foreground">
        Each chat thread gets its own isolated sandbox (1 CPU, 1 GB memory, 4 GB of files, 15 minutes per command). It reaches only the hosts allowed on its{' '}
        <Link to={`/settings/agentic/agents/${agentId}/internet`} className="text-foreground underline underline-offset-2">Internet access</Link> page. Off: no new commands and no internet.
      </p>
      {draft.enabled ? (
        <FieldGrid>
          <Field>
            <FieldLabel htmlFor="agent-sandbox-idle">Stop after (minutes without commands)</FieldLabel>
            <Input id="agent-sandbox-idle" type="number" inputMode="numeric" min={5} max={120} required value={draft.idle} onChange={(event) => set({ idle: event.target.value })} />
            <FieldDescription>5 to 120. Its files stay.</FieldDescription>
          </Field>
          <Field>
            <FieldLabel htmlFor="agent-sandbox-cpu">CPU minutes per month</FieldLabel>
            <Input id="agent-sandbox-cpu" type="number" inputMode="numeric" min={0} required value={draft.cpu} onChange={(event) => set({ cpu: event.target.value })} />
            <FieldDescription>Used: {Math.round(usage.cpu_ms / 60_000)} this month.</FieldDescription>
          </Field>
          <Field>
            <FieldLabel htmlFor="agent-sandbox-traffic">Internet traffic per month (GB)</FieldLabel>
            <Input id="agent-sandbox-traffic" type="number" inputMode="decimal" min={0} step="any" required value={draft.trafficGb} onChange={(event) => set({ trafficGb: event.target.value })} />
            <FieldDescription>Used: {formatBytes(usage.egress_bytes)} this month.</FieldDescription>
          </Field>
          <Field>
            <FieldLabel htmlFor="agent-sandbox-tokens">Model tokens per month (millions)</FieldLabel>
            <Input id="agent-sandbox-tokens" type="number" inputMode="decimal" min={0} step="any" required value={draft.tokensM} onChange={(event) => set({ tokensM: event.target.value })} />
            <FieldDescription>For all runs that use the sandbox.</FieldDescription>
            </Field>
        </FieldGrid>
      ) : null}
      {draft.enabled ? <p className="text-xs text-muted-foreground">When the CPU or traffic limit is spent, the sandbox refuses new commands and internet access until next month.</p> : null}
    </fieldset>
  )
}
