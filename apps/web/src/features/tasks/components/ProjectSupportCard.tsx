import { useId, useState, type FormEvent } from 'react'
import { Copy, Danger } from 'reicon-react'
import { toast } from 'sonner'
import { ApiProblem, fieldErrors } from '@orbit/api/problem'
import type { SupportMailbox, SupportMailboxBody } from '@orbit/api/generated/types.gen'
import { confirmAction } from '@/components/common/confirmAction'
import { SettingsCard } from '@/components/common/SettingsCard'
import { Button } from '@/components/ui/button'
import { Field, FieldDescription, FieldLabel } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { InputGroup, InputGroupAddon, InputGroupButton, InputGroupInput } from '@/components/ui/input-group'
import { PasswordInput } from '@/components/ui/password-input'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { Switch } from '@/components/ui/switch'
import { LoadingText } from '@/components/ui/spinner'
import { SettingsRow } from '@/features/settings/components/SettingsParts'
import { useDeleteSupportMailbox, useRotateSupportMailbox, useSaveSupportMailbox, useSupportMailbox } from '@/features/tasks/api/support'
import { useTeamMembers } from '@/features/workspaces/api'
import { useCan } from '@/features/workspaces/permissions'
import { useWorkspace } from '@/features/workspaces/workspaceContext'

const SECURITY = [
  { value: 'starttls', label: 'STARTTLS (587)' },
  { value: 'tls', label: 'TLS (465)' },
  { value: 'none', label: 'None (development only)' },
]
const NOBODY = 'nobody'

function copy(text: string, done: string) {
  navigator.clipboard.writeText(text).then(
    () => toast.success(done),
    () => toast.error('Could not copy. Select the text and copy it.'),
  )
}

/** What a failed save says: the field the server refused, or its detail. */
function saveError(error: unknown): string {
  if (error instanceof ApiProblem) {
    const field = fieldErrors(error)[0]?.path.join('.')
    if (field) return `Check ${field.replaceAll('_', ' ')}.`
    return error.detail
  }
  return 'The mailbox could not be saved.'
}

/** The inbound URL after a connect or rotate, with a test command. It does not show again. */
function InboundUrl({ url, onDone }: { url: string; onDone: () => void }) {
  const urlId = useId()
  const example = `curl -X POST -H 'Content-Type: message/rfc822' \\\n  --data-binary @email.eml \\\n  '${url}'`
  return (
    <div data-slot="support-inbound-url" className="grid gap-3 rounded-lg border p-3">
      <Field>
        <FieldLabel htmlFor={urlId}>Inbound URL</FieldLabel>
        <InputGroup>
          <InputGroupInput id={urlId} readOnly value={url} className="font-mono text-xs" onFocus={(event) => event.target.select()} />
          <InputGroupAddon align="inline-end">
            <InputGroupButton aria-label="Copy inbound URL" size="icon-xs" onClick={() => copy(url, 'URL copied')}>
              <Copy />
            </InputGroupButton>
          </InputGroupAddon>
        </InputGroup>
      </Field>
      <p className="flex items-start gap-2 text-xs text-muted-foreground">
        <Danger aria-hidden="true" className="mt-px size-4 shrink-0 text-amber-500" />
        Copy it now: Orbit shows this URL only once. Anyone who has it can put email into this project. Put it in your Cloudflare Email Worker (see docs/support.md).
      </p>
      <div className="grid gap-1.5">
        <div className="flex items-center justify-between">
          <span className="text-xs font-medium">Try it with a saved email</span>
          <Button type="button" variant="ghost" size="xs" onClick={() => copy(example, 'Command copied')}>
            <Copy data-icon="inline-start" />
            Copy
          </Button>
        </div>
        <pre className="overflow-x-auto rounded-md bg-muted px-3 py-2 font-mono text-xs leading-relaxed whitespace-pre">{example}</pre>
      </div>
      <Button type="button" size="sm" variant="outline" className="justify-self-end" onClick={onDone}>Done</Button>
    </div>
  )
}

function MailboxForm({ projectId, mailbox, onUrl }: { projectId: string; mailbox: SupportMailbox | null; onUrl: (url: string) => void }) {
  const { workspace } = useWorkspace()
  const members = useTeamMembers(workspace.id)
  const save = useSaveSupportMailbox(workspace.id, projectId, mailbox !== null)
  const [address, setAddress] = useState(mailbox?.address ?? '')
  const [displayName, setDisplayName] = useState(mailbox?.display_name ?? '')
  const [host, setHost] = useState(mailbox?.smtp_host ?? '')
  const [port, setPort] = useState(String(mailbox?.smtp_port ?? 587))
  const [security, setSecurity] = useState(mailbox?.smtp_security ?? 'starttls')
  const [username, setUsername] = useState(mailbox?.smtp_username ?? '')
  const [password, setPassword] = useState('')
  const [moveToPending, setMoveToPending] = useState(mailbox?.move_to_pending ?? true)
  const [assignee, setAssignee] = useState(mailbox?.default_assignee_id ?? NOBODY)
  const ids = { address: useId(), name: useId(), host: useId(), port: useId(), user: useId(), password: useId() }
  const people = [
    { value: NOBODY, label: 'Nobody' },
    ...(members.data ?? []).filter((member) => !member.suspended).sort((a, b) => a.name.localeCompare(b.name)).map((member) => ({ value: member.id, label: member.name })),
  ]

  const submit = async (event: FormEvent) => {
    event.preventDefault()
    const body: SupportMailboxBody = {
      address,
      display_name: displayName,
      smtp_host: host.trim() || null,
      smtp_port: host.trim() ? Number(port) : null,
      smtp_security: host.trim() ? security : null,
      smtp_username: username.trim() || null,
      move_to_pending: moveToPending,
      default_assignee_id: assignee === NOBODY ? null : assignee,
    }
    // an empty field keeps the saved password
    if (password) body.smtp_password = password
    try {
      const saved = await save.mutateAsync(body)
      setPassword('')
      if (saved.inbound_url) onUrl(saved.inbound_url)
      else toast.success('Mailbox saved')
    } catch {
      // shown below
    }
  }

  return (
    <form className="grid gap-4 p-4" onSubmit={(event) => void submit(event)}>
      <div className="grid gap-4 min-[600px]:grid-cols-2">
        <Field>
          <FieldLabel htmlFor={ids.address}>Support address</FieldLabel>
          <Input id={ids.address} type="email" required placeholder="support@example.com" value={address} onChange={(event) => setAddress(event.target.value)} />
          <FieldDescription>Customers write here; replies come from it.</FieldDescription>
        </Field>
        <Field>
          <FieldLabel htmlFor={ids.name}>Sender name</FieldLabel>
          <Input id={ids.name} placeholder="Acme Support" value={displayName} onChange={(event) => setDisplayName(event.target.value)} />
        </Field>
        <Field>
          <FieldLabel htmlFor={ids.host}>SMTP server</FieldLabel>
          <Input id={ids.host} placeholder="smtp.example.com" value={host} onChange={(event) => setHost(event.target.value)} />
          <FieldDescription>The server of your support domain sends the replies. Without one, the project only receives email.</FieldDescription>
        </Field>
        <div className="grid grid-cols-2 gap-3">
          <Field>
            <FieldLabel htmlFor={ids.port}>Port</FieldLabel>
            <Input id={ids.port} inputMode="numeric" value={port} onChange={(event) => setPort(event.target.value.replace(/\D/g, ''))} />
          </Field>
          <Field>
            <FieldLabel>Security</FieldLabel>
            <Select items={SECURITY} value={security} onValueChange={(value) => setSecurity(value ?? 'starttls')}>
              <SelectTrigger aria-label="SMTP security"><SelectValue /></SelectTrigger>
              <SelectContent>{SECURITY.map((item) => <SelectItem key={item.value} value={item.value}>{item.label}</SelectItem>)}</SelectContent>
            </Select>
          </Field>
        </div>
        <Field>
          <FieldLabel htmlFor={ids.user}>SMTP user name</FieldLabel>
          <Input id={ids.user} autoComplete="off" value={username} onChange={(event) => setUsername(event.target.value)} />
        </Field>
        <Field>
          <FieldLabel htmlFor={ids.password}>SMTP password</FieldLabel>
          <PasswordInput id={ids.password} autoComplete="new-password" placeholder={mailbox?.smtp_password_set ? 'Saved — type to replace' : ''} value={password} onChange={(event) => setPassword(event.target.value)} />
          <FieldDescription>Stored encrypted with the server's app key.</FieldDescription>
        </Field>
      </div>
      <div className="flex flex-wrap items-center justify-between gap-3 border-t pt-4">
        <div className="flex min-w-0 flex-col gap-0.5">
          <span className="text-sm font-medium text-foreground">Move to Pending after a reply</span>
          <span className="text-xs text-muted-foreground">A customer's answer moves the ticket back to Open.</span>
        </div>
        <Switch aria-label="Move to Pending after a reply" checked={moveToPending} onCheckedChange={(checked: boolean) => setMoveToPending(checked)} />
      </div>
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="flex min-w-0 flex-col gap-0.5">
          <span className="text-sm font-medium text-foreground">Assign new tickets to</span>
          <span className="text-xs text-muted-foreground">Assignees and followers get an Inbox notification for each customer email.</span>
        </div>
        <Select items={people} value={assignee} onValueChange={(value) => setAssignee(value ?? NOBODY)}>
          <SelectTrigger aria-label="Assign new tickets to" className="w-48"><SelectValue /></SelectTrigger>
          <SelectContent>{people.map((item) => <SelectItem key={item.value} value={item.value}>{item.label}</SelectItem>)}</SelectContent>
        </Select>
      </div>
      {save.isError ? <p role="alert" className="text-xs text-destructive">{saveError(save.error)}</p> : null}
      <div className="flex justify-end">
        <Button type="submit" disabled={save.isPending || !address.trim()}>{mailbox ? 'Save' : 'Connect mailbox'}</Button>
      </div>
    </form>
  )
}

/**
 * Support email of a project (owners and admins): the support address, the SMTP server that sends replies, and the
 * secret inbound URL a mail provider (a Cloudflare Email Worker) posts raw email to. A connected project's tasks are
 * tickets: customer email makes them, and the task composer gets "Reply to customer".
 */
export function ProjectSupportCard({ projectId }: { projectId: string }) {
  const { workspace } = useWorkspace()
  const canManage = useCan('integrations.manage')
  const mailbox = useSupportMailbox(workspace.id, projectId, canManage)
  const rotate = useRotateSupportMailbox(workspace.id, projectId)
  const disconnect = useDeleteSupportMailbox(workspace.id, projectId)
  const [url, setUrl] = useState<string>()
  const description = 'Customer email becomes tasks in this project: a ticket per conversation, replies from the task.'

  if (!canManage) {
    return <SettingsCard title="Support email" description={description}><p className="text-sm text-muted-foreground">Only workspace owners and admins manage the support mailbox.</p></SettingsCard>
  }
  if (mailbox.isPending) return <SettingsCard title="Support email" description={description}><LoadingText>Loading mailbox…</LoadingText></SettingsCard>
  if (mailbox.isError) return <SettingsCard title="Support email" description={description}><p role="alert" className="text-sm text-destructive">The mailbox could not be loaded.</p></SettingsCard>
  const current = mailbox.data.mailbox ?? null

  const remove = async () => {
    if (!await confirmAction({ title: 'Disconnect this mailbox?', description: 'Email to its inbound URL stops at once. Tickets and their emails stay, but they can no longer be answered.', confirmLabel: 'Disconnect', danger: true })) return
    disconnect.mutate()
  }
  const newUrl = async () => {
    if (!await confirmAction({ title: 'Make a new inbound URL?', description: 'The old URL stops working at once. Put the new one in your mail provider.', confirmLabel: 'Make new URL' })) return
    const rotated = await rotate.mutateAsync().catch(() => null)
    if (rotated?.inbound_url) setUrl(rotated.inbound_url)
  }

  return (
    <div className="grid gap-4">
      {url ? <InboundUrl url={url} onDone={() => setUrl(undefined)} /> : null}
      <SettingsCard title="Support email" description={description} flush>
        <MailboxForm key={current?.updated_at ?? 'new'} projectId={projectId} mailbox={current} onUrl={setUrl} />
      </SettingsCard>
      {current ? (
        <SettingsCard title="Inbound URL" description="Your mail provider posts each raw email to this secret URL." flush>
          <div className="flex flex-col divide-y">
            <SettingsRow className="flex-wrap justify-between gap-3">
              <div className="flex min-w-0 flex-col gap-0.5">
                <span className="truncate font-mono text-xs text-foreground">{current.inbound_url_base}/…</span>
                <span className="text-xs text-muted-foreground">
                  {current.last_received_at ? `Last email ${new Date(current.last_received_at).toLocaleString()}` : 'No email received yet.'}
                </span>
              </div>
              <div className="flex gap-2">
                <Button type="button" variant="outline" size="sm" disabled={rotate.isPending} onClick={() => void newUrl()}>Make new URL</Button>
                <Button type="button" variant="outline" size="sm" className="text-destructive" disabled={disconnect.isPending} onClick={() => void remove()}>Disconnect</Button>
              </div>
            </SettingsRow>
            {rotate.isError || disconnect.isError ? <SettingsRow role="alert" className="text-xs text-destructive">The change could not be saved.</SettingsRow> : null}
          </div>
        </SettingsCard>
      ) : null}
    </div>
  )
}
