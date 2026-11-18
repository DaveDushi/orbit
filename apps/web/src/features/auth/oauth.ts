import { useQuery } from '@tanstack/react-query'
import type { OAuthConnections } from '@orbit/api/mcp'

export type ConsentProject = { id: string; name: string; workspace_id: string; workspace_name: string }
/** An external agent that an owner or admin may connect the client as; its card is read only here. */
export type ConsentAgent = { id: string; name: string; avatar_url: string | null; workspace_id: string; workspace_name: string; scopes: string[]; allow_dms: boolean; channels: string[]; projects: string[] }
export type OAuthConsent = { client_name: string; redirect_to: string; scope: string; csrf_token: string; workspaces: { id: string; name: string; logo_url?: string | null }[]; projects: ConsentProject[]; agents?: ConsentAgent[] }

export async function oauthRequest<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(path, { credentials: 'same-origin', ...init })
  if (!response.ok) throw new Error('The connection request could not be completed. Sign in again or restart the connection.')
  if (response.status === 204) return undefined as T
  return response.json() as Promise<T>
}

/** The approved MCP clients, and the separate MCP address when the server has one. */
export const useOAuthGrants = () => useQuery({ queryKey: ['oauth-grants'], queryFn: () => oauthRequest<OAuthConnections>('/api/v1/oauth/grants') })

// Keep login redirects confined to the one OAuth consent route. Never accept
// external URLs, protocol-relative paths, or client callback URLs here.
// A "Sign in with Orbit" request of a workspace app that had to wait for the login. It is a server route, so it needs a
// full page load. Only this path and only clients of apps: the server then checks the redirect URI exactly.
export function appSignInDestination(value: string | null): string | null {
  if (!value?.startsWith('/oauth/authorize?')) return null
  const url = new URL(value, 'https://orbit.invalid')
  if (url.origin !== 'https://orbit.invalid' || url.pathname !== '/oauth/authorize' || !url.searchParams.get('client_id')?.startsWith('orbit-app-')) return null
  return `/oauth/authorize${url.search}`
}

export function oauthLoginDestination(value: string | null): string {
  if (!value?.startsWith('/oauth/consent?')) return '/'
  const url = new URL(value, 'https://orbit.invalid')
  const request = url.searchParams.get('request')
  if (url.origin !== 'https://orbit.invalid' || url.pathname !== '/oauth/consent' || !request || !/^[A-Za-z0-9_-]{43}$/.test(request)) return '/'
  return `/oauth/consent?request=${request}`
}
