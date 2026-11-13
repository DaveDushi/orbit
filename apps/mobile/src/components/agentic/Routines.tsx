import { router } from 'expo-router'
import { useState } from 'react'
import { ScrollView, View } from 'react-native'
import type { Routine, RoutineRun, Script } from '@orbit/api/scheduled'

import { ConfirmDialog, type ConfirmRequest } from '../settings/Dialog'
import { SelectField } from '../settings/Fields'
import { Badge, type BadgeTone, Note, OptionHint, SettingsCard } from '../settings/Shell'
import { FormSheet } from '../chat/Sheet'
import { NO_SCRIPT, problemDetail, routineActions, routineChange, routineFormOf, routineFormValid, routineTarget, routineTargets, timeZoneChoices, whenLabel, type RoutineForm } from '../../data/agentic'
import { conversationTitle } from '../../data/chat'
import { useAgentProfiles, useAgentStatus, useDeleteRoutine, useDeleteScript, useRoutineRuns, useRoutines, useScripts, useTestRoutine, useUpdateRoutine, useUpdateScript } from '../../hooks/agent'
import { useOrbit } from '../../orbit-context'
import { useConversations, usePeople } from '../../queries'
import { Button, Input, Text } from '../../ui'

/**
 * The web's Agentic → Routines: the person's routines (every routine for owners and admins), to pause, edit, test or
 * delete, with each routine's run history; and the saved scripts that routines can run. What each person may do comes
 * from the server (`can_edit`, `can_stop`, `can_delete`).
 */
export function RoutinesSection() {
  const status = useAgentStatus().data
  const name = status?.enabled ? status.name : 'the agent'
  const routines = useRoutines()
  const update = useUpdateRoutine()
  const remove = useDeleteRoutine()
  const test = useTestRoutine()
  const [editing, setEditing] = useState<Routine | null>(null)
  const [approving, setApproving] = useState<Routine | null>(null)
  const [history, setHistory] = useState<string | null>(null)
  const [confirm, setConfirm] = useState<ConfirmRequest | null>(null)
  const [notice, setNotice] = useState<{ text: string; error: boolean } | null>(null)
  const items = routines.data ?? []
  const fail = (error: unknown) => setNotice({ text: problemDetail(error, 'The routine could not be changed.'), error: true })
  const change = (routine: Routine, status: 'active' | 'paused') => {
    setNotice(null)
    update.mutate({ id: routine.id, change: { status } }, { onError: fail })
  }
  const runTest = (routine: Routine) => {
    setNotice(null)
    test.mutate(routine.id, {
      onSuccess: () => {
        setHistory(routine.id)
        setNotice({ text: `Test run started: the result comes in your DM with ${routine.agent_name}, marked “Test run”.`, error: false })
      },
      onError: (error) => setNotice({ text: problemDetail(error, 'The test run could not start.'), error: true }),
    })
  }

  return (
    <>
      <SettingsCard
        title="Routines"
        description={`Recurring jobs that ${name} does for you on a schedule and posts where you chose. Ask ${name} in chat to make one, for example “every weekday at 9, summarize yesterday’s #support messages here”. Each run acts with your permissions only.`}
        flush
      >
        {routines.isPending ? <View className="p-4"><Note loading>Loading routines…</Note></View> : null}
        {routines.isError ? (
          <View className="p-4 gap-2 items-start">
            <Note error>The routines could not be loaded.</Note>
            <Button label="Retry" variant="ghost" onPress={() => void routines.refetch()} />
          </View>
        ) : null}
        {routines.isSuccess && items.length === 0 ? <View className="p-4"><Note>No routines yet.</Note></View> : null}
        {notice ? <View className="px-4 pt-3"><Note error={notice.error}>{notice.text}</Note></View> : null}
        {items.map((routine, index) => {
          const can = routineActions(routine)
          return (
            <View key={routine.id} className={`px-4 py-3 gap-1.5 border-border ${index === 0 ? '' : 'border-t'}`}>
              <View className="flex-row flex-wrap items-center gap-1.5">
                <Text className="font-noto-medium text-[14px]">{routine.name}</Text>
                <Badge label={routine.status === 'active' ? 'Active' : 'Paused'} tone={routine.status === 'active' ? 'secondary' : 'outline'} />
                {routine.can_edit ? null : <Text className="text-[12px] text-muted-foreground">by {routine.creator_name}</Text>}
              </View>
              <Text numberOfLines={2} className="text-[13px] text-muted-foreground">{routine.prompt}</Text>
              <Detail label="Agent" value={routine.agent_name} />
              <Detail label="Schedule" value={`${routine.schedule} (${routine.time_zone})`} mono />
              <Detail label="Posts in" value={routineTarget(routine)} />
              <Detail label="Next run" value={routine.next_run_at ? whenLabel(routine.next_run_at) : '—'} />
              <LastRun routine={routine} />
              {routine.script_name ? <Detail label="Script" value={`${routine.script_name} · ${routine.script_approved ? 'its writes are approved' : 'changed or not approved: its runs make no writes'}`} error={!routine.script_approved} /> : null}
              {routine.script_id && !routine.script_approved && routine.can_edit ? (
                <View className="gap-2 rounded-md border border-destructive/40 px-3 py-2">
                  <Text className="text-[12px] text-destructive">The script “{routine.script_name}” changed since you approved it. Until you approve this version, its runs make no writes and list what they would do.</Text>
                  <View className="items-start"><Button label="Review and approve" variant="outline" size="sm" onPress={() => setApproving(routine)} /></View>
                </View>
              ) : null}
              {routine.paused_reason ? <Text className="text-[12px] text-destructive">{routine.paused_reason}</Text> : null}
              <View className="flex-row flex-wrap -ml-2">
                {can.pause ? <Button label="Pause" variant="ghost" size="sm" onPress={() => change(routine, 'paused')} /> : null}
                {can.resume ? <Button label="Resume" variant="ghost" size="sm" onPress={() => change(routine, 'active')} /> : null}
                {can.edit ? <Button label="Edit" variant="ghost" size="sm" onPress={() => setEditing(routine)} /> : null}
                {can.test ? <Button label="Test run" variant="ghost" size="sm" disabled={test.isPending} onPress={() => runTest(routine)} /> : null}
                <Button label={history === routine.id ? 'Hide history' : 'History'} variant="ghost" size="sm" onPress={() => setHistory(history === routine.id ? null : routine.id)} />
                {can.remove ? (
                  <Button
                    label="Delete"
                    variant="ghost"
                    size="sm"
                    onPress={() =>
                      setConfirm({
                        title: 'Delete routine?',
                        description: `“${routine.name}” stops for good. Its earlier results stay in chat.`,
                        confirmLabel: 'Delete',
                        danger: true,
                        onConfirm: () => remove.mutate(routine.id, { onError: fail }),
                      })
                    }
                  />
                ) : null}
              </View>
              {history === routine.id ? <RunHistory routine={routine} /> : null}
            </View>
          )
        })}
      </SettingsCard>
      <ScriptsCard name={name} />
      {editing ? <EditRoutineSheet routine={editing} onClose={() => setEditing(null)} /> : null}
      {approving ? <ApproveScriptSheet routine={approving} onClose={() => setApproving(null)} /> : null}
      <ConfirmDialog request={confirm} onClose={() => setConfirm(null)} />
    </>
  )
}

function Detail({ label, value, mono = false, error = false }: { label: string; value: string; mono?: boolean; error?: boolean }) {
  return (
    <View className="flex-row gap-3">
      <Text className="w-[72px] text-[12px] text-muted-foreground">{label}</Text>
      <Text className={`flex-1 text-[12px] ${mono ? 'font-mono' : ''} ${error ? 'text-destructive' : 'text-foreground'}`}>{value}</Text>
    </View>
  )
}

function LastRun({ routine }: { routine: Routine }) {
  if (!routine.last_run_at) return <Detail label="Last run" value="Not yet" />
  const when = whenLabel(routine.last_run_at)
  if (routine.last_run_status === 'skipped') return <Detail label="Last run" value={`${when} · ${routine.last_error ?? 'Skipped'}`} error />
  const failed = routine.last_run_status === 'failed'
  const label = routine.last_run_status === 'started' ? 'running' : failed ? 'failed' : 'done'
  const text = `${when} · ${label}${failed && routine.last_error ? ` · ${routine.last_error}` : ''}`
  const open = routine.last_run_message_id && routine.conversation_id ? `/chat/${routine.conversation_id}?message=${routine.last_run_message_id}` : null
  return (
    <View className="flex-row gap-3 items-center">
      <Text className="w-[72px] text-[12px] text-muted-foreground">Last run</Text>
      <Text className={`flex-1 text-[12px] ${failed ? 'text-destructive' : 'text-foreground'}`}>{text}</Text>
      {open ? <Button label="See result" variant="ghost" size="xs" onPress={() => router.push(open as never)} /> : null}
    </View>
  )
}

const RUN_STATUS: Record<RoutineRun['status'], { label: string; tone: BadgeTone }> = {
  started: { label: 'Running', tone: 'outline' },
  done: { label: 'Done', tone: 'secondary' },
  failed: { label: 'Failed', tone: 'destructive' },
  skipped: { label: 'Skipped', tone: 'outline' },
}

/** The routine's last 20 runs: time, status, a summary of the result, the error and the tool calls. */
function RunHistory({ routine }: { routine: Routine }) {
  const runs = useRoutineRuns(routine.id)
  if (runs.isPending) return <Note loading>Loading runs…</Note>
  if (runs.isError) return <Note error>The run history could not be loaded.</Note>
  if (runs.data.length === 0) return <Note>No runs yet.</Note>
  return (
    <View accessibilityLabel={`Runs of ${routine.name}`} className="gap-2 rounded-md bg-muted p-2">
      {runs.data.map((run) => <RunItem key={run.id} run={run} />)}
    </View>
  )
}

function RunItem({ run }: { run: RoutineRun }) {
  const [calls, setCalls] = useState(false)
  const status = RUN_STATUS[run.status]
  return (
    <View className="gap-1">
      <View className="flex-row flex-wrap items-center gap-1.5">
        <Text className="text-[12px]">{whenLabel(run.started_at)}</Text>
        {run.test ? <Badge label="Test run" tone="outline" /> : null}
        <Badge label={status.label} tone={status.tone} />
        {run.script_name ? <Text className="text-[12px] text-muted-foreground">script “{run.script_name}”</Text> : null}
        {run.writes === 'dry' && !run.test ? <Text className="text-[12px] text-destructive">no writes (script not approved)</Text> : null}
      </View>
      {run.output ? <Text numberOfLines={4} className="text-[12px] text-muted-foreground">{run.output}</Text> : null}
      {run.error ? <Text className="text-[12px] text-destructive">{run.error}</Text> : null}
      {run.tool_calls.length > 0 ? (
        <View className="items-start">
          <Button label={`${calls ? 'Hide' : 'Show'} ${run.tool_calls.length} tool ${run.tool_calls.length === 1 ? 'call' : 'calls'}`} variant="ghost" size="xs" muted onPress={() => setCalls(!calls)} />
          {calls
            ? run.tool_calls.map((call, index) => (
                <Text key={index} className="font-mono text-[11px]">
                  {call.name} <Text className="font-mono text-[11px] text-muted-foreground">{call.args}</Text>{' '}
                  <Text className={`font-mono text-[11px] ${call.status === 'error' ? 'text-destructive' : call.status === 'dry_run' ? 'text-primary' : 'text-muted-foreground'}`}>{call.status === 'dry_run' ? `would do: ${call.would_do ?? call.name}` : call.status}</Text>
                </Text>
              ))
            : null}
        </View>
      ) : null}
    </View>
  )
}

function CodeBlock({ code }: { code: string }) {
  return (
    <ScrollView horizontal className="max-h-80 rounded-md bg-muted">
      <ScrollView nestedScrollEnabled>
        <Text selectable className="p-3 font-mono text-[12px] leading-[18px]">{code}</Text>
      </ScrollView>
    </ScrollView>
  )
}

/** The routine's script as it is now, to approve its writes for exactly this code. Only the creator opens this. */
function ApproveScriptSheet({ routine, onClose }: { routine: Routine; onClose: () => void }) {
  const script = useScripts().data?.find((item) => item.id === routine.script_id)
  const update = useUpdateRoutine()
  return (
    <FormSheet
      visible
      title="Approve script"
      submitLabel="Approve"
      canSubmit={Boolean(script) && !update.isPending}
      busy={update.isPending}
      onSubmit={() => script && update.mutate({ id: routine.id, change: { approve_script_hash: script.code_hash } }, { onSuccess: onClose })}
      onClose={onClose}
      description={`Each run of “${routine.name}” runs this code as you. After your approval its writes need no new yes. A later change of the script turns them off again until you approve it.`}
    >
      {script ? (
        <>
          <Text className="text-[12px] text-muted-foreground">“{script.name}”, version <Text className="font-mono text-[12px]">{script.code_hash.slice(0, 12)}</Text></Text>
          <CodeBlock code={script.code} />
        </>
      ) : (
        <Note loading>Loading the script…</Note>
      )}
      {update.isError ? <Note error>{problemDetail(update.error, 'The script could not be approved.')}</Note> : null}
    </FormSheet>
  )
}

/** The routine's name, agent, instruction, schedule, zone, target and script. Only its creator opens this. */
function EditRoutineSheet({ routine, onClose }: { routine: Routine; onClose: () => void }) {
  const { user } = useOrbit()
  const people = usePeople()
  const status = useAgentStatus().data
  const profiles = useAgentProfiles().data
  const conversations = useConversations().data ?? []
  const allScripts = useScripts().data ?? []
  const update = useUpdateRoutine()
  const [form, setForm] = useState<RoutineForm>(() => routineFormOf(routine))
  const set = (change: Partial<RoutineForm>) => {
    update.reset()
    setForm((current) => ({ ...current, ...change }))
  }
  const agents = [
    ...(status?.enabled && status.bot_user_id ? [{ value: status.bot_user_id, label: status.name }] : []),
    ...(profiles?.agents ?? []).map((agent) => ({ value: agent.user_id, label: agent.name })),
  ]
  if (!agents.some((agent) => agent.value === routine.agent_user_id)) agents.push({ value: routine.agent_user_id, label: routine.agent_name })
  const targets = routineTargets(conversations, user.id, form.agentId).map((conversation) => ({
    value: conversation.id,
    label: conversation.kind === 'dm' ? conversationTitle(conversation, people, user.id) : `#${conversation.name}`,
  }))
  // Only the creator's own scripts can run in their routine.
  const scripts: Script[] = allScripts.filter((script) => script.owner_id === routine.creator_id)
  const script = scripts.find((item) => item.id === form.scriptId)
  const newScript = form.scriptId !== (routine.script_id ?? NO_SCRIPT)
  const zones = timeZoneChoices(form.timeZone).map((zone) => ({ value: zone, label: zone }))

  return (
    <FormSheet
      visible
      title="Edit routine"
      submitLabel="Save"
      canSubmit={routineFormValid(form) && !update.isPending}
      busy={update.isPending}
      onSubmit={() => update.mutate({ id: routine.id, change: routineChange(routine, form, allScripts) }, { onSuccess: onClose })}
      onClose={onClose}
    >
      <Input label="Name" value={form.name} maxLength={80} onChangeText={(name) => set({ name })} />
      <SelectField label="Agent" value={form.agentId} options={agents} onChange={(agentId) => set({ agentId })} description="It runs the routine as you. Another agent must work in the channel where the routine posts." />
      <Input label="Instruction" multiline value={form.prompt} maxLength={4000} onChangeText={(prompt) => set({ prompt })} />
      <Input
        label="Schedule (cron: minute hour day month weekday)"
        value={form.schedule}
        autoCapitalize="none"
        autoCorrect={false}
        onChangeText={(schedule) => set({ schedule })}
        description="0 9 * * 1-5 is every weekday at 9:00, 30 8 * * 1 every Monday at 8:30. At most every 15 minutes."
      />
      <SelectField label="Time zone" value={form.timeZone} options={zones} onChange={(timeZone) => set({ timeZone })} />
      <SelectField label="Posts in" value={form.target} options={targets} placeholder={routine.conversation_name ? `#${routine.conversation_name}` : 'Pick a conversation'} onChange={(target) => set({ target })} />
      <SelectField label="Script" value={form.scriptId} options={[{ value: NO_SCRIPT, label: 'None: only the instruction' }, ...scripts.map((item) => ({ value: item.id, label: item.name }))]} placeholder={routine.script_name ?? undefined} onChange={(scriptId) => set({ scriptId })} />
      {newScript && script ? (
        <>
          <CodeBlock code={script.code} />
          <OptionHint>Saving approves this version: its writes run without a new yes. A later change of the script needs a new yes.</OptionHint>
        </>
      ) : null}
      {update.isError ? <Note error>{problemDetail(update.error, 'The routine could not be saved.')}</Note> : null}
    </FormSheet>
  )
}

/** The saved scripts: the person's own (every script for owners and admins), to view, edit or delete. */
function ScriptsCard({ name }: { name: string }) {
  const scripts = useScripts()
  const remove = useDeleteScript()
  const [open, setOpen] = useState<Script | null>(null)
  const [confirm, setConfirm] = useState<ConfirmRequest | null>(null)
  const items = scripts.data ?? []
  return (
    <SettingsCard title="Scripts" description={`Scripts that ${name} saved for you. A script calls ${name}’s tools, as you, in a sandbox with no internet and no files. A routine can run one on each run; a changed script makes no writes until you approve it again on its routine.`} flush>
      {scripts.isPending ? <View className="p-4"><Note loading>Loading scripts…</Note></View> : null}
      {scripts.isError ? <View className="p-4"><Note error>The scripts could not be loaded.</Note></View> : null}
      {scripts.isSuccess && items.length === 0 ? <View className="p-4"><Note>No scripts yet. Ask {name} to save one, for example “save this as a script”.</Note></View> : null}
      {items.map((script, index) => (
        <View key={script.id} className={`px-4 py-3 gap-1 border-border ${index === 0 ? '' : 'border-t'}`}>
          <View className="flex-row flex-wrap items-center gap-1.5">
            <Text className="font-noto-medium text-[14px]">{script.name}</Text>
            <Text className="font-mono text-[11px] text-muted-foreground">{script.code_hash.slice(0, 12)}</Text>
            {script.can_edit ? null : <Text className="text-[12px] text-muted-foreground">by {script.owner_name}</Text>}
          </View>
          {script.description ? <Text className="text-[13px] text-muted-foreground">{script.description}</Text> : null}
          <Text className="text-[12px] text-muted-foreground">{script.routines.length === 0 ? 'No routine runs it.' : `Runs in: ${script.routines.map((routine) => `${routine.name}${routine.approved ? '' : ' (not approved)'}`).join(', ')}`}</Text>
          <View className="flex-row -ml-2">
            <Button label={script.can_edit ? 'Edit' : 'View'} variant="ghost" size="sm" onPress={() => setOpen(script)} />
            {script.can_delete ? (
              <Button
                label="Delete"
                variant="ghost"
                size="sm"
                onPress={() => setConfirm({ title: 'Delete script?', description: `“${script.name}” is deleted for good. Routines that run it go back to their instruction.`, confirmLabel: 'Delete', danger: true, onConfirm: () => remove.mutate(script.id) })}
              />
            ) : null}
          </View>
        </View>
      ))}
      {remove.isError ? <View className="p-4"><Note error>{problemDetail(remove.error, 'The script could not be deleted.')}</Note></View> : null}
      {open ? <ScriptSheet script={open} onClose={() => setOpen(null)} /> : null}
      <ConfirmDialog request={confirm} onClose={() => setConfirm(null)} />
    </SettingsCard>
  )
}

/** A script's name, description and code: editable by its owner, read-only for owners and admins. */
function ScriptSheet({ script, onClose }: { script: Script; onClose: () => void }) {
  const update = useUpdateScript()
  const [name, setName] = useState(script.name)
  const [description, setDescription] = useState(script.description)
  const [code, setCode] = useState(script.code)
  const readOnly = !script.can_edit
  const changedCode = code.trimEnd() !== script.code
  return (
    <FormSheet
      visible
      title={readOnly ? script.name : 'Edit script'}
      submitLabel={readOnly ? undefined : 'Save'}
      canSubmit={!update.isPending && name.trim() !== '' && code.trim() !== ''}
      busy={update.isPending}
      onSubmit={() => update.mutate({ id: script.id, change: { name, description, ...(changedCode ? { code } : {}) } }, { onSuccess: onClose })}
      onClose={onClose}
    >
      <Input label="Name" value={name} maxLength={80} editable={!readOnly} onChangeText={setName} />
      <Input label="Description" value={description} maxLength={500} editable={!readOnly} onChangeText={setDescription} />
      {readOnly ? (
        <CodeBlock code={code} />
      ) : (
        <Input label="Code (JavaScript; calls tools as await tools.name(args))" multiline value={code} maxLength={20000} autoCapitalize="none" autoCorrect={false} spellCheck={false} onChangeText={setCode} />
      )}
      {changedCode && script.routines.length > 0 ? <Note error>New code: {script.routines.map((routine) => routine.name).join(', ')} will make no writes until you approve the new version on each routine.</Note> : null}
      {update.isError ? <Note error>{problemDetail(update.error, 'The script could not be saved.')}</Note> : null}
    </FormSheet>
  )
}
