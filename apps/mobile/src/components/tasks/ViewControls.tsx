/**
 * The web's filter bar, Display popover and Save view dialog at phone size: a Filter sheet (one row per field, a tap
 * opens its choices in place), a Display sheet (grouping, ordering, completed tasks) and a dialog for a view's name,
 * description and visibility.
 */
import { useEffect, useState } from 'react'
import { Pressable, View } from 'react-native'

import { PRIORITY_CHOICES, PRIORITY_LABEL, toggleId } from '../../data/tasks'
import {
  DUE_LABEL,
  type DueFilter,
  EMPTY_QUICK,
  GROUP_LABEL,
  labelNameChoices,
  ORDER_LABEL,
  type OrderBy,
  PHONE_GROUPS,
  type QuickFilter,
  type ShowCompleted,
  toggleLabelChoice,
  type ViewFields,
  type ViewState,
  type Visibility,
} from '../../data/views'
import { useOrbit } from '../../orbit-context'
import { useLabels, usePeople, useProjects } from '../../queries'
import { useTheme } from '../../theme'
import { Avatar, Button, Icon, Input, PriorityIcon, StatusIcon, Text } from '../../ui'
import { Dialog } from '../settings/Dialog'
import { Sheet, SheetList, SheetRow } from './Sheet'

type Field = 'statusCategory' | 'assignee' | 'priority' | 'project' | 'label' | 'due'

const FIELD_LABEL: Record<Field, string> = { statusCategory: 'Status type', assignee: 'Assignee', priority: 'Priority', project: 'Project', label: 'Label', due: 'Due date' }

const CATEGORIES = [
  { key: 'unstarted', label: 'Unstarted' },
  { key: 'started', label: 'Started' },
  { key: 'completed', label: 'Completed' },
  { key: 'cancelled', label: 'Cancelled' },
  { key: 'duplicate', label: 'Duplicate' },
] as const

const DUE_CHOICES: readonly DueFilter[] = ['overdue', 'today', 'this_week', 'next_7_days', 'none', 'any']

/**
 * The Filter sheet. Each change applies at once. `kept` counts a view's conditions the phone cannot edit (made on the
 * web); they stay and still apply. `withProject: false` hides the Project row on a page that picks its project itself.
 */
export function FilterSheet({ visible, quick, kept, withProject, onChange, onClose }: { visible: boolean; quick: QuickFilter; kept: number; withProject: boolean; onChange: (next: QuickFilter) => void; onClose: () => void }) {
  const { colors } = useTheme()
  const { user } = useOrbit()
  const people = usePeople()
  const projects = useProjects().data ?? []
  const labels = labelNameChoices(useLabels().data ?? [])
  const [field, setField] = useState<Field | null>(null)
  useEffect(() => {
    if (visible) setField(null)
  }, [visible])

  const peopleChoices = [{ id: 'me', name: 'Me' }, ...people.filter((person) => person.id !== user.id).sort((a, b) => a.name.localeCompare(b.name))]
  const nameOf = (field: Field): string => {
    const names = (() => {
      switch (field) {
        case 'statusCategory':
          return CATEGORIES.filter((item) => quick.statusCategory.includes(item.key)).map((item) => item.label)
        case 'assignee':
          return quick.assignee.map((id) => (id === 'me' || id === user.id ? 'Me' : (people.find((person) => person.id === id)?.name ?? 'Someone')))
        case 'priority':
          return quick.priority.map((priority) => PRIORITY_LABEL[priority as keyof typeof PRIORITY_LABEL] ?? priority)
        case 'project':
          return quick.project.map((id) => projects.find((project) => project.id === id)?.name ?? 'Deleted project')
        case 'label':
          return labels.filter((choice) => choice.ids.some((id) => quick.label.includes(id))).map((choice) => choice.name)
        case 'due':
          return quick.due ? [DUE_LABEL[quick.due]] : []
      }
    })()
    return names.length === 0 ? 'Any' : names.join(', ')
  }
  const fields: Field[] = ['statusCategory', 'assignee', 'priority', ...(withProject ? (['project'] as const) : []), 'label', 'due']
  const list = (key: 'statusCategory' | 'assignee' | 'priority' | 'project', id: string) => onChange({ ...quick, [key]: toggleId(quick[key], id) })

  return (
    <Sheet visible={visible} title={field ? FIELD_LABEL[field] : 'Filter'} onClose={onClose}>
      {field === null ? (
        <SheetList>
          {fields.map((item) => (
            <SheetRow key={item} label={FIELD_LABEL[item]} subtitle={nameOf(item)} icon="filter" onPress={() => setField(item)} />
          ))}
          {kept > 0 ? (
            <Text className="text-[12px] px-4 py-2 text-muted-foreground">
              {kept === 1 ? 'One more condition' : `${kept} more conditions`} from the web also apply. Change {kept === 1 ? 'it' : 'them'} on the web.
            </Text>
          ) : null}
          <SheetRow label="Clear filters" icon="close" onPress={() => onChange({ ...EMPTY_QUICK, text: quick.text })} />
        </SheetList>
      ) : (
        <SheetList>
          <Pressable accessibilityRole="button" accessibilityLabel="Back to filters" onPress={() => setField(null)} className="min-h-11 flex-row items-center gap-2 px-4 active:bg-muted">
            <Icon name="chevronLeft" size={16} color={colors.mutedForeground} />
            <Text className="text-[14px] text-muted-foreground">All filters</Text>
          </Pressable>
          {field === 'statusCategory'
            ? CATEGORIES.map((item) => (
                <SheetRow key={item.key} label={item.label} leading={<StatusIcon status={{ category: item.key, color: colors.mutedForeground }} />} checked={quick.statusCategory.includes(item.key)} onPress={() => list('statusCategory', item.key)} />
              ))
            : null}
          {field === 'assignee'
            ? peopleChoices.map((person) => (
                <SheetRow key={person.id} label={person.name} leading={<Avatar id={person.id === 'me' ? user.id : person.id} name={person.id === 'me' ? user.display_name : person.name} size={22} />} checked={quick.assignee.includes(person.id)} onPress={() => list('assignee', person.id)} />
              ))
            : null}
          {field === 'priority'
            ? PRIORITY_CHOICES.map((priority) => (
                <SheetRow key={priority} label={PRIORITY_LABEL[priority]} leading={<PriorityIcon priority={priority} />} checked={quick.priority.includes(priority)} onPress={() => list('priority', priority)} />
              ))
            : null}
          {field === 'project'
            ? projects.map((project) => (
                <SheetRow key={project.id} label={project.name} leading={<View className="size-2.5 rounded-full" style={{ backgroundColor: project.color }} />} checked={quick.project.includes(project.id)} onPress={() => list('project', project.id)} />
              ))
            : null}
          {field === 'label'
            ? labels.map((choice) => (
                <SheetRow
                  key={choice.key}
                  label={choice.name}
                  leading={<View className="size-2.5 rounded-full" style={{ backgroundColor: choice.color }} />}
                  checked={choice.ids.some((id) => quick.label.includes(id))}
                  onPress={() => onChange({ ...quick, label: toggleLabelChoice(quick.label, choice.ids) })}
                />
              ))
            : null}
          {field === 'due'
            ? DUE_CHOICES.map((due) => <SheetRow key={due} label={DUE_LABEL[due]} icon="calendar" checked={quick.due === due} onPress={() => onChange({ ...quick, due: quick.due === due ? null : due })} />)
            : null}
        </SheetList>
      )}
    </Sheet>
  )
}

const COMPLETED_LABEL: Record<ShowCompleted, string> = { all: 'All', past_week: 'Past week', past_month: 'Past month', none: 'None' }

/** The Display sheet: grouping, ordering and direction, and which completed tasks show. */
export function DisplaySheet({ visible, state, onChange, onClose }: { visible: boolean; state: ViewState; onChange: (next: ViewState) => void; onClose: () => void }) {
  const display = state.display
  const set = (changes: Partial<ViewState['display']>) => onChange({ ...state, display: { ...display, ...changes } })
  return (
    <Sheet visible={visible} title="Display" onClose={onClose}>
      <SheetList>
        <Heading>Grouping</Heading>
        {PHONE_GROUPS.map((group) => (
          <SheetRow key={group} label={GROUP_LABEL[group]} checked={display.group_by === group} onPress={() => set({ group_by: group, sub_group_by: 'none' })} />
        ))}
        <Heading>Ordering</Heading>
        {(Object.keys(ORDER_LABEL) as OrderBy[]).map((order) => (
          <SheetRow key={order} label={ORDER_LABEL[order]} checked={display.order_by === order} onPress={() => set({ order_by: order })} />
        ))}
        <SheetRow label={display.order_direction === 'asc' ? 'Ascending' : 'Descending'} subtitle="Tap to reverse" icon={display.order_direction === 'asc' ? 'arrowUp' : 'arrowDown'} onPress={() => set({ order_direction: display.order_direction === 'asc' ? 'desc' : 'asc' })} />
        <Heading>Completed tasks</Heading>
        {(Object.keys(COMPLETED_LABEL) as ShowCompleted[]).map((option) => (
          <SheetRow key={option} label={COMPLETED_LABEL[option]} checked={display.show_completed === option} onPress={() => set({ show_completed: option })} />
        ))}
      </SheetList>
    </Sheet>
  )
}

function Heading({ children }: { children: string }) {
  return <Text className="font-noto-medium text-[12px] px-4 pt-3 pb-1 text-muted-foreground">{children}</Text>
}

export interface ViewDialogRequest {
  title: string
  confirmLabel: string
  initial: ViewFields
  /** Only the owner (or an admin) may change who sees a view. */
  canChangeVisibility: boolean
  onSubmit: (fields: ViewFields) => Promise<unknown>
}

/** Name, description and visibility of a new, edited or duplicated view. `request: null` hides it. */
export function ViewDialog({ request, onClose }: { request: ViewDialogRequest | null; onClose: () => void }) {
  const { colors } = useTheme()
  const [fields, setFields] = useState<ViewFields>({ name: '', description: '', visibility: 'personal' })
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  useEffect(() => {
    if (request) setFields(request.initial)
    setError(null)
    setBusy(false)
  }, [request])
  const submit = () => {
    if (!request || busy || !fields.name.trim()) return
    setBusy(true)
    request.onSubmit({ ...fields, name: fields.name.trim(), description: fields.description.trim() }).then(onClose, (reason: unknown) => {
      setBusy(false)
      setError(reason instanceof Error ? reason.message : 'Could not save the view.')
    })
  }
  const visibility = (value: Visibility, label: string, detail: string) => (
    <Pressable
      key={value}
      accessibilityRole="radio"
      accessibilityState={{ checked: fields.visibility === value, disabled: !request?.canChangeVisibility }}
      disabled={!request?.canChangeVisibility}
      onPress={() => setFields({ ...fields, visibility: value })}
      className={`flex-1 rounded-lg border p-2.5 gap-0.5 ${fields.visibility === value ? 'border-primary bg-primary/8' : 'border-border'} ${request?.canChangeVisibility ? 'opacity-100' : 'opacity-60'}`}
    >
      <View className="flex-row items-center gap-1.5">
        <Icon name={value === 'personal' ? 'lock' : 'people'} size={14} color={fields.visibility === value ? colors.primary : colors.mutedForeground} />
        <Text className="font-noto-medium text-[13px]">{label}</Text>
      </View>
      <Text className="text-[11px] text-muted-foreground">{detail}</Text>
    </Pressable>
  )
  return (
    <Dialog
      visible={request !== null}
      title={request?.title ?? ''}
      onClose={onClose}
      footer={
        <>
          <Button label="Cancel" variant="outline" onPress={onClose} />
          <Button label={request?.confirmLabel ?? 'Save'} disabled={busy || !fields.name.trim()} onPress={submit} />
        </>
      }
    >
      <Input label="Name" value={fields.name} onChangeText={(name) => setFields({ ...fields, name })} placeholder="View name" autoFocus maxLength={120} returnKeyType="done" onSubmitEditing={submit} />
      <Input label="Description" value={fields.description} onChangeText={(description) => setFields({ ...fields, description })} placeholder="Optional" maxLength={500} />
      <View className="flex-row gap-2">
        {visibility('personal', 'Only me', 'Only you see it.')}
        {visibility('workspace', 'Workspace', 'Every member sees it.')}
      </View>
      {error ? <Text className="text-[13px] text-destructive">{error}</Text> : null}
    </Dialog>
  )
}
