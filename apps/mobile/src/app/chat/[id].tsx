import { useInfiniteQuery, useQueryClient } from '@tanstack/react-query'
import { Stack, useLocalSearchParams } from 'expo-router'
import { useHeaderHeight } from 'expo-router/react-navigation'
import { useEffect, useMemo, useState } from 'react'
import { Alert, FlatList, KeyboardAvoidingView, Platform, View } from 'react-native'

import { Composer } from '../../components/Composer'
import { buildMessageRows, conversationTitle, decodeMentions, type MessageRow, newNonce, type Person } from '../../data/chat'
import { keys } from '../../data/keys'
import { markdownToText } from '../../data/markdown'
import * as api from '../../data/orbit'
import { applyChatActions } from '../../live'
import { useOrbit } from '../../orbit-context'
import { useChatStates, useConversations, usePeople } from '../../queries'
import { errorMessage } from '../../session-context'
import { fonts, useTheme } from '../../theme'
import { dayLabel, shortTime } from '../../time'
import { Avatar, ErrorView, LoadingView, Spinner, Text } from '../../ui'

/** A message the server has not confirmed yet. */
interface Pending {
  nonce: string
  body: string
}

type Row = MessageRow | { type: 'pending'; key: string; pending: Pending }

export default function ConversationScreen() {
  const { id } = useLocalSearchParams<{ id: string }>()
  const { client, workspaceId, user } = useOrbit()
  const queryClient = useQueryClient()
  const { spacing } = useTheme()
  const headerHeight = useHeaderHeight()
  const people = usePeople()
  const conversations = useConversations()
  const states = useChatStates()
  const [draft, setDraft] = useState('')
  const [pending, setPending] = useState<Pending[]>([])

  const messages = useInfiniteQuery({
    queryKey: keys.messages(workspaceId, id),
    initialPageParam: undefined as string | undefined,
    queryFn: ({ pageParam }) => api.messages(client, workspaceId, id, pageParam),
    getPreviousPageParam: (first) => first.before ?? undefined,
    getNextPageParam: () => undefined,
  })

  const conversation = conversations.data?.find((item) => item.id === id)
  const channelNames: Person[] = useMemo(() => (conversations.data ?? []).map((item) => ({ id: item.id, name: item.name })), [conversations.data])
  const loaded = useMemo(() => messages.data?.pages.flatMap((page) => page.items) ?? [], [messages.data])
  const rows: Row[] = useMemo(
    () => [
      ...pending
        // the live echo can arrive before the response: then the message is already in the list
        .filter((item) => !loaded.some((message) => message.nonce === item.nonce))
        .map((item): Row => ({ type: 'pending', key: item.nonce, pending: item }))
        .reverse(),
      ...buildMessageRows(loaded).reverse(),
    ],
    [loaded, pending],
  )

  // Read up to the newest message when the conversation opens and whenever a new one arrives while it is open.
  const newestId = loaded.at(-1)?.id
  const lastReadId = states.data?.find((state) => state.conversation_id === id)?.last_read_message_id
  useEffect(() => {
    if (!newestId || newestId === lastReadId) return
    api.markConversationRead(client, workspaceId, id).then(
      (state) => applyChatActions(queryClient, workspaceId, [{ kind: 'state', state }]),
      () => undefined,
    )
  }, [client, id, lastReadId, newestId, queryClient, workspaceId])

  const send = () => {
    const body = draft.trim()
    if (!body) return
    const item = { nonce: newNonce(), body }
    setDraft('')
    setPending((list) => [...list, item])
    api.sendMessage(client, workspaceId, id, body, item.nonce).then(
      (message) => {
        applyChatActions(queryClient, workspaceId, [{ kind: 'upsert-message', conversationId: id, message }])
        setPending((list) => list.filter((other) => other.nonce !== item.nonce))
      },
      (error) => {
        setPending((list) => list.filter((other) => other.nonce !== item.nonce))
        setDraft((current) => current || body)
        Alert.alert('Message not sent', errorMessage(error))
      },
    )
  }

  const title = conversation ? `${conversation.kind === 'dm' ? '' : '#'}${conversationTitle(conversation, people, user.id)}` : ''
  const text = (body: string) => markdownToText(decodeMentions(body, people, channelNames))

  return (
    <KeyboardAvoidingView style={{ flex: 1 }} behavior={Platform.OS === 'ios' ? 'padding' : undefined} keyboardVerticalOffset={headerHeight}>
      <Stack.Screen options={{ title }} />
      {!messages.data ? (
        messages.isError ? (
          <ErrorView error={messages.error} onRetry={() => void messages.refetch()} />
        ) : (
          <LoadingView />
        )
      ) : (
        <FlatList
          inverted
          data={rows}
          keyExtractor={(row) => row.key}
          keyboardShouldPersistTaps="handled"
          keyboardDismissMode="interactive"
          contentContainerStyle={{ paddingVertical: spacing[3] }}
          onEndReached={() => {
            if (messages.hasPreviousPage && !messages.isFetchingPreviousPage) void messages.fetchPreviousPage()
          }}
          onEndReachedThreshold={0.5}
          ListFooterComponent={messages.isFetchingPreviousPage ? <Spinner /> : undefined}
          ListEmptyComponent={
            <View style={{ padding: spacing[6], transform: [{ scaleY: -1 }] }}>
              <Text color="muted" style={{ textAlign: 'center' }}>
                No messages yet. Say hello.
              </Text>
            </View>
          }
          renderItem={({ item }) => {
            if (item.type === 'day') {
              return (
                <Text variant="caption" color="muted" style={{ textAlign: 'center', paddingVertical: spacing[2], fontFamily: fonts.medium }}>
                  {dayLabel(item.at)}
                </Text>
              )
            }
            if (item.type === 'pending') {
              return (
                <View style={{ paddingLeft: spacing[4] + 36 + spacing[2.5], paddingRight: spacing[4], paddingVertical: 2, opacity: 0.5 }}>
                  <Text>{text(item.pending.body)}</Text>
                </View>
              )
            }
            const { message, groupStart } = item
            const author = people.find((person) => person.id === message.author_id)?.name ?? 'Unknown'
            if (message.kind !== 'message') {
              const what = message.kind === 'join' ? 'joined' : message.kind === 'leave' ? 'left' : 'pinned a message'
              return (
                <Text variant="caption" color="muted" style={{ paddingHorizontal: spacing[4], paddingVertical: spacing[1] }}>
                  {author} {what} · {shortTime(message.created_at)}
                </Text>
              )
            }
            const body = message.deleted ? 'Message deleted.' : text(message.body)
            const extras = [
              message.sticker ? 'Sticker' : null,
              message.attachments.length > 0 ? `${message.attachments.length} attachment${message.attachments.length === 1 ? '' : 's'}` : null,
              message.reply_count > 0 ? `${message.reply_count} repl${message.reply_count === 1 ? 'y' : 'ies'}` : null,
              message.edited_at ? 'edited' : null,
            ].filter(Boolean)
            return (
              <View style={{ flexDirection: 'row', gap: spacing[2.5], paddingHorizontal: spacing[4], paddingTop: groupStart ? spacing[2.5] : 2 }}>
                <View style={{ width: 36 }}>{groupStart ? <Avatar id={message.author_id} name={author} size={36} /> : null}</View>
                <View style={{ flex: 1, gap: 2 }}>
                  {groupStart ? (
                    <View style={{ flexDirection: 'row', alignItems: 'baseline', gap: spacing[2] }}>
                      <Text variant="label" numberOfLines={1} style={{ flexShrink: 1 }}>
                        {author}
                      </Text>
                      <Text variant="caption" color="muted">
                        {shortTime(message.created_at)}
                      </Text>
                    </View>
                  ) : null}
                  {body ? (
                    <Text selectable color={message.deleted ? 'muted' : 'default'}>
                      {body}
                    </Text>
                  ) : null}
                  {extras.length > 0 ? (
                    <Text variant="caption" color="muted">
                      {extras.join(' · ')}
                    </Text>
                  ) : null}
                </View>
              </View>
            )
          }}
        />
      )}
      <Composer value={draft} onChangeText={setDraft} onSend={send} placeholder={conversation ? `Message ${title}` : 'Message'} />
    </KeyboardAvoidingView>
  )
}
