import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import * as DocumentPicker from 'expo-document-picker'
import { type AgentSkill, type AgentSkillChange, createAgentSkill, deleteAgentSkill, installAgentSkill, readAgentSkill, readAgentSkills, updateAgentSkill } from '@orbit/api/agentSkills'
import { useEffect, useState } from 'react'
import { View } from 'react-native'

import { CheckRow } from '../settings/Fields'
import { ConfirmDialog, type ConfirmRequest, Dialog } from '../settings/Dialog'
import { Badge, Note, SettingsCard, Segmented } from '../settings/Shell'
import { skillAgents, skillDraftReady, type SkillDraft, type SkillMode, uploadSkillZip } from '../../data/agentic'
import { useOrbit } from '../../orbit-context'
import { errorMessage } from '../../session-context'
import { uploadPart } from '../../upload-part'
import { Button, Input, Text } from '../../ui'

const SOURCE_LABELS: Record<AgentSkill['source'], string> = { manual: 'Written here', zip: 'Uploaded .zip', github: 'GitHub', skills_sh: 'skills.sh' }

const MODES: readonly { value: SkillMode; label: string }[] = [
  { value: 'install', label: 'GitHub or skills.sh' },
  { value: 'upload', label: 'Upload .zip' },
  { value: 'write', label: 'Write' },
]

const DESCRIPTION =
  'Skills teach the agents how to do kinds of tasks (the open SKILL.md format). Turn each skill on for the agents that need it: an agent sees only the name and description of its skills, and reads a skill when a request needs it. A skill never gives an agent more permissions: it still acts as the person who asked.'

const skillsKey = (workspaceId: string) => ['agent', 'skills', workspaceId] as const

/** The web's Settings → Agentic → Skills: owners and admins add skills and turn each one on per agent. */
export function SkillsCard() {
  const { workspace } = useOrbit()
  if (!workspace.permissions.includes('workspace.update')) {
    return (
      <SettingsCard title="Skills" description="Skills teach the agents how to do kinds of tasks (the open SKILL.md format).">
        <Note>Only owners and admins can see and manage the agents' skills.</Note>
      </SettingsCard>
    )
  }
  return <ManageSkills />
}

function useSkillWrites() {
  const { client, workspaceId } = useOrbit()
  const queryClient = useQueryClient()
  const onSettled = () => void queryClient.invalidateQueries({ queryKey: skillsKey(workspaceId) })
  return {
    update: useMutation({ mutationFn: ({ id, change }: { id: string; change: AgentSkillChange }) => updateAgentSkill(client, workspaceId, id, change), onSettled }),
    remove: useMutation({ mutationFn: (id: string) => deleteAgentSkill(client, workspaceId, id), onSettled }),
    add: useMutation({
      mutationFn: async ({ mode, draft }: { mode: SkillMode; draft: SkillDraft }) =>
        mode === 'install'
          ? installAgentSkill(client, workspaceId, draft.source.trim())
          : mode === 'upload' && draft.file
            ? uploadSkillZip(client, workspaceId, draft.file)
            : createAgentSkill(client, workspaceId, { name: draft.name, description: draft.description, body: draft.body }),
      onSettled,
    }),
  }
}

function ManageSkills() {
  const { client, workspaceId } = useOrbit()
  const skills = useQuery({ queryKey: skillsKey(workspaceId), queryFn: () => readAgentSkills(client, workspaceId) })
  const { update, remove } = useSkillWrites()
  const [adding, setAdding] = useState(false)
  const [editing, setEditing] = useState<AgentSkill | null>(null)
  const [confirm, setConfirm] = useState<ConfirmRequest | null>(null)
  const data = skills.data

  return (
    <SettingsCard title="Skills" description={DESCRIPTION} actions={data ? <Button label="Add skill" size="sm" disabled={data.skills.length >= data.max_skills} onPress={() => setAdding(true)} /> : null}>
      {skills.isPending ? <Note loading>Loading the skills…</Note> : null}
      {skills.isError ? <Note error>The skills could not be loaded.</Note> : null}
      {data ? (
        <View className="gap-3">
          <Note>
            {data.sandbox
              ? "Scripts in a skill run only in the agent's sandbox (no internet, no access to this server), for agents with the media tools."
              : data.sandbox === false
                ? "This server has no agent sandbox, so scripts in skills are stored but cannot run. The agents still read the skills' instructions and files."
                : 'The agent service is offline, so it is not known whether scripts in skills can run.'}
          </Note>
          {data.skills.length === 0 ? <Note>No skills yet. Install one from GitHub or skills.sh, upload a .zip, or write one.</Note> : null}
          {data.skills.map((skill, index) => (
            <View key={skill.id} className={`gap-1.5 py-3 ${index === 0 ? '' : 'border-t border-border'}`}>
              <View className="flex-row flex-wrap items-center gap-2">
                <Text className="font-mono text-[14px] font-noto-medium">{skill.name}</Text>
                <Badge label={SOURCE_LABELS[skill.source]} tone="outline" />
                {skill.files.length ? <Badge label={skill.files.length === 1 ? '1 file' : `${skill.files.length} files`} tone="secondary" /> : null}
                {skill.has_scripts ? <Badge label={data.sandbox ? 'Scripts' : 'Scripts cannot run'} tone={data.sandbox ? 'secondary' : 'outline'} /> : null}
              </View>
              <Text numberOfLines={2} className="text-[14px] text-muted-foreground">{skill.description}</Text>
              {skill.source_url ? <Text numberOfLines={1} className="text-[12px] text-muted-foreground">{skill.source_url}</Text> : null}
              <View accessibilityLabel={`Agents that use ${skill.name}`} className="pt-1">
                {data.agents.map((agent) => (
                  <CheckRow key={agent.key} label={agent.name} checked={skill.agents.includes(agent.key)} onChange={(on) => !update.isPending && update.mutate({ id: skill.id, change: { agents: skillAgents(skill.agents, agent.key, on) } })} />
                ))}
              </View>
              <View className="flex-row gap-1">
                <Button label="Edit" variant="ghost" size="sm" onPress={() => setEditing(skill)} />
                <Button
                  label="Delete"
                  variant="ghost"
                  size="sm"
                  onPress={() => setConfirm({ title: `Delete ${skill.name}?`, description: 'Every agent stops using it. Runs that already loaded it keep what they read.', confirmLabel: 'Delete', danger: true, onConfirm: () => remove.mutate(skill.id) })}
                />
              </View>
            </View>
          ))}
          {update.isError ? <Note error>The skill could not be changed. {errorMessage(update.error)}</Note> : null}
          {remove.isError ? <Note error>The skill could not be deleted. {errorMessage(remove.error)}</Note> : null}
        </View>
      ) : null}
      <AddSkillDialog visible={adding} onClose={() => setAdding(false)} />
      {editing ? <EditSkillDialog skill={editing} onClose={() => setEditing(null)} /> : null}
      <ConfirmDialog request={confirm} onClose={() => setConfirm(null)} />
    </SettingsCard>
  )
}

const EMPTY: SkillDraft = { source: '', file: null, name: '', description: '', body: '' }

function AddSkillDialog({ visible, onClose }: { visible: boolean; onClose: () => void }) {
  const { add } = useSkillWrites()
  const [mode, setMode] = useState<SkillMode>('install')
  const [draft, setDraft] = useState<SkillDraft>(EMPTY)
  const [fileName, setFileName] = useState<string | null>(null)
  const [added, setAdded] = useState<string | null>(null)
  useEffect(() => {
    if (!visible) return
    setDraft(EMPTY)
    setFileName(null)
    setAdded(null)
    add.reset()
    // only when the dialog opens
  }, [visible])
  const set = (fields: Partial<SkillDraft>) => {
    add.reset()
    setDraft((current) => ({ ...current, ...fields }))
  }
  const ready = skillDraftReady(mode, draft)

  const pick = async () => {
    const result = await DocumentPicker.getDocumentAsync({ type: ['application/zip', 'application/x-zip-compressed'], copyToCacheDirectory: true })
    const asset = result.canceled ? undefined : result.assets[0]
    if (!asset) return
    setFileName(asset.name)
    set({ file: await uploadPart(asset.uri, asset.name) })
  }

  return (
    <Dialog
      visible={visible}
      title="Add skill"
      onClose={onClose}
      footer={
        added ? (
          <Button label="Done" onPress={onClose} />
        ) : (
          <>
            <Button label="Cancel" variant="outline" disabled={add.isPending} onPress={onClose} />
            <Button
              label={add.isPending ? 'Adding…' : mode === 'install' ? 'Install' : mode === 'upload' ? 'Upload' : 'Add skill'}
              loading={add.isPending}
              disabled={!ready}
              onPress={() => add.mutate({ mode, draft }, { onSuccess: (skill) => setAdded(skill.name) })}
            />
          </>
        )
      }
    >
      {added ? (
        <Note>Skill {added} added. Turn it on for the agents that need it.</Note>
      ) : (
        <>
          <Segmented
            label="How to add the skill"
            value={mode}
            options={MODES}
            onChange={(next) => {
              add.reset()
              setMode(next)
            }}
          />
          {mode === 'install' ? (
            <Input
              label="Address"
              autoCapitalize="none"
              autoCorrect={false}
              keyboardType="url"
              placeholder="https://skills.sh/owner/repo/skill"
              value={draft.source}
              onChangeText={(source) => set({ source })}
              description="A skills.sh page, a GitHub folder with SKILL.md (https://github.com/owner/repo/tree/main/skills/name), or owner/repo@name. Public repositories only; at most 100 files and 2 MiB."
            />
          ) : mode === 'upload' ? (
            <View className="gap-2">
              <View className="flex-row items-center gap-3">
                <Button label={fileName ? 'Choose another' : 'Choose .zip file'} variant="outline" size="sm" onPress={() => void pick()} />
                {fileName ? <Text numberOfLines={1} className="shrink text-[13px]">{fileName}</Text> : null}
              </View>
              <Note>SKILL.md at the top of the .zip or in one folder, with optional scripts/, references/ and assets/. At most 4 MiB, 100 files and 2 MiB unpacked; no links.</Note>
            </View>
          ) : (
            <SkillFields draft={draft} onChange={set} />
          )}
          {add.isError ? <Note error>{errorMessage(add.error)}</Note> : null}
        </>
      )}
    </Dialog>
  )
}

function SkillFields({ draft, onChange }: { draft: Pick<SkillDraft, 'name' | 'description' | 'body'>; onChange: (fields: Partial<SkillDraft>) => void }) {
  return (
    <>
      <Input label="Name" autoCapitalize="none" autoCorrect={false} maxLength={64} placeholder="weekly-report" value={draft.name} onChangeText={(name) => onChange({ name })} description="Lowercase letters, digits and single hyphens, at most 64 characters." />
      <Input label="Description" multiline maxLength={1024} value={draft.description} onChangeText={(description) => onChange({ description })} description="What the skill does and when to use it. The agent decides from this text when to read the skill." />
      <Input label="Instructions" multiline value={draft.body} onChangeText={(body) => onChange({ body })} description="Markdown: the steps, examples and rules. Orbit's own rules stay: the agent acts with the person's permissions and asks before changes." />
    </>
  )
}

function EditSkillDialog({ skill, onClose }: { skill: AgentSkill; onClose: () => void }) {
  const { client, workspaceId } = useOrbit()
  const full = useQuery({ queryKey: [...skillsKey(workspaceId), skill.id], queryFn: () => readAgentSkill(client, workspaceId, skill.id), gcTime: 0 })
  if (full.isPending) return null
  if (full.isError) {
    return (
      <Dialog visible title={`Edit ${skill.name}`} onClose={onClose} footer={<Button label="Close" variant="outline" onPress={onClose} />}>
        <Note error>The skill could not be loaded.</Note>
      </Dialog>
    )
  }
  return <EditSkillForm skill={full.data} onClose={onClose} />
}

function EditSkillForm({ skill, onClose }: { skill: AgentSkill; onClose: () => void }) {
  const { update } = useSkillWrites()
  const [draft, setDraft] = useState({ name: skill.name, description: skill.description, body: skill.body ?? '' })
  return (
    <Dialog
      visible
      title={`Edit ${skill.name}`}
      onClose={onClose}
      footer={
        <>
          <Button label="Cancel" variant="outline" disabled={update.isPending} onPress={onClose} />
          <Button label={update.isPending ? 'Saving…' : 'Save'} loading={update.isPending} onPress={() => update.mutate({ id: skill.id, change: draft }, { onSuccess: onClose })} />
        </>
      }
    >
      <SkillFields draft={draft} onChange={(fields) => setDraft((current) => ({ ...current, ...fields }))} />
      {skill.files.length ? (
        <View className="gap-1">
          <Text className="font-noto-medium text-[14px]">Files</Text>
          {skill.files.map((file) => <Text key={file.path} className="font-mono text-[12px] text-muted-foreground">{file.path} ({file.size} bytes)</Text>)}
          <Note>To change the files, delete the skill and add it again.</Note>
        </View>
      ) : null}
      {update.isError ? <Note error>{errorMessage(update.error)}</Note> : null}
    </Dialog>
  )
}
