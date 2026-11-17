import { useState } from 'react'
import { View } from 'react-native'

import { type LabelRuleRecord, ownerName } from '../../data/project-settings'
import { projectLabels } from '../../data/tasks'
import { useLabelRules, useLabelRuleWrite } from '../../hooks/settings'
import { useOrbit } from '../../orbit-context'
import { useLabels, useMembers } from '../../queries'
import { errorMessage } from '../../session-context'
import { useTheme } from '../../theme'
import { Button, Icon, IconButton, Text } from '../../ui'
import { FormSheet } from '../chat/Sheet'
import { ConfirmDialog, type ConfirmRequest } from '../settings/Dialog'
import { SelectField } from '../settings/Fields'
import { Note, RowText, SettingsCard, SettingsRow, Toggle } from '../settings/Shell'
import { LabelPill } from './pickers'

const DESCRIPTION = 'Assign people automatically when a task gets a label. Rules only add assignees; they never remove one.'

/** The web's project Rules card: label → person rules, on/off, edit and delete, and the owner:<name> switch and map. */
export function ProjectRulesCard({ projectId }: { projectId: string }) {
  const { colors } = useTheme()
  const { workspace } = useOrbit()
  const canManage = workspace.permissions.includes('label_rules.manage')
  const rules = useLabelRules(projectId, canManage)
  const labels = useLabels()
  const members = useMembers()
  const write = useLabelRuleWrite(projectId)
  const [form, setForm] = useState<{ rule?: LabelRuleRecord; ownerLabel?: string } | null>(null)
  const [confirm, setConfirm] = useState<ConfirmRequest | null>(null)

  if (!canManage) {
    return (
      <SettingsCard title="Rules" description={DESCRIPTION}>
        <Note>Only workspace owners and admins manage rules.</Note>
      </SettingsCard>
    )
  }
  const own = projectLabels(labels.data ?? [], projectId).sort((a, b) => a.name.localeCompare(b.name))
  const labelOptions = own.filter((label) => !ownerName(label.name)).map((label) => ({ value: label.id, label: label.name }))
  const people = (members.data ?? [])
    .filter((member) => !member.suspended_at && member.role !== 'guest')
    .sort((a, b) => a.display_name.localeCompare(b.display_name))
    .map((member) => ({ value: member.user_id, label: member.display_name }))
  const ownerLabelsOn = rules.data?.owner_labels ?? false
  const unmatched = rules.data?.unmatched_owner_names ?? []
  const unmapped = ownerLabelsOn ? own.filter((label) => unmatched.includes(ownerName(label.name) ?? '')) : []
  const list = rules.data?.rules ?? []

  return (
    <SettingsCard title="Rules" description={DESCRIPTION} actions={<Button label="Add rule" size="sm" variant="outline" icon="plus" onPress={() => setForm({})} />} flush>
      {rules.isPending ? <View className="p-4"><Note loading>Loading rules…</Note></View> : null}
      {rules.isError ? <View className="p-4"><Note error>Rules could not be loaded.</Note></View> : null}
      {rules.isSuccess && list.length === 0 ? <View className="p-4"><Note>No rules yet.</Note></View> : null}
      {list.map((rule, index) => (
        <SettingsRow key={rule.id} first={index === 0}>
          <View className={`flex-1 min-w-0 flex-row items-center gap-2 ${rule.enabled ? 'opacity-100' : 'opacity-60'}`}>
            <LabelPill label={{ name: rule.label_name, color: rule.label_color }} />
            <Icon name="arrowRight" size={14} color={colors.mutedForeground} />
            <Text numberOfLines={1} className="shrink font-noto-medium text-[14px]">{rule.user_name}</Text>
          </View>
          <Toggle value={rule.enabled} label={`${rule.label_name} → ${rule.user_name} on`} disabled={write.isPending} onChange={(on) => write.mutate({ kind: 'change', ruleId: rule.id, body: { enabled: on } })} />
          <IconButton icon="edit" label={`Edit rule ${rule.label_name} → ${rule.user_name}`} color={colors.mutedForeground} onPress={() => setForm({ rule })} />
          <IconButton
            icon="trash"
            label={`Delete rule ${rule.label_name} → ${rule.user_name}`}
            color={colors.mutedForeground}
            disabled={write.isPending}
            onPress={() =>
              setConfirm({
                title: 'Delete this rule?',
                description: `Tasks that get ${rule.label_name} will no longer be assigned to ${rule.user_name}. People assigned so far stay.`,
                confirmLabel: 'Delete',
                danger: true,
                onConfirm: () => write.mutate({ kind: 'delete', ruleId: rule.id }),
              })
            }
          />
        </SettingsRow>
      ))}
      <SettingsRow first={list.length === 0 && !rules.isPending}>
        <RowText title="Assign owner:<name> labels automatically" subtitle="A label such as owner:raymond assigns the member whose handle (email before @) or name is raymond, in any case." />
        <Toggle value={ownerLabelsOn} label="Assign owner:<name> labels automatically" disabled={!rules.isSuccess || write.isPending} onChange={(on) => write.mutate({ kind: 'owner-labels', on })} />
      </SettingsRow>
      {unmapped.map((label) => (
        <SettingsRow key={label.id} label={`Map ${label.name} to a member`} onPress={() => setForm({ ownerLabel: label.id })}>
          <LabelPill label={label} />
          <RowText title="Matches nobody" subtitle="Tap to choose the member it assigns." />
        </SettingsRow>
      ))}
      {write.isError ? <View className="p-4"><Note error>The rule could not be saved. This label may already assign this person. {errorMessage(write.error)}</Note></View> : null}
      {form ? (
        <RuleSheet
          title={form.rule ? 'Edit rule' : form.ownerLabel ? 'Map owner label' : 'Add rule'}
          labels={form.ownerLabel ? own.filter((label) => label.id === form.ownerLabel).map((label) => ({ value: label.id, label: label.name })) : labelOptions}
          people={people}
          initial={{ labelId: form.rule?.label_id ?? form.ownerLabel ?? '', userId: form.rule?.user_id ?? '' }}
          onClose={() => setForm(null)}
          onSave={(labelId, userId) => {
            if (form.rule) write.mutate({ kind: 'change', ruleId: form.rule.id, body: { label_id: labelId, user_id: userId } })
            else write.mutate({ kind: 'add', body: { label_id: labelId, user_id: userId } })
            setForm(null)
          }}
        />
      ) : null}
      <ConfirmDialog request={confirm} onClose={() => setConfirm(null)} />
    </SettingsCard>
  )
}

function RuleSheet({ title, labels, people, initial, onClose, onSave }: {
  title: string
  labels: { value: string; label: string }[]
  people: { value: string; label: string }[]
  initial: { labelId: string; userId: string }
  onClose: () => void
  onSave: (labelId: string, userId: string) => void
}) {
  const [labelId, setLabelId] = useState(initial.labelId)
  const [userId, setUserId] = useState(initial.userId)
  return (
    <FormSheet visible title={title} submitLabel="Save" canSubmit={Boolean(labelId && userId)} onSubmit={() => onSave(labelId, userId)} onClose={onClose}>
      <SelectField label="If a task has" value={labelId} options={labels} placeholder="Label" onChange={setLabelId} description={labels.length === 0 ? 'This project has no labels yet. Add one in Labels.' : undefined} />
      <SelectField label="Assign" value={userId} options={people} placeholder="Person" onChange={setUserId} />
    </FormSheet>
  )
}
