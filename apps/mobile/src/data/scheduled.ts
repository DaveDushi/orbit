/**
 * Send later and the person's scheduled messages and reminders, as on the web (`features/scheduled/times.ts`,
 * `chat/components/composer/SendLater.tsx`). Times are the phone's local time; the server gets an ISO time. No React,
 * so the tests drive it.
 */
import type { ScheduledMessage } from '@orbit/api/scheduled'

export type { ScheduledMessage }

/** Quick choices for Send later: in an hour, tomorrow at 9:00, next Monday at 9:00 (local time). */
export function sendLaterPresets(now: Date): { label: string; at: Date }[] {
  const daysToMonday = (8 - now.getDay()) % 7 || 7
  return [
    { label: 'In 1 hour', at: new Date(now.getTime() + 60 * 60 * 1000) },
    { label: 'Tomorrow at 9:00', at: atNine(now, 1) },
    { label: 'Monday at 9:00', at: atNine(now, daysToMonday) },
  ]
}

/** 9:00 local, `days` days on: the calendar day moves, so a daylight saving change does not shift the hour. */
function atNine(now: Date, days: number): Date {
  const date = new Date(now)
  date.setDate(date.getDate() + days)
  date.setHours(9, 0, 0, 0)
  return date
}

/** "Today 17:00", "Tomorrow 09:00", or "Mon, Nov 16, 09:00" (local). */
export function whenLabel(at: number | Date, now: Date = new Date()): string {
  const date = new Date(at)
  const time = date.toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' })
  const day = new Date(date.getFullYear(), date.getMonth(), date.getDate()).getTime()
  const today = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime()
  const days = Math.round((day - today) / 86_400_000)
  if (days === 0) return `Today ${time}`
  if (days === 1) return `Tomorrow ${time}`
  return `${date.toLocaleDateString(undefined, { weekday: 'short', month: 'short', day: 'numeric' })}, ${time}`
}

/** The first pick of a time field: the message's own time when it is still ahead (a failed one starts an hour from now). */
export function initialSendTime(now: Date, current?: Date): Date {
  return current && current > now ? current : new Date(now.getTime() + 60 * 60 * 1000)
}

/** Where a scheduled message goes, as its row says. */
export function scheduledWhere(message: ScheduledMessage): string {
  if (message.kind === 'reminder') return 'Reminder'
  if (message.conversation_kind === 'dm') return 'Direct message'
  return message.conversation_name ? `#${message.conversation_name}` : 'Deleted conversation'
}

/** The person's messages that wait for one conversation (not reminders), soonest first, and how many failed. */
export function scheduledHere(list: readonly ScheduledMessage[], conversationId: string): { items: ScheduledMessage[]; failed: number } {
  const items = list.filter((message) => message.kind === 'message' && message.conversation_id === conversationId).sort((a, b) => a.send_at - b.send_at)
  return { items, failed: items.filter((message) => message.status === 'failed').length }
}

/**
 * What can be scheduled: a new message of text only (files and quotes stay out: a scheduled message is text), in the
 * main list of a conversation (not a thread, not Ask Donna), and not by a guest.
 */
export const canScheduleFrom = (input: { text: string; files: number; editing: boolean; inThread: boolean; askDonna: boolean; guest: boolean }) =>
  input.text.trim() !== '' && input.files === 0 && !input.editing && !input.inThread && !input.askDonna && !input.guest
