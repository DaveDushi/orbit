import { router } from 'expo-router'
import type { ReactNode } from 'react'
import { Pressable, RefreshControl, ScrollView, View } from 'react-native'

import { TaskRow } from '../../components/TaskRow'
import { WorkspaceSwitcher } from '../../components/WorkspaceSwitcher'
import { identifier, myOpenTasks } from '../../data/tasks'
import { useOrbit } from '../../orbit-context'
import { useChatUnreadCount, useConversations, useProjects, useStatuses, useTasks, useUnreadNotificationCount } from '../../queries'
import { useTheme } from '../../theme'
import { Button, Icon, type IconName, Spinner, Text } from '../../ui'

function greetingFor(hour: number): string {
  if (hour < 12) return 'Good morning'
  if (hour < 18) return 'Good afternoon'
  return 'Good evening'
}

export default function HomeScreen() {
  const { user } = useOrbit()
  const { colors, radius, spacing } = useTheme()
  const tasks = useTasks()
  const statuses = useStatuses()
  const projects = useProjects()
  const conversations = useConversations()
  const notifications = useUnreadNotificationCount()
  const chatUnread = useChatUnreadCount()
  const now = new Date()
  const firstName = user.display_name.trim().split(/\s+/)[0] || user.display_name

  const queries = [tasks, statuses, projects, conversations, notifications]
  const refreshing = queries.some((query) => query.isRefetching)
  const refresh = () => Promise.all(queries.map((query) => query.refetch()))
  const mine = tasks.data && statuses.data ? myOpenTasks(tasks.data, statuses.data, user.id, 5) : null
  const card = { borderWidth: 1, borderColor: colors.border, borderRadius: radius.xl, overflow: 'hidden' as const, backgroundColor: colors.card }

  return (
    <ScrollView
      contentContainerStyle={{ padding: spacing[4], gap: spacing[4] }}
      refreshControl={<RefreshControl refreshing={refreshing} onRefresh={() => void refresh()} tintColor={colors.mutedForeground} />}
    >
      <View style={{ gap: spacing[1] }}>
        <Text variant="title">
          {greetingFor(now.getHours())}, {firstName}
        </Text>
        <WorkspaceSwitcher />
      </View>

      <View style={{ flexDirection: 'row', gap: spacing[3] }}>
        <Stat icon="inbox" label="Unread notifications" value={notifications.data} failed={notifications.isError} onPress={() => router.navigate('/inbox')} />
        <Stat icon="chat" label="Unread in chat" value={conversations.data ? chatUnread : undefined} failed={conversations.isError} onPress={() => router.navigate('/chat')} />
      </View>

      <View style={{ gap: spacing[2] }}>
        <Text variant="heading">My tasks</Text>
        <View style={card}>
          {mine === null ? (
            tasks.isError || statuses.isError ? (
              <Inline>
                <Text color="muted">Could not load your tasks.</Text>
                <Button label="Retry" variant="secondary" onPress={() => void refresh()} />
              </Inline>
            ) : (
              <Inline>
                <Spinner />
              </Inline>
            )
          ) : mine.length === 0 ? (
            <Inline>
              <Text color="muted">No open tasks assigned to you.</Text>
            </Inline>
          ) : (
            mine.map((task, index) => (
              <View key={task.id} style={index > 0 ? { borderTopWidth: 1, borderTopColor: colors.border } : undefined}>
                <TaskRow
                  task={task}
                  status={statuses.data?.find((status) => status.id === task.status_id)}
                  identifier={identifier(task, projects.data ?? [])}
                  assignee={undefined}
                  now={now}
                  onPress={() => router.push(`/task/${task.id}`)}
                />
              </View>
            ))
          )}
        </View>
        {mine && mine.length > 0 ? (
          <Pressable accessibilityRole="button" onPress={() => router.navigate('/tasks')} style={{ alignSelf: 'flex-start', paddingVertical: spacing[1] }}>
            <Text variant="label" color="primary">
              All tasks
            </Text>
          </Pressable>
        ) : null}
      </View>
    </ScrollView>
  )
}

function Inline({ children }: { children: ReactNode }) {
  const { spacing } = useTheme()
  return <View style={{ padding: spacing[4], gap: spacing[3], alignItems: 'flex-start' }}>{children}</View>
}

function Stat({ icon, label, value, failed, onPress }: { icon: IconName; label: string; value: number | undefined; failed: boolean; onPress: () => void }) {
  const { colors, radius, spacing } = useTheme()
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={`${label}: ${value ?? 'loading'}`}
      onPress={onPress}
      style={({ pressed }) => ({
        flex: 1,
        gap: spacing[2],
        padding: spacing[3],
        borderWidth: 1,
        borderColor: colors.border,
        borderRadius: radius.xl,
        backgroundColor: pressed ? colors.accent : colors.card,
      })}
    >
      <Icon name={icon} size={20} color={value ? colors.primary : colors.mutedForeground} />
      {value !== undefined ? <Text variant="title">{value}</Text> : failed ? <Text variant="title" color="muted">–</Text> : <Spinner />}
      <Text variant="caption" color="muted">
        {label}
      </Text>
    </Pressable>
  )
}
