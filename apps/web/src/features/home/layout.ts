/**
 * The Home layout: an ordered list of cards, each once, with an optional row count. Pure helpers, no React. The
 * server owns the catalog and the role rules (`available`); these only edit the list the person customizes.
 */

import type { HomeCard } from '@orbit/api/generated/types.gen'

export type HomeCardEntry = HomeCard

/** What the Home page knows about each card the server may offer; a card the web does not know is not shown. */
export const HOME_CARDS: Record<string, { title: string; viewAllTo?: string }> = {
  my_tasks: { title: 'My tasks', viewAllTo: '/tasks?view=mine' },
  due_soon: { title: 'Due soon', viewAllTo: '/tasks?view=due_soon' },
  overdue: { title: 'Overdue', viewAllTo: '/tasks?view=overdue' },
  inbox: { title: 'Unread in Inbox', viewAllTo: '/inbox' },
  activity: { title: 'Activity', viewAllTo: '/inbox' },
  unread_chat: { title: 'Unread chat', viewAllTo: '/chat/unreads' },
  recent_docs: { title: 'Recently updated docs', viewAllTo: '/docs' },
  recent_files: { title: 'Recent Drive files', viewAllTo: '/drive?date=30' },
  ask_donna: { title: 'Ask Donna' },
  routines: { title: 'Routines', viewAllTo: '/settings/agentic/routines' },
}

/** Rows a card shows when the layout does not say; the server keeps a limit in 3–20. */
export const DEFAULT_CARD_LIMIT = 7
export const CARD_LIMITS = [3, 5, 7, 10, 15, 20] as const

export function cardLimit(card: HomeCardEntry): number {
  return card.limit ?? DEFAULT_CARD_LIMIT
}

/** Adds `id` at the end; a card already on Home stays where it is. */
export function addCard(cards: readonly HomeCardEntry[], id: string): HomeCardEntry[] {
  return cards.some((card) => card.id === id) ? [...cards] : [...cards, { id }]
}

export function removeCard(cards: readonly HomeCardEntry[], id: string): HomeCardEntry[] {
  return cards.filter((card) => card.id !== id)
}

/** Moves the card `id` to index `to` (clamped to the list); an unknown id changes nothing. */
export function moveCard(cards: readonly HomeCardEntry[], id: string, to: number): HomeCardEntry[] {
  const from = cards.findIndex((card) => card.id === id)
  if (from === -1) return [...cards]
  const next = cards.filter((card) => card.id !== id)
  next.splice(Math.max(0, Math.min(to, next.length)), 0, cards[from])
  return next
}

/** Where a dragged card lands when dropped on `targetId`: in the target's place, so the target moves aside. */
export function dropCard(cards: readonly HomeCardEntry[], draggedId: string, targetId: string): HomeCardEntry[] {
  const to = cards.findIndex((card) => card.id === targetId)
  return to === -1 || draggedId === targetId ? [...cards] : moveCard(cards, draggedId, to)
}

/** The default row count is not stored, so the saved layout stays short. */
export function setCardLimit(cards: readonly HomeCardEntry[], id: string, limit: number): HomeCardEntry[] {
  return cards.map((card) => (card.id === id ? (limit === DEFAULT_CARD_LIMIT ? { id } : { id, limit }) : card))
}

/** Cards of `available` that are not on Home yet, in picker order. */
export function addableCards(cards: readonly HomeCardEntry[], available: readonly string[]): string[] {
  return available.filter((id) => !cards.some((card) => card.id === id))
}
