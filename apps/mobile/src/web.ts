import { Linking } from 'react-native'

import { useOrbit } from './orbit-context'

/** Opens a page of Orbit on the web in the browser: the place for what the app does not do yet. */
export function useOpenOnWeb(): (path: string) => void {
  const { session } = useOrbit()
  return (path) => void Linking.openURL(`${session.server}${path}`)
}
