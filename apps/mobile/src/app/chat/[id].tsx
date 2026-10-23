import { useQueryClient } from '@tanstack/react-query'
import { applyChatEvents } from '../../chat-cache'
import { router, useLocalSearchParams } from 'expo-router'
import { useEffect, useMemo, useState } from 'react'
import { KeyboardAvoidingView, Platform, View } from 'react-native'
import { SafeAreaView } from 'react-native-safe-area-context'

import { AddPeopleSheet } from '../../components/chat/ChannelSheets'
import { Composer, useComposer } from '../../components/chat/Composer'
import { ConversationOptions } from '../../components/chat/ConversationOptions'
import { type ListRow, MessageList, useMessageSheet } from '../../components/chat/MessageList'
import { buildMessageRows, conversationPermissions, conversationTitle } from '../../data/chat'
import * as chat from '../../data/chat-api'
import { fullDate, timeOfDay } from '../../data/format'
import type { ConversationRecord } from '../../data/orbit'
import { useChatWrite, useIsChatAdmin, useMessages } from '../../hooks/chat'
import { useOrbit } from '../../orbit-context'
import { useChatStates, useConversations, usePeople } from '../../queries'
import { alpha, type, useTheme } from '../../theme'
import { Avatar, Button, ErrorView, Icon, IconButton, LoadingView, Text } from '../../ui'

/**
 * A conversation on a phone, painted on the card surface like the web. `?message=` opens it around that message (a
 * search result) with the message marked; "Jump to present" goes back to the newest.
 */
export default function ConversationScreen() {
  const { id, message: around } = useLocalSearchParams<{ id: string; message?: string }>()
  const { client, workspaceId, user } = useOrbit()
  const queryClient = useQueryClient()
  const { colors } = useTheme()
  const people = usePeople()
  const conversations = useConversations()
  const states = useChatStates()
  const admin = useIsChatAdmin()
  const run = useChatWrite()
  const [options, setOptions] = useState(false)
  const [addPeople, setAddPeople] = useState(false)
  /** After "Mark as unread" the open conversation stops reading itself until a new message arrives. */
  const [readPausedAt, setReadPausedAt] = useState<string | null>(null)

  const messages = useMessages(id, around)
  const composer = useComposer(id, null)
  const conversation = conversations.data?.find((item) => item.id === id)
  const loaded = useMemo(() => messages.data?.pages.flatMap((page) => page.items) ?? [], [messages.data])
  const { sheet, handlers } = useMessageSheet({ inThread: false, composer, loaded, onMarkedUnread: () => setReadPausedAt(loaded.at(-1)?.id ?? '') })
  const rows: ListRow[] = useMemo(
    () => [
      ...composer.pending
        // the live echo can arrive before the response: then the message is already in the list
        .filter((item) => !loaded.some((message) => message.nonce === item.nonce))
        .map((item): ListRow => ({ type: 'pending', key: item.nonce, pending: item }))
        .reverse(),
      ...buildMessageRows(loaded).reverse(),
    ],
    [loaded, composer.pending],
  )

  // Read up to the newest message when the conversation opens and whenever a new one arrives while it is open.
  const atPresent = messages.data !== undefined && !messages.hasNextPage
  const newestId = atPresent ? loaded.at(-1)?.id : undefined
  const lastReadId = states.data?.find((state) => state.conversation_id === id)?.last_read_message_id
  useEffect(() => {
    if (!newestId || newestId === lastReadId || newestId === readPausedAt) return
    chat.markRead(client, workspaceId, id).then(
      ({ events }) => applyChatEvents(queryClient, workspaceId, events),
      () => undefined,
    )
  }, [client, id, lastReadId, newestId, queryClient, readPausedAt, workspaceId])

  const name = conversation ? conversationTitle(conversation, people, user.id) : ''
  const dmPeer = conversation?.kind === 'dm' ? (conversation.member_ids.find((member) => member !== user.id) ?? user.id) : null
  const placeholder = conversation ? `Message ${conversation.kind === 'dm' ? name : `#${name}`}` : 'Message'
  const member = conversation?.member_ids.includes(user.id) ?? false
  const permissions = conversation ? conversationPermissions(conversation, user.id, admin) : null
  const back = () => (router.canGoBack() ? router.back() : router.navigate('/chat'))

  return (
    <SafeAreaView edges={['top', 'left', 'right', 'bottom']} style={{ flex: 1, backgroundColor: colors.card }}>
      <View style={{ height: 48, flexDirection: 'row', alignItems: 'center', gap: 4, paddingHorizontal: 8 }}>
        <IconButton icon="back" label="Back to chat" size={32} iconSize={20} color={colors.mutedForeground} onPress={back} />
        <View style={{ flex: 1, flexDirection: 'row', alignItems: 'center', gap: 8, minWidth: 0 }}>
          {dmPeer ? <Avatar id={dmPeer} name={name} size={20} /> : <Icon name={conversation?.kind === 'private' ? 'lock' : 'hashtag'} size={18} color={colors.mutedForeground} />}
          <Text numberOfLines={1} style={[type(14, 'semibold'), { flexShrink: 1 }]}>
            {name}
          </Text>
        </View>
        <View style={{ flexDirection: 'row', gap: 2 }}>
          <IconButton icon="search" label="Search" size={32} iconSize={20} color={colors.mutedForeground} onPress={() => router.push(`/chat/${id}/search`)} />
          <IconButton icon="people" label="Members" size={32} iconSize={20} color={colors.mutedForeground} onPress={() => router.push(`/chat/${id}/members`)} />
          <IconButton icon="more" label={`Options for ${name}`} size={32} iconSize={20} color={colors.mutedForeground} disabled={!conversation} onPress={() => setOptions(true)} />
        </View>
      </View>
      <KeyboardAvoidingView style={{ flex: 1 }} behavior={Platform.OS === 'ios' ? 'padding' : undefined}>
        {!messages.data ? (
          messages.isError ? (
            <ErrorView error={messages.error} onRetry={() => void messages.refetch()} />
          ) : (
            <LoadingView />
          )
        ) : (
          <MessageList
            rows={rows}
            handlers={handlers}
            highlightedId={around}
            loadingOlder={messages.isFetchingPreviousPage}
            loadingNewer={messages.isFetchingNextPage}
            onOlder={() => {
              if (messages.hasPreviousPage && !messages.isFetchingPreviousPage) void messages.fetchPreviousPage()
            }}
            onNewer={() => {
              if (messages.hasNextPage && !messages.isFetchingNextPage) void messages.fetchNextPage()
            }}
            start={
              !messages.hasPreviousPage && conversation ? (
                <ConversationStart conversation={conversation} title={name} creator={handlers.nameOf(conversation.created_by)} onAddPeople={permissions?.canAddPeople ? () => setAddPeople(true) : undefined} />
              ) : undefined
            }
          />
        )}
        {around ? (
          <View style={{ alignItems: 'center', paddingTop: 6 }}>
            <Button label="Jump to present" size="sm" variant="secondary" icon="arrowDown" onPress={() => router.setParams({ message: undefined })} />
          </View>
        ) : null}
        {!conversation ? null : conversation.archived ? (
          <Notice text="This channel is archived. Its messages stay searchable." />
        ) : member ? (
          <Composer state={composer} placeholder={placeholder} nameOf={handlers.nameOf} />
        ) : (
          <View style={{ margin: 8, padding: 12, gap: 8, alignItems: 'center', borderRadius: 10, borderWidth: 1, borderColor: colors.border }}>
            <Text style={[type(14), { color: colors.mutedForeground }]}>You are viewing #{conversation.name}.</Text>
            <Button label="Join channel" onPress={() => void run('Could not join the channel', (client, workspaceId) => chat.joinChannel(client, workspaceId, conversation.id))} />
          </View>
        )}
      </KeyboardAvoidingView>
      {sheet}
      {conversation ? <ConversationOptions conversation={conversation} title={name} placement="header" visible={options} onClose={() => setOptions(false)} /> : null}
      {addPeople && conversation ? <AddPeopleSheet conversation={conversation} onClose={() => setAddPeople(false)} /> : null}
    </SafeAreaView>
  )
}

function Notice({ text }: { text: string }) {
  const { colors } = useTheme()
  return (
    <View style={{ margin: 8, padding: 12, borderRadius: 10, backgroundColor: colors.muted }}>
      <Text style={[type(13), { textAlign: 'center', color: colors.mutedForeground }]}>{text}</Text>
    </View>
  )
}

/** The block above the first message: who made the channel and when, and "Add people" for those who may. */
function ConversationStart({ conversation, title, creator, onAddPeople }: { conversation: ConversationRecord; title: string; creator: string; onAddPeople?: () => void }) {
  const { colors } = useTheme()
  if (conversation.kind === 'dm') {
    return (
      <View style={{ paddingHorizontal: 12, paddingTop: 32, paddingBottom: 16 }}>
        <Text style={[type(16, 'semibold'), { lineHeight: 24 }]}>{conversation.self_dm ? 'This is your space' : `This is the start of your conversation with ${title}`}</Text>
      </View>
    )
  }
  return (
    <View style={{ paddingHorizontal: 12, paddingTop: 32, paddingBottom: 16, gap: 4, alignItems: 'flex-start' }}>
      <Text style={[type(16, 'semibold'), { lineHeight: 24 }]}>This is the start of #{title}</Text>
      {conversation.topic ? <Text style={[type(14), { color: colors.mutedForeground }]}>{conversation.topic}</Text> : null}
      <Text style={[type(12), { lineHeight: 16, color: colors.mutedForeground }]}>
        Created by <Text style={[type(12, 'medium'), { lineHeight: 16, color: alpha(colors.foreground, 0.85) }]}>{creator}</Text> on {fullDate(conversation.created_at)}, {timeOfDay(conversation.created_at)}
      </Text>
      {onAddPeople ? (
        <View style={{ marginTop: 8 }}>
          <Button label="Add people" variant="outline" size="sm" icon="userAdd" onPress={onAddPeople} />
        </View>
      ) : null}
    </View>
  )
}
