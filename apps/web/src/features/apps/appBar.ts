/** A person can close the bar of an app that sends no ready signal. This browser remembers it, per workspace and app. */

type Host = { localStorage: Pick<Storage, 'getItem' | 'setItem'> }

const closedKey = (workspaceId: string, slug: string) => `orbit:app-bar-closed:${workspaceId}:${slug}`

export function readAppBarClosed(host: Host, workspaceId: string, slug: string): boolean {
  try {
    return host.localStorage.getItem(closedKey(workspaceId, slug)) === '1'
  } catch {
    return false
  }
}

export function rememberAppBarClosed(host: Host, workspaceId: string, slug: string): void {
  try {
    host.localStorage.setItem(closedKey(workspaceId, slug), '1')
  } catch {
    // Storage full or disabled: the bar closes until the page loads again.
  }
}

type AppBarState = { embedded: boolean; ready: boolean | null; signingIn: boolean; popupBlocked: boolean; closed: boolean }

/**
 * The bar is for when something did not load. Without a signal in time it shows, so no blank page lacks a way out,
 * unless the person closed it: then the corner menu has the actions. A blocked popup always says so.
 */
export function appBarView({ embedded, ready, signingIn, popupBlocked, closed }: AppBarState): { bar: boolean; notice: 'popup-blocked' | 'help' | null } {
  const bar = !embedded || (!closed && (ready === false || signingIn || popupBlocked))
  if (!embedded) return { bar, notice: null }
  return { bar, notice: popupBlocked ? 'popup-blocked' : bar ? 'help' : null }
}
