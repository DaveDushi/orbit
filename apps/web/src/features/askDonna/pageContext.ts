import { useEffect, useSyncExternalStore } from 'react'

/**
 * The page a message to the agent was sent from (the Ask bar and its panel). The server checks the shape
 * (`agent::inputs::PageContext`) and the agent gets it as data after the message: it reads the item with its tools, as
 * the person, so the context names nothing the person cannot read.
 */
export type PageArea = 'home' | 'tasks' | 'docs' | 'drive' | 'activity' | 'settings' | 'account' | 'admin' | 'apps' | 'other'

export type PageItemKind = 'task' | 'doc' | 'drive_folder'

/** The task, Docs page or Drive folder open on the page. */
export interface PageItem {
  kind: PageItemKind
  id: string
  title: string
}

export interface PageContext {
  page: PageArea
  item: PageItem | null
}

const AREAS: [prefix: string, area: PageArea][] = [
  ['/tasks', 'tasks'],
  ['/views', 'tasks'],
  ['/tasks-trash', 'tasks'],
  ['/docs', 'docs'],
  ['/drive', 'drive'],
  ['/activity', 'activity'],
  ['/settings', 'settings'],
  ['/profile', 'account'],
  ['/admin', 'admin'],
  ['/apps', 'apps'],
]

/** Which area an item belongs to: an item published by a page that is no longer shown is not sent. */
const ITEM_AREA: Record<PageItemKind, PageArea> = { task: 'tasks', doc: 'docs', drive_folder: 'drive' }

const under = (pathname: string, prefix: string) => pathname === prefix || pathname.startsWith(`${prefix}/`)

/** The area of a path; `null` in Chat, which has no Ask bar (the conversation is there already). */
export function pageAreaOf(pathname: string): PageArea | null {
  if (under(pathname, '/chat') || under(pathname, '/dm')) return null
  if (pathname === '/' || pathname === '') return 'home'
  return AREAS.find(([prefix]) => under(pathname, prefix))?.[1] ?? 'other'
}

/** What a message sent on this path tells the agent, or `null` in Chat. */
export function pageContextFor(pathname: string, item: PageItem | null): PageContext | null {
  const page = pageAreaOf(pathname)
  if (!page) return null
  return { page, item: item && ITEM_AREA[item.kind] === page ? item : null }
}

// The one item of the page on screen. Pages publish it while they show it (`usePublishPageItem`).
let current: PageItem | null = null
const listeners = new Set<() => void>()

export function setPageItem(item: PageItem | null) {
  if (item === current) return
  current = item
  for (const listener of listeners) listener()
}

function subscribe(listener: () => void) {
  listeners.add(listener)
  return () => listeners.delete(listener)
}

export function usePageItem(): PageItem | null {
  return useSyncExternalStore(subscribe, () => current)
}

/** A page names its open task, Docs page or Drive folder while it shows it; `null` (or a missing title) names none. */
export function usePublishPageItem(kind: PageItemKind, id: string | null | undefined, title: string | null | undefined) {
  useEffect(() => {
    if (!id || title == null) return
    const item = { kind, id, title }
    setPageItem(item)
    return () => {
      if (current === item) setPageItem(null)
    }
  }, [kind, id, title])
}
