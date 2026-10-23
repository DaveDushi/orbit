import { useQuery } from '@tanstack/react-query'
import { type Href, router } from 'expo-router'
import type { ReactNode } from 'react'
import { Pressable, RefreshControl, ScrollView, View } from 'react-native'
import { SafeAreaView } from 'react-native-safe-area-context'

import { MenuButton } from '../../components/Drawer'
import { relativeTime } from '../../data/format'
import { notificationCopy, notificationTarget } from '../../data/inbox'
import { keys } from '../../data/keys'
import * as api from '../../data/orbit'
import { dueLabel, identifier } from '../../data/tasks'
import { useOrbit } from '../../orbit-context'
import { usePages, usePeople, useProjects, useStatuses, useUnreadNotifications } from '../../queries'
import { alpha, type, useTheme } from '../../theme'
import { HeaderTitle, Icon, IconButton, PaneHeader, StatusIcon, Text } from '../../ui'

/** Most rows a card shows: Home is a short brief, the full lists live in Tasks, Activity and Docs. */
const CARD_LIMIT = 7

function greetingFor(hour: number): string {
  if (hour < 12) return 'Good morning'
  if (hour < 18) return 'Good afternoon'
  return 'Good evening'
}

export default function HomeScreen() {
  const { client, user, workspaceId } = useOrbit()
  const { colors } = useTheme()
  const projects = useProjects()
  const statuses = useStatuses()
  const people = usePeople()
  const pages = usePages()
  const unread = useUnreadNotifications()
  // the web's cards: the first rows of the "My week" and "Overdue" task pages
  const week = useQuery({
    queryKey: keys.taskView(workspaceId, 'home-week'),
    queryFn: () => api.taskPage(client, workspaceId, { view: 'my_week', sort: 'due_date', order: 'asc', limit: CARD_LIMIT }),
  })
  const overdue = useQuery({
    queryKey: keys.taskView(workspaceId, 'home-overdue'),
    queryFn: () => api.taskPage(client, workspaceId, { view: 'overdue', assignee_id: user.id, sort: 'due_date', order: 'asc', limit: CARD_LIMIT }),
  })
  const now = new Date()
  const firstName = user.display_name.split(' ')[0] ?? user.display_name

  const queries = [week, overdue, unread, pages, projects, statuses]
  const refresh = () => Promise.all(queries.map((query) => query.refetch()))
  const weekTasks = week.data?.items ?? []
  const weekCount = `${weekTasks.length}${week.data?.more ? '+' : ''}`
  const unreadItems = unread.data ?? []
  const recentDocs = [...(pages.data ?? [])].sort((a, b) => b.updated_at.localeCompare(a.updated_at)).slice(0, CARD_LIMIT)
  const statusOf = (task: api.TaskRecord) => statuses.data?.find((status) => status.id === task.status_id)
  const taskRow = (task: api.TaskRecord, index: number, list: readonly api.TaskRecord[]) => {
    const due = task.due_at ? dueLabel(task.due_at, now) : null
    return (
      <CardRow key={task.id} href={`/tasks/${task.id}`} last={index === list.length - 1}>
        <StatusIcon status={statusOf(task)} size={14} />
        <Meta>{identifier(task, projects.data ?? [])}</Meta>
        <Text numberOfLines={1} style={[type(13), { flex: 1 }]}>
          {task.title}
        </Text>
        {due ? <Meta destructive={due === 'Overdue'}>{due}</Meta> : null}
      </CardRow>
    )
  }

  return (
    <SafeAreaView edges={['top', 'left', 'right']} style={{ flex: 1, backgroundColor: colors.background }}>
      <PaneHeader>
        <MenuButton />
        <HeaderTitle size={14}>Home</HeaderTitle>
        <View style={{ flex: 1 }} />
        <IconButton icon="search" label="Search" onPress={() => router.navigate('/tasks?search=1')} />
      </PaneHeader>
      <ScrollView
        contentContainerStyle={{ paddingHorizontal: 16, paddingTop: 32, paddingBottom: 24, gap: 24 }}
        refreshControl={<RefreshControl refreshing={queries.some((query) => query.isRefetching)} onRefresh={() => void refresh()} tintColor={colors.mutedForeground} />}
      >
        <View>
          <Text style={{ ...type(24, 'semibold'), lineHeight: 28, letterSpacing: -0.6, marginBottom: 6 }}>
            {greetingFor(now.getHours())}, {firstName}
          </Text>
          <Text style={[type(13), { color: colors.mutedForeground }]}>
            <Text onPress={() => router.navigate('/tasks?view=my_week')} style={[type(13), { color: colors.mutedForeground }]}>
              {weekCount} {weekCount === '1' ? 'task' : 'tasks'} due this week
            </Text>
            {' · '}
            <Text onPress={() => router.navigate('/activity')} style={[type(13), { color: colors.mutedForeground }]}>
              {unreadItems.length} unread {unreadItems.length === 1 ? 'notification' : 'notifications'}
            </Text>
          </Text>
        </View>

        <View style={{ gap: 16 }}>
          {overdue.data && overdue.data.items.length > 0 ? (
            <HomeCard title="Overdue" href="/tasks?view=overdue">
              {overdue.data.items.map(taskRow)}
            </HomeCard>
          ) : null}

          <HomeCard title="Due this week" href="/tasks?view=my_week">
            {weekTasks.length === 0 ? <CardEmpty>Nothing due this week.</CardEmpty> : weekTasks.map(taskRow)}
          </HomeCard>

          <HomeCard title="Unread in Inbox" href="/activity">
            {unreadItems.length === 0 ? (
              <CardEmpty>You are all caught up.</CardEmpty>
            ) : (
              unreadItems.slice(0, CARD_LIMIT).map((notification, index, list) => {
                const page = pages.data?.find((item) => item.id === notification.page_id)
                const actor = people.find((person) => person.id === notification.actor_user_id)
                return (
                  <CardRow key={notification.id} href={(notificationTarget(notification) ?? '/activity') as Href} last={index === list.length - 1}>
                    <Icon name="directInbox" size={16} color={alpha(colors.mutedForeground, 0.7)} />
                    <Text numberOfLines={1} style={[type(13), { flex: 1 }]}>
                      {notificationCopy(notification, page?.title, actor?.name).title}
                    </Text>
                    <Meta>{relativeTime(notification.created_at, now)}</Meta>
                  </CardRow>
                )
              })
            )}
          </HomeCard>

          <HomeCard title="Recently updated docs" href="/docs">
            {recentDocs.length === 0 ? (
              <CardEmpty>No docs yet.</CardEmpty>
            ) : (
              recentDocs.map((page, index) => (
                <CardRow key={page.id} href={`/docs/${page.id}`} last={index === recentDocs.length - 1}>
                  {page.icon ? <Text style={type(16)}>{page.icon}</Text> : <Icon name="docs" size={16} color={alpha(colors.mutedForeground, 0.7)} />}
                  <Text numberOfLines={1} style={[type(13), { flex: 1 }]}>
                    {page.title.trim() || 'Untitled'}
                  </Text>
                  <Meta>{relativeTime(page.updated_at, now)}</Meta>
                </CardRow>
              ))
            )}
          </HomeCard>
        </View>
      </ScrollView>
    </SafeAreaView>
  )
}

/** A card with a muted title and "View all" on the card surface, the rows on an inset background panel. */
function HomeCard({ title, href, children }: { title: string; href: Href; children: ReactNode }) {
  const { colors } = useTheme()
  return (
    <View style={{ borderRadius: 10, backgroundColor: colors.card, boxShadow: `0 0 0 1px ${colors.border}` }}>
      <View style={{ minHeight: 48, flexDirection: 'row', alignItems: 'center', paddingVertical: 8, paddingRight: 8, paddingLeft: 16 }}>
        <Text style={[type(14, 'medium'), { flex: 1, color: colors.mutedForeground }]}>{title}</Text>
        <Pressable
          accessibilityRole="link"
          accessibilityLabel={`View all: ${title}`}
          onPress={() => router.navigate(href)}
          style={({ pressed }) => ({ height: 32, paddingHorizontal: 10, borderRadius: 10, justifyContent: 'center', backgroundColor: pressed ? colors.muted : 'transparent' })}
        >
          <Text style={[type(14, 'medium'), { color: colors.mutedForeground }]}>View all</Text>
        </Pressable>
      </View>
      <View style={{ borderRadius: 10, overflow: 'hidden', backgroundColor: colors.background, boxShadow: `0 0 0 1px ${colors.muted}` }}>{children}</View>
    </View>
  )
}

function CardRow({ href, last, children }: { href: Href; last: boolean; children: ReactNode }) {
  const { colors } = useTheme()
  return (
    <Pressable
      accessibilityRole="link"
      onPress={() => router.navigate(href)}
      style={({ pressed }) => ({
        minHeight: 40,
        flexDirection: 'row',
        alignItems: 'center',
        gap: 10,
        paddingHorizontal: 12,
        paddingVertical: 6,
        borderBottomWidth: last ? 0 : 1,
        borderBottomColor: colors.border,
        backgroundColor: pressed ? alpha(colors.foreground, 0.02) : 'transparent',
      })}
    >
      {children}
    </Pressable>
  )
}

function CardEmpty({ children }: { children: ReactNode }) {
  const { colors } = useTheme()
  return <Text style={[type(13), { padding: 16, color: alpha(colors.mutedForeground, 0.7) }]}>{children}</Text>
}

function Meta({ children, destructive = false }: { children: ReactNode; destructive?: boolean }) {
  const { colors } = useTheme()
  return <Text style={[type(12), { lineHeight: 16, color: destructive ? colors.destructive : alpha(colors.mutedForeground, 0.7) }]}>{children}</Text>
}
