/**
 * Custom web apps of a workspace. `/apps` is what this member may open; `/workspace-apps` is the setup for owners and
 * admins. Like the agent's integrations, these routes are not in the OpenAPI contract.
 */
import type { Client } from './generated/client/index'

export type AppOpenMode = 'embed' | 'new_tab'
export type AppRole = 'owner' | 'admin' | 'member'

/** One app as a member sees it. */
export interface WorkspaceApp {
  slug: string
  name: string
  icon: string
  url: string
  open_mode: AppOpenMode
}

export interface AppSignIn {
  client_id: string
  redirect_uris: string[]
  issuer: string
  discovery_url: string
}

export interface ManagedApp extends WorkspaceApp {
  id: string
  allowed_roles: AppRole[]
  sign_in: AppSignIn | null
  created_at: number
  updated_at: number
  /** Only in the answer that made the secret. Orbit cannot show it again. */
  client_secret?: string
}

export interface ManagedAppList {
  items: ManagedApp[]
  /** The server has `ORBIT_APP_KEY`, which sign-in needs. */
  can_sign_in: boolean
}

export interface ManagedAppInput {
  name?: string
  slug?: string
  icon?: string
  url?: string
  allowed_roles?: AppRole[]
  open_mode?: AppOpenMode
  /** Non-empty turns sign-in on, empty turns it off. */
  redirect_uris?: string[]
}

const apps = (workspaceId: string) => `/api/v1/workspaces/${workspaceId}/apps`
const managed = (workspaceId: string) => `/api/v1/workspaces/${workspaceId}/workspace-apps`
const json = { 'Content-Type': 'application/json' }

/** The address of the wrapper document that frames an app. */
export const appFrameUrl = (workspaceId: string, slug: string) => `${apps(workspaceId)}/${encodeURIComponent(slug)}/frame`

export async function readApps(client: Client, workspaceId: string): Promise<WorkspaceApp[]> {
  const { data } = await client.get({ url: apps(workspaceId), throwOnError: true })
  return (data as { items: WorkspaceApp[] }).items
}

export async function readManagedApps(client: Client, workspaceId: string): Promise<ManagedAppList> {
  const { data } = await client.get({ url: managed(workspaceId), throwOnError: true })
  return data as ManagedAppList
}

export async function createManagedApp(client: Client, workspaceId: string, input: ManagedAppInput): Promise<ManagedApp> {
  const { data } = await client.post({ url: managed(workspaceId), body: input, headers: json, throwOnError: true })
  return data as ManagedApp
}

export async function updateManagedApp(client: Client, workspaceId: string, id: string, input: ManagedAppInput): Promise<ManagedApp> {
  const { data } = await client.patch({ url: `${managed(workspaceId)}/${id}`, body: input, headers: json, throwOnError: true })
  return data as ManagedApp
}

export async function deleteManagedApp(client: Client, workspaceId: string, id: string): Promise<void> {
  await client.delete({ url: `${managed(workspaceId)}/${id}`, throwOnError: true })
}

export async function rotateManagedAppSecret(client: Client, workspaceId: string, id: string): Promise<ManagedApp> {
  const { data } = await client.post({ url: `${managed(workspaceId)}/${id}/rotate-secret`, throwOnError: true })
  return data as ManagedApp
}
