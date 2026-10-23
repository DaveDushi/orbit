import { router } from 'expo-router'
import { useMemo } from 'react'
import { Pressable, RefreshControl, SectionList, View } from 'react-native'

import { chatSections, conversationBadge, conversationTitle } from '../../data/chat'
import type { ConversationRecord, ConversationStateRecord } from '../../data/orbit'
import { useOrbit } from '../../orbit-context'
import { useChatCategories, useChatStates, useConversations, usePeople } from '../../queries'
import { fonts, useTheme } from '../../theme'
import { Avatar, Badge, EmptyState, ErrorView, Icon, LoadingView, Text } from '../../ui'

export default function ChatScreen() {
  const { user } = useOrbit()
  const { colors, spacing } = useTheme()
  const conversations = useConversations()
  const categories = useChatCategories()
  const states = useChatStates()
  const people = usePeople()

  const sections = useMemo(
    () =>
      conversations.data && categories.data && states.data
        ? chatSections(conversations.data, categories.data, states.data, user.id).map((section) => ({ ...section, data: section.conversations }))
        : [],
    [categories.data, conversations.data, states.data, user.id],
  )
  const refresh = () => Promise.all([conversations.refetch(), categories.refetch(), states.refetch()])

  if (!conversations.data || !categories.data || !states.data) {
    const error = conversations.error ?? categories.error ?? states.error
    return error ? <ErrorView error={error} onRetry={() => void refresh()} /> : <LoadingView />
  }
  const stateById = new Map(states.data.map((state) => [state.conversation_id, state]))

  return (
    <SectionList
      sections={sections}
      keyExtractor={(conversation) => conversation.id}
      stickySectionHeadersEnabled={false}
      contentContainerStyle={sections.length === 0 ? { flexGrow: 1 } : { paddingBottom: spacing[6] }}
      refreshControl={<RefreshControl refreshing={conversations.isRefetching} onRefresh={() => void refresh()} tintColor={colors.mutedForeground} />}
      ListEmptyComponent={<EmptyState icon="chat" title="No conversations" description="Join a channel or start a direct message in Orbit on the web." />}
      renderSectionHeader={({ section }) => (
        <Text variant="caption" color="muted" style={{ paddingHorizontal: spacing[4], paddingTop: spacing[4], paddingBottom: spacing[1], fontFamily: fonts.medium }}>
          {section.title}
        </Text>
      )}
      renderItem={({ item }) => (
        <ConversationRow
          conversation={item}
          title={conversationTitle(item, people, user.id)}
          state={stateById.get(item.id)}
          userId={user.id}
          onPress={() => router.push(`/chat/${item.id}`)}
        />
      )}
    />
  )
}

function ConversationRow({
  conversation,
  title,
  state,
  userId,
  onPress,
}: {
  conversation: ConversationRecord
  title: string
  state: ConversationStateRecord | undefined
  userId: string
  onPress: () => void
}) {
  const { colors, spacing } = useTheme()
  const badge = conversationBadge(conversation, state, userId)
  const peer = conversation.member_ids.find((id) => id !== userId) ?? userId
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={`${title}${badge.count ? `, ${badge.count} unread` : ''}`}
      onPress={onPress}
      style={({ pressed }) => ({
        flexDirection: 'row',
        alignItems: 'center',
        gap: spacing[3],
        minHeight: 44,
        paddingHorizontal: spacing[4],
        backgroundColor: pressed ? colors.accent : 'transparent',
      })}
    >
      <View style={{ width: 24, alignItems: 'center' }}>
        {conversation.kind === 'dm' ? (
          <Avatar id={peer} name={title} size={24} />
        ) : (
          <Icon name={conversation.kind === 'private' ? 'lock' : 'hashtag'} size={18} color={badge.unread ? colors.foreground : colors.mutedForeground} />
        )}
      </View>
      <Text variant="label" numberOfLines={1} style={{ flex: 1, fontFamily: badge.unread ? fonts.semibold : fonts.regular, color: badge.unread ? colors.foreground : colors.mutedForeground }}>
        {title}
      </Text>
      {badge.count > 0 ? <Badge label={String(badge.count)} variant="primary" /> : null}
    </Pressable>
  )
}
