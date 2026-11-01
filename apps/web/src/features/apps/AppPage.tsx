import { useEffect, useRef, useState } from 'react'
import { Link, useNavigate, useParams } from 'react-router'
import { Menu, MoreH as MoreHorizontal } from 'reicon-react'
import { appFrameUrl, type WorkspaceApp } from '@orbit/api/apps'
import { Pane, PaneHeader, PaneTitle } from '@/components/common/Pane'
import { Button } from '@/components/ui/button'
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from '@/components/ui/dropdown-menu'
import { useAgentStatus } from '@/features/agent/api'
import { useCan } from '@/features/workspaces/permissions'
import { useWorkspace } from '@/features/workspaces/workspaceContext'
import { useApps } from './api'
import { watchAppReady } from './appReady'
import { openSignInPopup, watchSignInPopup } from './signInPopup'

/** `/apps/:slug`: one workspace app in a full-size frame, or a link when the app opens in its own tab. */
export function AppPage() {
  const { slug = '' } = useParams()
  const { workspace } = useWorkspace()
  const apps = useApps(workspace.id)
  const app = apps.data?.find((item) => item.slug === slug)

  if (apps.isPending) return <Pane />
  if (!app) {
    return (
      <Pane className="items-center justify-center p-6 text-center">
        <h1 className="m-0 text-base font-semibold">App not found</h1>
        <p className="m-0 mt-1 text-muted-foreground">This app does not exist, or your role cannot open it.</p>
      </Pane>
    )
  }
  return <AppView key={app.slug} app={app} workspaceId={workspace.id} />
}

function AppView({ app, workspaceId }: { app: WorkspaceApp; workspaceId: string }) {
  const navigate = useNavigate()
  const canSetUpTools = useCan('integrations.manage')
  const agent = useAgentStatus().data?.enabled === true
  const showTools = canSetUpTools && agent
  // A new key loads the frame again, and the rule below starts again.
  const [reloads, setReloads] = useState(0)
  const [popupBlocked, setPopupBlocked] = useState(false)
  const [signingIn, setSigningIn] = useState(false)
  // Did the app say it loaded? `null`: not yet, and the time to say so has not ended.
  const [ready, setReady] = useState<boolean | null>(null)
  const frameRef = useRef<HTMLIFrameElement>(null)
  const stopWatching = useRef<(() => void) | null>(null)
  useEffect(() => () => stopWatching.current?.(), [])

  const embedded = app.open_mode === 'embed'
  useEffect(() => {
    if (!embedded) return undefined
    // The app runs in the wrapper document's one frame.
    return watchAppReady(window as never, new URL(app.url).origin, () => frameRef.current?.contentWindow?.frames[0], () => setReady(true), () => setReady(false))
  }, [embedded, app.url, reloads])

  // The bar is for when something did not load. Without a signal in time it shows, so no blank page lacks a way out.
  const showBar = !embedded || ready === false || signingIn || popupBlocked
  const reload = () => {
    setSigningIn(false)
    setPopupBlocked(false)
    setReady(null)
    setReloads((count) => count + 1)
  }
  const signIn = () => {
    stopWatching.current?.()
    const popup = openSignInPopup(window, app.url)
    setPopupBlocked(popup === null)
    setSigningIn(popup !== null)
    if (popup) stopWatching.current = watchSignInPopup(window, popup, reload)
  }
  const openMenu = () => window.dispatchEvent(new CustomEvent('open-sidebar'))

  const openInTab = (label: string) => (
    <Button variant="outline" size="sm" nativeButton={false} render={<a href={app.url} target="_blank" rel="noopener noreferrer" />}>{label}</Button>
  )
  return (
    <Pane className="relative">
      {showBar ? (
        <PaneHeader>
          <Button type="button" variant="ghost" size="icon-sm" className="hidden shrink-0 text-muted-foreground/70 max-[899px]:inline-flex" aria-label="Menu" onClick={openMenu}>
            <Menu className="size-[18px]" />
          </Button>
          <PaneTitle className="min-w-0" render={<h1 className="m-0" />}>{app.name}</PaneTitle>
          {/* a phone keeps the two ways out of a blank frame in the header: the rest go in the menu */}
          <div className="ml-auto flex shrink-0 items-center gap-2 max-[899px]:hidden">
            {showTools ? <Button variant="ghost" size="sm" nativeButton={false} render={<Link to="/settings/agent-tools" />}>Agent tools</Button> : null}
            {embedded ? <Button variant="outline" size="sm" type="button" onClick={signIn}>Sign in to app</Button> : null}
            {embedded ? <Button variant="outline" size="sm" type="button" onClick={reload}>Reload</Button> : null}
            {openInTab('Open in a new tab')}
          </div>
          <div className="ml-auto hidden shrink-0 items-center gap-1 max-[899px]:flex">
            {embedded ? <Button variant="outline" size="sm" type="button" aria-label="Sign in to app" onClick={signIn}>Sign in</Button> : null}
            {openInTab('Open')}
            {embedded || showTools ? (
              <DropdownMenu>
                <DropdownMenuTrigger render={<Button type="button" variant="ghost" size="icon-sm" aria-label="More actions" />}>
                  <MoreHorizontal className="size-4" />
                </DropdownMenuTrigger>
                <DropdownMenuContent align="end" className="min-w-40">
                  {embedded ? <DropdownMenuItem onClick={reload}>Reload</DropdownMenuItem> : null}
                  {showTools ? <DropdownMenuItem onClick={() => void navigate('/settings/agent-tools')}>Agent tools</DropdownMenuItem> : null}
                </DropdownMenuContent>
              </DropdownMenu>
            ) : null}
          </div>
        </PaneHeader>
      ) : null}
      {embedded ? (
        <>
          {showBar ? (
            <p className="m-0 border-b px-3 py-1.5 text-xs text-muted-foreground" role={popupBlocked ? 'alert' : undefined}>
              {popupBlocked ? 'Your browser blocked the sign-in window. Allow pop-ups for Orbit, then try again.' : 'If the app stays empty, use Sign in to app. Sign in once in the window that opens, then close it: the app reloads here.'}
            </p>
          ) : null}
          <iframe ref={frameRef} key={reloads} data-slot="app-frame" className="min-h-0 w-full flex-1 border-0" src={appFrameUrl(workspaceId, app.slug)} title={app.name} />
          {/* with the bar hidden the actions stay one tap away, and on a phone also the navigation drawer */}
          {showBar ? null : (
            <DropdownMenu>
              <DropdownMenuTrigger render={<Button type="button" variant="outline" size="icon-sm" className="absolute right-5 bottom-3 z-10 bg-background/90 opacity-70 transition-opacity hover:opacity-100 focus-visible:opacity-100" aria-label="App actions" />}>
                <MoreHorizontal className="size-4" />
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end" side="top" className="min-w-44">
                <DropdownMenuItem className="min-[900px]:hidden" onClick={openMenu}>Menu</DropdownMenuItem>
                <DropdownMenuItem onClick={signIn}>Sign in to app</DropdownMenuItem>
                <DropdownMenuItem onClick={reload}>Reload</DropdownMenuItem>
                <DropdownMenuItem render={<a href={app.url} target="_blank" rel="noopener noreferrer" />}>Open in a new tab</DropdownMenuItem>
                {showTools ? <DropdownMenuItem onClick={() => void navigate('/settings/agent-tools')}>Agent tools</DropdownMenuItem> : null}
              </DropdownMenuContent>
            </DropdownMenu>
          )}
        </>
      ) : (
        <div className="flex flex-1 flex-col items-center justify-center gap-3 p-6 text-center">
          <p className="m-0 text-muted-foreground">This app opens in its own tab.</p>
          {openInTab('Open in a new tab')}
        </div>
      )}
    </Pane>
  )
}
