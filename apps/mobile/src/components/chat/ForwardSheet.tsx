import { useQueryClient } from '@tanstack/react-query'
import { useRef, useState } from 'react'
import { Pressable, View } from 'react-native'

import { applyChatEvents } from '../../chat-cache'
import { conversationTitle, newNonce, type Person } from '../../data/chat'
import * as chat from '../../data/chat-api'
import { FORWARD_LIMIT, forwardDestinations, forwardToEach, toggleDestination } from '../../data/forward'
import { encodeMentions } from '../../data/mentions'
import type { ConversationRecord, MessageRecord } from '../../data/orbit'
import { useOrbit } from '../../orbit-context'
import { useConversations, usePeople } from '../../queries'
import { errorMessage } from '../../session-context'
import { useTheme } from '../../theme'
import { Icon, Input, Text } from '../../ui'
import { ConversationGlyph } from './Pane'
import { FormSheet } from './Sheet'

/**
 * The web's Forward dialog as a page sheet: the message, up to five conversations (channels the reader is in and DMs)
 * with a search field, and an optional message of the reader sent to each one right after the forward.
 */
export function ForwardSheet({ message, preview, onClose }: { message: MessageRecord; preview: string; onClose: () => void }) {
  const { client, workspaceId, user } = useOrbit()
  const queryClient = useQueryClient()
  const { colors } = useTheme()
  const people = usePeople()
  const conversations = useConversations().data ?? []
  const [search, setSearch] = useState('')
  const [selectedIds, setSelectedIds] = useState<string[]>([])
  const [comment, setComment] = useState('')
  const [pending, setPending] = useState(false)
  const [error, setError] = useState<string | null>(null)
  // one nonce per destination: a forward tried again after a failure returns the first copy, not a second one
  const nonces = useRef(new Map<string, string>())
  const nonceFor = (conversationId: string) => {
    const nonce = nonces.current.get(conversationId) ?? newNonce()
    nonces.current.set(conversationId, nonce)
    return nonce
  }
  const titleOf = (conversation: ConversationRecord) => (conversation.kind === 'dm' ? conversationTitle(conversation, people, user.id) : `#${conversation.name}`)
  const destinations = forwardDestinations(conversations, user.id, titleOf, search)
  const channels: Person[] = conversations.filter((item) => item.kind !== 'dm').map((item) => ({ id: item.id, name: item.name }))

  const submit = async () => {
    if (selectedIds.length === 0 || pending) return
    const body = encodeMentions(comment.trim(), people, channels)
    setPending(true)
    setError(null)
    const result = await forwardToEach(
      selectedIds,
      async (conversationId) => applyChatEvents(queryClient, workspaceId, (await chat.forward(client, workspaceId, message.id, conversationId, nonceFor(conversationId))).events),
      body ? async (conversationId) => applyChatEvents(queryClient, workspaceId, (await chat.send(client, workspaceId, conversationId, { body, nonce: newNonce() })).events) : undefined,
    )
    setPending(false)
    if (result.sentIds.length === selectedIds.length) return onClose()
    // the failed destination and those after it stay selected, so Forward tries them again
    const failed = conversations.find((item) => item.id === selectedIds[result.sentIds.length])
    setSelectedIds(selectedIds.filter((id) => !result.sentIds.includes(id)))
    setError(`${failed ? `${titleOf(failed)}: ` : ''}${errorMessage(result.error)}`)
  }

  return (
    <FormSheet
      visible
      title="Forward message"
      submitLabel="Forward"
      canSubmit={selectedIds.length > 0}
      busy={pending}
      onSubmit={() => void submit()}
      onClose={() => (pending ? undefined : onClose())}
    >
      <View className="border-l-4 border-muted-foreground/40 pl-3">
        <Text numberOfLines={4} className="text-[14px] leading-[20px] text-foreground/85">
          {preview || (message.attachments.length > 0 ? `${message.attachments.length} ${message.attachments.length === 1 ? 'file' : 'files'}` : message.sticker ? 'Sticker' : '')}
        </Text>
      </View>
      <Input placeholder="Search conversations" value={search} onChangeText={setSearch} autoCapitalize="none" autoCorrect={false} accessibilityLabel="Search conversations" />
      <View>
        {destinations.length === 0 ? <Text className="text-[14px] p-2 text-muted-foreground">No conversations found.</Text> : null}
        {destinations.map((conversation) => {
          const checked = selectedIds.includes(conversation.id)
          const full = !checked && selectedIds.length >= FORWARD_LIMIT
          const title = titleOf(conversation)
          return (
            <Pressable
              key={conversation.id}
              accessibilityRole="checkbox"
              accessibilityState={{ checked, disabled: full || pending }}
              accessibilityLabel={title}
              disabled={full || pending}
              onPress={() => setSelectedIds(toggleDestination(selectedIds, conversation.id))}
              className={`h-11 flex-row items-center gap-2.5 px-2 rounded-md active:bg-muted/80 ${full ? 'opacity-50' : 'opacity-100'}`}
            >
              <View className="w-6 items-center">
                <ConversationGlyph conversation={conversation} title={title} userId={user.id} size={18} />
              </View>
              <Text numberOfLines={1} className="text-[14px] flex-1">
                {title.replace(/^#/, '')}
              </Text>
              <View className={`size-4.5 rounded-[5px] border items-center justify-center ${checked ? 'border-primary bg-primary' : 'border-input'}`}>
                {checked ? <Icon name="check" size={12} color={colors.primaryForeground} /> : null}
              </View>
            </Pressable>
          )
        })}
      </View>
      <Text className="text-[12px] text-muted-foreground">
        {selectedIds.length} of {FORWARD_LIMIT} selected
      </Text>
      <Input label="Add a message (optional)" value={comment} onChangeText={setComment} multiline accessibilityLabel="Add a message" />
      {error ? (
        <Text accessibilityRole="alert" className="text-[13px] text-destructive">
          {error}
        </Text>
      ) : null}
    </FormSheet>
  )
}
