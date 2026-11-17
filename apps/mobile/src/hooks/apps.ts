import { readApps } from '@orbit/api/apps'
import { useQuery } from '@tanstack/react-query'
import * as WebBrowser from 'expo-web-browser'

import { appAddress, type WorkspaceApp } from '../data/apps'
import { useOrbit } from '../orbit-context'
import { showError } from './chat'

/** The apps this member may open; the list changes rarely. */
export function useWorkspaceApps(enabled = true) {
  const { client, workspaceId } = useOrbit()
  return useQuery({ queryKey: ['workspace', workspaceId, 'apps'], queryFn: () => readApps(client, workspaceId), staleTime: 5 * 60_000, enabled })
}

/** Opens an app in the in-app browser (Safari on iOS), over the app; Done comes back. */
export function useOpenWorkspaceApp() {
  return async (app: WorkspaceApp) => {
    const address = appAddress(app)
    if (!address) return showError(`Could not open ${app.name}`, new Error('The app has no web address.'))
    try {
      await WebBrowser.openBrowserAsync(address, { presentationStyle: WebBrowser.WebBrowserPresentationStyle.PAGE_SHEET })
    } catch (error) {
      showError(`Could not open ${app.name}`, error)
    }
  }
}
