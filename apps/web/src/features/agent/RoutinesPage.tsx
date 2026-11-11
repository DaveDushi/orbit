import { useState } from 'react'
import { Link } from 'react-router'
import { toast } from 'sonner'
import type { Routine } from '@orbit/api/scheduled'
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
import { useDeleteRoutine, useRoutines, useUpdateRoutine } from '@/features/scheduled/api'
import { problemDetail } from '@/features/scheduled/problem'
import { whenLabel } from '@/features/scheduled/times'
import { useWorkspace } from '@/features/workspaces/workspaceContext'
import { useAgentProfiles, useAgentStatus } from './api'

/** Settings → Agentic → Routines: the person's routines (every routine for owners and admins), to pause, edit or delete. */
export function RoutinesPage() {
  const { workspace } = useWorkspace()
  const status = useAgentStatus().data
  const name = status?.enabled ? status.name : 'the agent'
  const routines = useRoutines(workspace.id)
  const update = useUpdateRoutine(workspace.id)
  const remove = useDeleteRoutine(workspace.id)
  const [editing, setEditing] = useState<Routine | null>(null)
  const [deleting, setDeleting] = useState<Routine | null>(null)
  const items = routines.data ?? []
  const fail = (error: unknown) => toast.error(problemDetail(error, 'The routine could not be changed.'))

  return (
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
              </dl>
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
                {routine.can_stop ? (
                  <Button type="button" size="sm" variant="ghost" className="text-destructive" onClick={() => setDeleting(routine)}>
                    Delete
                  </Button>
                ) : null}
              </div>
            </li>
          ))}
        </ul>
      )}
      {editing ? <EditRoutineDialog routine={editing} onClose={() => setEditing(null)} /> : null}
      {deleting ? (
        <ConfirmDeleteModal
          title="Delete routine?"
          description={`“${deleting.name}” stops for good. Its earlier results stay in chat.`}
          onClose={() => setDeleting(null)}
          onConfirm={() => remove.mutate(deleting.id, { onSuccess: () => setDeleting(null), onError: fail })}
        />
      ) : null}
    </SettingsCard>
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

/** The routine's name, instruction, schedule, zone and target. Only its creator opens this. */
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
