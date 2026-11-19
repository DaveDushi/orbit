import type { ExternalAgentScope } from '@orbit/api/agent'

/** The permissions on an external agent's card, by area. `tasks:read` is always on. */
export const SCOPE_ROWS: { label: string; read: ExternalAgentScope; write: ExternalAgentScope; hint: string }[] = [
  { label: 'Chat', read: 'chat:read', write: 'chat:write', hint: 'Write: answer and show that it is working, in its channels and DMs only.' },
  { label: 'Tasks', read: 'tasks:read', write: 'tasks:write', hint: 'In its projects only. It never deletes a task.' },
  { label: 'Docs', read: 'docs:read', write: 'docs:write', hint: "In its projects' teamspaces only." },
]
/** "Last seen": its last MCP call. */
export function lastSeenLabel(millis: number | null | undefined): string {
  if (!millis) return 'Never connected'
  const minutes = Math.round((Date.now() - millis) / 60_000)
  if (minutes < 2) return 'Seen just now'
  if (minutes < 60) return `Seen ${minutes} min ago`
  return `Seen ${new Date(millis).toLocaleString(undefined, { month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit' })}`
}

/** The permissions of an external agent's card, in one short line. */
export function scopesLabel(scopes: ExternalAgentScope[]): string {
  const areas = SCOPE_ROWS.flatMap((row) => (scopes.includes(row.write) ? [`${row.label} write`] : scopes.includes(row.read) && row.read !== 'tasks:read' ? [`${row.label} read`] : []))
  return areas.length ? areas.join(', ') : 'Task reads only'
}
