// What people see of a member's email. Agent bot users have an address that never receives mail:
// `<handle>@agents.orbit.invalid` (the server makes the handle from the agent's name), and before
// that `agent-<user id>@orbit.invalid`.

const OLD_AGENT_HANDLE = /^agent-[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i

/** An address of the reserved `.invalid` domain: never shown. */
export const isPlaceholderEmail = (email: string) => /\.invalid$/i.test(email.trim())

/** The email people may see: empty for a placeholder. */
export const visibleEmail = (email: string) => (isPlaceholderEmail(email) ? '' : email)

/** A handle from a name, as the server makes an agent's: `Research Bot` is `research-bot`. */
export function nameHandle(name: string): string {
  const handle = name.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-+/, '').slice(0, 32).replace(/-+$/, '')
  return handle || 'agent'
}

/**
 * A member's @handle: the part of the email before `@`. An old agent `agent-<user id>` gets the handle of its name;
 * a member without a visible address (as a guest sees people) gets their name.
 */
export function memberHandle(email: string, name: string): string {
  const local = email.split('@')[0] ?? ''
  if (OLD_AGENT_HANDLE.test(local)) return nameHandle(name)
  return local || name
}

/** The line under a member's name: their email, or `@handle` for an agent. */
export const emailLine = (email: string, name: string) => visibleEmail(email) || `@${memberHandle(email, name)}`
