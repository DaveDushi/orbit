import { useState, type ComponentProps, type ReactNode } from 'react'
import { Link } from 'react-router'
import { useQuery } from '@tanstack/react-query'
import { DirectInbox as Inbox, DocumentText as FileText, Folder2, MessageText, Notification, Repeat } from 'reicon-react'
import { queryTasks } from '@orbit/api/generated/sdk.gen'
import type { TaskQueryBody } from '@orbit/api/generated/types.gen'
import { cn } from 'cn'
import { apiClient } from '@/api/client'
import { queryKeys } from '@/api/queryKeys'
import { relativeTime } from '@/lib/format'
import { taskPath } from '@/lib/taskLinks'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Emoji } from '@/components/common/Emoji'
import { TaskStatusIcon } from '@/features/tasks/components/TaskStatusIcon'
import { useAllStatuses, useProjects } from '@/features/tasks/api/projects'
import { taskFromRecord, type Task } from '@/features/tasks/api/models'
import { notificationActor, notificationCopy, notificationTarget, useLatestNotifications, useNotifications } from '@/features/inbox/api'
import { usePageTree } from '@/features/docs/api/pages'
import { useDriveSearch } from '@/features/drive/api'
import { useRoutines } from '@/features/scheduled/api'
import { useConversations, useConversationStates } from '@/features/chat/api/queries'
import { useChatContext } from '@/features/chat/api/chatContext'
import { conversationTitle, unreadGroups } from '@/features/chat/lib/sidebar'
import { useAskDonnaContext } from '@/features/askDonna/askDonnaContext'
import { useWorkspace } from '@/features/workspaces/workspaceContext'
import { docsHidden } from '@/app/shell/productNavigation'
import { useMembers } from '@/features/workspaces/api'
import type { FilterNode } from '@/features/views/viewState'
import type { NotificationRecord } from '@orbit/api/generated/types.gen'

/** One linked row inside a home card. */
function CardRow({ className, ...props }: ComponentProps<typeof Link>) {
  return <Link data-slot="card-row" className={cn('flex min-h-10 w-full min-w-0 cursor-pointer items-center gap-2.5 border-b px-3 py-1.5 text-left transition-colors last:border-b-0 hover:bg-foreground/[0.02]', className)} {...props} />
}

function CardEmpty({ children }: { children: ReactNode }) {
  return <div data-slot="card-empty" className="p-4 text-[13px] text-muted-foreground/70">{children}</div>
}

/** Quiet trailing detail of a row (identifier, time). */
function RowMeta({ className, children }: { className?: string; children: ReactNode }) {
  return <span data-slot="row-meta" className={cn('shrink-0 text-xs text-muted-foreground/70', className)}>{children}</span>
}

/** `Overdue`, `Today`, `Tomorrow` or the weekday and date. */
function dueLabel(dueAt: string, now: Date): string {
  const day = (date: Date) => new Date(date.getFullYear(), date.getMonth(), date.getDate()).getTime()
  const days = Math.round((day(new Date(dueAt)) - day(now)) / 86_400_000)
  if (days < 0) return 'Overdue'
  if (days === 0) return 'Today'
  if (days === 1) return 'Tomorrow'
  return new Date(dueAt).toLocaleDateString(undefined, { weekday: 'short', month: 'short', day: 'numeric' })
}

const mine: FilterNode = { field: 'assignee', operator: 'is', value: ['me'] }
/** The open tasks of each task card, as the matching Tasks page (`view=…`) filters them, but always the caller's. */
const TASK_CARDS: Record<string, Pick<TaskQueryBody, 'order_by' | 'order_direction'> & { children: FilterNode[] }> = {
  my_tasks: { children: [mine], order_by: 'updated', order_direction: 'desc' },
  due_soon: {
    children: [mine, { field: 'due_date', operator: 'between', value: [{ relative: 'today' }, { relative: 'today', offset_days: 6 }] }],
    order_by: 'due_date',
    order_direction: 'asc',
  },
  overdue: { children: [mine, { field: 'due_date', operator: 'before', value: { relative: 'today' } }], order_by: 'due_date', order_direction: 'asc' },
}

function TasksCard({ card, limit }: { card: string; limit: number }) {
  const { workspace } = useWorkspace()
  const projects = useProjects(workspace.id)
  const statuses = useAllStatuses(workspace.id, projects.data ?? [])
  const { children, ...order } = TASK_CARDS[card]
  const body: TaskQueryBody = { filter: { op: 'and', children } as unknown as TaskQueryBody['filter'], ...order, show_completed: 'none', limit }
  // under the task query key, so a task change refreshes the card like a task view
  const tasks = useQuery({
    queryKey: queryKeys.taskQuery(workspace.id, body),
    queryFn: async () => {
      const { data } = await queryTasks({ client: apiClient, path: { workspace_id: workspace.id }, body, throwOnError: true })
      if (!data) throw new Error('Task query response was empty.')
      return data.items
    },
  })
  const now = new Date()
  const rows: Task[] = (tasks.data ?? []).map((record) => taskFromRecord(record, projects.data?.find((project) => project.id === record.project_id)))
  if (tasks.isPending) return null
  if (rows.length === 0) return <CardEmpty>{card === 'overdue' ? 'Nothing overdue.' : card === 'due_soon' ? 'Nothing due in the next 7 days.' : 'No open tasks for you.'}</CardEmpty>
  return rows.map((task) => (
    <CardRow key={task.id} to={taskPath(task)}>
      <TaskStatusIcon status={statuses.data.find((status) => status.id === task.statusId)} />
      <RowMeta>{task.identifier}</RowMeta>
      <span className="flex-1 truncate text-[13px]">{task.title}</span>
      {task.dueAt ? <RowMeta className={cn(dueLabel(task.dueAt, now) === 'Overdue' && 'text-destructive')}>{dueLabel(task.dueAt, now)}</RowMeta> : null}
    </CardRow>
  ))
}

function NotificationRows({ notifications, icon: Icon }: { notifications: NotificationRecord[]; icon: typeof Inbox }) {
  const { workspace } = useWorkspace()
  const members = useMembers(workspace.id)
  // a doc notification names its page
  const pageTree = usePageTree(workspace.id, !docsHidden)
  return notifications.map((notification) => {
    const page = pageTree.data?.find((item) => item.id === notification.page_id)
    const actor = notificationActor(notification, members.data)
    return (
      <CardRow key={notification.id} to={notificationTarget(notification) ?? '/inbox'}>
        <Icon className="size-4 shrink-0 text-muted-foreground/70" />
        <span className="flex-1 truncate text-[13px]">{notificationCopy(notification, page?.title, actor?.name).title}</span>
        <RowMeta>{relativeTime(notification.created_at)}</RowMeta>
      </CardRow>
    )
  })
}

function InboxCard({ limit }: { limit: number }) {
  const { workspace } = useWorkspace()
  const unread = useNotifications(workspace.id, true)
  if (unread.isPending) return null
  const rows = (unread.data ?? []).slice(0, limit)
  return rows.length === 0 ? <CardEmpty>You are all caught up.</CardEmpty> : <NotificationRows notifications={rows} icon={Inbox} />
}

function ActivityCard({ limit }: { limit: number }) {
  const { workspace } = useWorkspace()
  const latest = useLatestNotifications(workspace.id, { limit })
  if (latest.isPending) return null
  const rows = latest.data?.items ?? []
  return rows.length === 0 ? <CardEmpty>Assignments, mentions and tasks you follow appear here.</CardEmpty> : <NotificationRows notifications={rows} icon={Notification} />
}

function UnreadChatCard({ limit }: { limit: number }) {
  const { workspaceId, currentUserId } = useChatContext()
  const members = useMembers(workspaceId).data ?? []
  const conversations = useConversations().data
  const states = useConversationStates().data
  // Unread conversations only (no threads), as the card's title says.
  const unread = unreadGroups(conversations ?? [], states ?? [], [])
    .flatMap((group) => (group.kind === 'conversation' ? [group] : []))
    .slice(0, limit)
  if (unread.length === 0) return <CardEmpty>No unread messages.</CardEmpty>
  return unread.map(({ conversation, state }) => (
    <CardRow key={conversation.id} to={`/chat/${conversation.id}`}>
      <MessageText className="size-4 shrink-0 text-muted-foreground/70" />
      <span className="flex-1 truncate text-[13px]">
        {conversation.kind === 'dm' ? '' : '#'}
        {conversationTitle(conversation, members, currentUserId ?? '')}
      </span>
      <RowMeta className={cn(state.mentionCount > 0 && 'text-primary')}>{state.unreadCount > 0 ? `${state.unreadCount} new` : 'New'}</RowMeta>
    </CardRow>
  ))
}

function RecentDocsCard({ limit }: { limit: number }) {
  const { workspace } = useWorkspace()
  const pageTree = usePageTree(workspace.id)
  if (pageTree.isPending) return null
  const recent = [...(pageTree.data ?? [])].sort((a, b) => b.updated_at.localeCompare(a.updated_at)).slice(0, limit)
  if (recent.length === 0) return <CardEmpty>No docs yet.</CardEmpty>
  return recent.map((page) => (
    <CardRow key={page.id} to={`/docs/${page.id}`}>
      {page.icon ? <Emoji value={page.icon} /> : <FileText className="size-4 shrink-0 text-muted-foreground/70" />}
      <span className="flex-1 truncate text-[13px]">{page.title.trim() || 'Untitled'}</span>
      <RowMeta>{relativeTime(page.updated_at)}</RowMeta>
    </CardRow>
  ))
}

/** Drive search (files changed in the last 30 days) sorts by folder and name, so the card sorts the first page by time. */
function RecentFilesCard({ limit }: { limit: number }) {
  const { workspace } = useWorkspace()
  // a whole day, so the query key stays the same between renders
  const [since] = useState(() => {
    const today = new Date()
    return new Date(today.getFullYear(), today.getMonth(), today.getDate() - 30).toISOString()
  })
  const files = useDriveSearch(workspace.id, { q: '', kind: 'file', updatedAfter: since }, true)
  if (files.isPending) return null
  const recent = (files.data?.pages[0]?.items ?? []).map((entry) => entry.node).sort((a, b) => b.updated_at.localeCompare(a.updated_at)).slice(0, limit)
  if (recent.length === 0) return <CardEmpty>No files changed in the last 30 days.</CardEmpty>
  return recent.map((node) => (
    <CardRow key={node.id} to={node.parent_id ? `/drive/${node.parent_id}` : '/drive'}>
      <Folder2 className="size-4 shrink-0 text-muted-foreground/70" />
      <span className="flex-1 truncate text-[13px]">{node.name}</span>
      <RowMeta>{relativeTime(node.updated_at)}</RowMeta>
    </CardRow>
  ))
}

function RoutinesCard({ limit }: { limit: number }) {
  const { workspace } = useWorkspace()
  const routines = useRoutines(workspace.id)
  if (routines.isPending) return null
  // the next run first; paused routines last
  const rows = [...(routines.data ?? [])].sort((a, b) => (a.next_run_at ?? Infinity) - (b.next_run_at ?? Infinity)).slice(0, limit)
  if (rows.length === 0) return <CardEmpty>No routines yet. Ask the agent in Chat to make one.</CardEmpty>
  return rows.map((routine) => (
    <CardRow key={routine.id} to="/settings/agentic/routines">
      <Repeat className="size-4 shrink-0 text-muted-foreground/70" />
      <span className="flex-1 truncate text-[13px]">{routine.name}</span>
      <RowMeta>
        {routine.status === 'paused' || routine.next_run_at === null
          ? 'Paused'
          : new Date(routine.next_run_at).toLocaleString(undefined, { weekday: 'short', hour: 'numeric', minute: '2-digit' })}
      </RowMeta>
    </CardRow>
  ))
}

function AskDonnaCard() {
  const { ask, enabled } = useAskDonnaContext()
  const [text, setText] = useState('')
  if (!enabled) return <CardEmpty>The agent is not on in this workspace, or you have not connected ChatGPT (Settings → Agentic).</CardEmpty>
  if (!ask) return <CardEmpty>Ask in Chat on a small screen: <Link to="/chat" className="underline">open Chat</Link>.</CardEmpty>
  return (
    <form
      className="flex items-center gap-2 p-3"
      onSubmit={(event) => {
        event.preventDefault()
        if (!text.trim()) return
        void ask.ask(text).then((sent) => sent && setText(''))
      }}
    >
      <Input value={text} onChange={(event) => setText(event.target.value)} placeholder={`Ask ${ask.name} anything…`} aria-label={`Ask ${ask.name}`} />
      <Button type="submit" disabled={ask.busy || !text.trim()}>Ask</Button>
    </form>
  )
}

/** The rows of one card; each card loads only its own data. */
export function HomeCardBody({ id, limit }: { id: string; limit: number }) {
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
      return <AskDonnaCard />
    default:
      return null
  }
}
