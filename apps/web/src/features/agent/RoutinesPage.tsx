import { useState } from 'react'
import { Link } from 'react-router'
import { toast } from 'sonner'
import type { Routine, RoutineRun, Script } from '@orbit/api/scheduled'
import { ConfirmDeleteModal } from '@/components/common/ConfirmDeleteModal'
import { Modal } from '@/components/common/Modal'
import { SettingsCard } from '@/components/common/SettingsCard'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { DialogClose, DialogFooter } from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { Textarea } from '@/components/ui/textarea'
import { useChatContext } from '@/features/chat/api/chatContext'
import { useConversations } from '@/features/chat/api/queries'
import { conversationPath } from '@/features/chat/chatRoutes'
import { conversationTitle, useChatPeople } from '@/features/chat/components/messages/people'
import { TimeZonePicker } from '@/features/profile/components/TimeZonePicker'
import { useDeleteRoutine, useDeleteScript, useRoutineRuns, useRoutines, useScripts, useTestRoutine, useUpdateRoutine, useUpdateScript } from '@/features/scheduled/api'
import { problemDetail } from '@/features/scheduled/problem'
import { whenLabel } from '@/features/scheduled/times'
import { useWorkspace } from '@/features/workspaces/workspaceContext'
import { useAgentProfiles, useAgentStatus } from './api'

/**
 * Settings → Agentic → Routines: the person's routines (every routine for owners and admins), to pause, edit, test or
 * delete, with each routine's run history; and the saved scripts that routines can run.
 */
export function RoutinesPage() {
  const { workspace } = useWorkspace()
  const status = useAgentStatus().data
  const name = status?.enabled ? status.name : 'the agent'
  const routines = useRoutines(workspace.id)
  const update = useUpdateRoutine(workspace.id)
  const remove = useDeleteRoutine(workspace.id)
  const test = useTestRoutine(workspace.id)
  const [editing, setEditing] = useState<Routine | null>(null)
  const [deleting, setDeleting] = useState<Routine | null>(null)
  const [approving, setApproving] = useState<Routine | null>(null)
  const [history, setHistory] = useState<string | null>(null)
  const items = routines.data ?? []
  const fail = (error: unknown) => toast.error(problemDetail(error, 'The routine could not be changed.'))
  const runTest = (routine: Routine) =>
    test.mutate(routine.id, {
      onSuccess: () => {
        setHistory(routine.id)
        toast.success(`Test run started: the result comes in your DM with ${routine.agent_name}, marked “Test run”.`)
      },
      onError: (error) => toast.error(problemDetail(error, 'The test run could not start.')),
    })

  return (
    <div className="flex flex-col gap-6">
      <SettingsCard
        title="Routines"
        description={`Recurring jobs that ${name} does for you on a schedule and posts where you chose. Ask ${name} in chat to make one, for example “every weekday at 9, summarize yesterday’s #support messages here”. Each run acts with your permissions only.`}
      >
        {routines.isPending ? null : items.length === 0 ? (
          <p className="text-sm text-muted-foreground">No routines yet.</p>
        ) : (
          <ul className="-mx-2 flex flex-col">
            {items.map((routine) => (
              <li key={routine.id} data-slot="routine" className="flex flex-col gap-1.5 rounded-md px-2 py-3 not-last:border-b not-last:border-border">
                <div className="flex flex-wrap items-center gap-2">
                  <span className="text-sm font-medium text-foreground">{routine.name}</span>
                  <Badge variant={routine.status === 'active' ? 'secondary' : 'outline'}>{routine.status === 'active' ? 'Active' : 'Paused'}</Badge>
                  {routine.can_edit ? null : <span className="text-xs text-muted-foreground">by {routine.creator_name}</span>}
                </div>
                <p className="line-clamp-2 text-sm text-muted-foreground">{routine.prompt}</p>
                <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-0.5 text-xs text-muted-foreground">
                  <dt>Agent</dt>
                  <dd className="text-foreground">{routine.agent_name}</dd>
                  <dt>Schedule</dt>
                  <dd>
                    <code className="font-mono text-foreground">{routine.schedule}</code> ({routine.time_zone})
                  </dd>
                  <dt>Posts in</dt>
                  <dd className="text-foreground">{routine.conversation_kind === 'dm' ? 'Direct message' : routine.conversation_name ? `#${routine.conversation_name}` : 'Deleted conversation'}</dd>
                  <dt>Next run</dt>
                  <dd className="text-foreground">{routine.next_run_at ? whenLabel(routine.next_run_at) : '—'}</dd>
                  <dt>Last run</dt>
                  <dd>
                    <LastRun routine={routine} />
                  </dd>
                  {routine.script_name ? (
                    <>
                      <dt>Script</dt>
                      <dd className="text-foreground">
                        {routine.script_name}{' '}
                        {routine.script_approved ? (
                          <span className="text-muted-foreground">· its writes are approved</span>
                        ) : (
                          <span className="text-destructive">· changed or not approved: its runs make no writes</span>
                        )}
                      </dd>
                    </>
                  ) : null}
                </dl>
                {routine.script_id && !routine.script_approved && routine.can_edit ? (
                  <div role="status" className="flex flex-wrap items-center gap-2 rounded-md border border-destructive/40 px-3 py-2 text-xs text-destructive">
                    <span className="flex-1">The script “{routine.script_name}” changed since you approved it. Until you approve this version, its runs make no writes and list what they would do.</span>
                    <Button type="button" size="sm" variant="outline" onClick={() => setApproving(routine)}>
                      Review and approve
                    </Button>
                  </div>
                ) : null}
                {routine.paused_reason ? (
                  <p role="status" className="text-xs text-destructive">
                    {routine.paused_reason}
                  </p>
                ) : null}
                <div className="flex flex-wrap gap-1">
                  {routine.status === 'active' ? (
                    routine.can_stop ? (
                      <Button type="button" size="sm" variant="ghost" onClick={() => update.mutate({ id: routine.id, change: { status: 'paused' } }, { onError: fail })}>
                        Pause
                      </Button>
                    ) : null
                  ) : routine.can_edit ? (
                    <Button type="button" size="sm" variant="ghost" onClick={() => update.mutate({ id: routine.id, change: { status: 'active' } }, { onError: fail })}>
                      Resume
                    </Button>
                  ) : null}
                  {routine.can_edit ? (
                    <Button type="button" size="sm" variant="ghost" onClick={() => setEditing(routine)}>
                      Edit
                    </Button>
                  ) : null}
                  {routine.can_edit ? (
                    <Button type="button" size="sm" variant="ghost" disabled={test.isPending} onClick={() => runTest(routine)}>
                      Test run
                    </Button>
                  ) : null}
                  <Button type="button" size="sm" variant="ghost" aria-expanded={history === routine.id} onClick={() => setHistory(history === routine.id ? null : routine.id)}>
                    History
                  </Button>
                  {routine.can_stop ? (
                    <Button type="button" size="sm" variant="ghost" className="text-destructive" onClick={() => setDeleting(routine)}>
                      Delete
                    </Button>
                  ) : null}
                </div>
                {history === routine.id ? <RunHistory routine={routine} /> : null}
              </li>
            ))}
          </ul>
        )}
        {editing ? <EditRoutineDialog routine={editing} onClose={() => setEditing(null)} /> : null}
        {approving ? <ApproveScriptDialog routine={approving} onClose={() => setApproving(null)} /> : null}
        {deleting ? (
          <ConfirmDeleteModal
            title="Delete routine?"
            description={`“${deleting.name}” stops for good. Its earlier results stay in chat.`}
            onClose={() => setDeleting(null)}
            onConfirm={() => remove.mutate(deleting.id, { onSuccess: () => setDeleting(null), onError: fail })}
          />
        ) : null}
      </SettingsCard>
      <ScriptsCard name={name} />
    </div>
  )
}

const RUN_STATUS: Record<RoutineRun['status'], { label: string; variant: 'secondary' | 'outline' | 'destructive' }> = {
  started: { label: 'Running', variant: 'outline' },
  done: { label: 'Done', variant: 'secondary' },
  failed: { label: 'Failed', variant: 'destructive' },
  skipped: { label: 'Skipped', variant: 'outline' },
}

/** The routine's last 20 runs: time, status, a summary of the result, the error and the tool calls. */
function RunHistory({ routine }: { routine: Routine }) {
  const { workspace } = useWorkspace()
  const runs = useRoutineRuns(workspace.id, routine.id, true)
  if (runs.isPending) return null
  if (runs.isError) return <p role="alert" className="text-xs text-destructive">The run history could not be loaded.</p>
  if (runs.data.length === 0) return <p className="text-xs text-muted-foreground">No runs yet.</p>
  return (
    <ul data-slot="run-history" aria-label={`Runs of ${routine.name}`} className="flex flex-col gap-2 rounded-md bg-muted/40 p-2">
      {runs.data.map((run) => (
        <RunItem key={run.id} run={run} />
      ))}
    </ul>
  )
}

function RunItem({ run }: { run: RoutineRun }) {
  const [calls, setCalls] = useState(false)
  const status = RUN_STATUS[run.status]
  return (
    <li data-slot="run" className="flex flex-col gap-1 text-xs">
      <div className="flex flex-wrap items-center gap-2">
        <span className="text-foreground">{whenLabel(run.started_at)}</span>
        {run.test ? <Badge variant="outline">Test run</Badge> : null}
        <Badge variant={status.variant}>{status.label}</Badge>
        {run.script_name ? <span className="text-muted-foreground">script “{run.script_name}”</span> : null}
        {run.writes === 'dry' && !run.test ? <span className="text-destructive">no writes (script not approved)</span> : null}
        {run.message_id && run.conversation_id ? (
          <Link to={conversationPath(run.conversation_id, { m: run.message_id })} className="text-foreground underline underline-offset-2">
            Open
          </Link>
        ) : null}
      </div>
      {run.output ? <p className="line-clamp-4 whitespace-pre-wrap text-muted-foreground">{run.output}</p> : null}
      {run.error ? <p className="text-destructive">{run.error}</p> : null}
      {run.tool_calls.length > 0 ? (
        <div className="flex flex-col gap-1">
          <button type="button" aria-expanded={calls} className="self-start text-muted-foreground underline underline-offset-2" onClick={() => setCalls(!calls)}>
            {calls ? 'Hide' : 'Show'} {run.tool_calls.length} tool {run.tool_calls.length === 1 ? 'call' : 'calls'}
          </button>
          {calls ? (
            <ol className="flex flex-col gap-0.5 font-mono">
              {run.tool_calls.map((call, index) => (
                <li key={index} className="break-all">
                  <span className="text-foreground">{call.name}</span> <span className="text-muted-foreground">{call.args}</span>{' '}
                  <span className={call.status === 'error' ? 'text-destructive' : call.status === 'dry_run' ? 'text-primary' : 'text-muted-foreground'}>
                    {call.status === 'dry_run' ? `would do: ${call.would_do ?? call.name}` : call.status}
                  </span>
                </li>
              ))}
            </ol>
          ) : null}
        </div>
      ) : null}
    </li>
  )
}

/** The routine's script as it is now, to approve its writes for exactly this code. Only the creator opens this. */
function ApproveScriptDialog({ routine, onClose }: { routine: Routine; onClose: () => void }) {
  const { workspace } = useWorkspace()
  const script = useScripts(workspace.id).data?.find((item) => item.id === routine.script_id)
  const update = useUpdateRoutine(workspace.id)
  return (
    <Modal title={`Approve the script of “${routine.name}”`} onClose={onClose} className="sm:max-w-2xl">
      <div className="flex flex-col gap-3 text-sm">
        <p className="text-muted-foreground">
          Each run of the routine runs this code as you. After your approval its writes need no new yes. A later change of the script turns them off again until you approve it.
        </p>
        {script ? (
          <>
            <p className="text-xs text-muted-foreground">
              “{script.name}”, version <code className="font-mono">{script.code_hash.slice(0, 12)}</code>
            </p>
            <pre className="max-h-80 overflow-auto rounded-md bg-muted p-3 font-mono text-xs whitespace-pre-wrap">{script.code}</pre>
          </>
        ) : (
          <p className="text-muted-foreground">Loading the script…</p>
        )}
        {update.isError ? (
          <p role="alert" className="text-destructive">
            {problemDetail(update.error, 'The script could not be approved.')}
          </p>
        ) : null}
      </div>
      <DialogFooter>
        <DialogClose render={<Button type="button" variant="outline" />}>Cancel</DialogClose>
        <Button
          type="button"
          disabled={!script || update.isPending}
          onClick={() => script && update.mutate({ id: routine.id, change: { approve_script_hash: script.code_hash } }, { onSuccess: onClose })}
        >
          Approve this version
        </Button>
      </DialogFooter>
    </Modal>
  )
}

/** The saved scripts: the person's own (every script for owners and admins), to view, edit or delete. */
function ScriptsCard({ name }: { name: string }) {
  const { workspace } = useWorkspace()
  const scripts = useScripts(workspace.id)
  const remove = useDeleteScript(workspace.id)
  const [open, setOpen] = useState<Script | null>(null)
  const [deleting, setDeleting] = useState<Script | null>(null)
  const items = scripts.data ?? []
  return (
    <SettingsCard
      title="Scripts"
      description={`Scripts that ${name} saved for you. A script calls ${name}’s tools, as you, in a sandbox with no internet and no files. A routine can run one on each run; a changed script makes no writes until you approve it again on its routine.`}
    >
      {scripts.isPending ? null : items.length === 0 ? (
        <p className="text-sm text-muted-foreground">No scripts yet. Ask {name} to save one, for example “save this as a script”.</p>
      ) : (
        <ul className="-mx-2 flex flex-col">
          {items.map((script) => (
            <li key={script.id} data-slot="script" className="flex flex-col gap-1 rounded-md px-2 py-3 not-last:border-b not-last:border-border">
              <div className="flex flex-wrap items-center gap-2">
                <span className="text-sm font-medium text-foreground">{script.name}</span>
                <code className="font-mono text-xs text-muted-foreground">{script.code_hash.slice(0, 12)}</code>
                {script.can_edit ? null : <span className="text-xs text-muted-foreground">by {script.owner_name}</span>}
              </div>
              {script.description ? <p className="text-sm text-muted-foreground">{script.description}</p> : null}
              <p className="text-xs text-muted-foreground">
                {script.routines.length === 0
                  ? 'No routine runs it.'
                  : `Runs in: ${script.routines.map((routine) => `${routine.name}${routine.approved ? '' : ' (not approved)'}`).join(', ')}`}
              </p>
              <div className="flex flex-wrap gap-1">
                <Button type="button" size="sm" variant="ghost" onClick={() => setOpen(script)}>
                  {script.can_edit ? 'Edit' : 'View'}
                </Button>
                {script.can_delete ? (
                  <Button type="button" size="sm" variant="ghost" className="text-destructive" onClick={() => setDeleting(script)}>
                    Delete
                  </Button>
                ) : null}
              </div>
            </li>
          ))}
        </ul>
      )}
      {open ? <ScriptDialog script={open} onClose={() => setOpen(null)} /> : null}
      {deleting ? (
        <ConfirmDeleteModal
          title="Delete script?"
          description={`“${deleting.name}” is deleted for good. Routines that run it go back to their instruction.`}
          onClose={() => setDeleting(null)}
          onConfirm={() =>
            remove.mutate(deleting.id, { onSuccess: () => setDeleting(null), onError: (error) => toast.error(problemDetail(error, 'The script could not be deleted.')) })
          }
        />
      ) : null}
    </SettingsCard>
  )
}

/** A script's name, description and code: editable by its owner, read-only for owners and admins. */
function ScriptDialog({ script, onClose }: { script: Script; onClose: () => void }) {
  const { workspace } = useWorkspace()
  const update = useUpdateScript(workspace.id)
  const [name, setName] = useState(script.name)
  const [description, setDescription] = useState(script.description)
  const [code, setCode] = useState(script.code)
  const readOnly = !script.can_edit
  const changedCode = code.trimEnd() !== script.code
  return (
    <Modal title={readOnly ? script.name : 'Edit script'} onClose={onClose} className="sm:max-w-2xl">
      <div className="flex flex-col gap-3">
        <label className="flex flex-col gap-1.5 text-sm">
          <span className="text-muted-foreground">Name</span>
          <Input value={name} maxLength={80} readOnly={readOnly} onChange={(event) => setName(event.target.value)} />
        </label>
        <label className="flex flex-col gap-1.5 text-sm">
          <span className="text-muted-foreground">Description</span>
          <Input value={description} maxLength={500} readOnly={readOnly} onChange={(event) => setDescription(event.target.value)} />
        </label>
        <label className="flex flex-col gap-1.5 text-sm">
          <span className="text-muted-foreground">Code (JavaScript; calls tools as await tools.name(args))</span>
          <Textarea value={code} maxLength={20000} readOnly={readOnly} spellCheck={false} className="min-h-64 font-mono text-xs" onChange={(event) => setCode(event.target.value)} />
        </label>
        {changedCode && script.routines.length > 0 ? (
          <p role="status" className="text-xs text-destructive">
            New code: {script.routines.map((routine) => routine.name).join(', ')} will make no writes until you approve the new version on each routine.
          </p>
        ) : null}
        {update.isError ? (
          <p role="alert" className="text-sm text-destructive">
            {problemDetail(update.error, 'The script could not be saved.')}
          </p>
        ) : null}
      </div>
      <DialogFooter>
        <DialogClose render={<Button type="button" variant="outline" />}>{readOnly ? 'Close' : 'Cancel'}</DialogClose>
        {readOnly ? null : (
          <Button
            type="button"
            disabled={update.isPending || name.trim() === '' || code.trim() === ''}
            onClick={() => update.mutate({ id: script.id, change: { name, description, ...(changedCode ? { code } : {}) } }, { onSuccess: onClose })}
          >
            Save
          </Button>
        )}
      </DialogFooter>
    </Modal>
  )
}

function LastRun({ routine }: { routine: Routine }) {
  if (!routine.last_run_at) return <span>Not yet</span>
  const when = whenLabel(routine.last_run_at)
  if (routine.last_run_status === 'skipped') return <span className="text-destructive">{when} · {routine.last_error ?? 'Skipped'}</span>
  if (routine.last_run_message_id && routine.conversation_id) {
    return (
      <Link to={conversationPath(routine.conversation_id, { m: routine.last_run_message_id })} className="text-foreground underline underline-offset-2">
        {when} · see the result
      </Link>
    )
  }
  return <span className="text-foreground">{when} · started</span>
}

/** The script choice "none" (Select values cannot be empty). */
const NO_SCRIPT = 'none'

/** The routine's name, instruction, schedule, zone, target and script. Only its creator opens this. */
function EditRoutineDialog({ routine, onClose }: { routine: Routine; onClose: () => void }) {
  const { workspace } = useWorkspace()
  const { currentUserId } = useChatContext()
  const people = useChatPeople()
  const status = useAgentStatus().data
  const profiles = useAgentProfiles(workspace.id).data
  const [agentId, setAgentId] = useState(routine.agent_user_id)
  const defaultId = status?.enabled ? status.bot_user_id : null
  const agents = [
    ...(defaultId && status?.enabled ? [{ id: defaultId, name: status.name }] : []),
    ...(profiles?.agents ?? []).map((agent) => ({ id: agent.user_id, name: agent.name })),
  ]
  if (!agents.some((agent) => agent.id === routine.agent_user_id)) agents.push({ id: routine.agent_user_id, name: routine.agent_name })
  const conversations = (useConversations().data ?? []).filter(
    (conversation) => conversation.isMember && !conversation.archived && (conversation.kind !== 'dm' || conversation.memberIds.includes(agentId)),
  )
  const update = useUpdateRoutine(workspace.id)
  const [name, setName] = useState(routine.name)
  const [prompt, setPrompt] = useState(routine.prompt)
  const [schedule, setSchedule] = useState(routine.schedule)
  const [timeZone, setTimeZone] = useState(routine.time_zone)
  const [target, setTarget] = useState(routine.conversation_id ?? '')
  // Only the creator's own scripts can run in their routine.
  const scripts = (useScripts(workspace.id).data ?? []).filter((script) => script.owner_id === routine.creator_id)
  const [scriptId, setScriptId] = useState(routine.script_id ?? NO_SCRIPT)
  const script = scripts.find((item) => item.id === scriptId)
  const newScript = scriptId !== (routine.script_id ?? NO_SCRIPT)
  const title = (id: string) => {
    const conversation = conversations.find((item) => item.id === id)
    if (!conversation) return routine.conversation_name ? `#${routine.conversation_name}` : 'Pick a conversation'
    return conversation.kind === 'dm' ? conversationTitle(conversation, people, currentUserId) : `#${conversation.name}`
  }
  const save = () =>
    update.mutate(
      {
        id: routine.id,
        change: {
          name,
          prompt,
          schedule,
          time_zone: timeZone,
          ...(target && target !== routine.conversation_id ? { conversation_id: target } : {}),
          ...(agentId !== routine.agent_user_id ? { agent_user_id: agentId } : {}),
          // Picking a script here, with its code in view, is the creator's yes to that exact version.
          ...(newScript ? { script_id: scriptId === NO_SCRIPT ? '' : scriptId, ...(script ? { approve_script_hash: script.code_hash } : {}) } : {}),
        },
      },
      { onSuccess: onClose },
    )

  return (
    <Modal title="Edit routine" onClose={onClose} className="sm:max-w-lg">
      <div className="flex flex-col gap-3">
        <label className="flex flex-col gap-1.5 text-sm">
          <span className="text-muted-foreground">Name</span>
          <Input value={name} maxLength={80} onChange={(event) => setName(event.target.value)} />
        </label>
        <div className="flex flex-col gap-1.5 text-sm">
          <span className="text-muted-foreground">Agent</span>
          <Select value={agentId} onValueChange={(value) => setAgentId(String(value ?? routine.agent_user_id))}>
            <SelectTrigger aria-label="Agent">
              <SelectValue>{agents.find((agent) => agent.id === agentId)?.name ?? routine.agent_name}</SelectValue>
            </SelectTrigger>
            <SelectContent>
              {agents.map((agent) => (
                <SelectItem key={agent.id} value={agent.id}>
                  {agent.name}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <span className="text-xs text-muted-foreground">It runs the routine as you. Another agent must work in the channel where the routine posts.</span>
        </div>
        <label className="flex flex-col gap-1.5 text-sm">
          <span className="text-muted-foreground">Instruction</span>
          <Textarea value={prompt} maxLength={4000} className="min-h-24" onChange={(event) => setPrompt(event.target.value)} />
        </label>
        <label className="flex flex-col gap-1.5 text-sm">
          <span className="text-muted-foreground">Schedule (cron: minute hour day month weekday)</span>
          <Input value={schedule} className="font-mono" onChange={(event) => setSchedule(event.target.value)} />
          <span className="text-xs text-muted-foreground">
            <code>0 9 * * 1-5</code> is every weekday at 9:00, <code>30 8 * * 1</code> every Monday at 8:30. At most every 15 minutes.
          </span>
        </label>
        <div className="flex flex-col gap-1.5 text-sm">
          <span className="text-muted-foreground">Time zone</span>
          <TimeZonePicker id="routine-time-zone" value={timeZone} onChange={setTimeZone} />
        </div>
        <div className="flex flex-col gap-1.5 text-sm">
          <span className="text-muted-foreground">Posts in</span>
          <Select value={target} onValueChange={(value) => setTarget(String(value ?? ''))}>
            <SelectTrigger aria-label="Posts in">
              <SelectValue>{title(target)}</SelectValue>
            </SelectTrigger>
            <SelectContent>
              {conversations.map((conversation) => (
                <SelectItem key={conversation.id} value={conversation.id}>
                  {title(conversation.id)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
        <div className="flex flex-col gap-1.5 text-sm">
          <span className="text-muted-foreground">Script</span>
          <Select value={scriptId} onValueChange={(value) => setScriptId(String(value ?? NO_SCRIPT))}>
            <SelectTrigger aria-label="Script">
              <SelectValue>{script?.name ?? (scriptId === NO_SCRIPT ? 'None: only the instruction' : routine.script_name)}</SelectValue>
            </SelectTrigger>
            <SelectContent>
              <SelectItem value={NO_SCRIPT}>None: only the instruction</SelectItem>
              {scripts.map((item) => (
                <SelectItem key={item.id} value={item.id}>
                  {item.name}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          {newScript && script ? (
            <>
              <pre className="max-h-48 overflow-auto rounded-md bg-muted p-2 font-mono text-xs whitespace-pre-wrap">{script.code}</pre>
              <span className="text-xs text-muted-foreground">Saving approves this version: its writes run without a new yes. A later change of the script needs a new yes.</span>
            </>
          ) : null}
        </div>
        {update.isError ? (
          <p role="alert" className="text-sm text-destructive">
            {problemDetail(update.error, 'The routine could not be saved.')}
          </p>
        ) : null}
      </div>
      <DialogFooter>
        <DialogClose render={<Button type="button" variant="outline" />}>Cancel</DialogClose>
        <Button type="button" disabled={update.isPending || name.trim() === '' || prompt.trim() === '' || schedule.trim() === ''} onClick={save}>
          Save
        </Button>
      </DialogFooter>
    </Modal>
  )
}
