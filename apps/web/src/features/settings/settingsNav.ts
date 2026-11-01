import { useContext, type ComponentType } from 'react'
import { ArchiveBox, Brush, Category, MagicStar, Card, Chart, ClipboardText, CloudConnection, Cpu, SecurityUser, Driver, Key, Keyboard, Link2, Lock, Notification as Bell, Setting2 as Settings, ShieldTick as ShieldCheck, SmileCircle, Sms as Mail, Sticker, Danger as TriangleAlert, User, People as Users } from 'reicon-react'
import type { Permission } from '@orbit/api/generated/types.gen'
import { WorkspaceContext } from '@/features/workspaces/workspaceContext'
import { useBillingEnabled } from '@/features/workspaces/billing'
import { useAgentStatus } from '@/features/agent/api'

/** A sub-tab inside a settings page. */
export interface SettingsTab {
  to: string
  label: string
  /** Shown only to roles that hold this permission. */
  permission?: Permission
}

export interface SettingsNavItem {
  to: string
  label: string
  icon: ComponentType<{ className?: string }>
  end?: boolean
  /** Shown only to roles that hold this permission. */
  permission?: Permission
  /** Shown only in Orbit Cloud (billing on). */
  cloud?: boolean
  /** Shown only when the server runs the team agent. */
  agent?: boolean
  /** The page's sub-tabs: the link shows when the person may open at least one. */
  tabs?: readonly SettingsTab[]
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

/** Every member reads the team instructions (the server's rule), so only Tools needs a permission. */
export const WORKSPACE_AGENT_TABS: readonly SettingsTab[] = [
  { to: '/settings/agent', label: 'General' },
  { to: '/settings/agent/tools', label: 'Tools', permission: 'integrations.manage' },
]

export const ACCOUNT_AGENT_TABS: readonly SettingsTab[] = [
  { to: '/profile/agent', label: 'Settings' },
  { to: '/profile/agent/memory', label: 'Memory' },
]

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
          { to: '/settings/agent', label: 'Agent', icon: MagicStar, agent: true, tabs: WORKSPACE_AGENT_TABS },
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
          { to: '/settings/file-storage', label: 'File storage', icon: CloudConnection, permission: 'storage.manage' },
          { to: '/settings/github', label: 'GitHub', icon: Link2 },
          { to: '/settings/api-tokens', label: 'API tokens', icon: Key, permission: 'api_tokens.manage' },
          { to: '/settings/apps', label: 'Apps', icon: Category, permission: 'apps.manage' },
        ],
      },
      {
        label: 'Advanced',
        items: [
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
          { to: '/profile/appearance', label: 'Appearance', icon: Brush },
          { to: '/profile/notifications', label: 'Notifications', icon: Bell },
          { to: '/profile/security', label: 'Security', icon: Lock },
          { to: '/profile/sessions', label: 'Sessions', icon: ShieldCheck },
          { to: '/profile/shortcuts', label: 'Keyboard shortcuts', icon: Keyboard },
          { to: '/profile/storage', label: 'Storage', icon: Driver },
          { to: '/profile/agent', label: 'Agent', icon: MagicStar, agent: true, tabs: ACCOUNT_AGENT_TABS },
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
          { to: '/admin/agent', label: 'Agent', icon: MagicStar, agent: true },
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

const under = (pathname: string, root: string) => pathname === root || pathname.startsWith(`${root}/`)

/** The settings area a route belongs to; the sidebar shows that area's links instead of the app links. */
export function settingsAreaOf(pathname: string): SettingsAreaName | null {
  if (under(pathname, '/settings')) return 'settings'
  if (under(pathname, '/profile')) return 'profile'
  if (under(pathname, '/admin')) return 'admin'
  return null
}

const permitted = (permission: Permission | undefined, permissions: readonly Permission[]) => !permission || permissions.includes(permission)

/** What decides which links and tabs a person gets. */
export interface SettingsAccess {
  permissions: readonly Permission[]
  cloud: boolean
  agent: boolean
}

export function navItemVisible(item: SettingsNavItem, { permissions, cloud, agent }: SettingsAccess) {
  return permitted(item.permission, permissions) && (!item.cloud || cloud) && (!item.agent || agent) && (!item.tabs || item.tabs.some((tab) => permitted(tab.permission, permissions)))
}

/**
 * The agent page's tabs for this person, and where to go when the address is a tab they may not open. With the agent
 * off (or its status unknown) there are no tabs and no redirect: the page answers for itself, as it does for a direct link.
 */
export function agentTabsView(tabs: readonly SettingsTab[], permissions: readonly Permission[], agent: boolean, pathname: string) {
  const allowed = agent ? tabs.filter((tab) => permitted(tab.permission, permissions)) : []
  const path = pathname.replace(/\/+$/, '')
  const current = allowed.find((tab) => tab.to === path)
  return { tabs: allowed, current, redirect: allowed.length > 0 && !current ? allowed[0].to : null }
}

/** An area's title, icon and the links this person may open. */
export function useSettingsArea(name: SettingsAreaName): SettingsArea {
  const permissions = useContext(WorkspaceContext)?.workspace.permissions ?? []
  const cloud = useBillingEnabled()
  const agent = useAgentStatus().data?.enabled === true
  const area = AREAS[name]
  const sections = area.sections
    .map((section) => ({
      ...section,
      items: section.items.filter((item) => navItemVisible(item, { permissions, cloud, agent })),
    }))
    .filter((section) => section.items.length > 0)
  return { ...area, sections }
}
