import { router } from 'expo-router'
import { Pressable, RefreshControl, ScrollView, View } from 'react-native'

import { useMessageText } from '../../components/chat/MessageList'
import { ChatPane, ConversationGlyph, MessagePreview } from '../../components/chat/Pane'
import { conversationTitle, sortFollowedThreads } from '../../data/chat'
import { relativeAgo } from '../../data/format'
import { useFollowedThreads } from '../../hooks/chat'
import { useOrbit } from '../../orbit-context'
import { useConversations, usePeople } from '../../queries'
import { alpha, type, useTheme } from '../../theme'
import { CountBadge, EmptyState, ErrorView, LoadingView, Text } from '../../ui'

/** The Threads view: the threads you follow, unread first, then by last reply. A row opens the thread. */
export default function ThreadsScreen() {
  const { user } = useOrbit()
  const { colors } = useTheme()
  const threads = useFollowedThreads()
  const conversations = useConversations().data ?? []
  const people = usePeople()
  const words = useMessageText()
  const rows = sortFollowedThreads(threads.data ?? [])

  return (
    <ChatPane title="Threads">
      {threads.isError ? (
        <ErrorView error={threads.error} onRetry={() => void threads.refetch()} />
      ) : !threads.data ? (
        <LoadingView />
      ) : rows.length === 0 ? (
        <EmptyState icon="layer" title="Threads you follow appear here" description="Reply to a message or follow its thread, and new replies show up in this list." />
      ) : (
        <ScrollView refreshControl={<RefreshControl refreshing={threads.isRefetching} onRefresh={() => void threads.refetch()} tintColor={colors.mutedForeground} />}>
          {rows.map((thread) => {
            const conversation = conversations.find((item) => item.id === thread.conversation_id)
            const title = conversation ? conversationTitle(conversation, people, user.id) : 'Unknown conversation'
            const unread = thread.state.unread_replies
            const replies = `${thread.root.reply_count} ${thread.root.reply_count === 1 ? 'reply' : 'replies'}`
            const lastReplyAt = thread.last_reply?.created_at ?? thread.root.last_reply_at
            return (
              <Pressable
                key={thread.root.id}
                accessibilityRole="link"
                accessibilityLabel={`Thread in ${title}, ${replies}${unread > 0 ? `, ${unread} unread` : ''}`}
                onPress={() => router.push(`/chat/thread/${thread.root.id}`)}
                style={({ pressed }) => ({ gap: 4, paddingHorizontal: 8, paddingVertical: 12, borderBottomWidth: 1, borderBottomColor: colors.border, backgroundColor: pressed ? alpha(colors.muted, 0.4) : 'transparent' })}
              >
                <View style={{ flexDirection: 'row', alignItems: 'center', gap: 6, paddingHorizontal: 4 }}>
                  {conversation ? <ConversationGlyph conversation={conversation} title={title} userId={user.id} /> : null}
                  <Text numberOfLines={1} style={[type(13, unread > 0 ? 'semibold' : 'medium'), { flexShrink: 1 }]}>
                    {title}
                  </Text>
                  <Text style={[type(12), { color: colors.mutedForeground }]}>{replies}</Text>
                  {lastReplyAt ? <Text style={[type(12), { color: colors.mutedForeground }]}>last reply {relativeAgo(lastReplyAt)}</Text> : null}
                  <View style={{ flex: 1 }} />
                  {unread > 0 ? <CountBadge count={unread} /> : null}
                </View>
                <MessagePreview message={thread.root} author={words.nameOf(thread.root.author_id)} text={words.text(thread.root.body)} clamp={2} />
                {thread.last_reply ? (
                  <View style={{ marginLeft: 12, borderLeftWidth: 2, borderLeftColor: colors.border }}>
                    <MessagePreview message={thread.last_reply} author={words.nameOf(thread.last_reply.author_id)} text={words.text(thread.last_reply.body)} clamp={2} />
                  </View>
                ) : null}
              </Pressable>
            )
          })}
        </ScrollView>
      )}
    </ChatPane>
  )
}
