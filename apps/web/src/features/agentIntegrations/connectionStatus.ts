import type { AgentIntegrationConnection } from '@orbit/api/agentIntegrations'
import { fullDate } from '@/lib/format'

/** Admins see a warning this long before a sign-in ends. Matches the server's notice. */
const EXPIRY_WARNING_MS = 14 * 24 * 60 * 60 * 1000

/** What the list says about an MCP server's sign-in, and whether admins must act soon. */
export function connectionStatus(connection: AgentIntegrationConnection, now: number) {
  const who = connection.account ? `Connected as ${connection.account}` : 'Connected'
  if (connection.needs_reconnect) return { text: 'Sign-in stopped working. Click Reconnect.', warning: true }
  const end = connection.grant_expires_at
  if (end === null) return { text: who, warning: false }
  const until = fullDate(new Date(end).toISOString())
  if (end <= now) return { text: `Sign-in ended on ${until}. Click Reconnect.`, warning: true }
  if (end - now <= EXPIRY_WARNING_MS) {
    const days = Math.ceil((end - now) / (24 * 60 * 60 * 1000))
    return { text: `${who} · sign-in ends in ${days} ${days === 1 ? 'day' : 'days'} (${until}). Reconnect before then.`, warning: true }
  }
  return { text: `${who} · until ${until}`, warning: false }
}
