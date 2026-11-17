import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import * as Clipboard from 'expo-clipboard'
import { useLocalSearchParams } from 'expo-router'
import { useState } from 'react'
import { ScrollView, View } from 'react-native'

import { BotAvatar } from '../../../components/chat/BotAvatar'
import { ChatColorScope } from '../../../components/chat/ChatColorScope'
import { ChatPane } from '../../../components/chat/Pane'
import { FormSheet } from '../../../components/chat/Sheet'
import { ActionSheet, type SheetAction } from '../../../components/settings/Dialog'
import { Note, Toggle } from '../../../components/settings/Shell'
import * as chat from '../../../data/chat-api'
import { relativeAgo } from '../../../data/format'
import { confirmAction } from '../../../hooks/chat'
import { useOrbit } from '../../../orbit-context'
import { useConversations, usePeople } from '../../../queries'
import { errorMessage } from '../../../session-context'
import { useTheme } from '../../../theme'
import { Button, IconButton, Input, LoadingView, Text } from '../../../ui'

/**
 * The web's Edit channel → Integrations: the channel's incoming webhooks. Each has a secret URL that a tool (CI,
 * monitoring) posts JSON messages to; it shows only right after it is made or rotated. Reading the channel, buttons
 * (request URL, signing secret) and events are per webhook. The channel's creator and workspace owners and admins.
 */
export default function IntegrationsScreen() {
  const { id } = useLocalSearchParams<{ id: string }>()
  const { client, workspaceId } = useOrbit()
  const { colors } = useTheme()
  const queryClient = useQueryClient()
  const conversation = useConversations().data?.find((item) => item.id === id)
  const people = usePeople()
  const key = ['conversations', workspaceId, id, 'webhooks']
  const list = useQuery({ queryKey: key, queryFn: () => chat.webhooks(client, workspaceId, id) })
  const [form, setForm] = useState<{ webhook?: chat.WebhookRecord } | null>(null)
  const [shown, setShown] = useState<chat.ChatWebhookWithUrl | null>(null)
  const [menu, setMenu] = useState<SheetAction[] | null>(null)
  const [buttonsOf, setButtonsOf] = useState<chat.WebhookRecord | null>(null)
  const write = useMutation({
    mutationFn: ({ webhookId, change }: { webhookId: string; change: chat.WebhookWrite }) => chat.writeWebhook(client, workspaceId, id, webhookId, change),
    onSuccess: (rotated) => rotated && setShown(rotated),
    onSettled: () => void queryClient.invalidateQueries({ queryKey: key }),
  })
  const create = useMutation({
    mutationFn: ({ name, iconUrl }: { name: string; iconUrl: string | null }) => chat.createWebhook(client, workspaceId, id, name, iconUrl),
    onSuccess: setShown,
    onSettled: () => void queryClient.invalidateQueries({ queryKey: key }),
  })
  const nameOf = (userId: string) => people.find((person) => person.id === userId)?.name ?? 'a former member'
  const change = (webhook: chat.WebhookRecord, next: chat.WebhookWrite) => write.mutate({ webhookId: webhook.id, change: next })

  const options = (webhook: chat.WebhookRecord): SheetAction[] => [
    { label: 'Edit', icon: 'edit', onPress: () => setForm({ webhook }) },
    {
      label: 'Make new URL',
      icon: 'key',
      onPress: () =>
        void confirmAction(`Make a new URL for ${webhook.name}?`, 'The old URL stops working now. Tools that use it must get the new one.', 'Make new URL').then(
          (yes) => yes && change(webhook, { kind: 'rotate' }),
        ),
    },
    { label: 'Buttons and events', icon: 'flash', onPress: () => setButtonsOf(webhook) },
    {
      label: webhook.can_read ? 'Stop reading this channel' : 'Can read this channel',
      icon: 'book',
      onPress: () =>
        webhook.can_read
          ? change(webhook, { kind: 'read', canRead: false })
          : void confirmAction(`Let ${webhook.name} read this channel?`, 'Anyone who has its webhook URL can then read the messages and threads of this channel.', 'Allow reading').then(
              (yes) => yes && change(webhook, { kind: 'read', canRead: true }),
            ),
    },
    {
      label: 'Delete',
      icon: 'trash',
      destructive: true,
      onPress: () =>
        void confirmAction(`Delete ${webhook.name}?`, 'Its URL stops working. The messages it posted stay.', 'Delete').then((yes) => {
          if (!yes) return
          if (shown?.webhook.id === webhook.id) setShown(null)
          change(webhook, { kind: 'delete' })
        }),
    },
  ]

  const error = write.error ?? create.error
  return (
    <ChatColorScope surface="page">
      <ChatPane
        title="Integrations"
        subtitle={conversation ? `#${conversation.name}` : undefined}
        actions={<IconButton icon="plus" label="New webhook" size={32} iconSize={20} color={colors.mutedForeground} onPress={() => setForm({})} />}
      >
        <ScrollView contentContainerClassName="px-4 pb-6 gap-4">
          <Text className="text-[13px] leading-[19px] text-muted-foreground">
            Incoming webhooks let tools such as CI, monitoring or ticketing post messages to #{conversation?.name ?? 'this channel'}. Each one has a secret URL that takes a JSON message.
          </Text>
          {shown ? <WebhookUrl created={shown} onDone={() => setShown(null)} /> : null}
          {error ? <Note error>{errorMessage(error)}</Note> : null}
          {list.isPending ? <LoadingView /> : null}
          {list.isError ? <Note error>Could not load the webhooks. {errorMessage(list.error)}</Note> : null}
          {list.isSuccess && list.data.length === 0 ? (
            <View className="items-start gap-3 rounded-lg border border-dashed border-border p-4">
              <Text className="text-[13px] text-muted-foreground">No incoming webhooks yet.</Text>
              <Button label="New webhook" size="sm" icon="plus" onPress={() => setForm({})} />
            </View>
          ) : null}
          {(list.data ?? []).map((webhook) => (
            <View key={webhook.id} className="gap-2">
              <View className="flex-row items-center gap-3">
                <BotAvatar bot={webhook} size={32} />
                <View className="flex-1 min-w-0">
                  <Text numberOfLines={1} className="font-noto-medium text-[14px]">{webhook.name}</Text>
                  <Text numberOfLines={2} className="text-[12px] text-muted-foreground">
                    Made by {nameOf(webhook.created_by)} · {webhook.last_used_at ? `last used ${relativeAgo(webhook.last_used_at)}` : 'never used'}
                    {webhook.request_url ? ' · buttons on' : ''}
                    {webhook.events ? ' · events on' : ''}
                    {webhook.can_read ? ' · can read' : ''}
                  </Text>
                </View>
                <IconButton icon="more" label={`Options for ${webhook.name}`} size={32} iconSize={18} color={colors.mutedForeground} onPress={() => setMenu(options(webhook))} />
              </View>
              {webhook.messages_url ? <CopyField label="Read URL" value={webhook.messages_url} /> : null}
            </View>
          ))}
        </ScrollView>
        <ActionSheet visible={menu !== null} actions={menu ?? []} onClose={() => setMenu(null)} />
        {form ? (
          <WebhookForm
            webhook={form.webhook}
            busy={write.isPending || create.isPending}
            onClose={() => setForm(null)}
            onSave={(name, iconUrl) => {
              if (form.webhook) change(form.webhook, { kind: 'update', name, iconUrl })
              else create.mutate({ name, iconUrl })
              setForm(null)
            }}
          />
        ) : null}
        {buttonsOf ? <ButtonsSheet conversationId={id} webhook={(list.data ?? []).find((item) => item.id === buttonsOf.id) ?? buttonsOf} onChange={(next) => change(buttonsOf, next)} onClose={() => setButtonsOf(null)} /> : null}
      </ChatPane>
    </ChatColorScope>
  )
}

/** A read-only value with Copy (a URL or a secret). */
function CopyField({ label, value }: { label: string; value: string }) {
  const [copied, setCopied] = useState(false)
  return (
    <View className="gap-1">
      <Text className="font-noto-medium text-[12px] text-muted-foreground">{label}</Text>
      <Text selectable className="font-mono text-[12px] leading-[17px] rounded-md bg-muted px-2.5 py-2">
        {value}
      </Text>
      <View className="items-end">
        <Button label={copied ? 'Copied' : 'Copy'} size="sm" variant="outline" icon="copy" onPress={() => void Clipboard.setStringAsync(value).then(() => setCopied(true))} />
      </View>
    </View>
  )
}

/** The URL of a webhook that was just made or rotated, with a curl example. It does not show again. */
function WebhookUrl({ created, onDone }: { created: chat.ChatWebhookWithUrl; onDone: () => void }) {
  return (
    <View className="gap-3 rounded-lg border border-border p-3">
      <CopyField label={`URL of ${created.webhook.name}`} value={created.url} />
      <Note>Copy it now: Orbit shows this URL only once. Anyone who has it can post to this channel.</Note>
      <CopyField label="Try it" value={chat.curlExample(created.url)} />
      <View className="items-end">
        <Button label="Done" size="sm" variant="outline" onPress={onDone} />
      </View>
    </View>
  )
}

function WebhookForm({ webhook, busy, onClose, onSave }: { webhook?: chat.WebhookRecord; busy: boolean; onClose: () => void; onSave: (name: string, iconUrl: string | null) => void }) {
  const [name, setName] = useState(webhook?.name ?? '')
  const [iconUrl, setIconUrl] = useState(webhook?.icon_url ?? '')
  const iconInvalid = iconUrl.trim() !== '' && !iconUrl.trim().startsWith('https://')
  return (
    <FormSheet
      visible
      title={webhook ? 'Edit webhook' : 'New webhook'}
      submitLabel={webhook ? 'Save' : 'Create webhook'}
      canSubmit={Boolean(name.trim()) && !iconInvalid}
      busy={busy}
      onSubmit={() => onSave(name.trim(), iconUrl.trim() || null)}
      onClose={onClose}
    >
      <Input label="Name" required autoFocus maxLength={80} placeholder="CI" value={name} onChangeText={setName} />
      <Input label="Icon URL (optional)" placeholder="https://example.com/icon.png" keyboardType="url" autoCapitalize="none" autoCorrect={false} value={iconUrl} onChangeText={setIconUrl} />
      {iconInvalid ? <Note error>Use an https:// image URL.</Note> : null}
    </FormSheet>
  )
}

/**
 * Where clicks on the webhook's buttons (and, with events on, members' messages) go, the secret that signs them, and
 * the last deliveries with their answers.
 */
function ButtonsSheet({ conversationId, webhook, onChange, onClose }: { conversationId: string; webhook: chat.WebhookRecord; onChange: (next: chat.WebhookWrite) => void; onClose: () => void }) {
  const { client, workspaceId } = useOrbit()
  const [requestUrl, setRequestUrl] = useState(webhook.request_url ?? '')
  const [secret, setSecret] = useState<string | null>(null)
  const [secretError, setSecretError] = useState<string | null>(null)
  const deliveries = useQuery({
    queryKey: ['conversations', workspaceId, conversationId, 'webhooks', webhook.id, 'deliveries'],
    queryFn: () => chat.webhookDeliveries(client, workspaceId, conversationId, webhook.id),
    enabled: webhook.request_url !== null,
  })
  const loadSecret = async (rotate: boolean) => {
    if (rotate && !(await confirmAction('Make a new signing secret?', 'Clicks are signed with the new secret at once. The receiver must check them with it.', 'Make new secret'))) return
    setSecretError(null)
    chat.webhookSigningSecret(client, workspaceId, conversationId, webhook.id, rotate).then(setSecret, (error: unknown) => setSecretError(errorMessage(error)))
  }
  const changed = requestUrl.trim() !== (webhook.request_url ?? '')
  return (
    <FormSheet visible title={`${webhook.name}: buttons and events`} onClose={onClose}>
      <Input label="Request URL" placeholder="https://example.com/slack/actions" keyboardType="url" autoCapitalize="none" autoCorrect={false} value={requestUrl} onChangeText={setRequestUrl} />
      <Text className="text-[12px] leading-[17px] text-muted-foreground">A click on a button without a link sends a signed block_actions payload here, as Slack does.</Text>
      <View className="flex-row gap-2 justify-end">
        {webhook.request_url ? <Button label="Remove request URL" variant="ghost" onPress={() => onChange({ kind: 'interactivity', requestUrl: null, events: false })} /> : null}
        <Button label="Save" disabled={!requestUrl.trim() || !changed} onPress={() => onChange({ kind: 'interactivity', requestUrl: requestUrl.trim(), events: webhook.events })} />
      </View>
      {webhook.request_url ? (
        <>
          <View className="flex-row items-center gap-3">
            <Text className="flex-1 font-noto-medium text-[14px]">Send messages as events</Text>
            <Toggle value={webhook.events} label="Send messages as events" onChange={(on) => onChange({ kind: 'interactivity', requestUrl: webhook.request_url, events: on })} />
          </View>
          {secret ? <CopyField label="Signing secret" value={secret} /> : null}
          <View className="flex-row gap-2">
            {secret ? null : <Button label="Reveal signing secret" variant="outline" onPress={() => void loadSecret(false)} />}
            <Button label="Make new secret" variant="outline" icon="key" onPress={() => void loadSecret(true)} />
          </View>
          {secretError ? <Note error>Could not read the signing secret. {secretError}</Note> : null}
          <Text className="font-noto-medium text-[14px]">Last deliveries</Text>
          {(deliveries.data ?? []).length === 0 ? <Note>Nothing sent yet.</Note> : null}
          {(deliveries.data ?? []).map((delivery) => (
            <View key={delivery.id} className="flex-row items-center gap-2">
              <Text className={`text-[12px] ${delivery.status !== null && delivery.status >= 200 && delivery.status < 300 ? 'text-emerald-500' : 'text-destructive'}`}>{delivery.status ?? delivery.error}</Text>
              <Text numberOfLines={1} className="flex-1 font-mono text-[12px]">{delivery.action_id}</Text>
              <Text className="text-[12px] text-muted-foreground">{delivery.duration_ms} ms · {relativeAgo(delivery.created_at)}</Text>
            </View>
          ))}
        </>
      ) : null}
    </FormSheet>
  )
}
