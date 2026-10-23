import { createOrbitClient, type OrbitClientOptions } from '@orbit/api/client'

export { CONTRACT_ID } from '@orbit/api/generated/contract'

type ApiClientOptions = Pick<OrbitClientOptions, 'fetch' | 'onUnauthorized' | 'onReadOnly'>

export const UNAUTHORIZED_EVENT = 'orbit:unauthorized'
/** A write was refused because the workspace has no subscription (Orbit Cloud); `detail` is the server's message. */
export const READ_ONLY_EVENT = 'orbit:workspace-read-only'

export function createApiClient(options: ApiClientOptions = {}) {
  return createOrbitClient({
    ...options,
    baseUrl: globalThis.location?.origin ?? 'http://localhost',
    credentials: 'include',
  })
}

export const apiClient = createApiClient({
  onUnauthorized: () => window.dispatchEvent(new Event(UNAUTHORIZED_EVENT)),
  onReadOnly: (detail) => window.dispatchEvent(new CustomEvent(READ_ONLY_EVENT, { detail })),
})
