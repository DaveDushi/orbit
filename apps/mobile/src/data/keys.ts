/**
 * Every TanStack Query key of the app. The resource comes first and the workspace second, so a prefix such as
 * `['task', workspaceId]` reaches every task detail of one workspace.
 */
export const keys = {
  workspaces: ['workspaces'] as const,
  /** The number on the app icon: unread chat and inbox of every workspace (`GET /api/v1/push/badge`). */
  appBadge: ['app-badge'] as const,
  tasks: (workspaceId: string) => ['tasks', workspaceId] as const,
  /** A server view (`mine`, `my_week` …) or a Home card query, under the `tasks` prefix so task changes reach it. */
  taskView: (workspaceId: string, view: string) => ['tasks', workspaceId, view] as const,
  taskActivity: (workspaceId: string, taskId: string) => ['task-activity', workspaceId, taskId] as const,
  /** A filter's tasks (`POST /tasks/query`, the body as JSON) and the task trash, under `tasks` so task changes reach them. */
  taskQuery: (workspaceId: string, body: string) => ['tasks', workspaceId, 'query', body] as const,
  taskTrash: (workspaceId: string) => ['tasks', workspaceId, 'trash'] as const,
  /** Saved task views; one view's detail is under the list's prefix. */
  views: (workspaceId: string) => ['views', workspaceId] as const,
  view: (workspaceId: string, viewId: string) => ['views', workspaceId, viewId] as const,
  task: (workspaceId: string, taskId: string) => ['task', workspaceId, taskId] as const,
  comments: (workspaceId: string, taskId: string) => ['comments', workspaceId, taskId] as const,
  projects: (workspaceId: string) => ['projects', workspaceId] as const,
  statuses: (workspaceId: string) => ['statuses', workspaceId] as const,
  members: (workspaceId: string) => ['members', workspaceId] as const,
  labels: (workspaceId: string) => ['labels', workspaceId] as const,
  milestones: (workspaceId: string, projectId: string) => ['milestones', workspaceId, projectId] as const,
  cycles: (workspaceId: string, projectId: string) => ['cycles', workspaceId, projectId] as const,
  conversations: (workspaceId: string) => ['conversations', workspaceId] as const,
  chatCategories: (workspaceId: string) => ['chat-categories', workspaceId] as const,
  chatStates: (workspaceId: string) => ['chat-states', workspaceId] as const,
  /** The workspace's custom emoji, which chat events change. */
  chatEmoji: (workspaceId: string) => ['chat-emoji', workspaceId] as const,
  messages: (workspaceId: string, conversationId: string) => ['messages', workspaceId, conversationId] as const,
  /** The reader's taps and form sends on one card of the agent. */
  sentActions: (workspaceId: string, messageId: string) => ['chat-sent-actions', workspaceId, messageId] as const,
  notifications: (workspaceId: string) => ['notifications', workspaceId] as const,
  unreadNotifications: (workspaceId: string) => ['notifications', workspaceId, 'unread'] as const,
  pages: (workspaceId: string) => ['pages', workspaceId] as const,
  drive: (workspaceId: string) => ['drive', workspaceId] as const,
  /** A Drive folder's items (`top`: the top level). */
  driveChildren: (workspaceId: string, parentId: string | null) => ['drive', workspaceId, 'children', parentId ?? 'top'] as const,
  driveNode: (workspaceId: string, nodeId: string) => ['drive', workspaceId, 'node', nodeId] as const,
  page: (workspaceId: string, pageId: string) => ['page', workspaceId, pageId] as const,
  teamspaces: (workspaceId: string) => ['teamspaces', workspaceId] as const,
}

export type QueryKey = readonly string[]

/**
 * What the workspace events socket refreshes: everything but chat, which has its own live socket. Members and
 * notifications are in it, as on the web, where the event refreshes the whole workspace. `billing` is the trial or
 * subscription banner, `workspace` the agent's routines and scripts, and `agent` the agents, skills and agent tools
 * (keyed `['agent', <part>, workspaceId]`).
 */
export function workspaceDataKeys(workspaceId: string): QueryKey[] {
  return [
    ...(['tasks', 'task', 'comments', 'task-activity', 'projects', 'statuses', 'labels', 'milestones', 'cycles', 'members', 'notifications', 'pages', 'page', 'teamspaces', 'drive', 'billing', 'workspace', 'views', 'home-layout', 'apps', 'managed-apps', 'agent-memory', 'workspace-exports'] as const).map(
      (resource) => [resource, workspaceId],
    ),
    ['agent'],
  ]
}

/** What a chat resync refetches. */
export function chatKeys(workspaceId: string): QueryKey[] {
  return [keys.conversations(workspaceId), keys.chatCategories(workspaceId), keys.chatStates(workspaceId), keys.chatEmoji(workspaceId), ['chat-stickers', workspaceId], ['messages', workspaceId]]
}
