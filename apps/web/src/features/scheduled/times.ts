/** Quick choices for Send later: in an hour, tomorrow at 9:00, next Monday at 9:00 (local time). */
export function sendLaterPresets(now: Date): { label: string; at: Date }[] {
  const inAnHour = new Date(now.getTime() + 60 * 60 * 1000)
  const tomorrow = atNine(now, 1)
  const daysToMonday = ((8 - now.getDay()) % 7) || 7
  const monday = atNine(now, daysToMonday)
  return [
    { label: 'In 1 hour', at: inAnHour },
    { label: 'Tomorrow at 9:00', at: tomorrow },
    { label: 'Monday at 9:00', at: monday },
  ]
}

function atNine(now: Date, days: number): Date {
  const date = new Date(now)
  date.setDate(date.getDate() + days)
  date.setHours(9, 0, 0, 0)
  return date
}

/** The value of a `datetime-local` input for a time (local, to the minute). */
export function localInputValue(date: Date): string {
  const pad = (value: number) => String(value).padStart(2, '0')
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}T${pad(date.getHours())}:${pad(date.getMinutes())}`
}

/** A `datetime-local` value as a time, or `null` when it is empty or not valid. */
export function fromLocalInput(value: string): Date | null {
  if (!/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}/.test(value)) return null
  const date = new Date(value)
  return Number.isNaN(date.getTime()) ? null : date
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
