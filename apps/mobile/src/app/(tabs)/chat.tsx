import { router } from 'expo-router'
import { type ReactNode, useMemo, useState } from 'react'
import { Pressable, RefreshControl, ScrollView, View } from 'react-native'
import { SafeAreaView } from 'react-native-safe-area-context'

import { CategorySheet, NewChannelSheet, NewMessageSheet } from '../../components/chat/ChannelSheets'
import { ConversationOptions } from '../../components/chat/ConversationOptions'
import { ActionSheet, type SheetItem } from '../../components/chat/Sheet'
import { chatSections, conversationBadge, conversationTitle, unreadThreadCount } from '../../data/chat'
import * as chat from '../../data/chat-api'
import type { CategoryRecord, ConversationRecord, ConversationStateRecord } from '../../data/orbit'
import { confirmAction, useChatWrite, useFollowedThreads, useIsChatAdmin } from '../../hooks/chat'
import { useOrbit } from '../../orbit-context'
import { useChatCategories, useChatStates, useChatUnreadCount, useConversations, usePeople } from '../../queries'
import { alpha, type, useTheme } from '../../theme'
import { Avatar, CountBadge, EmptyState, ErrorView, HeaderTitle, Icon, type IconName, IconButton, LoadingView, PaneHeader, Text } from '../../ui'

const PRESENCE_LABEL = { online: 'Online', idle: 'Idle', dnd: 'Do not disturb', invisible: 'Invisible' } as const

/** The sheet or form open over the list. */
type Overlay =
  | { kind: 'new' }
  | { kind: 'message' }
  | { kind: 'channel'; categoryId: string | null }
  | { kind: 'category'; category?: CategoryRecord }
  | { kind: 'category-options'; category: CategoryRecord }

/** The web's mobile chat list: Unreads and Threads, then each section, then the user bar. */
export default function ChatScreen() {
  const { user } = useOrbit()
  const { colors } = useTheme()
  const conversations = useConversations()
  const categories = useChatCategories()
  const states = useChatStates()
  const people = usePeople()
  const [collapsed, setCollapsed] = useState<ReadonlySet<string>>(new Set())
  const [overlay, setOverlay] = useState<Overlay | null>(null)
  const admin = useIsChatAdmin()
  const run = useChatWrite()
  const threads = useFollowedThreads()
  const unreads = useChatUnreadCount()
  const close = () => setOverlay(null)

  const sections = useMemo(
    () => (conversations.data && categories.data && states.data ? chatSections(conversations.data, categories.data, states.data, user.id) : []),
    [categories.data, conversations.data, states.data, user.id],
  )
  const refresh = () => Promise.all([conversations.refetch(), categories.refetch(), states.refetch()])
  const stateById = new Map((states.data ?? []).map((state) => [state.conversation_id, state]))
  const toggle = (key: string) =>
    setCollapsed((current) => {
      const next = new Set(current)
      if (!next.delete(key)) next.add(key)
      return next
    })

  return (
    <SafeAreaView edges={['top', 'left', 'right']} style={{ flex: 1, backgroundColor: colors.card }}>
      <PaneHeader style={{ gap: 4, paddingLeft: 12, paddingRight: 8, backgroundColor: colors.card }}>
        <View style={{ flex: 1 }}>
          <HeaderTitle>Chat</HeaderTitle>
        </View>
        <IconButton icon="plus" label="New" size={32} iconSize={20} color={colors.mutedForeground} onPress={() => setOverlay({ kind: 'new' })} />
      </PaneHeader>
      {!conversations.data || !categories.data || !states.data ? (
        conversations.error || categories.error || states.error ? (
          <ErrorView error={conversations.error ?? categories.error ?? states.error} onRetry={() => void refresh()} />
        ) : (
          <LoadingView />
        )
      ) : (
        <ScrollView refreshControl={<RefreshControl refreshing={conversations.isRefetching} onRefresh={() => void refresh()} tintColor={colors.mutedForeground} />}>
          <View style={{ paddingHorizontal: 8, paddingTop: 8, gap: 1 }}>
            <PinnedRow icon="directInbox" name="Unreads" count={unreads} onPress={() => router.push('/chat/unreads')} />
            <PinnedRow icon="layer" name="Threads" count={unreadThreadCount(threads.data ?? [])} onPress={() => router.push('/chat/threads')} />
          </View>
          <View style={{ padding: 8, paddingTop: 12, gap: 12 }}>
            {sections.length === 0 ? <EmptyState icon="chat" title="No conversations" description="Start a direct message or create a channel with New." /> : null}
            {sections.map((section) => {
              const open = !collapsed.has(section.key)
              const category = admin ? categories.data?.find((item) => `category-${item.id}` === section.key) : undefined
              const onAdd =
                section.key === 'dms'
                  ? () => setOverlay({ kind: 'message' })
                  : section.key === 'favorites'
                    ? null
                    : () => setOverlay({ kind: 'channel', categoryId: section.key.startsWith('category-') ? section.key.slice('category-'.length) : null })
              return (
                <View key={section.key} style={{ gap: 1 }}>
                  <View style={{ height: 28, flexDirection: 'row', alignItems: 'center', gap: 2, paddingRight: 6 }}>
                    <Pressable
                      accessibilityRole="button"
                      accessibilityState={{ expanded: open }}
                      accessibilityLabel={section.title}
                      onPress={() => toggle(section.key)}
                      style={{ flex: 1, height: 24, flexDirection: 'row', alignItems: 'center', gap: 4, paddingHorizontal: 8 }}
                    >
                      <Icon name={open ? 'chevronDown' : 'chevronRight'} size={12} color={colors.mutedForeground} />
                      <Text numberOfLines={1} style={[type(12), { lineHeight: 16, color: colors.mutedForeground }]}>
                        {section.title}
                      </Text>
                    </Pressable>
                    {category ? <IconButton icon="more" label={`Options for the category ${category.name}`} size={24} color={colors.mutedForeground} onPress={() => setOverlay({ kind: 'category-options', category })} /> : null}
                    {onAdd ? <IconButton icon="plus" label={section.key === 'dms' ? 'New message' : `New channel in ${section.title}`} size={24} color={colors.mutedForeground} onPress={onAdd} /> : null}
                  </View>
                  {open
                    ? section.conversations.map((conversation) => (
                        <ConversationRow
                          key={conversation.id}
                          conversation={conversation}
                          title={conversationTitle(conversation, people, user.id)}
                          state={stateById.get(conversation.id)}
                          userId={user.id}
                        />
                      ))
                    : null}
                </View>
              )
            })}
          </View>
        </ScrollView>
      )}
      <View style={{ height: 48, flexDirection: 'row', alignItems: 'center', paddingHorizontal: 8, borderTopWidth: 1, borderTopColor: colors.border }}>
        <Pressable
          accessibilityRole="button"
          accessibilityLabel={`Status menu for ${user.display_name}`}
          onPress={() => router.navigate('/settings')}
          style={({ pressed }) => ({ flex: 1, height: 36, flexDirection: 'row', alignItems: 'center', gap: 8, paddingHorizontal: 6, borderRadius: 10, backgroundColor: pressed ? colors.muted : 'transparent' })}
        >
          <Avatar id={user.id} name={user.display_name} size={28} presence={user.status.presence} surface={colors.card} />
          <View style={{ flex: 1, minWidth: 0 }}>
            <Text numberOfLines={1} style={[type(13, 'medium'), { lineHeight: 16 }]}>
              {user.display_name}
            </Text>
            <Text numberOfLines={1} style={[type(11), { lineHeight: 14, color: colors.mutedForeground }]}>
              {user.status.text || PRESENCE_LABEL[user.status.presence]}
            </Text>
          </View>
        </Pressable>
      </View>
      <ActionSheet
        visible={overlay?.kind === 'new'}
        items={[
          { key: 'message', label: 'New message', icon: 'messages2', onPress: () => setOverlay({ kind: 'message' }) },
          { key: 'channel', label: 'New channel', icon: 'hashtag', onPress: () => setOverlay({ kind: 'channel', categoryId: null }) },
          { key: 'browse', label: 'Browse channels', icon: 'search', onPress: () => router.push('/chat/browse') },
          ...(admin ? [{ key: 'category', label: 'New category', icon: 'category' as const, onPress: () => setOverlay({ kind: 'category' }) }] : []),
        ]}
        onClose={close}
      />
      <ActionSheet
        visible={overlay?.kind === 'category-options'}
        title={overlay?.kind === 'category-options' ? overlay.category.name : undefined}
        items={overlay?.kind === 'category-options' ? categoryItems(overlay.category) : []}
        onClose={close}
      />
      {overlay?.kind === 'message' ? <NewMessageSheet onClose={close} /> : null}
      {overlay?.kind === 'channel' ? <NewChannelSheet categoryId={overlay.categoryId} onClose={close} /> : null}
      {overlay?.kind === 'category' ? <CategorySheet category={overlay.category} onClose={close} /> : null}
    </SafeAreaView>
  )

  function categoryItems(category: CategoryRecord): SheetItem[] {
    const remove = async () => {
      const confirmed = await confirmAction(`Delete the category "${category.name}"?`, 'Its channels are kept and move to "Channels".', 'Delete category')
      if (confirmed) await run('Could not delete the category', (client, workspaceId) => chat.deleteCategory(client, workspaceId, category.id))
    }
    return [
      { key: 'rename', label: 'Rename', icon: 'edit', onPress: () => setOverlay({ kind: 'category', category }) },
      { key: 'delete', label: 'Delete category', icon: 'trash', destructive: true, onPress: () => void remove() },
    ]
  }
}

/** `Unreads` and `Threads`, each with its count. */
function PinnedRow({ icon, name, count, onPress }: { icon: IconName; name: string; count: number; onPress: () => void }) {
  return <Row leading={<Glyph icon={icon} />} name={name} unread={count > 0} label={count > 0 ? `${name}, ${count} unread` : name} onPress={onPress} end={count > 0 ? <CountBadge count={count} /> : undefined} />
}

function Glyph({ icon }: { icon: IconName }) {
  const { colors } = useTheme()
  return (
    <View style={{ width: 18, height: 18, alignItems: 'center', justifyContent: 'center' }}>
      <Icon name={icon} size={16} color={colors.mutedForeground} />
    </View>
  )
}

/** The web `ChatRow` at phone size: 40px, 14px text at 80% foreground, bold and full color when unread. */
function Row({ leading, name, unread = false, muted = false, end, onPress, onLongPress, label }: { leading: ReactNode; name: string; unread?: boolean; muted?: boolean; end?: ReactNode; onPress?: () => void; onLongPress?: () => void; label?: string }) {
  const { colors } = useTheme()
  return (
    <View style={{ height: 40, flexDirection: 'row', alignItems: 'center', borderRadius: 6 }}>
      <Pressable
        accessibilityRole={onPress ? 'link' : undefined}
        accessibilityLabel={label ?? name}
        disabled={!onPress}
        onPress={onPress}
        onLongPress={onLongPress}
        style={({ pressed }) => ({ flex: 1, height: 40, flexDirection: 'row', alignItems: 'center', gap: 8, paddingLeft: 8, borderRadius: 6, backgroundColor: pressed ? alpha(colors.muted, 0.6) : 'transparent' })}
      >
        {leading}
        <Text numberOfLines={1} style={[type(14, unread ? 'semibold' : 'regular'), { lineHeight: 20, flexShrink: 1, color: unread ? colors.foreground : muted ? colors.mutedForeground : alpha(colors.foreground, 0.8) }]}>
          {name}
        </Text>
      </Pressable>
      {end ? <View style={{ flexDirection: 'row', alignItems: 'center', gap: 6, paddingLeft: 4, paddingRight: 8 }}>{end}</View> : null}
    </View>
  )
}

function ConversationRow({ conversation, title, state, userId }: { conversation: ConversationRecord; title: string; state: ConversationStateRecord | undefined; userId: string }) {
  const { colors } = useTheme()
  const badge = conversationBadge(conversation, state, userId)
  const [options, setOptions] = useState(false)
  const peer = conversation.member_ids.find((id) => id !== userId) ?? userId
  const leading =
    conversation.kind === 'dm' ? (
      <Avatar id={peer} name={title} size={20} presence={peer === userId ? undefined : 'offline'} surface={colors.card} />
    ) : (
      <View accessibilityLabel={conversation.kind === 'private' ? 'Private channel' : undefined}>
        <Glyph icon={conversation.kind === 'private' ? 'lock' : 'hashtag'} />
      </View>
    )
  return (
    <>
      <Row
        leading={leading}
        name={title}
        unread={badge.unread}
        muted={state?.notify === 'muted'}
        label={badge.count ? `${title}, ${badge.count} unread` : title}
        onPress={() => router.push(`/chat/${conversation.id}`)}
        onLongPress={() => setOptions(true)}
        end={
          <>
            <IconButton icon="more" label={`Options for ${title}`} size={24} color={colors.mutedForeground} onPress={() => setOptions(true)} />
            {badge.count > 0 ? <CountBadge count={badge.count} /> : badge.unread ? <View style={{ width: 6, height: 6, backgroundColor: colors.primary }} /> : null}
          </>
        }
      />
      <ConversationOptions conversation={conversation} title={title} placement="row" visible={options} onClose={() => setOptions(false)} />
    </>
  )
}
