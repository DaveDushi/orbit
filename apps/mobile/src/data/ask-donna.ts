/**
 * Ask Donna on the phone (the web's `features/askDonna`): who sees it, and which page a message to the agent was sent
 * from. No React, so a Bun test checks the same functions.
 *
 * The server checks the context's shape (`agent::inputs::PageContext`), and the agent reads the item with its tools as
 * the person, so the context names nothing the person cannot read.
 */
import type { AgentStatus } from '@orbit/api/agent'

export type PageArea = 'home' | 'tasks' | 'docs' | 'drive' | 'activity' | 'settings' | 'account' | 'admin' | 'apps' | 'other'

export type PageItemKind = 'task' | 'doc' | 'drive_folder'

/** The task or Docs page open on the screen. */
export interface PageItem {
  kind: PageItemKind
  id: string
  title: string
}

export interface PageContext {
  page: PageArea
  item: PageItem | null
}

/** What Ask Donna needs to show: the agent's name and its user (the other member of the DM). */
export interface AskDonnaAgent {
  name: string
  botId: string
}

/**
 * The agent the person can ask, or `null`: guests never have it, and the others only when the server runs the agent
 * and their own connection for the agent's model (today ChatGPT) is connected. The same rule as the web's Ask bar.
 */
export function askDonnaAgent(status: AgentStatus | undefined, guest: boolean): AskDonnaAgent | null {
  if (guest || !status?.enabled || !status.bot_user_id || !status.chatgpt.connected) return null
  return { name: status.name, botId: status.bot_user_id }
}

/** The account pages of the phone's Settings tab (the web's `/profile/*`). */
const ACCOUNT_PAGES = ['profile', 'security', 'sessions', 'notifications', 'appearance', 'agentic', 'file-storage']

const under = (pathname: string, prefix: string) => pathname === prefix || pathname.startsWith(`${prefix}/`)

/** The area of an app path; `null` in Chat, where the conversation is there already. */
export function pageAreaOf(pathname: string): PageArea | null {
  if (under(pathname, '/chat')) return null
  if (pathname === '/' || pathname === '') return 'home'
  if (under(pathname, '/tasks')) return 'tasks'
  if (under(pathname, '/docs')) return 'docs'
  if (under(pathname, '/drive')) return 'drive'
  if (under(pathname, '/activity')) return 'activity'
  if (under(pathname, '/settings/admin')) return 'admin'
  if (ACCOUNT_PAGES.some((page) => under(pathname, `/settings/${page}`))) return 'account'
  if (under(pathname, '/settings')) return 'settings'
  return 'other'
}

/** Which area an item belongs to: an item of a screen that is no longer shown is not sent. */
const ITEM_AREA: Record<PageItemKind, PageArea> = { task: 'tasks', doc: 'docs', drive_folder: 'drive' }

/** What a message sent on this path tells the agent, or `null` in Chat. */
export function pageContextFor(pathname: string, item: PageItem | null): PageContext | null {
  const page = pageAreaOf(pathname)
  if (!page) return null
  return { page, item: item && ITEM_AREA[item.kind] === page ? item : null }
}

// The one item of the screen in front. Screens publish it while they show it (`usePublishPageItem`).
let current: PageItem | null = null
const listeners = new Set<() => void>()

export function currentPageItem(): PageItem | null {
  return current
}

export function setPageItem(item: PageItem | null) {
  if (item === current) return
  current = item
  for (const listener of listeners) listener()
}

/** Drops `item` if it is still the current one (a screen that goes away). */
export function clearPageItem(item: PageItem) {
  if (current === item) setPageItem(null)
}

export function subscribePageItem(listener: () => void): () => void {
  listeners.add(listener)
  return () => listeners.delete(listener)
}
