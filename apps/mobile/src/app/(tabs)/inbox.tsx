import { useMutation, useQueryClient } from '@tanstack/react-query'
import { router, useNavigation } from 'expo-router'
import { useLayoutEffect } from 'react'
import { Alert, FlatList, Pressable, RefreshControl, View } from 'react-native'

import { notificationCopy, notificationTarget } from '../../data/inbox'
import * as api from '../../data/orbit'
import { useOrbit } from '../../orbit-context'
import { useNotifications, usePeople } from '../../queries'
import { errorMessage } from '../../session-context'
import { useTheme } from '../../theme'
import { shortTime } from '../../time'
import { Avatar, EmptyState, ErrorView, LoadingView, Spinner, Text } from '../../ui'

export default function InboxScreen() {
  const { client, workspaceId } = useOrbit()
  const queryClient = useQueryClient()
  const navigation = useNavigation()
  const { colors, spacing } = useTheme()
  const notifications = useNotifications()
  const people = usePeople()
  // both the list and the unread count live under this prefix
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
  const anyUnread = notifications.data?.some((item) => !item.read_at) ?? false
  const { isPending: markingAll, mutate: markAllRead } = markAll

  useLayoutEffect(() => {
    navigation.setOptions({
      headerRight: () =>
        markingAll ? (
          <View style={{ paddingHorizontal: spacing[4] }}>
            <Spinner />
          </View>
        ) : (
          <Pressable accessibilityRole="button" disabled={!anyUnread} hitSlop={8} onPress={() => markAllRead()} style={{ paddingHorizontal: spacing[4] }}>
            <Text variant="label" color={anyUnread ? 'primary' : 'muted'}>
              Mark all read
            </Text>
          </Pressable>
        ),
    })
  }, [anyUnread, markAllRead, markingAll, navigation, spacing])

  if (!notifications.data) {
    return notifications.isError ? <ErrorView error={notifications.error} onRetry={() => void notifications.refetch()} /> : <LoadingView />
  }

  return (
    <FlatList
      data={notifications.data}
      keyExtractor={(item) => item.id}
      contentContainerStyle={notifications.data.length === 0 ? { flexGrow: 1 } : undefined}
      refreshControl={<RefreshControl refreshing={notifications.isRefetching} onRefresh={() => void notifications.refetch()} tintColor={colors.mutedForeground} />}
      ListEmptyComponent={<EmptyState icon="inbox" title="All caught up" description="Mentions and assignments show here." />}
      ItemSeparatorComponent={() => <View style={{ height: 1, marginLeft: spacing[4] + 32 + spacing[3], backgroundColor: colors.border }} />}
      renderItem={({ item }) => {
        const actor = people.find((person) => person.id === item.actor_user_id)
        const copy = notificationCopy(item, actor?.name)
        const unread = !item.read_at
        return (
          <Pressable
            accessibilityRole="button"
            accessibilityLabel={`${unread ? 'Unread. ' : ''}${copy.title}`}
            onPress={() => {
              if (unread) markRead.mutate(item.id)
              const target = notificationTarget(item)
              if (target) router.push(target)
            }}
            style={({ pressed }) => ({
              flexDirection: 'row',
              gap: spacing[3],
              padding: spacing[4],
              backgroundColor: pressed ? colors.accent : 'transparent',
            })}
          >
            <Avatar id={item.actor_user_id} name={actor?.name ?? '?'} size={32} />
            <View style={{ flex: 1, gap: 2 }}>
              <Text variant="label" color={unread ? 'default' : 'muted'}>
                {copy.title}
              </Text>
              <Text variant="caption" color="muted">
                {copy.body}
              </Text>
              <Text variant="caption" color="muted">
                {shortTime(item.created_at)}
              </Text>
            </View>
            {unread ? <View accessibilityLabel="Unread" style={{ width: 8, height: 8, borderRadius: 4, marginTop: 6, backgroundColor: colors.primary }} /> : null}
          </Pressable>
        )
      }}
    />
  )
}
