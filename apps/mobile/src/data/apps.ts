/**
 * The workspace's own web apps (`/apps/:slug` on the web) on the phone, and the web's Settings → Apps rules, without
 * React: the icon names, the address a tap opens, the redirect list and the form checks.
 */
import type { AppRole, WorkspaceApp } from '@orbit/api/apps'

import type { IconName } from '../ui/Icon'

export type { WorkspaceApp }

export const APP_ROLES: readonly AppRole[] = ['owner', 'admin', 'member']

export const APP_OPEN_MODES = [
  { value: 'embed', label: 'Inside Orbit' },
  { value: 'new_tab', label: 'In a new tab' },
] as const

/**
 * The icon names the server accepts, in the web's picker order (`APP_ICONS`), with the app's nearest icon. The app
 * draws `category` for the ones it has no picture of; the name saved is always the web's.
 */
const ICONS: readonly [string, IconName][] = [
  ['shop', 'category'],
  ['shopping-cart', 'category'],
  ['shopping-bag', 'category'],
  ['box', 'box'],
  ['truck', 'category'],
  ['tag', 'tag'],
  ['discount', 'tag'],
  ['receipt', 'note2'],
  ['card', 'card'],
  ['wallet', 'card'],
  ['barcode', 'category'],
  ['gift', 'category'],
  ['people', 'people'],
  ['message', 'chat'],
  ['instagram', 'category'],
  ['global', 'link2'],
  ['category', 'category'],
  ['code', 'code'],
  ['chart', 'chart'],
  ['cloud', 'cloudConnection'],
  ['book', 'book'],
  ['driver', 'driver'],
  ['cpu', 'cpu'],
  ['link', 'link2'],
]

export const APP_ICON_OPTIONS = ICONS.map(([value]) => ({ value, label: value[0].toUpperCase() + value.slice(1).replace('-', ' ') }))

export const appIconName = (name: string): IconName => ICONS.find(([value]) => value === name)?.[1] ?? 'category'

/**
 * What a tap opens: the app's own address in the in-app browser. The web frames an `embed` app in an Orbit page that
 * signs in with the browser's cookie; the phone signs in with a token, so both kinds open the app itself. Only https
 * (and http for a local test server) opens.
 */
export function appAddress(app: Pick<WorkspaceApp, 'url'>): string | null {
  try {
    const url = new URL(app.url)
    return url.protocol === 'https:' || url.protocol === 'http:' ? url.toString() : null
  } catch {
    return null
  }
}

/** One redirect URI per line, without blank lines. */
export const redirectLines = (text: string) => text.split('\n').map((line) => line.trim()).filter(Boolean)

export const sameRoles = (a: readonly AppRole[], b: readonly AppRole[]) => a.length === b.length && a.every((role) => b.includes(role))

/** A new app needs a name, an address and a role; with sign-in on, also a redirect URI. */
export const appFormReady = (form: { name: string; url: string; roles: readonly AppRole[]; signIn: boolean; redirects: string }) =>
  form.name.trim() !== '' && form.url.trim() !== '' && form.roles.length > 0 && (!form.signIn || redirectLines(form.redirects).length > 0)
