import { router } from 'expo-router'
import { Pressable, RefreshControl, ScrollView, View } from 'react-native'

import { useMessageText } from '../../components/chat/MessageList'
import { ChatPane, ConversationGlyph, MessagePreview } from '../../components/chat/Pane'
import { conversationTitle, type UnreadConversation, unreadConversations, unreadMessages } from '../../data/chat'
import * as chat from '../../data/chat-api'
import { useChatWrite, useMessages } from '../../hooks/chat'
import { useOrbit } from '../../orbit-context'
import { useChatStates, useConversations, usePeople } from '../../queries'
import { type, useTheme } from '../../theme'
import { Button, EmptyState, ErrorView, LoadingView, Text } from '../../ui'

/** The Unreads view: one group for each unread conversation that is not muted, newest activity first. */
export default function UnreadsScreen() {
  const { user } = useOrbit()
  const { colors } = useTheme()
  const conversations = useConversations()
  const states = useChatStates()
  const run = useChatWrite()
  const groups = conversations.data && states.data ? unreadConversations(conversations.data, states.data, user.id) : []
  const refresh = () => Promise.all([conversations.refetch(), states.refetch()])

  return (
    <ChatPane
      title="Unreads"
      actions={groups.length > 0 ? <Button label="Mark all as read" variant="ghost" size="sm" onPress={() => void run('Could not mark everything as read', chat.markAllRead)} /> : undefined}
    >
      {conversations.isError || states.isError ? (
        <ErrorView error={conversations.error ?? states.error} onRetry={() => void refresh()} />
      ) : !conversations.data || !states.data ? (
        <LoadingView />
      ) : groups.length === 0 ? (
        <EmptyState icon="checkCircle" title="You're all caught up" description="New messages in your channels and direct messages show here." />
      ) : (
        <ScrollView refreshControl={<RefreshControl refreshing={states.isRefetching} onRefresh={() => void refresh()} tintColor={colors.mutedForeground} />}>
          {groups.map((group) => (
            <UnreadGroup key={group.conversation.id} {...group} />
          ))}
        </ScrollView>
      )}
    </ChatPane>
  )
}

function UnreadGroup({ conversation, state }: UnreadConversation) {
  const { user } = useOrbit()
  const { colors } = useTheme()
  const people = usePeople()
  const messages = useMessages(conversation.id, undefined)
  const words = useMessageText()
  const run = useChatWrite()
  const title = conversationTitle(conversation, people, user.id)
  const shown = unreadMessages(messages.data?.pages.flatMap((page) => page.items) ?? [], state, user.id)
  const more = Math.max(0, state.unread_count - shown.length)
  const open = () => router.push(`/chat/${conversation.id}`)

  return (
    <View accessibilityLabel={title} style={{ gap: 4, paddingHorizontal: 8, paddingVertical: 12, borderBottomWidth: 1, borderBottomColor: colors.border }}>
      <View style={{ flexDirection: 'row', alignItems: 'center', gap: 8, paddingHorizontal: 4 }}>
        <Pressable accessibilityRole="link" accessibilityLabel={`Open ${title}`} onPress={open} style={{ flexShrink: 1, flexDirection: 'row', alignItems: 'center', gap: 8 }}>
          <ConversationGlyph conversation={conversation} title={title} userId={user.id} />
          <Text numberOfLines={1} style={[type(13, 'semibold'), { flexShrink: 1 }]}>
            {title}
          </Text>
        </Pressable>
        <Text style={[type(12), { color: colors.mutedForeground }]}>{state.unread_count} unread</Text>
        <View style={{ flex: 1 }} />
        <Button label="Mark as read" variant="ghost" size="sm" onPress={() => void run('Could not mark as read', (client, workspaceId) => chat.markRead(client, workspaceId, conversation.id))} />
      </View>
      {messages.isError ? (
        <Text style={[type(13), { paddingHorizontal: 4, color: colors.mutedForeground }]}>Could not load these messages.</Text>
      ) : (
        shown.map((message) => <MessagePreview key={message.id} message={message} author={words.nameOf(message.author_id)} text={words.text(message.body)} onPress={open} />)
      )}
      {more > 0 && !messages.isPending ? (
        <Pressable accessibilityRole="link" onPress={open} style={{ paddingHorizontal: 4 }}>
          <Text style={[type(12), { color: colors.mutedForeground }]}>
            {more} more unread {more === 1 ? 'message' : 'messages'}
          </Text>
        </Pressable>
      ) : null}
    </View>
  )
}
