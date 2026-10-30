import { useEffect, useRef, useState } from 'react'
import { Link, useParams } from 'react-router'
import { appFrameUrl } from '@orbit/api/apps'
import { Pane, PaneHeader, PaneTitle } from '@/components/common/Pane'
import { Button } from '@/components/ui/button'
import { useAgentStatus } from '@/features/agent/api'
import { useCan } from '@/features/workspaces/permissions'
import { useWorkspace } from '@/features/workspaces/workspaceContext'
import { useApps } from './api'
import { openSignInPopup, watchSignInPopup } from './signInPopup'

/** `/apps/:slug`: one workspace app in a full-size frame, or a link when the app opens in its own tab. */
export function AppPage() {
  const { slug = '' } = useParams()
  const { workspace } = useWorkspace()
  const apps = useApps(workspace.id)
  const canSetUpTools = useCan('integrations.manage')
  const agent = useAgentStatus().data?.enabled === true
  // A new key loads the frame again.
  const [reloads, setReloads] = useState(0)
  const app = apps.data?.find((item) => item.slug === slug)
  const [popupBlocked, setPopupBlocked] = useState(false)
  const stopWatching = useRef<(() => void) | null>(null)
  useEffect(() => () => stopWatching.current?.(), [])

  const signIn = () => {
    if (!app) return
    stopWatching.current?.()
    const popup = openSignInPopup(window, app.url)
    setPopupBlocked(popup === null)
    if (popup) stopWatching.current = watchSignInPopup(window, popup, () => setReloads((count) => count + 1))
  }

  if (apps.isPending) return <Pane />
  if (!app) {
    return (
      <Pane className="items-center justify-center p-6 text-center">
        <h1 className="m-0 text-base font-semibold">App not found</h1>
        <p className="m-0 mt-1 text-muted-foreground">This app does not exist, or your role cannot open it.</p>
      </Pane>
    )
  }

  const embedded = app.open_mode === 'embed'
  const openInTab = <Button variant="outline" size="sm" nativeButton={false} render={<a href={app.url} target="_blank" rel="noopener noreferrer" />}>Open in a new tab</Button>
  return (
    <Pane>
      <PaneHeader>
        <PaneTitle render={<h1 className="m-0" />}>{app.name}</PaneTitle>
        <div className="ml-auto flex items-center gap-2">
          {canSetUpTools && agent ? <Button variant="ghost" size="sm" nativeButton={false} render={<Link to="/settings/agent-tools" />}>Agent tools</Button> : null}
          {embedded ? <Button variant="outline" size="sm" type="button" onClick={signIn}>Sign in to app</Button> : null}
          {embedded ? <Button variant="outline" size="sm" type="button" onClick={() => setReloads((count) => count + 1)}>Reload</Button> : null}
          {openInTab}
        </div>
      </PaneHeader>
      {embedded ? (
        <>
          <p className="m-0 border-b px-3 py-1.5 text-xs text-muted-foreground" role={popupBlocked ? 'alert' : undefined}>
            {popupBlocked ? 'Your browser blocked the sign-in window. Allow pop-ups for Orbit, then try again.' : 'If the app stays empty, use Sign in to app. Sign in once in the window that opens, then close it: the app reloads here.'}
          </p>
          <iframe key={reloads} data-slot="app-frame" className="min-h-0 w-full flex-1 border-0" src={appFrameUrl(workspace.id, app.slug)} title={app.name} />
        </>
      ) : (
        <div className="flex flex-1 flex-col items-center justify-center gap-3 p-6 text-center">
          <p className="m-0 text-muted-foreground">This app opens in its own tab.</p>
          {openInTab}
        </div>
      )}
    </Pane>
  )
}
