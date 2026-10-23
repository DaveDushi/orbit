import { useQueryClient } from '@tanstack/react-query'
import { applyChatEvents } from '../../../chat-cache'
import { router, useLocalSearchParams } from 'expo-router'
import { useEffect, useMemo } from 'react'
import { KeyboardAvoidingView, Platform, View } from 'react-native'
import { SafeAreaView } from 'react-native-safe-area-context'

import { Composer, useComposer } from '../../../components/chat/Composer'
import { MessageItem } from '../../../components/chat/Message'
import { type ListRow, MessageList, useMessageSheet } from '../../../components/chat/MessageList'
import { buildMessageRows, conversationTitle } from '../../../data/chat'
import * as chat from '../../../data/chat-api'
import { useChatWrite, useFollowedThreads, useThread, useThreadFollowsRoot } from '../../../hooks/chat'
import { useOrbit } from '../../../orbit-context'
import { useConversations, usePeople } from '../../../queries'
import { type, useTheme } from '../../../theme'
import { ErrorView, IconButton, LoadingView, Text } from '../../../ui'

/**
 * A thread in full view, as the web's `/chat/:id/thread/:messageId` on a phone: the root, its replies, and a composer
 * that replies in the thread (and can also send to the conversation). The bell follows or unfollows it.
 */
export default function ThreadScreen() {
  const { id: rootId } = useLocalSearchParams<{ id: string }>()
  const { client, workspaceId, user } = useOrbit()
  const queryClient = useQueryClient()
  const { colors } = useTheme()
  const people = usePeople()
  const conversations = useConversations()
  const followed = useFollowedThreads()
  const run = useChatWrite()
  const thread = useThread(rootId)

  const root = thread.data?.pages.at(-1)?.root
  const conversationId = root?.conversation_id ?? ''
  const conversation = conversations.data?.find((item) => item.id === conversationId)
  const replies = useMemo(() => thread.data?.pages.flatMap((page) => page.items) ?? [], [thread.data])
  const composer = useComposer(conversationId, rootId)
  const { sheet, handlers } = useMessageSheet({ inThread: true, composer, loaded: replies })
  useThreadFollowsRoot(root)

  const state = followed.data?.find((item) => item.root.id === rootId)?.state ?? thread.data?.pages.at(-1)?.state ?? null
  const following = state?.following ?? false
  const unread = state?.unread_replies ?? 0
  const newestId = replies.at(-1)?.id
  useEffect(() => {
    if (!root || unread === 0) return
    chat.markThreadRead(client, workspaceId, rootId).then(
      ({ events }) => applyChatEvents(queryClient, workspaceId, events),
      () => undefined,
    )
  }, [client, newestId, queryClient, root, rootId, unread, workspaceId])

  const rows: ListRow[] = useMemo(
    () => [
      ...composer.pending
        .filter((item) => !replies.some((message) => message.nonce === item.nonce))
        .map((item): ListRow => ({ type: 'pending', key: item.nonce, pending: item }))
        .reverse(),
      ...buildMessageRows(replies).reverse(),
    ],
    [replies, composer.pending],
  )
  const title = conversation ? conversationTitle(conversation, people, user.id) : ''
  const where = conversation ? (conversation.kind === 'dm' ? title : `#${title}`) : ''
  const member = conversation?.member_ids.includes(user.id) ?? false

  return (
    <SafeAreaView edges={['top', 'left', 'right', 'bottom']} style={{ flex: 1, backgroundColor: colors.card }}>
      <View style={{ height: 48, flexDirection: 'row', alignItems: 'center', gap: 4, paddingHorizontal: 8 }}>
        <IconButton icon="back" label="Back" size={32} iconSize={20} color={colors.mutedForeground} onPress={() => (router.canGoBack() ? router.back() : router.navigate(conversationId ? `/chat/${conversationId}` : '/chat'))} />
        <View style={{ flex: 1, minWidth: 0 }}>
          <Text numberOfLines={1} style={[type(14, 'semibold'), { lineHeight: 18 }]}>
            Thread
          </Text>
          {where ? (
            <Text numberOfLines={1} style={[type(12), { lineHeight: 16, color: colors.mutedForeground }]}>
              {where}
            </Text>
          ) : null}
        </View>
        <IconButton
          icon={following ? 'bell' : 'bellOff'}
          label={following ? 'Unfollow thread' : 'Follow thread'}
          size={32}
          iconSize={20}
          color={following ? colors.primary : colors.mutedForeground}
          disabled={!root}
          onPress={() => void run(following ? 'Could not unfollow the thread' : 'Could not follow the thread', (client, workspaceId) => chat.followThread(client, workspaceId, rootId, !following))}
        />
        {conversationId ? <IconButton icon="chat" label={`Open ${where}`} size={32} iconSize={20} color={colors.mutedForeground} onPress={() => router.push(`/chat/${conversationId}?message=${rootId}`)} /> : null}
      </View>
      <KeyboardAvoidingView style={{ flex: 1 }} behavior={Platform.OS === 'ios' ? 'padding' : undefined}>
        {!thread.data || !root ? (
          thread.isError ? (
            <ErrorView error={thread.error} onRetry={() => void thread.refetch()} />
          ) : (
            <LoadingView />
          )
        ) : (
          <MessageList
            rows={rows}
            handlers={handlers}
            fromTop
            loadingOlder={thread.isFetchingPreviousPage}
            onOlder={() => {
              if (thread.hasPreviousPage && !thread.isFetchingPreviousPage) void thread.fetchPreviousPage()
            }}
            start={
              thread.hasPreviousPage ? undefined : (
                <View style={{ paddingTop: 8 }}>
                  <MessageItem message={root} groupStart handlers={handlers} />
                  <View style={{ flexDirection: 'row', alignItems: 'center', gap: 8, paddingHorizontal: 12, paddingTop: 12, paddingBottom: 4 }}>
                    <Text style={[type(12, 'medium'), { color: colors.mutedForeground }]}>
                      {root.reply_count} {root.reply_count === 1 ? 'reply' : 'replies'}
                    </Text>
                    <View style={{ flex: 1, height: 1, backgroundColor: colors.border }} />
                  </View>
                </View>
              )
            }
          />
        )}
        {root && member && !conversation?.archived ? <Composer state={composer} placeholder="Reply…" nameOf={handlers.nameOf} alsoSendTo={conversation?.kind === 'dm' ? 'this conversation' : where} /> : null}
      </KeyboardAvoidingView>
      {sheet}
    </SafeAreaView>
  )
}
