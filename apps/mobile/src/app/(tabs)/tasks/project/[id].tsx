import { router, useLocalSearchParams } from 'expo-router'
import { useState } from 'react'
import { View } from 'react-native'
import type { LabelRecord, ProjectRecord, StatusRecord } from '@orbit/api/generated/types.gen'

import { ChoiceGroup, FormSheet } from '../../../../components/chat/Sheet'
import { ConfirmDialog, type ConfirmRequest } from '../../../../components/settings/Dialog'
import { CheckRow, ColorSwatches, WebSetupRow } from '../../../../components/settings/Fields'
import { ProfilePicture } from '../../../../components/settings/ProfilePicture'
import { FieldLabel, Note, OptionHint, OptionText, RowText, SettingsCard, SettingsRow, SettingsShell, Toggle } from '../../../../components/settings/Shell'
import {
  CATEGORY_LABEL,
  CATEGORY_ORDER,
  PROJECT_COLORS,
  STATUS_COLORS,
  canDeleteStatus,
  categoryStatuses,
  draftChanged,
  draftValid,
  moveStatus,
  projectDraftOf,
  projectKey,
  projectUpdate,
  type ProjectDraft,
  type StatusValues,
} from '../../../../data/project-settings'
import { defaultStatusOf, LABEL_PALETTE, projectLabels } from '../../../../data/tasks'
import { useProjectMembers, useProjectMemberWrites, useProjectWrites } from '../../../../hooks/settings'
import { useOrbit } from '../../../../orbit-context'
import { useLabels, useMembers, useProjects, useStatuses, useTasks } from '../../../../queries'
import { errorMessage } from '../../../../session-context'
import { useTheme } from '../../../../theme'
import { Button, IconButton, Input, StatusIcon, Text } from '../../../../ui'

/**
 * The web's project settings on one page: General, Statuses, Access, Labels, Sub-issues and Danger zone. Every member
 * may change a project, as on the web (the server decides). GitHub and the label rules open the web app in this app.
 */
export default function ProjectSettingsScreen() {
  const { id } = useLocalSearchParams<{ id: string }>()
  const projects = useProjects()
  const project = projects.data?.find((item) => item.id === id)
  return (
    <SettingsShell title={project ? `${project.name} settings` : 'Project settings'} nav={[]} refreshing={projects.isRefetching} onRefresh={() => void projects.refetch()}>
      {projects.isPending ? <Note loading>Loading project…</Note> : null}
      {projects.isError ? <Note error>The server could not load this project.</Note> : null}
      {projects.isSuccess && !project ? <Note>This project does not exist or was removed.</Note> : null}
      {project ? (
        <>
          <GeneralCard key={`${project.id}:${project.name}:${project.key}:${project.color}`} project={project} />
          <StatusesCard project={project} />
          <AccessCard project={project} />
          <LabelsCard project={project} />
          <SubIssuesCard project={project} />
          <SettingsCard title="In the web app" description="These open the web app in this app. Sign in there if it asks." flush>
            <WebSetupRow first path={`/tasks/projects/${project.id}/settings`} label="GitHub and label rules" description="Connect a GitHub repository, and assign tasks by label." icon="link2" />
          </SettingsCard>
          <DangerZone project={project} />
        </>
      ) : null}
    </SettingsShell>
  )
}

/** Name, tag and colour: edits stay local until Save. */
function GeneralCard({ project }: { project: ProjectRecord }) {
  const { update } = useProjectWrites(project.id)
  const [draft, setDraft] = useState<ProjectDraft>(() => projectDraftOf(project))
  const set = (change: Partial<ProjectDraft>) => setDraft((current) => ({ ...current, ...change }))
  const dirty = draftChanged(project, draft)
  return (
    <SettingsCard title="General" description="Name, tag and color of this project.">
      <View className="gap-4">
        <Input label="Name" value={draft.name} onChangeText={(name) => set({ name })} />
        <Input label="Tag" value={draft.key} maxLength={5} autoCapitalize="characters" autoCorrect={false} placeholder="INF" onChangeText={(key) => set({ key: projectKey(key) })} description={`Short prefix used in task ids, for example ${draft.key || project.key}-101.`} />
        <View className="gap-2">
          <FieldLabel>Color</FieldLabel>
          <ColorSwatches label="Project color" colors={PROJECT_COLORS} value={draft.color} onChange={(color) => set({ color })} />
        </View>
        {update.isError ? <Note error>Project update failed. {errorMessage(update.error)}</Note> : null}
        {dirty ? (
          <View className="flex-row gap-2 justify-end">
            <Button label="Discard" variant="outline" onPress={() => setDraft(projectDraftOf(project))} />
            <Button label="Save" loading={update.isPending} disabled={!draftValid(draft)} onPress={() => update.mutate(projectUpdate(project, draft))} />
          </View>
        ) : null}
      </View>
    </SettingsCard>
  )
}

type StatusEditing = { mode: 'new'; category: string } | { mode: 'edit'; status: StatusRecord }

function StatusesCard({ project }: { project: ProjectRecord }) {
  const { colors } = useTheme()
  const statuses = useStatuses()
  const tasks = useTasks()
  const writes = useProjectWrites(project.id)
  const [editing, setEditing] = useState<StatusEditing | null>(null)
  const [confirm, setConfirm] = useState<ConfirmRequest | null>(null)
  const all = statuses.data ?? []
  const defaultStatus = defaultStatusOf(all, project.id)
  const countFor = (statusId: string) => (tasks.data ?? []).filter((task) => task.status_id === statusId).length
  const failed = [writes.addStatus, writes.saveStatus, writes.removeStatus, writes.reorder].find((mutation) => mutation.isError)

  return (
    <SettingsCard title="Statuses" description="The workflow a task goes through from start to completion." flush>
      {statuses.isPending ? <View className="p-4"><Note loading>Loading statuses…</Note></View> : null}
      {statuses.isError ? <View className="p-4"><Note error>The statuses could not be loaded.</Note></View> : null}
      {statuses.isSuccess
        ? CATEGORY_ORDER.map((category) => {
            const own = categoryStatuses(all, project.id, category)
            return (
              <View key={category}>
                <View className="flex-row items-center justify-between bg-muted px-4 py-1.5">
                  <Text className="text-[13px] text-muted-foreground">{CATEGORY_LABEL[category]}</Text>
                  {category === 'duplicate' ? null : <IconButton icon="plus" label={`Add ${CATEGORY_LABEL[category].toLowerCase()} status`} color={colors.mutedForeground} onPress={() => setEditing({ mode: 'new', category })} />}
                </View>
                {own.map((status, index) => {
                  const count = countFor(status.id)
                  const up = moveStatus(own, status.id, -1)
                  const down = moveStatus(own, status.id, 1)
                  return (
                    <SettingsRow key={status.id} first={index === 0}>
                      <View className="size-9 rounded-lg bg-muted items-center justify-center">
                        <StatusIcon status={status} size={16} />
                      </View>
                      <RowText title={`${status.name}${status.id === defaultStatus?.id ? ' · Default' : ''}`} subtitle={status.description || (count > 0 ? `${count} ${count === 1 ? 'task' : 'tasks'}` : undefined)} />
                      <IconButton icon="arrowUp" label={`Move ${status.name} up`} color={colors.mutedForeground} disabled={!up || writes.reorder.isPending} onPress={() => up && writes.reorder.mutate(up)} />
                      <IconButton icon="arrowDown" label={`Move ${status.name} down`} color={colors.mutedForeground} disabled={!down || writes.reorder.isPending} onPress={() => down && writes.reorder.mutate(down)} />
                      <IconButton icon="edit" label={`Edit ${status.name}`} color={colors.mutedForeground} onPress={() => setEditing({ mode: 'edit', status })} />
                      <IconButton
                        icon="trash"
                        label={`Delete ${status.name}`}
                        color={colors.mutedForeground}
                        disabled={!canDeleteStatus(status, all)}
                        onPress={() =>
                          setConfirm({
                            title: `Delete "${status.name}"?`,
                            description: count > 0 ? `${count} ${count === 1 ? 'task uses' : 'tasks use'} this status. They will move to the next status of the project.` : 'This status is not used by any task.',
                            confirmLabel: 'Delete',
                            danger: true,
                            onConfirm: () => writes.removeStatus.mutate(status),
                          })
                        }
                      />
                    </SettingsRow>
                  )
                })}
              </View>
            )
          })
        : null}
      {failed ? <View className="p-4"><Note error>The statuses could not be saved. {errorMessage(failed.error)}</Note></View> : null}
      {editing ? (
        <StatusSheet
          category={editing.mode === 'new' ? editing.category : editing.status.category}
          initial={editing.mode === 'edit' ? editing.status : undefined}
          onClose={() => setEditing(null)}
          onSave={(values) => {
            if (editing.mode === 'new') writes.addStatus.mutate({ category: editing.category, values })
            else writes.saveStatus.mutate({ status: editing.status, values })
            setEditing(null)
          }}
        />
      ) : null}
      <ConfirmDialog request={confirm} onClose={() => setConfirm(null)} />
    </SettingsCard>
  )
}

function StatusSheet({ category, initial, onClose, onSave }: { category: string; initial?: StatusRecord; onClose: () => void; onSave: (values: StatusValues) => void }) {
  const [name, setName] = useState(initial?.name ?? '')
  const [description, setDescription] = useState(initial?.description ?? '')
  const [color, setColor] = useState(initial?.color ?? STATUS_COLORS[0])
  return (
    <FormSheet visible title={initial ? 'Edit status' : `New ${CATEGORY_LABEL[category]?.toLowerCase() ?? ''} status`} submitLabel="Save" canSubmit={name.trim().length > 0} onSubmit={() => onSave({ name: name.trim(), description: description.trim(), color })} onClose={onClose}>
      <View className="flex-row items-center gap-3">
        <View className="size-10 rounded-lg bg-muted items-center justify-center">
          <StatusIcon status={{ category, color }} size={16} />
        </View>
        <Text className="text-[13px] text-muted-foreground">{CATEGORY_LABEL[category]}</Text>
      </View>
      <Input label="Name" required value={name} placeholder="Status name" autoFocus onChangeText={setName} />
      <Input label="Description" value={description} placeholder="Description…" onChangeText={setDescription} />
      <View className="gap-2">
        <FieldLabel>Color</FieldLabel>
        <ColorSwatches label="Status color" colors={STATUS_COLORS} value={color} onChange={setColor} />
      </View>
    </FormSheet>
  )
}

const VISIBILITY = [
  { value: 'workspace', label: 'Workspace', description: 'Every member of the workspace sees it.', icon: 'people' },
  { value: 'private', label: 'Private', description: 'Only the people you add, and workspace owners and admins.', icon: 'lock' },
] as const

/** Workspace or Private, saved on choice. A private project lists its members; owners and admins see it without being listed. */
function AccessCard({ project }: { project: ProjectRecord }) {
  const { user, workspace } = useOrbit()
  const { update } = useProjectWrites(project.id)
  const listedQuery = useProjectMembers(project.id, project.private)
  const team = useMembers().data ?? []
  const { add, remove } = useProjectMemberWrites(project.id)
  const [adding, setAdding] = useState(false)
  const [confirm, setConfirm] = useState<ConfirmRequest | null>(null)
  const seesEveryProject = workspace.role === 'owner' || workspace.role === 'admin'
  const isPrivate = update.isPending ? (update.variables?.private ?? project.private) : project.private
  const memberIds = listedQuery.data?.map((member) => member.user_id) ?? []
  const listed = team.filter((person) => memberIds.includes(person.user_id)).sort((a, b) => a.display_name.localeCompare(b.display_name))
  // guests and suspended members cannot join a project
  const pickable = team.filter((person) => !memberIds.includes(person.user_id) && person.role !== 'guest' && !person.suspended_at)

  const choose = (next: string) => {
    const wantPrivate = next === 'private'
    if (wantPrivate === project.private || update.isPending) return
    update.mutate(projectUpdate(project, { private: wantPrivate }))
  }
  const removeMember = (userId: string) => {
    const leaving = userId === user.id && !seesEveryProject
    if (!leaving) {
      remove.mutate(userId)
      return
    }
    setConfirm({
      title: 'Leave this project?',
      description: `You will no longer see "${project.name}" or its tasks until someone adds you again.`,
      confirmLabel: 'Leave project',
      danger: true,
      onConfirm: () => remove.mutate(userId, { onSuccess: () => router.replace('/tasks') }),
    })
  }

  return (
    <SettingsCard title="Access" description="Who can see this project and its tasks." actions={isPrivate && project.private ? <Button label="Add people" variant="outline" size="sm" icon="userAdd" onPress={() => setAdding(true)} /> : null}>
      <View className="gap-4">
        <ChoiceGroup label="Visibility" value={isPrivate ? 'private' : 'workspace'} options={VISIBILITY} onChange={choose} />
        {update.isError ? <Note error>Couldn’t change who can see this project. Try again.</Note> : null}
        {isPrivate && project.private ? (
          <View className="gap-2">
            <OptionHint>Workspace owners and admins always have access, also when they are not listed.</OptionHint>
            {listedQuery.isPending ? <Note loading>Loading members…</Note> : null}
            {listedQuery.isError ? <Note error>The members could not be loaded.</Note> : null}
            {listedQuery.isSuccess && listed.length === 0 ? <Note>Nobody is listed yet. Add the people who should see this project.</Note> : null}
            {listed.map((person) => (
              <View key={person.user_id} className="flex-row items-center gap-3 py-1.5">
                <ProfilePicture id={person.user_id} name={person.display_name} url={person.avatar_url} size={28} />
                <Text numberOfLines={1} className="flex-1 font-noto-medium text-[14px]">{person.user_id === user.id ? `${person.display_name} (you)` : person.display_name}</Text>
                <Button label={person.user_id === user.id ? 'Leave' : 'Remove'} variant="ghost" size="sm" disabled={remove.isPending} onPress={() => removeMember(person.user_id)} />
              </View>
            ))}
            {add.isError ? <Note error>Couldn’t add these people. Guests and suspended members cannot join a project.</Note> : null}
            {remove.isError ? <Note error>Couldn’t remove this person. Try again.</Note> : null}
          </View>
        ) : null}
      </View>
      {adding ? <AddPeopleSheet people={pickable} busy={add.isPending} onClose={() => setAdding(false)} onAdd={(ids) => add.mutate(ids, { onSuccess: () => setAdding(false) })} /> : null}
      <ConfirmDialog request={confirm} onClose={() => setConfirm(null)} />
    </SettingsCard>
  )
}

function AddPeopleSheet({ people, busy, onClose, onAdd }: { people: { user_id: string; display_name: string }[]; busy: boolean; onClose: () => void; onAdd: (ids: string[]) => void }) {
  const [selected, setSelected] = useState<string[]>([])
  return (
    <FormSheet visible title="Add people" submitLabel={selected.length > 1 ? `Add ${selected.length}` : 'Add'} canSubmit={selected.length > 0 && !busy} busy={busy} onSubmit={() => onAdd(selected)} onClose={onClose}>
      {people.length === 0 ? <Note>Everyone who can join is listed already.</Note> : null}
      {people.map((person) => <CheckRow key={person.user_id} label={person.display_name} checked={selected.includes(person.user_id)} onChange={(on) => setSelected((list) => (on ? [...list, person.user_id] : list.filter((id) => id !== person.user_id)))} />)}
    </FormSheet>
  )
}

const LABEL_COLORS = LABEL_PALETTE.map((swatch) => swatch.color)

/** The project's labels: create, rename, recolor and delete. Other projects keep their own. */
function LabelsCard({ project }: { project: ProjectRecord }) {
  const { colors } = useTheme()
  const labels = useLabels()
  const writes = useProjectWrites(project.id)
  const [editing, setEditing] = useState<LabelRecord | 'new' | null>(null)
  const [confirm, setConfirm] = useState<ConfirmRequest | null>(null)
  const sorted = projectLabels(labels.data ?? [], project.id).sort((a, b) => a.name.localeCompare(b.name))
  const failed = [writes.addLabel, writes.saveLabel, writes.removeLabel].find((mutation) => mutation.isError)
  return (
    <SettingsCard title="Labels" description="Labels for the tasks of this project. Each project has its own." actions={<Button label="New label" size="sm" variant="outline" icon="plus" onPress={() => setEditing('new')} />} flush>
      {labels.isPending ? <View className="p-4"><Note loading>Loading labels…</Note></View> : null}
      {labels.isError ? <View className="p-4"><Note error>Labels could not be loaded.</Note></View> : null}
      {labels.isSuccess && sorted.length === 0 ? <View className="p-4"><Note>No labels yet.</Note></View> : null}
      {sorted.map((label, index) => (
        <SettingsRow key={label.id} first={index === 0}>
          <View className="size-2.5 rounded-full" style={{ backgroundColor: label.color }} />
          <RowText title={label.name} />
          <IconButton icon="edit" label={`Edit ${label.name}`} color={colors.mutedForeground} onPress={() => setEditing(label)} />
          <IconButton
            icon="trash"
            label={`Delete ${label.name}`}
            color={colors.mutedForeground}
            disabled={writes.removeLabel.isPending}
            onPress={() => setConfirm({ title: `Delete ${label.name}?`, description: 'The label is removed from all tasks of this project. You cannot undo this.', confirmLabel: 'Delete', danger: true, onConfirm: () => writes.removeLabel.mutate(label) })}
          />
        </SettingsRow>
      ))}
      {failed ? <View className="p-4"><Note error>The label could not be saved. It may have been changed by someone else; try again.</Note></View> : null}
      {editing ? (
        <LabelSheet
          initial={editing === 'new' ? undefined : editing}
          onClose={() => setEditing(null)}
          onSave={(body) => {
            if (editing === 'new') writes.addLabel.mutate(body)
            else writes.saveLabel.mutate({ label: editing, body })
            setEditing(null)
          }}
        />
      ) : null}
      <ConfirmDialog request={confirm} onClose={() => setConfirm(null)} />
    </SettingsCard>
  )
}

function LabelSheet({ initial, onClose, onSave }: { initial?: LabelRecord; onClose: () => void; onSave: (body: { name: string; color: string }) => void }) {
  const [name, setName] = useState(initial?.name ?? '')
  const [color, setColor] = useState(initial?.color ?? LABEL_COLORS[0])
  return (
    <FormSheet visible title={initial ? 'Edit label' : 'New label'} submitLabel={initial ? 'Save' : 'Create label'} canSubmit={name.trim().length > 0} onSubmit={() => onSave({ name: name.trim(), color })} onClose={onClose}>
      <Input label="Name" required value={name} maxLength={100} placeholder="Label name" autoFocus onChangeText={setName} />
      <View className="gap-2">
        <FieldLabel>Color</FieldLabel>
        {/* a colour outside the palette (a GitHub label) is kept unless another is picked */}
        <ColorSwatches label="Label color" colors={LABEL_COLORS} value={color} onChange={setColor} />
      </View>
    </FormSheet>
  )
}

const SUB_ISSUE_SETTINGS = [
  { field: 'auto_close_parent', label: 'Close parent when all sub-issues are done', description: 'When every sub-issue is closed and at least one is done, the parent moves to its first done status.' },
  { field: 'auto_close_sub_issues', label: 'Close open sub-issues when parent is closed', description: 'Open sub-issues at every level move to done, or to cancelled when the parent is cancelled.' },
] as const

/** Saves on toggle: each switch is one decision, sent with the project's current version. */
function SubIssuesCard({ project }: { project: ProjectRecord }) {
  const { update } = useProjectWrites(project.id)
  const value = (field: (typeof SUB_ISSUE_SETTINGS)[number]['field']) => (update.isPending ? (update.variables?.[field] ?? project[field]) : project[field]) ?? true
  return (
    <SettingsCard title="Sub-issues" description="Automations that run when a task or its sub-issues close.">
      <View className="gap-3">
        {SUB_ISSUE_SETTINGS.map((setting) => (
          <View key={setting.field} className="flex-row items-start gap-4">
            <OptionText title={setting.label}>
              <OptionHint>{setting.description}</OptionHint>
            </OptionText>
            <Toggle label={setting.label} value={value(setting.field)} disabled={update.isPending} onChange={(checked) => update.mutate(projectUpdate(project, { [setting.field]: checked }))} />
          </View>
        ))}
        {update.isError ? <Note error>Couldn’t save the sub-issue settings. Try again.</Note> : null}
      </View>
    </SettingsCard>
  )
}

function DangerZone({ project }: { project: ProjectRecord }) {
  const { remove } = useProjectWrites(project.id)
  const tasks = (useTasks().data ?? []).filter((task) => task.project_id === project.id).length
  const [confirm, setConfirm] = useState<ConfirmRequest | null>(null)
  return (
    <SettingsCard title="Danger zone" description="Deleting a project moves the project and all of its tasks to trash.">
      <View className="items-start gap-2">
        <Button
          label="Delete project"
          variant="destructive"
          loading={remove.isPending}
          onPress={() =>
            setConfirm({
              title: 'Delete project?',
              description: `This will move "${project.name}" and its ${tasks} ${tasks === 1 ? 'task' : 'tasks'} to trash.`,
              confirmLabel: 'Delete',
              danger: true,
              onConfirm: () => remove.mutate(project, { onSuccess: () => router.replace('/tasks') }),
            })
          }
        />
        {remove.isError ? <Note error>Project deletion failed. {errorMessage(remove.error)}</Note> : null}
      </View>
      <ConfirmDialog request={confirm} onClose={() => setConfirm(null)} />
    </SettingsCard>
  )
}
