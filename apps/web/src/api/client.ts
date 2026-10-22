export { CONTRACT_ID } from '@/api/generated/contract'
import { CONTRACT_ID } from '@/api/generated/contract'
import { createClient } from '@/api/generated/client/index'
import { parseProblem } from './problem'

interface ApiClientOptions {
  fetch?: (request: Request) => Promise<Response>
  onUnauthorized?: () => void
  onReadOnly?: (detail: string) => void
}

export const UNAUTHORIZED_EVENT = 'orbit:unauthorized'
/** A write was refused because the workspace has no subscription (Orbit Cloud); `detail` is the server's message. */
export const READ_ONLY_EVENT = 'orbit:workspace-read-only'

export function createApiClient(options: ApiClientOptions = {}) {
  const client = createClient({
    baseUrl: globalThis.location?.origin ?? 'http://localhost',
    credentials: 'include',
    fetch: options.fetch as typeof globalThis.fetch,
    headers: { 'X-Orbit-Contract': CONTRACT_ID },
  })

  client.interceptors.response.use(async (response) => {
    if (response.ok) return response
    const problem = await parseProblem(response)
    if (problem.status === 401 && problem.code === 'authentication_required') {
      options.onUnauthorized?.()
    }
    if (problem.status === 402 && problem.code === 'workspace_read_only') {
      options.onReadOnly?.(problem.detail)
    }
    throw problem
  })
  return client
}

export const apiClient = createApiClient({
  onUnauthorized: () => window.dispatchEvent(new Event(UNAUTHORIZED_EVENT)),
  onReadOnly: (detail) => window.dispatchEvent(new CustomEvent(READ_ONLY_EVENT, { detail })),
})
