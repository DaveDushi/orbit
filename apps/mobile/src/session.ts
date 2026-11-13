import { storage } from './storage'

export interface StoredSession {
  server: string
  token: string
}

const KEY = 'orbit.session'

export async function loadSession(): Promise<StoredSession | null> {
  const raw = await storage.get(KEY)
  if (!raw) return null
  try {
    const value = JSON.parse(raw)
    return typeof value?.server === 'string' && typeof value?.token === 'string' ? value : null
  } catch {
    return null
  }
}

export const saveSession = (session: StoredSession) => storage.set(KEY, JSON.stringify(session))

const SERVER_KEY = 'orbit.server'

/** Ends the saved session but keeps its server, so the sign-in screen offers it again (also for older app versions' sessions). */
export async function clearSession() {
  const session = await loadSession()
  if (session) await storage.set(SERVER_KEY, session.server)
  await storage.remove(KEY)
}

/** The server of the last session that ended, or `null`. */
export const loadLastServer = () => storage.get(SERVER_KEY).catch(() => null)

const WORKSPACE_KEY = 'orbit.workspace'

/** The workspace the user chose last, so the app opens where they left it. */
export const loadWorkspaceId = () => storage.get(WORKSPACE_KEY)

export const saveWorkspaceId = (id: string) => storage.set(WORKSPACE_KEY, id)
