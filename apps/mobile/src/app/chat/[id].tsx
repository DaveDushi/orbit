import { useInfiniteQuery, useQueryClient } from '@tanstack/react-query'
import { router, useLocalSearchParams } from 'expo-router'
import { useEffect, useMemo, useState } from 'react'
import { Alert, FlatList, KeyboardAvoidingView, Platform, Pressable, TextInput, View } from 'react-native'
import { SafeAreaView } from 'react-native-safe-area-context'

import { buildMessageRows, conversationTitle, decodeMentions, type MessageRow, newNonce, type Person } from '../../data/chat'
import { dayLabel, fullDate, relativeAgo, timeOfDay } from '../../data/format'
import { keys } from '../../data/keys'
import { markdownToText } from '../../data/markdown'
import * as api from '../../data/orbit'
import { applyChatActions } from '../../live'
import { useOrbit } from '../../orbit-context'
import { useChatStates, useConversations, usePeople } from '../../queries'
import { errorMessage } from '../../session-context'
import { useOpenOnWeb } from '../../web'
import { alpha, type, useTheme } from '../../theme'
import { Avatar, Button, ErrorView, Icon, IconButton, LoadingView, Spinner, Text } from '../../ui'

/** A message the server has not confirmed yet. */
interface Pending {
  nonce: string
  body: string
}

type Row = MessageRow | { type: 'pending'; key: string; pending: Pending }

/** On a phone the web paints the whole conversation on the card surface (its chat `--background`). */
export default function ConversationScreen() {
  const { id } = useLocalSearchParams<{ id: string }>()
  const { client, workspaceId, user } = useOrbit()
  const queryClient = useQueryClient()
  const { colors } = useTheme()
  const people = usePeople()
  const conversations = useConversations()
  const states = useChatStates()
  const [draft, setDraft] = useState('')
  const [pending, setPending] = useState<Pending[]>([])
  const openOnWeb = useOpenOnWeb()
  const onWeb = () => openOnWeb(`/chat/${id}`)

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

  const name = conversation ? conversationTitle(conversation, people, user.id) : ''
  const dmPeer = conversation?.kind === 'dm' ? (conversation.member_ids.find((member) => member !== user.id) ?? user.id) : null
  const placeholder = conversation ? `Message ${conversation.kind === 'dm' ? name : `#${name}`}` : 'Message'
  const text = (body: string) => markdownToText(decodeMentions(body, people, channelNames))
  const nameOf = (userId: string) => people.find((person) => person.id === userId)?.name ?? 'Unknown'

  return (
    <SafeAreaView edges={['top', 'left', 'right', 'bottom']} style={{ flex: 1, backgroundColor: colors.card }}>
      <View style={{ height: 48, flexDirection: 'row', alignItems: 'center', gap: 4, paddingHorizontal: 8 }}>
        <IconButton icon="back" label="Back to chat" size={32} iconSize={20} color={colors.mutedForeground} onPress={() => (router.canGoBack() ? router.back() : router.navigate('/chat'))} />
        <View style={{ flex: 1, flexDirection: 'row', alignItems: 'center', gap: 8, minWidth: 0 }}>
          {dmPeer ? <Avatar id={dmPeer} name={name} size={20} /> : <Icon name={conversation?.kind === 'private' ? 'lock' : 'hashtag'} size={18} color={colors.mutedForeground} />}
          <Text numberOfLines={1} style={[type(14, 'semibold'), { flexShrink: 1 }]}>
            {name}
          </Text>
        </View>
        <View style={{ flexDirection: 'row', gap: 2 }}>
          <IconButton icon="search" label="Search" size={32} color={colors.mutedForeground} onPress={onWeb} />
          <IconButton icon="people" label="Members" size={32} color={colors.mutedForeground} onPress={onWeb} />
          <IconButton icon="more" label={`Options for ${name}`} size={32} color={colors.mutedForeground} onPress={onWeb} />
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
          <FlatList
            inverted
            data={rows}
            keyExtractor={(row) => row.key}
            keyboardShouldPersistTaps="handled"
            keyboardDismissMode="interactive"
            contentContainerStyle={{ paddingTop: 20 }}
            onEndReached={() => {
              if (messages.hasPreviousPage && !messages.isFetchingPreviousPage) void messages.fetchPreviousPage()
            }}
            onEndReachedThreshold={0.5}
            ListFooterComponent={
              messages.hasPreviousPage ? (
                <View style={{ padding: 16 }}>{messages.isFetchingPreviousPage ? <Spinner /> : null}</View>
              ) : conversation ? (
                <ConversationStart conversation={conversation} title={name} creator={nameOf(conversation.created_by)} isDm={conversation.kind === 'dm'} onAddPeople={onWeb} />
              ) : undefined
            }
            renderItem={({ item }) => {
              if (item.type === 'day') return <DayChip label={dayLabel(item.at)} />
              if (item.type === 'pending') {
                return (
                  <View style={{ paddingLeft: 12 + 30 + 8, paddingRight: 12, paddingVertical: 2, opacity: 0.5 }}>
                    <Text style={[type(14), { lineHeight: 21 }]}>{text(item.pending.body)}</Text>
                  </View>
                )
              }
              const { message, groupStart } = item
              const author = nameOf(message.author_id)
              if (message.kind !== 'message') {
                const what = message.kind === 'join' ? 'joined' : message.kind === 'leave' ? 'left' : 'pinned a message'
                return (
                  <Text style={[type(12), { lineHeight: 16, paddingHorizontal: 12, paddingVertical: 4, color: colors.mutedForeground }]}>
                    {author} {what} · {timeOfDay(message.created_at)}
                  </Text>
                )
              }
              const body = message.deleted ? 'Message deleted.' : text(message.body)
              const extras = [
                message.sticker ? 'Sticker' : null,
                message.attachments.length > 0 ? `${message.attachments.length} attachment${message.attachments.length === 1 ? '' : 's'}` : null,
                message.edited_at ? '(edited)' : null,
              ].filter(Boolean)
              return (
                <View style={{ flexDirection: 'row', gap: 8, paddingHorizontal: 12, paddingTop: groupStart ? 4 : 2, paddingBottom: 2, marginTop: groupStart ? 8 : 0 }}>
                  <View style={{ width: 30, paddingTop: 2 }}>{groupStart ? <Avatar id={message.author_id} name={author} size={30} /> : null}</View>
                  <View style={{ flex: 1, minWidth: 0 }}>
                    {groupStart ? (
                      <View style={{ flexDirection: 'row', alignItems: 'baseline', gap: 8 }}>
                        <Text numberOfLines={1} style={[type(14, 'medium'), { lineHeight: 20, flexShrink: 1 }]}>
                          {author}
                        </Text>
                        <Text style={[type(12), { lineHeight: 16, color: colors.mutedForeground }]}>{timeOfDay(message.created_at)}</Text>
                      </View>
                    ) : null}
                    {body ? (
                      <Text selectable style={[type(14), { lineHeight: 21, color: message.deleted ? colors.mutedForeground : alpha(colors.foreground, 0.85) }]}>
                        {body}
                      </Text>
                    ) : null}
                    {extras.length > 0 ? <Text style={[type(12), { lineHeight: 16, color: colors.mutedForeground }]}>{extras.join(' · ')}</Text> : null}
                    {message.reply_count > 0 ? <ReplySummary message={message} nameOf={nameOf} text={text} /> : null}
                  </View>
                </View>
              )
            }}
          />
        )}
        <ChatComposer value={draft} onChangeText={setDraft} onSend={send} placeholder={placeholder} onWeb={onWeb} />
      </KeyboardAvoidingView>
    </SafeAreaView>
  )
}

function DayChip({ label }: { label: string }) {
  const { colors } = useTheme()
  return (
    <View accessibilityRole="header" accessibilityLabel={label} style={{ alignItems: 'center', paddingVertical: 8 }}>
      <View style={{ paddingHorizontal: 12, paddingVertical: 2, borderRadius: 999, borderWidth: 1, borderColor: colors.border, backgroundColor: colors.card }}>
        <Text style={[type(12, 'medium'), { lineHeight: 16, color: colors.mutedForeground }]}>{label}</Text>
      </View>
    </View>
  )
}

/** The block above the first message: who made the channel and when, and "Add people". */
function ConversationStart({ conversation, title, creator, isDm, onAddPeople }: { conversation: api.ConversationRecord; title: string; creator: string; isDm: boolean; onAddPeople: () => void }) {
  const { colors } = useTheme()
  if (isDm) {
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
      <View style={{ marginTop: 8 }}>
        <Button label="Add people" variant="outline" size="sm" onPress={onAddPeople} />
      </View>
    </View>
  )
}

/** "2 replies · last reply Oct 15" with the last reply's preview, behind a 2px guide line. */
function ReplySummary({ message, nameOf, text }: { message: api.MessageRecord; nameOf: (userId: string) => string; text: (body: string) => string }) {
  const { colors } = useTheme()
  const reply = message.last_reply
  return (
    <View style={{ marginTop: 6, marginLeft: -4, paddingVertical: 4, paddingLeft: 4, paddingRight: 8 }}>
      <View style={{ borderLeftWidth: 2, borderLeftColor: colors.border, paddingLeft: 10, gap: 4 }}>
        {reply ? (
          <View style={{ flexDirection: 'row', alignItems: 'center', gap: 6, minWidth: 0 }}>
            <Avatar id={reply.author_id} name={nameOf(reply.author_id)} size={16} />
            <Text numberOfLines={1} style={[type(12, 'medium'), { lineHeight: 16, flexShrink: 0, color: alpha(colors.foreground, 0.9) }]}>
              {nameOf(reply.author_id)}
            </Text>
            <Text numberOfLines={1} style={[type(13), { lineHeight: 17, flexShrink: 1, color: colors.mutedForeground }]}>
              {reply.sticker ? 'Sticker' : text(reply.body)}
            </Text>
          </View>
        ) : null}
        <View style={{ flexDirection: 'row', alignItems: 'center', gap: 6 }}>
          <Text style={[type(12, 'medium'), { lineHeight: 16, color: alpha(colors.foreground, 0.9) }]}>
            {message.reply_count} {message.reply_count === 1 ? 'reply' : 'replies'}
          </Text>
          {message.last_reply_at ? <Text style={[type(12), { lineHeight: 16, color: colors.mutedForeground }]}>last reply {relativeAgo(message.last_reply_at)}</Text> : null}
        </View>
      </View>
    </View>
  )
}

/** The web chat composer: the field on top, attach / emoji / mention on the left and send on the right, in one box. */
function ChatComposer({ value, onChangeText, onSend, placeholder, onWeb }: { value: string; onChangeText: (text: string) => void; onSend: () => void; placeholder: string; onWeb: () => void }) {
  const { colors, scheme } = useTheme()
  const [focused, setFocused] = useState(false)
  const canSend = value.trim().length > 0
  return (
    <View style={{ margin: 8, borderRadius: 10, borderWidth: 1, borderColor: focused ? colors.ring : colors.input, backgroundColor: alpha(colors.input, 0.3) }}>
      <TextInput
        value={value}
        onChangeText={onChangeText}
        onFocus={() => setFocused(true)}
        onBlur={() => setFocused(false)}
        placeholder={placeholder}
        placeholderTextColor={colors.mutedForeground}
        selectionColor={colors.primary}
        keyboardAppearance={scheme}
        multiline
        numberOfLines={1}
        accessibilityLabel={placeholder}
        style={{ minHeight: 40, maxHeight: 160, paddingHorizontal: 12, paddingTop: 8, paddingBottom: 8, fontSize: 16, lineHeight: 24, fontFamily: type(16).fontFamily, color: colors.foreground }}
      />
      <View style={{ flexDirection: 'row', alignItems: 'center', gap: 4, paddingHorizontal: 6, paddingTop: 6, paddingBottom: 6 }}>
        <IconButton icon="attach" label="Attach files" size={32} iconSize={20} onPress={onWeb} />
        <IconButton icon="smile" label="Emoji" size={32} iconSize={20} onPress={onWeb} />
        <IconButton icon="at" label="Mention" size={32} iconSize={20} onPress={() => onChangeText(`${value}${value && !value.endsWith(' ') ? ' ' : ''}@`)} />
        <View style={{ flex: 1 }} />
        <Pressable
          accessibilityRole="button"
          accessibilityLabel="Send"
          accessibilityState={{ disabled: !canSend }}
          disabled={!canSend}
          onPress={onSend}
          hitSlop={4}
          style={({ pressed }) => ({ width: 32, height: 32, borderRadius: 10, alignItems: 'center', justifyContent: 'center', opacity: canSend ? 1 : 0.5, backgroundColor: pressed ? colors.muted : 'transparent' })}
        >
          <Icon name="send" size={20} color={colors.foreground} />
        </Pressable>
      </View>
    </View>
  )
}
