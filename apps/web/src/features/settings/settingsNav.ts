import { useContext, type ComponentType } from 'react'
import { ArchiveBox, Brush, DocumentDownload, Category, Card, Chart, ClipboardText, CloudConnection, Cpu, SecurityUser, Driver, Key, Keyboard, LampCharge, Link2, Lock, MagicStar, Notification as Bell, Setting2 as Settings, ShieldTick as ShieldCheck, SmileCircle, Sms as Mail, Sticker, Danger as TriangleAlert, User, People as Users } from 'reicon-react'
import type { Permission } from '@orbit/api/generated/types.gen'
import { WorkspaceContext } from '@/features/workspaces/workspaceContext'
import { useBillingEnabled } from '@/features/workspaces/billing'
import { useImprovementsEnabled } from '@/features/improvements/api'
import { guestMayOpen } from '@/app/shell/productNavigation'

export interface SettingsNavItem {
  to: string
  label: string
  icon: ComponentType<{ className?: string }>
  end?: boolean
  /** Shown only to roles that hold this permission. */
  permission?: Permission
  /** Shown only in Orbit Cloud (billing on). */
  cloud?: boolean
  /** Shown only while Suggest a change is on (Admin → General). */
  improvements?: boolean
}

export interface SettingsNavSection {
  label: string
  items: SettingsNavItem[]
}

export type SettingsAreaName = 'settings' | 'profile' | 'admin'

interface SettingsArea {
  title: string
  icon: SettingsNavItem['icon']
  sections: SettingsNavSection[]
}

const AREAS: Record<SettingsAreaName, SettingsArea> = {
  /** The workspace's settings. */
  settings: {
    title: 'Settings',
    icon: Settings,
    sections: [
      {
        label: 'Workspace',
        items: [
          { to: '/settings', label: 'General', icon: Settings, end: true },
          { to: '/settings/members', label: 'Members', icon: Users },
          { to: '/settings/billing', label: 'Billing', icon: Card, cloud: true },
        ],
      },
      {
        label: 'Chat',
        items: [
          { to: '/settings/emoji', label: 'Emoji', icon: SmileCircle, permission: 'chat.manage' },
          { to: '/settings/stickers', label: 'Stickers', icon: Sticker, permission: 'chat.manage' },
        ],
      },
      {
        label: 'Integrations',
        items: [
          { to: '/settings/file-storage', label: 'File storage', icon: CloudConnection },
          { to: '/settings/github', label: 'GitHub', icon: Link2 },
          { to: '/settings/agentic', label: 'Agentic', icon: MagicStar },
          { to: '/settings/api-tokens', label: 'API tokens', icon: Key, permission: 'api_tokens.manage' },
          { to: '/settings/apps', label: 'Apps', icon: Category, permission: 'apps.manage' },
        ],
      },
      {
        label: 'Advanced',
        items: [
          { to: '/settings/export', label: 'Export', icon: DocumentDownload, permission: 'workspace.export' },
          { to: '/settings/danger-zone', label: 'Danger zone', icon: TriangleAlert, permission: 'workspace.delete' },
        ],
      },
    ],
  },
  /** What belongs to the signed-in person, in every workspace. */
  profile: {
    title: 'Account settings',
    icon: User,
    sections: [
      {
        label: 'Account',
        items: [
          { to: '/profile', label: 'Profile', icon: User, end: true },
          { to: '/profile/security', label: 'Security', icon: Lock },
          { to: '/profile/sessions', label: 'Sessions', icon: ShieldCheck },
        ],
      },
      {
        label: 'Preferences',
        items: [
          { to: '/profile/appearance', label: 'Appearance', icon: Brush },
          { to: '/profile/notifications', label: 'Notifications', icon: Bell },
          { to: '/profile/shortcuts', label: 'Keyboard shortcuts', icon: Keyboard },
        ],
      },
      {
        label: 'Advanced',
        items: [
          { to: '/profile/agentic', label: 'Agentic', icon: MagicStar },
          { to: '/profile/storage', label: 'Storage', icon: Driver },
        ],
      },
    ],
  },
  /** What belongs to the whole Orbit instance. Only the root user (the account created at setup) sees it. */
  admin: {
    title: 'Admin',
    icon: SecurityUser,
    sections: [
      {
        label: 'Instance',
        items: [
          { to: '/admin', label: 'General', icon: Settings, end: true },
          { to: '/admin/users', label: 'Users', icon: Users },
          { to: '/admin/email', label: 'Email', icon: Mail },
          { to: '/admin/limits', label: 'Limits', icon: Chart },
          { to: '/admin/feedback', label: 'Feedback & fixes', icon: LampCharge, improvements: true },
        ],
      },
      {
        label: 'Data',
        items: [
          { to: '/admin/storage', label: 'Storage', icon: CloudConnection },
          { to: '/admin/backups', label: 'Backups', icon: ArchiveBox },
        ],
      },
      {
        label: 'Monitoring',
        items: [
          { to: '/admin/audit', label: 'Audit log', icon: ClipboardText },
          { to: '/admin/jobs', label: 'Background jobs', icon: Cpu },
        ],
      },
    ],
  },
}

/**
 * Where a link opens on a phone. There the area's root (`/settings`) is its menu, so the root page (`end`) has its
 * own path below it.
 */
export const phonePath = (item: SettingsNavItem) => (item.end ? `${item.to}/general` : item.to)

const under = (pathname: string, root: string) => pathname === root || pathname.startsWith(`${root}/`)

/** The settings area a route belongs to; the sidebar shows that area's links instead of the app links. */
export function settingsAreaOf(pathname: string): SettingsAreaName | null {
  if (under(pathname, '/settings')) return 'settings'
  if (under(pathname, '/profile')) return 'profile'
  if (under(pathname, '/admin')) return 'admin'
  return null
}

/** A section of the Agentic page. */
export interface AgenticTab {
  to: string
  label: string
  /** Shown only when the server runs the team agent. */
  agent?: boolean
  /** Only instance admins may open it. */
  instanceAdmin?: boolean
}

export const AGENTIC_TABS: readonly AgenticTab[] = [
  { to: '/settings/agentic', label: 'You' },
  { to: '/settings/agentic/workspace', label: 'Workspace', agent: true },
  { to: '/settings/agentic/routines', label: 'Routines', agent: true },
  { to: '/settings/agentic/admin', label: 'Admin', agent: true, instanceAdmin: true },
]

/**
 * The Agentic page's tabs for this person, and where to go when they may not open the address. With the agent off its
 * tabs hide, but a direct link still opens: the section answers for itself, as these pages always did.
 */
export function agenticTabsView(tabs: readonly AgenticTab[], instanceAdmin: boolean, agent: boolean, pathname: string) {
  const open = tabs.filter((tab) => !tab.instanceAdmin || instanceAdmin)
  const path = pathname.replace(/\/+$/, '')
  const current = open.find((tab) => tab.to === path)
  return { tabs: open.filter((tab) => !tab.agent || agent), current, redirect: current ? null : open[0].to }
}

/** An area's title, icon and the links this person may open. */
/** `profileOnly`: just the Profile page, all that someone with no workspace who is not an operator may open. */
export function useSettingsArea(name: SettingsAreaName, profileOnly = false): SettingsArea {
  const workspace = useContext(WorkspaceContext)?.workspace
  const permissions = workspace?.permissions ?? []
  // a guest sees only the pages it may open (`guestMayOpen`)
  const guest = workspace?.role === 'guest'
  const cloud = useBillingEnabled()
  const improvements = useImprovementsEnabled()
  const area = AREAS[name]
  const sections = area.sections
    .map((section) => ({
      ...section,
      items: section.items.filter((item) => (!item.permission || permissions.includes(item.permission)) && (!item.cloud || cloud) && (!item.improvements || improvements) && (!profileOnly || item.to === '/profile') && (!guest || guestMayOpen(item.to))),
    }))
    .filter((section) => section.items.length > 0)
  return { ...area, sections }
}
