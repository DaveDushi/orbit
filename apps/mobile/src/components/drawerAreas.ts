/** The settings areas whose links replace the app links in the new UI's drawer (the web's `settingsAreaOf`). */
export type DrawerArea = 'workspace' | 'account' | 'admin'

export const DRAWER_AREA_TITLES: Record<DrawerArea, string> = { workspace: 'Settings', account: 'Account settings', admin: 'Admin' }

interface AreaLink {
  href: string
}

const under = (pathname: string, root: string) => pathname === root || pathname.startsWith(`${root}/`)

/**
 * The area a route belongs to: the area of the link whose route is the longest prefix of it, so `/settings/admin/jobs`
 * is Admin and `/settings/profile` is Account settings, while any other page under `/settings` is the workspace's.
 */
export function drawerAreaOf(pathname: string, areas: Record<DrawerArea, readonly AreaLink[]>): DrawerArea | null {
  let found: DrawerArea | null = null
  let length = -1
  for (const [area, links] of Object.entries(areas) as [DrawerArea, readonly AreaLink[]][]) {
    for (const link of links) {
      if (link.href.length > length && under(pathname, link.href)) {
        found = area
        length = link.href.length
      }
    }
  }
  return found
}

/** The web's section labels (`settingsNav.ts`); a link not listed here sits in its area's first section. */
const SECTIONS: Record<DrawerArea, { label: string; hrefs?: readonly string[] }[]> = {
  workspace: [{ label: 'Configuration' }, { label: 'Team', hrefs: ['/settings/members'] }, { label: 'Workspace', hrefs: ['/settings/billing', '/settings/danger-zone'] }],
  account: [{ label: 'Account' }],
  admin: [{ label: 'Instance' }],
}

/** An area's links grouped under the web's section labels, in the web's section order; empty sections are left out. */
export function drawerSections<T extends AreaLink>(area: DrawerArea, links: readonly T[]): { label: string; items: T[] }[] {
  const sections = SECTIONS[area]
  const groups = sections.map((section) => ({ label: section.label, items: [] as T[] }))
  for (const link of links) {
    const index = sections.findIndex((section) => section.hrefs?.includes(link.href))
    groups[Math.max(index, 0)].items.push(link)
  }
  return groups.filter((group) => group.items.length > 0)
}
