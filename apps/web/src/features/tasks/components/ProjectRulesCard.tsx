import { useState, type FormEvent } from 'react'
import { ArrowRight } from 'reicon-react'
import type { LabelRuleRecord } from '@orbit/api/generated/types.gen'
import { confirmAction } from '@/components/common/confirmAction'
import { SettingsCard } from '@/components/common/SettingsCard'
import { Button } from '@/components/ui/button'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { Switch } from '@/components/ui/switch'
import { LoadingText } from '@/components/ui/spinner'
import { SettingsRow } from '@/features/settings/components/SettingsParts'
import { useLabels } from '@/features/tasks/api/labels'
import { useCreateLabelRule, useDeleteLabelRule, useLabelRules, useSetOwnerLabels, useUpdateLabelRule } from '@/features/tasks/api/labelRules'
import { LabelPill } from '@/features/tasks/components/TaskLabels'
import { projectLabels } from '@/features/tasks/taskMeta'
import { useTeamMembers } from '@/features/workspaces/api'
import { useCan } from '@/features/workspaces/permissions'
import { useWorkspace } from '@/features/workspaces/workspaceContext'

type Option = { value: string; label: string }

/** `<name>` of a label `owner:<name>` (any case), trimmed, like the server's `owner_name`. */
function ownerName(label: string) {
  if (label.slice(0, 6).toLowerCase() !== 'owner:') return undefined
  return label.slice(6).trim() || undefined
}

function PersonPicker({ people, value, label, disabled, onChange }: { people: Option[]; value?: string; label: string; disabled: boolean; onChange: (userId: string) => void }) {
  return (
    <Select items={people} value={value ?? null} disabled={disabled} onValueChange={(next) => { if (next && next !== value) onChange(next) }}>
      <SelectTrigger aria-label={label} className="w-44">
        <SelectValue placeholder="Choose member" />
      </SelectTrigger>
      <SelectContent>
        {people.map((item) => <SelectItem key={item.value} value={item.value}>{item.label}</SelectItem>)}
      </SelectContent>
    </Select>
  )
}

function RuleForm({ labels, people, initial, submitLabel, pending, onSubmit, onCancel }: {
  labels: Option[]
  people: Option[]
  initial?: LabelRuleRecord
  submitLabel: string
  pending: boolean
  onSubmit: (labelId: string, userId: string) => Promise<void>
  onCancel?: () => void
}) {
  const [labelId, setLabelId] = useState(initial?.label_id ?? '')
  const [userId, setUserId] = useState(initial?.user_id ?? '')
  const submit = async (event: FormEvent) => {
    event.preventDefault()
    if (!labelId || !userId || pending) return
    try {
      await onSubmit(labelId, userId)
      if (!initial) { setLabelId(''); setUserId('') }
    } catch {
      // The card shows the mutation error.
    }
  }
  return (
    <form className="flex w-full flex-wrap items-center gap-3" onSubmit={(event) => void submit(event)}>
      <span className="text-sm text-muted-foreground">If a task has</span>
      <Select items={labels} value={labelId || null} onValueChange={(value) => setLabelId(value ?? '')}>
        <SelectTrigger aria-label="Label" className="w-44">
          <SelectValue placeholder="Label" />
        </SelectTrigger>
        <SelectContent>
          {labels.map((item) => <SelectItem key={item.value} value={item.value}>{item.label}</SelectItem>)}
        </SelectContent>
      </Select>
      <span className="text-sm text-muted-foreground">assign</span>
      <Select items={people} value={userId || null} onValueChange={(value) => setUserId(value ?? '')}>
        <SelectTrigger aria-label="Person" className="w-44">
          <SelectValue placeholder="Person" />
        </SelectTrigger>
        <SelectContent>
          {people.map((item) => <SelectItem key={item.value} value={item.value}>{item.label}</SelectItem>)}
        </SelectContent>
      </Select>
      <div className="flex gap-2">
        <Button type="submit" disabled={!labelId || !userId || pending}>{submitLabel}</Button>
        {onCancel ? <Button type="button" variant="ghost" onClick={onCancel}>Cancel</Button> : null}
      </div>
    </form>
  )
}

/** Label rules: a label assigns people (only adds assignees). Owners and admins manage them. */
export function ProjectRulesCard({ projectId }: { projectId: string }) {
  const { workspace } = useWorkspace()
  const canManage = useCan('label_rules.manage')
  const rules = useLabelRules(workspace.id, projectId, canManage)
  const labels = useLabels(workspace.id)
  const members = useTeamMembers(workspace.id)
  const createRule = useCreateLabelRule(workspace.id, projectId)
  const updateRule = useUpdateLabelRule(workspace.id, projectId)
  const deleteRule = useDeleteLabelRule(workspace.id, projectId)
  const setOwnerLabels = useSetOwnerLabels(workspace.id, projectId)
  const [editingId, setEditingId] = useState<string>()

  const description = 'Assign people automatically when a task gets a label. Rules only add assignees; they never remove one.'
  if (!canManage) {
    return <SettingsCard title="Rules" description={description}><p className="text-sm text-muted-foreground">Only workspace owners and admins manage rules.</p></SettingsCard>
  }

  const ownLabels = projectLabels(labels.data ?? [], projectId).sort((a, b) => a.name.localeCompare(b.name))
  // owner: labels get their rules in the name map, not in the rules list.
  const labelOptions = ownLabels
    .filter((label) => !ownerName(label.name))
    .map((label) => ({ value: label.id, label: label.name }))
  const peopleOptions = (members.data ?? [])
    .filter((member) => !member.suspended)
    .sort((a, b) => a.name.localeCompare(b.name))
    .map((member) => ({ value: member.id, label: member.name }))
  const ownerLabelsOn = rules.data?.owner_labels ?? false
  const unmatched = rules.data?.unmatched_owner_names ?? []
  const plainRules = rules.data?.rules.filter((rule) => !ownerName(rule.label_name)) ?? []
  const mapped = rules.data?.rules.filter((rule) => ownerName(rule.label_name)) ?? []
  const unmapped = ownerLabelsOn ? ownLabels.filter((label) => unmatched.includes(ownerName(label.name) ?? '')) : []
  const mapPending = createRule.isPending || updateRule.isPending || deleteRule.isPending

  const remove = async (rule: LabelRuleRecord) => {
    if (!await confirmAction({ title: 'Delete this rule?', description: `Tasks that get ${rule.label_name} will no longer be assigned to ${rule.user_name}. People assigned so far stay.`, confirmLabel: 'Delete', danger: true })) return
    deleteRule.mutate(rule.id)
  }

  return (
    <SettingsCard title="Rules" description={description} flush>
      <div className="flex flex-col divide-y">
        <SettingsRow>
          <RuleForm labels={labelOptions} people={peopleOptions} submitLabel="Add rule" pending={createRule.isPending} onSubmit={async (labelId, userId) => { await createRule.mutateAsync({ label_id: labelId, user_id: userId }) }} />
        </SettingsRow>
        {createRule.isError ? <SettingsRow className="text-xs text-destructive" role="alert">The rule could not be added. This label may already assign this person.</SettingsRow> : null}
        {rules.isPending ? <SettingsRow><LoadingText>Loading rules…</LoadingText></SettingsRow> : null}
        {rules.isError ? <SettingsRow role="alert">Rules could not be loaded.</SettingsRow> : null}
        {rules.isSuccess && plainRules.length === 0 ? <SettingsRow className="text-sm text-muted-foreground">No rules yet.</SettingsRow> : null}
        {plainRules.map((rule) => editingId === rule.id
          ? <SettingsRow key={rule.id}>
              <RuleForm labels={labelOptions} people={peopleOptions} initial={rule} submitLabel="Save" pending={updateRule.isPending} onCancel={() => setEditingId(undefined)} onSubmit={async (labelId, userId) => { await updateRule.mutateAsync({ ruleId: rule.id, label_id: labelId, user_id: userId }); setEditingId(undefined) }} />
            </SettingsRow>
          : <SettingsRow key={rule.id}>
              <div className="flex min-w-0 flex-1 items-center gap-2 text-sm data-disabled:opacity-60" data-disabled={rule.enabled ? undefined : ''}>
                <LabelPill label={{ name: rule.label_name, color: rule.label_color }} />
                <ArrowRight className="size-3.5 shrink-0 text-muted-foreground" aria-label="assigns" />
                <span className="truncate font-medium text-foreground">{rule.user_name}</span>
              </div>
              <Switch aria-label={`${rule.label_name} → ${rule.user_name} on`} checked={rule.enabled} disabled={updateRule.isPending} onCheckedChange={(checked: boolean) => updateRule.mutate({ ruleId: rule.id, enabled: checked })} />
              <Button variant="ghost" type="button" aria-label={`Edit rule ${rule.label_name} → ${rule.user_name}`} onClick={() => { updateRule.reset(); setEditingId(rule.id) }}>Edit</Button>
              <Button variant="ghost" type="button" aria-label={`Delete rule ${rule.label_name} → ${rule.user_name}`} disabled={deleteRule.isPending} onClick={() => void remove(rule)}>Delete</Button>
            </SettingsRow>)}
        {updateRule.isError ? <SettingsRow className="text-xs text-destructive" role="alert">The rule could not be saved. This label may already assign this person.</SettingsRow> : null}
        {deleteRule.isError ? <SettingsRow className="text-xs text-destructive" role="alert">The rule could not be deleted.</SettingsRow> : null}
        <SettingsRow className="flex-nowrap items-start justify-between gap-4">
          <div className="flex min-w-0 flex-col gap-0.5">
            <span className="text-sm font-medium text-foreground">Assign owner:&lt;name&gt; labels automatically</span>
            <span className="text-xs text-muted-foreground">A label such as owner:raymond assigns the member whose handle (email before @) or name is raymond, in any case.</span>
            {setOwnerLabels.isError ? <span role="alert" className="text-xs text-destructive">The switch could not be saved.</span> : null}
          </div>
          <Switch aria-label="Assign owner:<name> labels automatically" checked={ownerLabelsOn} disabled={!rules.isSuccess || setOwnerLabels.isPending} onCheckedChange={(checked: boolean) => setOwnerLabels.mutate(checked)} />
        </SettingsRow>
        {mapped.length > 0 || unmapped.length > 0 ? (
          <SettingsRow className="flex-col items-start gap-0.5">
            <span className="text-sm font-medium text-foreground">Owner names</span>
            <span className="text-xs text-muted-foreground">Map an owner: name to a member. The mapped member is assigned instead of the handle match, even with the switch off.</span>
          </SettingsRow>
        ) : null}
        {mapped.map((rule) => (
          <SettingsRow key={rule.id}>
            <div className="flex min-w-0 flex-1 items-center gap-2 text-sm data-disabled:opacity-60" data-disabled={rule.enabled ? undefined : ''}>
              <LabelPill label={{ name: rule.label_name, color: rule.label_color }} />
              <ArrowRight className="size-3.5 shrink-0 text-muted-foreground" aria-label="assigns" />
            </div>
            <PersonPicker people={peopleOptions} value={rule.user_id} label={`Member for ${rule.label_name}`} disabled={mapPending} onChange={(userId) => updateRule.mutate({ ruleId: rule.id, user_id: userId })} />
            <Button variant="ghost" type="button" aria-label={`Remove name map ${rule.label_name}`} disabled={mapPending} onClick={() => deleteRule.mutate(rule.id)}>Remove</Button>
          </SettingsRow>
        ))}
        {unmapped.map((label) => (
          <SettingsRow key={label.id}>
            <div className="flex min-w-0 flex-1 items-center gap-2 text-sm">
              <LabelPill label={label} />
              <span className="text-xs text-muted-foreground">No member matches</span>
            </div>
            <PersonPicker people={peopleOptions} label={`Member for ${label.name}`} disabled={mapPending} onChange={(userId) => createRule.mutate({ label_id: label.id, user_id: userId })} />
          </SettingsRow>
        ))}
      </div>
    </SettingsCard>
  )
}
