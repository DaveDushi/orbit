/** The workspace's own web apps (`/apps/:slug` on the web) on the phone. No React, so the tests drive it. */
import type { WorkspaceApp } from '@orbit/api/apps'

import type { IconName } from '../ui/Icon'

export type { WorkspaceApp }

/** The app icons the phone has; any other of the web's names shows the generic one. */
const ICONS: Record<string, IconName> = {
  box: 'box',
  tag: 'tag',
  card: 'card',
  people: 'people',
  message: 'messages2',
  category: 'category',
  code: 'code',
  chart: 'chart',
  cloud: 'cloudConnection',
  book: 'book',
  driver: 'driver',
  cpu: 'cpu',
  link: 'link2',
}

export const appIconName = (icon: string): IconName => ICONS[icon] ?? 'category'

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
