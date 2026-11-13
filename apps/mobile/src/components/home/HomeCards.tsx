/**
 * The Home cards (the web's `features/home/components/HomeCards.tsx`): a card frame with its edit controls, and the
 * rows of each card. Each card loads only its own data, with the existing APIs and their checks.
 */
import { type Href, router } from 'expo-router'
import type { ReactNode } from 'react'
import { Pressable, View } from 'react-native'

import { requestAskDonna } from '../../data/ask-donna'
import { conversationTitle, unreadConversations } from '../../data/chat'
import { relativeTime } from '../../data/format'
import { newestFirst, routinesByNextRun } from '../../data/home'
import { notificationActor, notificationCopy, notificationTarget } from '../../data/inbox'
import type { NotificationRecord } from '../../data/orbit'
import { dueLabel, identifier } from '../../data/tasks'
import { useRoutines } from '../../hooks/agent'
import { useLatestNotifications, useRecentDriveFiles, useTaskCard } from '../../hooks/home'
import { useOrbit } from '../../orbit-context'
import { useChatStates, useConversations, usePages, usePeople, useProjects, useStatuses, useUnreadNotifications } from '../../queries'
import { alpha, useTheme } from '../../theme'
import { Icon, type IconName, IconButton, StatusIcon, Text } from '../../ui'
import { Emoji } from '../chat/EmojiPicker'

/** What a card shows in Customize: its row count, and move and remove buttons. */
export interface CardControls {
  first: boolean
  last: boolean
  /** `null`: the card has no rows to count (Ask Donna). */
  limit: number | null
  onLimit: () => void
  onMove: (step: -1 | 1) => void
  onRemove: () => void
}

/** A card with a muted title and "View all" on the card surface, the rows on an inset background panel. */
export function HomeCardFrame({ title, href, controls, children }: { title: string; href?: string; controls: CardControls | null; children: ReactNode }) {
  const { colors } = useTheme()
  return (
    <View accessibilityLabel={title} className="rounded-lg bg-card shadow-[0_0_0_1px_var(--color-border)]">
      <View className="min-h-12 flex-row items-center gap-1 py-2 pr-2 pl-4">
        <Text numberOfLines={1} className="font-noto-medium text-[14px] flex-1 text-muted-foreground">
          {title}
        </Text>
        {controls ? (
          <>
            {controls.limit !== null ? (
              <Pressable
                accessibilityRole="button"
                accessibilityLabel={`Rows in ${title}: ${controls.limit}`}
                onPress={controls.onLimit}
                className="h-8 flex-row items-center gap-1 px-2 rounded-lg border border-border active:bg-muted"
              >
                <Text className="text-[13px]">{controls.limit} rows</Text>
                <Icon name="chevronDown" size={14} color={colors.mutedForeground} />
              </Pressable>
            ) : null}
            <IconButton icon="arrowUp" label={`Move ${title} up`} size={32} iconSize={16} color={colors.mutedForeground} disabled={controls.first} onPress={() => controls.onMove(-1)} />
            <IconButton icon="arrowDown" label={`Move ${title} down`} size={32} iconSize={16} color={colors.mutedForeground} disabled={controls.last} onPress={() => controls.onMove(1)} />
            <IconButton icon="trash" label={`Remove ${title}`} size={32} iconSize={16} color={colors.mutedForeground} onPress={controls.onRemove} />
          </>
        ) : href ? (
          <Pressable
            accessibilityRole="link"
            accessibilityLabel={`View all: ${title}`}
            onPress={() => router.navigate(href as Href)}
            className="h-8 px-2.5 rounded-lg justify-center active:bg-muted"
          >
            <Text className="font-noto-medium text-[14px] text-muted-foreground">View all</Text>
          </Pressable>
        ) : null}
      </View>
      {/* while customizing, the rows are a preview: they do not take taps */}
      <View pointerEvents={controls ? 'none' : 'auto'} className="rounded-lg overflow-hidden bg-background shadow-[0_0_0_1px_var(--color-muted)]">
        {children}
      </View>
    </View>
  )
}

function CardRow({ href, last, children }: { href: string; last: boolean; children: ReactNode }) {
  return (
    <Pressable
      accessibilityRole="link"
      onPress={() => router.navigate(href as Href)}
      className={`min-h-10 flex-row items-center gap-2.5 px-3 py-1.5 border-border active:bg-foreground/2 ${last ? 'border-b-0' : 'border-b'}`}
    >
      {children}
    </Pressable>
  )
}

function CardEmpty({ children }: { children: ReactNode }) {
  return <Text className="text-[13px] p-4 text-muted-foreground/70">{children}</Text>
}

function Meta({ children, className = 'text-muted-foreground/70' }: { children: ReactNode; className?: string }) {
  return <Text className={`text-[12px] leading-[16px] ${className}`}>{children}</Text>
}

function RowIcon({ name }: { name: IconName }) {
  const { colors } = useTheme()
  return <Icon name={name} size={16} color={alpha(colors.mutedForeground, 0.7)} />
}

function RowTitle({ children }: { children: ReactNode }) {
  return (
    <Text numberOfLines={1} className="text-[13px] flex-1">
      {children}
    </Text>
  )
}

const TASK_EMPTY: Record<string, string> = {
  overdue: 'Nothing overdue.',
  due_soon: 'Nothing due in the next 7 days.',
  my_tasks: 'No open tasks for you.',
}

function TasksCard({ card, limit }: { card: string; limit: number }) {
  const tasks = useTaskCard(card, limit)
  const projects = useProjects()
  const statuses = useStatuses()
  if (tasks.isPending) return null
  const rows = tasks.data ?? []
  if (rows.length === 0) return <CardEmpty>{TASK_EMPTY[card]}</CardEmpty>
  const now = new Date()
  return rows.map((task, index) => {
    const due = task.due_at ? dueLabel(task.due_at, now) : null
    return (
      <CardRow key={task.id} href={`/tasks/${task.id}`} last={index === rows.length - 1}>
        <StatusIcon status={statuses.data?.find((status) => status.id === task.status_id)} size={14} />
        <Meta>{identifier(task, projects.data ?? [])}</Meta>
        <RowTitle>{task.title}</RowTitle>
        {due ? <Meta className={due === 'Overdue' ? 'text-destructive' : 'text-muted-foreground/70'}>{due}</Meta> : null}
      </CardRow>
    )
  })
}

function NotificationRows({ notifications, icon }: { notifications: readonly NotificationRecord[]; icon: IconName }) {
  const pages = usePages()
  const people = usePeople()
  const now = new Date()
  return notifications.map((notification, index) => {
    const page = pages.data?.find((item) => item.id === notification.page_id)
    const actor = notificationActor(notification, people)
    return (
      <CardRow key={notification.id} href={notificationTarget(notification) ?? '/activity'} last={index === notifications.length - 1}>
        <RowIcon name={icon} />
        <RowTitle>{notificationCopy(notification, page?.title, actor?.name).title}</RowTitle>
        <Meta>{relativeTime(notification.created_at, now)}</Meta>
      </CardRow>
    )
  })
}

function InboxCard({ limit }: { limit: number }) {
  const unread = useUnreadNotifications()
  if (unread.isPending) return null
  const rows = (unread.data ?? []).slice(0, limit)
  return rows.length === 0 ? <CardEmpty>You are all caught up.</CardEmpty> : <NotificationRows notifications={rows} icon="directInbox" />
}

function ActivityCard({ limit }: { limit: number }) {
  const latest = useLatestNotifications(limit)
  if (latest.isPending) return null
  const rows = latest.data ?? []
  return rows.length === 0 ? <CardEmpty>Assignments, mentions and tasks you follow appear here.</CardEmpty> : <NotificationRows notifications={rows} icon="bell" />
}

function UnreadChatCard({ limit }: { limit: number }) {
  const { user } = useOrbit()
  const conversations = useConversations()
  const states = useChatStates()
  const people = usePeople()
  if (conversations.isPending || states.isPending) return null
  const rows = unreadConversations(conversations.data ?? [], states.data ?? [], user.id).slice(0, limit)
  if (rows.length === 0) return <CardEmpty>No unread messages.</CardEmpty>
  return rows.map(({ conversation, state }, index) => (
    <CardRow key={conversation.id} href={`/chat/${conversation.id}`} last={index === rows.length - 1}>
      <RowIcon name="chat" />
      <RowTitle>
        {conversation.kind === 'dm' ? '' : '#'}
        {conversationTitle(conversation, people, user.id)}
      </RowTitle>
      <Meta className={state.mention_count > 0 ? 'text-primary' : 'text-muted-foreground/70'}>{state.unread_count > 0 ? `${state.unread_count} new` : 'New'}</Meta>
    </CardRow>
  ))
}

function RecentDocsCard({ limit }: { limit: number }) {
  const pages = usePages()
  if (pages.isPending) return null
  const rows = newestFirst(pages.data ?? [], limit)
  if (rows.length === 0) return <CardEmpty>No docs yet.</CardEmpty>
  const now = new Date()
  return rows.map((page, index) => (
    <CardRow key={page.id} href={`/docs/${page.id}`} last={index === rows.length - 1}>
      {page.icon ? <Emoji value={page.icon} size={16} /> : <RowIcon name="docs" />}
      <RowTitle>{page.title.trim() || 'Untitled'}</RowTitle>
      <Meta>{relativeTime(page.updated_at, now)}</Meta>
    </CardRow>
  ))
}

function RecentFilesCard({ limit }: { limit: number }) {
  const files = useRecentDriveFiles()
  if (files.isPending) return null
  const rows = newestFirst(files.data ?? [], limit)
  if (rows.length === 0) return <CardEmpty>No files changed in the last 30 days.</CardEmpty>
  const now = new Date()
  return rows.map((node, index) => (
    <CardRow key={node.id} href={node.parent_id ? `/drive/${node.parent_id}` : '/drive'} last={index === rows.length - 1}>
      <RowIcon name="folder" />
      <RowTitle>{node.name}</RowTitle>
      <Meta>{relativeTime(node.updated_at, now)}</Meta>
    </CardRow>
  ))
}

function RoutinesCard({ limit }: { limit: number }) {
  const routines = useRoutines()
  if (routines.isPending) return null
  const rows = routinesByNextRun(routines.data ?? [], limit)
  if (rows.length === 0) return <CardEmpty>No routines yet. Ask the agent in Chat to make one.</CardEmpty>
  return rows.map((routine, index) => (
    <CardRow key={routine.id} href="/settings/workspace-agentic?tab=routines" last={index === rows.length - 1}>
      <RowIcon name="repeat" />
      <RowTitle>{routine.name}</RowTitle>
      <Meta>
        {routine.status === 'paused' || routine.next_run_at == null
          ? 'Paused'
          : new Date(routine.next_run_at).toLocaleString(undefined, { weekday: 'short', hour: 'numeric', minute: '2-digit' })}
      </Meta>
    </CardRow>
  ))
}

/** Opens the Ask bar's sheet (the same DM as in Chat); Home shows this card only while the person can ask. */
function AskDonnaCard({ agentName }: { agentName: string }) {
  const { colors } = useTheme()
  return (
    <View className="p-3">
      <Pressable
        accessibilityRole="button"
        accessibilityLabel={`Ask ${agentName}`}
        onPress={requestAskDonna}
        className="h-10 flex-row items-center gap-2 px-3 rounded-lg border border-border bg-popover active:bg-muted"
      >
        <Icon name="magicStar" size={18} color={colors.primary} />
        <Text className="flex-1 text-[14px] text-muted-foreground">Ask {agentName} anything…</Text>
      </Pressable>
    </View>
  )
}

/** The rows of one card. */
export function HomeCardBody({ id, limit, agentName }: { id: string; limit: number; agentName: string | null }) {
  switch (id) {
    case 'my_tasks':
    case 'due_soon':
    case 'overdue':
      return <TasksCard card={id} limit={limit} />
    case 'inbox':
      return <InboxCard limit={limit} />
    case 'activity':
      return <ActivityCard limit={limit} />
    case 'unread_chat':
      return <UnreadChatCard limit={limit} />
    case 'recent_docs':
      return <RecentDocsCard limit={limit} />
    case 'recent_files':
      return <RecentFilesCard limit={limit} />
    case 'routines':
      return <RoutinesCard limit={limit} />
    case 'ask_donna':
      return agentName ? <AskDonnaCard agentName={agentName} /> : null
    default:
      return null
  }
}
