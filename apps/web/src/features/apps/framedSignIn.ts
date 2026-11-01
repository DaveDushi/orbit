/**
 * Whether this browser blocks third-party cookies in frames by default: Safari, and every browser on iOS, because they
 * all use WebKit. A login that passes through another site in the frame (Cloudflare Access has its own domain) then
 * cannot finish, and a page cannot detect this. So on these browsers the app opens in its own tab first.
 */
export function blocksCookiesInFrames({ userAgent, maxTouchPoints }: { userAgent: string; maxTouchPoints: number }): boolean {
  // iPadOS says it is a Mac, but a Mac has no touch screen.
  if (/iPhone|iPad|iPod/.test(userAgent) || (/Macintosh/.test(userAgent) && maxTouchPoints > 1)) return true
  return /Safari\//.test(userAgent) && !/Chrome|Chromium|Edg|OPR|Android/.test(userAgent)
}

const storageKey = (workspaceId: string, slug: string) => `orbit.apps.frame.${workspaceId}.${slug}`

/** The person chose to try this app inside Orbit: an app with no such login works there, so the choice stays. */
export function hasChosenFrame(workspaceId: string, slug: string): boolean {
  try {
    return globalThis.localStorage?.getItem(storageKey(workspaceId, slug)) === '1'
  } catch {
    return false
  }
}

export function chooseFrame(workspaceId: string, slug: string): void {
  try {
    globalThis.localStorage?.setItem(storageKey(workspaceId, slug), '1')
  } catch {
    // Private browsing: the choice lasts for this page only.
  }
}
