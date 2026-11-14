import { useState } from 'react'
import { toast } from 'sonner'
import type { AgentSkill, AgentSkills } from '@orbit/api/agentSkills'
import { ApiProblem } from '@orbit/api/problem'
import { ConfirmDeleteModal } from '@/components/common/ConfirmDeleteModal'
import { Modal } from '@/components/common/Modal'
import { SettingsCard } from '@/components/common/SettingsCard'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import { DialogClose, DialogFooter } from '@/components/ui/dialog'
import { Field, FieldDescription, FieldLabel } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { Textarea } from '@/components/ui/textarea'
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group'
import { useCan } from '@/features/workspaces/permissions'
import { useWorkspace } from '@/features/workspaces/workspaceContext'
import { useAddAgentSkill, useAgentSkill, useAgentSkills, useDeleteAgentSkill, useUpdateAgentSkill, type AddSkill } from './skillsApi'

const SOURCE_LABELS: Record<AgentSkill['source'], string> = { manual: 'Written here', zip: 'Uploaded .zip', github: 'GitHub', skills_sh: 'skills.sh' }

const failure = (error: unknown, fallback: string) => (error instanceof ApiProblem ? error.detail : fallback)

/** Settings → Agentic → Skills: owners and admins add skills (SKILL.md) and turn each one on per agent. */
export function SkillsPage() {
  const canManage = useCan('workspace.update')
  if (!canManage) {
    return (
      <SettingsCard title="Skills" description="Skills teach the agents how to do kinds of tasks (the open SKILL.md format).">
        <p className="text-sm text-muted-foreground">Only owners and admins can see and manage the agents' skills.</p>
      </SettingsCard>
    )
  }
  return <ManageSkills />
}

function ManageSkills() {
  const { workspace } = useWorkspace()
  const skills = useAgentSkills(workspace.id)
  const update = useUpdateAgentSkill(workspace.id)
  const remove = useDeleteAgentSkill(workspace.id)
  const [adding, setAdding] = useState(false)
  const [editing, setEditing] = useState<AgentSkill | null>(null)
  const [deleting, setDeleting] = useState<AgentSkill | null>(null)
  const data = skills.data

  function setAgent(skill: AgentSkill, agent: string, on: boolean) {
    const agents = on ? [...skill.agents, agent] : skill.agents.filter((key) => key !== agent)
    update.mutate({ id: skill.id, change: { agents } }, { onError: (error) => toast.error(failure(error, 'The skill could not be changed.')) })
  }

  return (
    <SettingsCard
      title="Skills"
      description="Skills teach the agents how to do kinds of tasks (the open SKILL.md format). Turn each skill on for the agents that need it: an agent sees only the name and description of its skills, and reads a skill when a request needs it. A skill never gives an agent more permissions: it still acts as the person who asked."
      actions={data ? <Button size="sm" disabled={data.skills.length >= data.max_skills} onClick={() => setAdding(true)}>Add skill</Button> : null}
    >
      {skills.isPending ? null : skills.isError || !data ? (
        <p role="alert" className="text-sm text-destructive">The skills could not be loaded.</p>
      ) : (
        <>
          <ScriptsNote sandbox={data.sandbox} />
          <ul className="-mx-2 flex flex-col">
            {data.skills.map((skill) => (
              <li key={skill.id} data-slot="agent-skill" className="flex items-start gap-3 rounded-md px-2 py-3 not-last:border-b not-last:border-border">
                <div className="flex min-w-0 flex-1 flex-col gap-1.5">
                  <div className="flex flex-wrap items-center gap-2">
                    <span className="font-mono text-sm font-medium text-foreground">{skill.name}</span>
                    <Badge variant="outline">{SOURCE_LABELS[skill.source]}</Badge>
                    {skill.files.length ? <Badge variant="secondary">{skill.files.length} files</Badge> : null}
                    {skill.has_scripts ? <Badge variant={data.sandbox ? 'secondary' : 'outline'}>{data.sandbox ? 'Scripts' : 'Scripts cannot run'}</Badge> : null}
                  </div>
                  <p className="line-clamp-2 text-sm text-muted-foreground">{skill.description}</p>
                  {skill.source_url ? <p className="truncate text-xs text-muted-foreground">{skill.source_url}</p> : null}
                  <div className="flex flex-wrap gap-x-4 gap-y-1.5 pt-1" role="group" aria-label={`Agents that use ${skill.name}`}>
                    {data.agents.map((agent) => (
                      <label key={agent.key} className="flex items-center gap-2 text-sm text-foreground">
                        <Checkbox checked={skill.agents.includes(agent.key)} disabled={update.isPending} onCheckedChange={(on) => setAgent(skill, agent.key, on)} />
                        {agent.name}
                      </label>
                    ))}
                  </div>
                </div>
                <div className="flex shrink-0 gap-1">
                  <Button size="sm" variant="ghost" onClick={() => setEditing(skill)}>Edit</Button>
                  <Button size="sm" variant="ghost" className="text-destructive" onClick={() => setDeleting(skill)}>Delete</Button>
                </div>
              </li>
            ))}
            {data.skills.length === 0 ? <li className="px-2 py-3 text-sm text-muted-foreground">No skills yet. Install one from GitHub or skills.sh, upload a .zip, or write one.</li> : null}
          </ul>
        </>
      )}
      {adding ? <AddSkillDialog onClose={() => setAdding(false)} /> : null}
      {editing ? <EditSkillDialog skill={editing} onClose={() => setEditing(null)} /> : null}
      {deleting ? (
        <ConfirmDeleteModal
          title={`Delete ${deleting.name}?`}
          description="Every agent stops using it. Runs that already loaded it keep what they read."
          onClose={() => setDeleting(null)}
          onConfirm={() =>
            remove.mutate(deleting.id, {
              onSuccess: () => setDeleting(null),
              onError: (error) => toast.error(failure(error, 'The skill could not be deleted.')),
            })
          }
        />
      ) : null}
    </SettingsCard>
  )
}

function ScriptsNote({ sandbox }: { sandbox: AgentSkills['sandbox'] }) {
  if (sandbox) return <p className="mb-2 text-xs text-muted-foreground">Scripts in a skill run only in the agent's sandbox (no internet, no access to this server), for agents with the media tools.</p>
  if (sandbox === false) return <p role="note" className="mb-2 text-xs text-muted-foreground">This server has no agent sandbox, so scripts in skills are stored but cannot run. The agents still read the skills' instructions and files.</p>
  return <p role="note" className="mb-2 text-xs text-muted-foreground">The agent service is offline, so it is not known whether scripts in skills can run.</p>
}

type Mode = AddSkill['kind']

function AddSkillDialog({ onClose }: { onClose: () => void }) {
  const { workspace } = useWorkspace()
  const add = useAddAgentSkill(workspace.id)
  const [mode, setMode] = useState<Mode>('install')
  const [source, setSource] = useState('')
  const [file, setFile] = useState<File | null>(null)
  const [name, setName] = useState('')
  const [description, setDescription] = useState('')
  const [body, setBody] = useState('')
  const ready = mode === 'install' ? source.trim() !== '' : mode === 'upload' ? file !== null : name.trim() !== '' && description.trim() !== '' && body.trim() !== ''

  function submit(event: React.FormEvent) {
    event.preventDefault()
    if (!ready || add.isPending) return
    const input: AddSkill = mode === 'install' ? { kind: 'install', source } : mode === 'upload' ? { kind: 'upload', file: file as File } : { kind: 'write', name, description, body }
    add.mutate(input, {
      onSuccess: (skill) => {
        toast(`Skill ${skill.name} added. Turn it on for the agents that need it.`)
        onClose()
      },
    })
  }

  return (
    <Modal title="Add skill" onClose={onClose} className="sm:max-w-2xl">
      <form id="agent-skill-add" onSubmit={submit} className="flex max-h-[70vh] flex-col gap-5 overflow-y-auto pr-1">
        <ToggleGroup
          aria-label="How to add the skill"
          variant="outline"
          value={[mode]}
          onValueChange={(value: string[]) => {
            const next = value[0] as Mode | undefined
            if (next) {
              add.reset()
              setMode(next)
            }
          }}
        >
          <ToggleGroupItem value="install" className="px-3">GitHub or skills.sh</ToggleGroupItem>
          <ToggleGroupItem value="upload" className="px-3">Upload .zip</ToggleGroupItem>
          <ToggleGroupItem value="write" className="px-3">Write</ToggleGroupItem>
        </ToggleGroup>
        {mode === 'install' ? (
          <Field>
            <FieldLabel htmlFor="agent-skill-source">Address</FieldLabel>
            <Input id="agent-skill-source" placeholder="https://skills.sh/owner/repo/skill" value={source} onChange={(event) => { add.reset(); setSource(event.target.value) }} />
            <FieldDescription>A skills.sh page, a GitHub folder with SKILL.md (https://github.com/owner/repo/tree/main/skills/name), or owner/repo@name. Public repositories only; at most 100 files and 2 MiB.</FieldDescription>
          </Field>
        ) : mode === 'upload' ? (
          <Field>
            <FieldLabel htmlFor="agent-skill-zip">.zip file</FieldLabel>
            <Input id="agent-skill-zip" type="file" accept=".zip,application/zip" onChange={(event) => { add.reset(); setFile(event.target.files?.[0] ?? null) }} />
            <FieldDescription>SKILL.md at the top of the .zip or in one folder, with optional scripts/, references/ and assets/. At most 4 MiB, 100 files and 2 MiB unpacked; no links.</FieldDescription>
          </Field>
        ) : (
          <SkillFields name={name} description={description} body={body} onName={setName} onDescription={setDescription} onBody={setBody} />
        )}
        {add.isError ? <p role="alert" className="text-sm text-destructive">{failure(add.error, 'The skill could not be added.')}</p> : null}
      </form>
      <DialogFooter>
        <DialogClose render={<Button type="button" variant="outline" />}>Cancel</DialogClose>
        <Button type="submit" form="agent-skill-add" disabled={!ready || add.isPending}>{add.isPending ? 'Adding…' : mode === 'install' ? 'Install' : mode === 'upload' ? 'Upload' : 'Add skill'}</Button>
      </DialogFooter>
    </Modal>
  )
}

function SkillFields(props: { name: string; description: string; body: string; onName: (value: string) => void; onDescription: (value: string) => void; onBody: (value: string) => void }) {
  return (
    <>
      <Field>
        <FieldLabel htmlFor="agent-skill-name">Name</FieldLabel>
        <Input id="agent-skill-name" className="font-mono" maxLength={64} placeholder="weekly-report" value={props.name} onChange={(event) => props.onName(event.target.value)} />
        <FieldDescription>Lowercase letters, digits and single hyphens, at most 64 characters.</FieldDescription>
      </Field>
      <Field>
        <FieldLabel htmlFor="agent-skill-description">Description</FieldLabel>
        <Textarea id="agent-skill-description" className="min-h-16" maxLength={1024} value={props.description} onChange={(event) => props.onDescription(event.target.value)} />
        <FieldDescription>What the skill does and when to use it. The agent decides from this text when to read the skill.</FieldDescription>
      </Field>
      <Field>
        <FieldLabel htmlFor="agent-skill-body">Instructions</FieldLabel>
        <Textarea id="agent-skill-body" className="min-h-48 font-mono text-xs" value={props.body} onChange={(event) => props.onBody(event.target.value)} />
        <FieldDescription>Markdown: the steps, examples and rules. Orbit's own rules stay: the agent acts with the person's permissions and asks before changes.</FieldDescription>
      </Field>
    </>
  )
}

function EditSkillDialog({ skill, onClose }: { skill: AgentSkill; onClose: () => void }) {
  const { workspace } = useWorkspace()
  const full = useAgentSkill(workspace.id, skill.id)
  if (full.isPending) return null
  if (full.isError) {
    return (
      <Modal title={`Edit ${skill.name}`} onClose={onClose}>
        <p role="alert" className="text-sm text-destructive">The skill could not be loaded.</p>
      </Modal>
    )
  }
  return <EditSkillForm skill={full.data} onClose={onClose} />
}

function EditSkillForm({ skill, onClose }: { skill: AgentSkill; onClose: () => void }) {
  const { workspace } = useWorkspace()
  const update = useUpdateAgentSkill(workspace.id)
  const [name, setName] = useState(skill.name)
  const [description, setDescription] = useState(skill.description)
  const [body, setBody] = useState(skill.body ?? '')

  function submit(event: React.FormEvent) {
    event.preventDefault()
    if (update.isPending) return
    update.mutate({ id: skill.id, change: { name, description, body } }, { onSuccess: onClose })
  }

  return (
    <Modal title={`Edit ${skill.name}`} onClose={onClose} className="sm:max-w-2xl">
      <form id="agent-skill-edit" onSubmit={submit} className="flex max-h-[70vh] flex-col gap-5 overflow-y-auto pr-1">
        <SkillFields name={name} description={description} body={body} onName={setName} onDescription={setDescription} onBody={setBody} />
        {skill.files.length ? (
          <div className="flex flex-col gap-1">
            <span className="text-sm font-medium text-foreground">Files</span>
            <ul className="font-mono text-xs text-muted-foreground">
              {skill.files.map((file) => <li key={file.path}>{file.path} ({file.size} bytes)</li>)}
            </ul>
            <p className="text-xs text-muted-foreground">To change the files, delete the skill and add it again.</p>
          </div>
        ) : null}
        {update.isError ? <p role="alert" className="text-sm text-destructive">{failure(update.error, 'The skill could not be saved.')}</p> : null}
      </form>
      <DialogFooter>
        <DialogClose render={<Button type="button" variant="outline" />}>Cancel</DialogClose>
        <Button type="submit" form="agent-skill-edit" disabled={update.isPending}>{update.isPending ? 'Saving…' : 'Save'}</Button>
      </DialogFooter>
    </Modal>
  )
}
