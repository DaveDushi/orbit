/**
 * The settings areas' links (the web's `settingsNav.ts`) and who may open each, without React, so Bun tests check the
 * role gates.
 */
import type { Href } from 'expo-router'
import type { Permission } from '@orbit/api/generated/types.gen'

import type { IconName } from '../ui/Icon'
import { guestMayOpen } from './guest'

export interface SettingsNavItem {
  href: Href & string
  label: string
  icon: IconName
  /** Shown only to roles that hold this permission. */
  permission?: Permission
  /** Shown only when the server runs the team agent. */
  agent?: boolean
  /** Shown only while Suggest a change is on (Admin → General). */
  improvements?: boolean
  /** Shown only in Orbit Cloud (billing on). */
  cloud?: boolean
}

/** What some links need besides a permission (`allowedNav`). */
export interface NavFlags {
  agent: boolean
  improvements: boolean
  cloud: boolean
}

/**
 * The web's settings areas at phone size. Keyboard shortcuts and Storage (this browser's chat cache) have no phone
 * counterpart, so they are left out.
 */
export const WORKSPACE_NAV: readonly SettingsNavItem[] = [
  { href: '/settings', label: 'General', icon: 'settings' },
  { href: '/settings/members', label: 'Members', icon: 'people' },
  { href: '/settings/billing', label: 'Billing', icon: 'card', cloud: true },
  { href: '/settings/emoji', label: 'Emoji', icon: 'emojiHappy', permission: 'chat.manage' },
  { href: '/settings/stickers', label: 'Stickers', icon: 'sticker', permission: 'chat.manage' },
  { href: '/settings/reports', label: 'Reports', icon: 'flag', permission: 'chat.manage' },
  { href: '/settings/file-storage', label: 'File storage', icon: 'cloudConnection' },
  { href: '/settings/github', label: 'GitHub', icon: 'link2' },
  { href: '/settings/agentic', label: 'Agentic', icon: 'magicStar' },
  { href: '/settings/api-tokens', label: 'API tokens', icon: 'key', permission: 'api_tokens.manage' },
  { href: '/settings/apps', label: 'Apps', icon: 'category', permission: 'apps.manage' },
  { href: '/settings/export', label: 'Export', icon: 'documentDownload', permission: 'workspace.export' },
  { href: '/settings/danger-zone', label: 'Danger zone', icon: 'danger', permission: 'workspace.delete' },
]

/**
 * The links of `nav` this workspace role may open: the agent's page only when the server runs the agent, Feedback &
 * fixes only while Suggest a change is on, Billing only in Orbit Cloud, and for a guest only its own settings
 * (`guestMayOpen`).
 */
export function allowedNav<T extends SettingsNavItem>(nav: readonly T[], permissions: readonly Permission[], flags: Partial<NavFlags> = {}, guest = false): T[] {
  return nav.filter(
    (item) =>
      (!item.permission || permissions.includes(item.permission)) &&
      (!item.agent || !!flags.agent) &&
      (!item.improvements || !!flags.improvements) &&
      (!item.cloud || !!flags.cloud) &&
      (!guest || guestMayOpen(item.href)),
  )
}

export const ACCOUNT_NAV: readonly SettingsNavItem[] = [
  { href: '/settings/profile', label: 'Profile', icon: 'user' },
  { href: '/settings/security', label: 'Security', icon: 'lock' },
  { href: '/settings/sessions', label: 'Sessions', icon: 'shield' },
  { href: '/settings/blocked-people', label: 'Blocked people', icon: 'shieldCross' },
  { href: '/settings/appearance', label: 'Appearance', icon: 'brush' },
  { href: '/settings/notifications', label: 'Notifications', icon: 'bell' },
]

/** The web's Admin pages in its order; General is the area's first page, as `/admin` is on the web. */
export const ADMIN_NAV: readonly SettingsNavItem[] = [
  { href: '/settings/admin', label: 'General', icon: 'settings' },
  { href: '/settings/admin/users', label: 'Users', icon: 'people' },
  { href: '/settings/admin/email', label: 'Email', icon: 'mail' },
  { href: '/settings/admin/limits', label: 'Limits', icon: 'chart' },
  { href: '/settings/admin/feedback', label: 'Feedback & fixes', icon: 'lampOn', improvements: true },
  { href: '/settings/admin/storage', label: 'Storage', icon: 'cloudConnection' },
  { href: '/settings/admin/backups', label: 'Backups', icon: 'archiveBox' },
  { href: '/settings/admin/audit', label: 'Audit log', icon: 'clipboardText' },
  { href: '/settings/admin/jobs', label: 'Background jobs', icon: 'cpu' },
]


export interface SettingsSection {
  title: 'Workspace' | 'Account' | 'Admin'
  items: SettingsNavItem[]
}

/**
 * The Settings tab's one page: a row for every settings page this person may open, in three sections. Workspace leaves
 * out General (the page itself shows it); a guest has none of it. Admin is the installation's, so it follows
 * `installationAdmin` only. An empty section is left out.
 */
export function settingsSections(permissions: readonly Permission[], flags: Partial<NavFlags>, guest: boolean, installationAdmin: boolean): SettingsSection[] {
  const sections: SettingsSection[] = [
    { title: 'Workspace', items: guest ? [] : allowedNav(WORKSPACE_NAV, permissions, flags).filter((item) => item.href !== '/settings') },
    { title: 'Account', items: allowedNav(ACCOUNT_NAV, permissions, flags, guest) },
    { title: 'Admin', items: installationAdmin ? allowedNav(ADMIN_NAV, permissions, flags) : [] },
  ]
  return sections.filter((section) => section.items.length > 0)
}
