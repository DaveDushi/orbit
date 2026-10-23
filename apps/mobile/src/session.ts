import * as SecureStore from 'expo-secure-store'

export interface StoredSession {
  server: string
  token: string
}

const KEY = 'orbit.session'

export async function loadSession(): Promise<StoredSession | null> {
  const raw = await SecureStore.getItemAsync(KEY)
  if (!raw) return null
  try {
    const value = JSON.parse(raw)
    return typeof value?.server === 'string' && typeof value?.token === 'string' ? value : null
  } catch {
    return null
  }
}

export const saveSession = (session: StoredSession) => SecureStore.setItemAsync(KEY, JSON.stringify(session))

export const clearSession = () => SecureStore.deleteItemAsync(KEY)

const WORKSPACE_KEY = 'orbit.workspace'

/** The workspace the user chose last, so the app opens where they left it. */
export const loadWorkspaceId = () => SecureStore.getItemAsync(WORKSPACE_KEY)

export const saveWorkspaceId = (id: string) => SecureStore.setItemAsync(WORKSPACE_KEY, id)
