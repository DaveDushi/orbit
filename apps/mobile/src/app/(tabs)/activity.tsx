import { useMutation, useQueryClient } from '@tanstack/react-query'
import { type Href, router } from 'expo-router'
import { useState } from 'react'
import { Alert, FlatList, Pressable, RefreshControl, View } from 'react-native'
import { SafeAreaView } from 'react-native-safe-area-context'

import { MenuButton } from '../../components/Drawer'
import { relativeTime } from '../../data/format'
import { isMention, notificationCopy, notificationTarget } from '../../data/inbox'
import * as api from '../../data/orbit'
import { useOrbit } from '../../orbit-context'
import { useNotifications, usePages, usePeople } from '../../queries'
import { errorMessage } from '../../session-context'
import { alpha, type, useTheme } from '../../theme'
import { Button, CountBadge, EmptyState, ErrorView, HeaderTitle, LoadingView, PaneHeader, Text } from '../../ui'

type Tab = 'all' | 'unread' | 'mentions'
const TABS: { value: Tab; label: string }[] = [
  { value: 'all', label: 'All' },
  { value: 'unread', label: 'Unread' },
  { value: 'mentions', label: 'Mentions' },
]

/** The web's Activity page: underline tabs and 56px rows. */
export default function ActivityScreen() {
  const { client, workspaceId } = useOrbit()
  const queryClient = useQueryClient()
  const { colors } = useTheme()
  const [tab, setTab] = useState<Tab>('all')
  const notifications = useNotifications()
  const people = usePeople()
  const pages = usePages()
  // both the list and the unread list live under this prefix
  const refreshInbox = () => queryClient.invalidateQueries({ queryKey: ['notifications', workspaceId] })

  const markRead = useMutation({
    mutationFn: (id: string) => api.markNotificationRead(client, workspaceId, id),
    onSettled: refreshInbox,
  })
  const markAll = useMutation({
    mutationFn: () => api.markAllNotificationsRead(client, workspaceId),
    onSettled: refreshInbox,
    onError: (error) => Alert.alert('Could not mark all read', errorMessage(error)),
  })
  const all = notifications.data ?? []
  const unreadCount = all.filter((item) => !item.read_at).length
  const items = all.filter((item) => (tab === 'unread' ? !item.read_at : tab === 'mentions' ? isMention(item) : true))

  return (
    <SafeAreaView edges={['top', 'left', 'right']} style={{ flex: 1, backgroundColor: colors.background }}>
      <PaneHeader style={{ paddingHorizontal: 12 }}>
        <MenuButton />
        <HeaderTitle>Activity</HeaderTitle>
        {unreadCount > 0 ? <CountBadge count={unreadCount} /> : null}
        <View style={{ flex: 1 }} />
        <Button label={markAll.isPending ? 'Marking…' : 'Mark all read'} variant="ghost" disabled={unreadCount === 0 || markAll.isPending} onPress={() => markAll.mutate()} />
      </PaneHeader>
      <View accessibilityRole="tablist" style={{ minHeight: 40, flexDirection: 'row', alignItems: 'flex-end', gap: 6, paddingHorizontal: 12, borderBottomWidth: 1, borderBottomColor: colors.border }}>
        {TABS.map((item) => {
          const selected = item.value === tab
          return (
            <Pressable
              key={item.value}
              accessibilityRole="tab"
              accessibilityState={{ selected }}
              onPress={() => setTab(item.value)}
              style={{ height: 33, justifyContent: 'flex-start', paddingTop: 5 }}
            >
              <Text style={[type(13, 'medium'), { paddingHorizontal: 10, color: selected ? colors.foreground : colors.mutedForeground }]}>{item.label}</Text>
              <View style={{ position: 'absolute', left: 0, right: 0, bottom: -1, height: 2, backgroundColor: selected ? colors.foreground : 'transparent' }} />
            </Pressable>
          )
        })}
      </View>
      {!notifications.data ? (
        notifications.isError ? (
          <ErrorView error={notifications.error} onRetry={() => void notifications.refetch()} />
        ) : (
          <LoadingView />
        )
      ) : (
        <FlatList
          data={items}
          keyExtractor={(item) => item.id}
          contentContainerStyle={items.length === 0 ? { flexGrow: 1 } : undefined}
          refreshControl={<RefreshControl refreshing={notifications.isRefetching} onRefresh={() => void notifications.refetch()} tintColor={colors.mutedForeground} />}
          ListEmptyComponent={<EmptyState icon="inbox" title="You're all caught up" description="Assignments and mentions appear here." />}
          renderItem={({ item }) => {
            const actor = people.find((person) => person.id === item.actor_user_id)
            const page = item.page_id ? pages.data?.find((entry) => entry.id === item.page_id) : undefined
            const copy = notificationCopy(item, page?.title, actor?.name)
            const unread = !item.read_at
            return (
              <Pressable
                accessibilityRole="button"
                accessibilityLabel={`${unread ? 'Unread. ' : ''}${copy.title}`}
                onPress={() => {
                  if (unread) markRead.mutate(item.id)
                  const target = notificationTarget(item)
                  if (target) router.push(target as Href)
                }}
                style={({ pressed }) => ({
                  minHeight: 56,
                  flexDirection: 'row',
                  alignItems: 'center',
                  gap: 10,
                  paddingHorizontal: 12,
                  paddingVertical: 6,
                  borderBottomWidth: 1,
                  borderBottomColor: colors.border,
                  backgroundColor: pressed ? colors.muted : 'transparent',
                })}
              >
                <View style={{ width: 8, alignItems: 'center' }}>{unread ? <View accessibilityLabel="Unread" style={{ width: 8, height: 8, borderRadius: 4, backgroundColor: colors.primary }} /> : null}</View>
                <View style={{ width: 28, height: 28, borderRadius: 14, alignItems: 'center', justifyContent: 'center', backgroundColor: colors.muted }}>
                  <Text style={[type(14), { lineHeight: 20, color: colors.mutedForeground }]}>{(actor?.name ?? '?').charAt(0).toUpperCase()}</Text>
                </View>
                <View style={{ flex: 1, minWidth: 0, gap: 2 }}>
                  <Text numberOfLines={1} style={[type(13, unread ? 'semibold' : 'regular'), { lineHeight: 18.5 }]}>
                    {copy.title}
                  </Text>
                  <Text numberOfLines={1} style={[type(12), { lineHeight: 16, color: alpha(colors.mutedForeground, 0.7) }]}>
                    {actor ? `${actor.name} · ${copy.body}` : copy.body}
                  </Text>
                </View>
                <Text style={[type(12), { lineHeight: 16, color: alpha(colors.mutedForeground, 0.7) }]}>{relativeTime(item.created_at)}</Text>
              </Pressable>
            )
          }}
        />
      )}
    </SafeAreaView>
  )
}
