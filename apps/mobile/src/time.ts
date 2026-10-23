/** `14:05` today, `Mon 14:05` this week, `Oct 3` before that. */
export function shortTime(iso: string, now = new Date()): string {
  const date = new Date(iso)
  const time = date.toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' })
  if (date.toDateString() === now.toDateString()) return time
  if (now.getTime() - date.getTime() < 6 * 86_400_000) return `${date.toLocaleDateString(undefined, { weekday: 'short' })} ${time}`
  return date.toLocaleDateString(undefined, { month: 'short', day: 'numeric' })
}

/** `Today`, `Yesterday`, or `Monday, October 20`: the chat day separator. */
export function dayLabel(iso: string, now = new Date()): string {
  const date = new Date(iso)
  if (date.toDateString() === now.toDateString()) return 'Today'
  const yesterday = new Date(now)
  yesterday.setDate(now.getDate() - 1)
  if (date.toDateString() === yesterday.toDateString()) return 'Yesterday'
  return date.toLocaleDateString(undefined, { weekday: 'long', month: 'long', day: 'numeric' })
}
