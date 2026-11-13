/**
 * Home cards on the phone (the web's `features/home/layout.ts`): one ordered list of cards per person, saved on the
 * server and shared with the web (`GET`/`PUT /api/v1/workspaces/{id}/home-layout`). The server owns the catalog and the
 * role rules (`available`: a guest gets the chat card only); these helpers only edit the list and decide what this
 * build shows. No React, so a Bun test checks them.
 */
import type { HomeCard, HomeLayout, TaskQueryBody } from '@orbit/api/generated/types.gen'

export type { HomeCard, HomeLayout }

/** What the phone knows about each card; a card it does not know (a newer server) is not shown, but stays saved. */
export const HOME_CARDS: Record<string, { title: string; href?: string }> = {
  my_tasks: { title: 'My tasks', href: '/tasks?view=mine' },
  due_soon: { title: 'Due soon', href: '/tasks?view=due_soon' },
  overdue: { title: 'Overdue', href: '/tasks?view=overdue' },
  inbox: { title: 'Unread in Inbox', href: '/activity' },
  activity: { title: 'Activity', href: '/activity' },
  unread_chat: { title: 'Unread chat', href: '/chat/unreads' },
  recent_docs: { title: 'Recently updated docs', href: '/docs' },
  recent_files: { title: 'Recent Drive files', href: '/drive' },
  ask_donna: { title: 'Ask Donna' },
  routines: { title: 'Routines', href: '/settings/workspace-agentic?tab=routines' },
}

/** Rows a card shows when the layout does not say; the server keeps a limit in 3–20. */
export const DEFAULT_CARD_LIMIT = 7
export const CARD_LIMITS = [3, 5, 7, 10, 15, 20] as const

export const cardLimit = (card: HomeCard): number => card.limit ?? DEFAULT_CARD_LIMIT

/** The card's title; Ask Donna carries the agent's name. */
export const cardTitle = (id: string, agentName: string | null): string =>
  id === 'ask_donna' && agentName ? `Ask ${agentName}` : (HOME_CARDS[id]?.title ?? id)

/**
 * Whether the phone shows a card: known here, allowed for the caller in this workspace (`available`, which also covers
 * an optimistic list that still has a card the role may not use) and, for Ask Donna, only while the person can ask the
 * agent (the rule of the Ask bar: their model connection, today ChatGPT, is connected).
 */
export function cardShown(id: string, available: readonly string[], askDonna: boolean): boolean {
  return id in HOME_CARDS && available.includes(id) && (id !== 'ask_donna' || askDonna)
}

/** The cards Home shows, in order. Edits still change the whole saved list, so a hidden card keeps its place. */
export function visibleCards(layout: Pick<HomeLayout, 'cards' | 'available'>, askDonna: boolean): HomeCard[] {
  return layout.cards.filter((card) => cardShown(card.id, layout.available, askDonna))
}

/** Cards of `available` the phone can show that are not on Home yet, in picker order. */
export function addableCards(layout: Pick<HomeLayout, 'cards' | 'available'>, askDonna: boolean): string[] {
  return layout.available.filter((id) => cardShown(id, layout.available, askDonna) && !layout.cards.some((card) => card.id === id))
}

/** Adds `id` at the end; a card already on Home stays where it is. */
export function addCard(cards: readonly HomeCard[], id: string): HomeCard[] {
  return cards.some((card) => card.id === id) ? [...cards] : [...cards, { id }]
}

export function removeCard(cards: readonly HomeCard[], id: string): HomeCard[] {
  return cards.filter((card) => card.id !== id)
}

/** Moves the card `id` into the place of `targetId`, which moves aside; an unknown id changes nothing. */
export function moveCardTo(cards: readonly HomeCard[], id: string, targetId: string): HomeCard[] {
  const from = cards.findIndex((card) => card.id === id)
  const to = cards.findIndex((card) => card.id === targetId)
  if (from === -1 || to === -1 || from === to) return [...cards]
  const next = cards.filter((card) => card.id !== id)
  next.splice(to, 0, cards[from])
  return next
}

/**
 * Moves a shown card one place up (`-1`) or down (`1`) among the shown cards, in the whole saved list: it takes the
 * place of its shown neighbour, so a hidden card between them does not swallow the tap.
 */
export function stepCard(cards: readonly HomeCard[], shown: readonly HomeCard[], id: string, step: -1 | 1): HomeCard[] {
  const index = shown.findIndex((card) => card.id === id)
  const neighbour = index === -1 ? undefined : shown[index + step]
  return neighbour ? moveCardTo(cards, id, neighbour.id) : [...cards]
}

/** The default row count is not stored, so the saved layout stays short (like the web). */
export function setCardLimit(cards: readonly HomeCard[], id: string, limit: number): HomeCard[] {
  return cards.map((card) => (card.id === id ? (limit === DEFAULT_CARD_LIMIT ? { id } : { id, limit }) : card))
}

const mine = { field: 'assignee', operator: 'is', value: ['me'] }

/** The open tasks of a task card, as the web's card asks for them (`POST /tasks/query`): always the caller's. */
export function taskCardQuery(card: string, limit: number): TaskQueryBody | null {
  const cards: Record<string, { children: object[]; order_by: TaskQueryBody['order_by']; order_direction: TaskQueryBody['order_direction'] }> = {
    my_tasks: { children: [mine], order_by: 'updated', order_direction: 'desc' },
    due_soon: {
      children: [mine, { field: 'due_date', operator: 'between', value: [{ relative: 'today' }, { relative: 'today', offset_days: 6 }] }],
      order_by: 'due_date',
      order_direction: 'asc',
    },
    overdue: { children: [mine, { field: 'due_date', operator: 'before', value: { relative: 'today' } }], order_by: 'due_date', order_direction: 'asc' },
  }
  const query = cards[card]
  if (!query) return null
  const { children, ...order } = query
  return { filter: { op: 'and', children } as unknown as TaskQueryBody['filter'], ...order, show_completed: 'none', limit }
}

/** The first `limit` items, newest `updated_at` first (Drive search sorts by folder and name). */
export function newestFirst<T extends { updated_at: string }>(items: readonly T[], limit: number): T[] {
  return [...items].sort((a, b) => b.updated_at.localeCompare(a.updated_at)).slice(0, limit)
}

/** Routines by their next run; paused ones (no next run) last. */
export function routinesByNextRun<T extends { next_run_at?: number | null }>(routines: readonly T[], limit: number): T[] {
  return [...routines].sort((a, b) => (a.next_run_at ?? Infinity) - (b.next_run_at ?? Infinity)).slice(0, limit)
}

export type SaveOutcome<R> = { kind: 'saved'; result: R } | { kind: 'failed'; error: unknown } | { kind: 'superseded' }

/**
 * Sends one save at a time and only the newest waiting one. Quick taps on a slow network then reach the server in
 * order, so an older list can never overwrite a newer one. A save that a newer one replaced before it was sent ends as
 * `superseded`. `pending()` is true while a save runs or waits.
 */
export function latestSaver<T, R>(send: (value: T) => Promise<R>) {
  let busy = false
  let waiting: { value: T; settle: (outcome: SaveOutcome<R>) => void } | null = null

  const run = async (value: T, settle: (outcome: SaveOutcome<R>) => void) => {
    busy = true
    let outcome: SaveOutcome<R>
    try {
      outcome = { kind: 'saved', result: await send(value) }
    } catch (error) {
      outcome = { kind: 'failed', error }
    }
    busy = false
    const next = waiting
    waiting = null
    // the next save starts before this one settles, so `pending()` is right in the caller's handler
    if (next) void run(next.value, next.settle)
    settle(outcome)
  }

  return {
    save(value: T): Promise<SaveOutcome<R>> {
      return new Promise((settle) => {
        if (!busy) return void run(value, settle)
        waiting?.settle({ kind: 'superseded' })
        waiting = { value, settle }
      })
    },
    pending: () => busy || waiting !== null,
  }
}
