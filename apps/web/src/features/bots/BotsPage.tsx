import { useState } from 'react'
import { toast } from 'sonner'
import type { WebhookBot, WebhookBotDelivery } from '@orbit/api/bots'
import { ApiProblem } from '@orbit/api/problem'
import { ConfirmDeleteModal } from '@/components/common/ConfirmDeleteModal'
import { confirmAction } from '@/components/common/confirmAction'
import { CopyCodeButton } from '@/components/common/CopyCodeButton'
import { Modal } from '@/components/common/Modal'
import { SettingsCard } from '@/components/common/SettingsCard'
import { UserAvatar } from '@/components/common/UserAvatar'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import { DialogClose, DialogFooter } from '@/components/ui/dialog'
import { Field, FieldDescription, FieldLabel } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { useConversations } from '@/features/chat/api/queries'
import { avatarImage } from '@/features/profile/avatarImage'
import { useCan } from '@/features/workspaces/permissions'
import { useWorkspace } from '@/features/workspaces/workspaceContext'
import { relativeTime } from '@/lib/format'
import { useDeleteWebhookBot, useRotateWebhookBotSecret, useSaveWebhookBot, useWebhookBotDeliveries, useWebhookBots } from './api'

/** Settings → Agentic → Bots: bots that answer in chat from an https URL. Owners and admins manage them. */
export function BotsPage() {
  const { workspace } = useWorkspace()
  const canManage = useCan('workspace.update')
  const query = useWebhookBots(workspace.id, canManage)
  const remove = useDeleteWebhookBot(workspace.id)
  const rotate = useRotateWebhookBotSecret(workspace.id)
  const [editing, setEditing] = useState<WebhookBot | 'new' | null>(null)
  const [deleting, setDeleting] = useState<WebhookBot | null>(null)
  const [deliveries, setDeliveries] = useState<WebhookBot | null>(null)
  const [secret, setSecret] = useState<{ name: string; secret: string } | null>(null)
  const data = query.data
  const bots = data?.bots ?? []

  async function rotateSecret(bot: WebhookBot) {
    if (!(await confirmAction({ title: `Rotate the secret of ${bot.name}?`, description: 'Orbit signs with a new secret at once. The bot refuses messages until you put the new secret in its service.', confirmLabel: 'Rotate secret', danger: true }))) return
    rotate.mutate(bot.id, {
      onSuccess: (value) => setSecret({ name: bot.name, secret: value }),
      onError: (error) => toast.error(error instanceof ApiProblem ? error.detail : 'The secret could not be rotated.'),
    })
  }
  const description = 'Bots answer in chat from your own https service. People @mention a bot in its channels or send it a direct message; Orbit sends the message, signed, to the bot’s URL, and posts its answer as the bot. Bots post plain messages only: they get no tools and no one’s permissions.'

  if (!canManage) return (
    <SettingsCard title="Bots" description={description}>
      <p className="text-sm text-muted-foreground">Only owners and admins can manage bots.</p>
    </SettingsCard>
  )

  return (
    <SettingsCard title="Bots" description={description} actions={<Button size="sm" disabled={!data || bots.length >= data.max_bots} onClick={() => setEditing('new')}>New bot</Button>}>
      {query.isPending ? null : query.isError ? (
        <p role="alert" className="text-sm text-destructive">The bots could not be loaded.</p>
      ) : bots.length === 0 ? (
        <p className="text-sm text-muted-foreground">No bots yet. Create one, then point it at your service. How a bot checks and answers messages: docs/webhook-bots.md.</p>
      ) : (
        <ul className="-mx-2 flex flex-col">
          {bots.map((bot) => (
            <li key={bot.id} data-slot="webhook-bot" className="flex items-start gap-3 rounded-md px-2 py-3 not-last:border-b not-last:border-border">
              <UserAvatar user={{ name: bot.name, color: 'var(--primary)', avatarUrl: bot.avatar_url }} size={36} />
              <div className="flex min-w-0 flex-1 flex-col gap-1">
                <div className="flex flex-wrap items-center gap-2">
                  <span className="text-sm font-medium text-foreground">{bot.name}</span>
                  <Badge variant="secondary">Bot</Badge>
                </div>
                <p className="truncate font-mono text-xs text-muted-foreground">{bot.url}</p>
                <p className="text-xs text-muted-foreground">{channelsLabel(bot)}</p>
              </div>
              <div className="flex shrink-0 flex-wrap justify-end gap-1">
                <Button size="sm" variant="ghost" onClick={() => setDeliveries(bot)}>Deliveries</Button>
                <Button size="sm" variant="ghost" disabled={rotate.isPending} onClick={() => void rotateSecret(bot)}>Rotate secret</Button>
                <Button size="sm" variant="ghost" onClick={() => setEditing(bot)}>Edit</Button>
                <Button size="sm" variant="ghost" className="text-destructive" onClick={() => setDeleting(bot)}>Delete</Button>
              </div>
            </li>
          ))}
        </ul>
      )}
      {editing ? <BotDialog bot={editing === 'new' ? null : editing} onClose={() => setEditing(null)} onCreated={(name, value) => setSecret({ name, secret: value })} /> : null}
      {secret ? <SecretDialog name={secret.name} secret={secret.secret} onClose={() => setSecret(null)} /> : null}
      {deliveries ? <DeliveriesDialog bot={deliveries} onClose={() => setDeliveries(null)} /> : null}
      {deleting ? (
        <ConfirmDeleteModal
          title={`Delete ${deleting.name}?`}
          description="Nobody can reach it any more. Its messages and direct messages stay readable."
          onClose={() => setDeleting(null)}
          onConfirm={() =>
            remove.mutate(deleting.id, {
              onSuccess: () => setDeleting(null),
              onError: (error) => toast.error(error instanceof ApiProblem ? error.detail : 'The bot could not be deleted.'),
            })
          }
        />
      ) : null}
    </SettingsCard>
  )
}

function channelsLabel(bot: WebhookBot): string {
  if (bot.channels.length === 0) return 'Direct messages only'
  return `${bot.channels.map((channel) => `#${channel.name}`).join(', ')} and direct messages`
}

/** Create or edit one bot. The avatar is uploaded after the bot is saved. */
function BotDialog({ bot, onClose, onCreated }: { bot: WebhookBot | null; onClose: () => void; onCreated: (name: string, secret: string) => void }) {
  const { workspace } = useWorkspace()
  const save = useSaveWebhookBot(workspace.id)
  const channels = (useConversations().data ?? []).filter((conversation) => conversation.kind !== 'dm' && !conversation.archived && (conversation.kind === 'public' || conversation.isMember))
  const [name, setName] = useState(bot?.name ?? '')
  const [url, setUrl] = useState(bot?.url ?? '')
  const [channelIds, setChannelIds] = useState<string[]>(bot?.channels.map((channel) => channel.id) ?? [])
  const [avatar, setAvatar] = useState<{ blob: Blob; url: string } | null | undefined>(undefined)
  const avatarUrl = avatar === undefined ? bot?.avatar_url : (avatar?.url ?? null)

  async function pickAvatar(file: File | undefined) {
    if (!file) return
    try {
      const blob = await avatarImage(file)
      setAvatar({ blob, url: URL.createObjectURL(blob) })
    } catch {
      toast.error('This image cannot be used. Pick a PNG, JPEG or WebP image.')
    }
  }

  function submit(event: React.FormEvent) {
    event.preventDefault()
    save.mutate(
      { id: bot?.id ?? null, input: { name: name.trim(), url: url.trim(), channel_ids: channelIds }, avatar: avatar === undefined ? undefined : (avatar?.blob ?? null) },
      {
        onSuccess: (saved) => {
          if (saved.signing_secret) onCreated(saved.bot.name, saved.signing_secret)
          else toast(`${saved.bot.name} saved`)
          onClose()
        },
      },
    )
  }

  return (
    <Modal title={bot ? `Edit ${bot.name}` : 'New bot'} onClose={onClose} className="sm:max-w-xl">
      <form id="webhook-bot-form" onSubmit={submit} className="flex max-h-[70vh] flex-col gap-5 overflow-y-auto pr-1">
        <div className="flex items-center gap-3">
          <UserAvatar user={{ name: name || '?', color: 'var(--primary)', avatarUrl }} size={48} />
          <div className="flex flex-wrap gap-2">
            <Button type="button" size="sm" variant="outline" render={<label />} nativeButton={false}>
              Upload avatar
              <input type="file" accept="image/png,image/jpeg,image/webp" className="sr-only" onChange={(event) => void pickAvatar(event.target.files?.[0])} />
            </Button>
            {avatarUrl ? <Button type="button" size="sm" variant="ghost" onClick={() => setAvatar(null)}>Remove</Button> : null}
          </div>
        </div>
        <Field>
          <FieldLabel htmlFor="webhook-bot-name">Name</FieldLabel>
          <Input id="webhook-bot-name" required maxLength={40} value={name} onChange={(event) => { save.reset(); setName(event.target.value) }} />
          <FieldDescription>Its @handle in chat. 2 to 40 characters: letters, digits, single spaces and - _ . ' It cannot be anyone else's name.</FieldDescription>
        </Field>
        <Field>
          <FieldLabel htmlFor="webhook-bot-url">URL</FieldLabel>
          <Input id="webhook-bot-url" required type="url" placeholder="https://bot.example.com/orbit" value={url} onChange={(event) => { save.reset(); setUrl(event.target.value) }} />
          <FieldDescription>Orbit sends each message here as a signed POST. It must be https and reach a public address. The bot answers within 30 seconds.</FieldDescription>
        </Field>
        <fieldset className="flex flex-col gap-2">
          <legend className="mb-1 text-sm font-medium text-foreground">Channels</legend>
          <div className="grid grid-cols-1 gap-1.5 min-[600px]:grid-cols-2">
            {channels.map((channel) => (
              <label key={channel.id} className="flex items-center gap-2 text-sm text-foreground">
                <Checkbox checked={channelIds.includes(channel.id)} onCheckedChange={(on) => setChannelIds((list) => (on ? [...list, channel.id] : list.filter((own) => own !== channel.id)))} />
                #{channel.name}
              </label>
            ))}
          </div>
          <p className="text-xs text-muted-foreground">Direct messages with it always work. A mention in another channel is not sent: the bot tells the person in a direct message where it works.</p>
        </fieldset>
        {save.isError ? <p role="alert" className="text-sm text-destructive">{save.error instanceof ApiProblem ? save.error.detail : 'The bot could not be saved.'}</p> : null}
      </form>
      <DialogFooter>
        <DialogClose render={<Button type="button" variant="outline" />}>Cancel</DialogClose>
        <Button type="submit" form="webhook-bot-form" disabled={save.isPending || name.trim().length < 2 || !url.trim()}>{save.isPending ? 'Saving…' : bot ? 'Save' : 'Create bot'}</Button>
      </DialogFooter>
    </Modal>
  )
}

/** The signing secret, the only time Orbit shows it. */
function SecretDialog({ name, secret, onClose }: { name: string; secret: string; onClose: () => void }) {
  return (
    <Modal title={`Signing secret of ${name}`} onClose={onClose} className="sm:max-w-xl">
      <div className="flex flex-col gap-3">
        <p className="text-sm text-muted-foreground">Copy it into your bot’s service now: Orbit shows it only this once. The bot checks the signature of each message with it.</p>
        <div className="relative rounded-md bg-muted/50 py-2.5 pr-20 pl-3">
          <pre data-slot="bot-signing-secret" className="font-mono text-xs leading-5 break-all whitespace-pre-wrap text-foreground">{secret}</pre>
          <CopyCodeButton getText={() => secret} className="absolute top-1.5 right-1.5" />
        </div>
      </div>
      <DialogFooter>
        <DialogClose render={<Button type="button" />}>Done</DialogClose>
      </DialogFooter>
    </Modal>
  )
}

const STATUS_LABELS: Record<WebhookBotDelivery['status'], string> = {
  pending: 'Waiting',
  sending: 'Sent, waiting for the answer',
  answered: 'Answered',
  late: 'No answer in time',
  failed: 'Failed',
  refused: 'Channel not allowed',
  limited: 'Rate limited',
}

/** The bot's newest deliveries: what happened, never the message. */
function DeliveriesDialog({ bot, onClose }: { bot: WebhookBot; onClose: () => void }) {
  const { workspace } = useWorkspace()
  const query = useWebhookBotDeliveries(workspace.id, bot.id)
  const deliveries = query.data ?? []
  return (
    <Modal title={`Deliveries of ${bot.name}`} onClose={onClose} className="sm:max-w-xl">
      {query.isPending ? null : query.isError ? (
        <p role="alert" className="text-sm text-destructive">The deliveries could not be loaded.</p>
      ) : deliveries.length === 0 ? (
        <p className="text-sm text-muted-foreground">No messages were sent to this bot yet.</p>
      ) : (
        <ul className="flex flex-col">
          {deliveries.map((delivery) => (
            <li key={delivery.id} className="flex items-center justify-between gap-3 py-2 text-sm not-last:border-b not-last:border-border">
              <span className="text-foreground">{STATUS_LABELS[delivery.status]}</span>
              <span className="text-xs text-muted-foreground">
                {[delivery.http_status ? `HTTP ${delivery.http_status}` : null, delivery.error, relativeTime(new Date(delivery.created_at).toISOString())].filter(Boolean).join(' · ')}
              </span>
            </li>
          ))}
        </ul>
      )}
    </Modal>
  )
}
