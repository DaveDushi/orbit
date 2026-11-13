/** Links that open the app: the share extension opens `orbit://expo-sharing`, which is the Share screen. */
export function redirectSystemPath({ path }: { path: string; initial: boolean }): string {
  try {
    if (new URL(path).hostname === 'expo-sharing') return '/share'
  } catch {
    // a plain path (`/chat/1`): kept as it is
  }
  return path
}
