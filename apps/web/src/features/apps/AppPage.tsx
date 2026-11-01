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
import { blocksCookiesInFrames, chooseFrame, hasChosenFrame } from './framedSignIn'
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
  // A new key loads the frame again.
  const [reloads, setReloads] = useState(0)
  const [popupBlocked, setPopupBlocked] = useState(false)
  // On a phone the help shows only while it is of use: the sign-in has started, or the popup was blocked.
  const [signingIn, setSigningIn] = useState(false)
  const [frameChosen, setFrameChosen] = useState(() => hasChosenFrame(workspaceId, app.slug) || !blocksCookiesInFrames(navigator))
  const stopWatching = useRef<(() => void) | null>(null)
  useEffect(() => () => stopWatching.current?.(), [])

  const embedded = app.open_mode === 'embed' && frameChosen
  const reload = () => {
    setSigningIn(false)
    setReloads((count) => count + 1)
  }
  const signIn = () => {
    stopWatching.current?.()
    const popup = openSignInPopup(window, app.url)
    setPopupBlocked(popup === null)
    setSigningIn(popup !== null)
    if (popup) stopWatching.current = watchSignInPopup(window, popup, reload)
  }

  const openInTab = (label: string, className?: string, variant: 'outline' | 'default' = 'outline', size: 'sm' | 'default' = 'sm') => (
    <Button variant={variant} size={size} className={className} nativeButton={false} render={<a href={app.url} target="_blank" rel="noopener noreferrer" />}>{label}</Button>
  )
  return (
    <Pane>
      <PaneHeader>
        <Button type="button" variant="ghost" size="icon-sm" className="hidden shrink-0 text-muted-foreground/70 max-[899px]:inline-flex" aria-label="Menu" onClick={() => window.dispatchEvent(new CustomEvent('open-sidebar'))}>
          <Menu className="size-[18px]" />
        </Button>
        <PaneTitle className="min-w-0" render={<h1 className="m-0" />}>{app.name}</PaneTitle>
        {/* a phone has room for one action: the rest go in the menu */}
        <div className="ml-auto flex shrink-0 items-center gap-2 max-[899px]:hidden">
          {showTools ? <Button variant="ghost" size="sm" nativeButton={false} render={<Link to="/settings/agent-tools" />}>Agent tools</Button> : null}
          {embedded ? <Button variant="outline" size="sm" type="button" onClick={signIn}>Sign in to app</Button> : null}
          {embedded ? <Button variant="outline" size="sm" type="button" onClick={reload}>Reload</Button> : null}
          {openInTab('Open in a new tab')}
        </div>
        <div className="ml-auto hidden shrink-0 items-center gap-1 max-[899px]:flex">
          {openInTab('Open')}
          {embedded || showTools ? (
            <DropdownMenu>
              <DropdownMenuTrigger render={<Button type="button" variant="ghost" size="icon-sm" aria-label="More actions" />}>
                <MoreHorizontal className="size-4" />
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end" className="min-w-40">
                {embedded ? <DropdownMenuItem onClick={signIn}>Sign in to app</DropdownMenuItem> : null}
                {embedded ? <DropdownMenuItem onClick={reload}>Reload</DropdownMenuItem> : null}
                {showTools ? <DropdownMenuItem onClick={() => void navigate('/settings/agent-tools')}>Agent tools</DropdownMenuItem> : null}
              </DropdownMenuContent>
            </DropdownMenu>
          ) : null}
        </div>
      </PaneHeader>
      {embedded ? (
        <>
          <p data-show-on-phone={popupBlocked || signingIn} className="m-0 border-b px-3 py-1.5 text-xs text-muted-foreground max-[899px]:not-data-[show-on-phone=true]:hidden" role={popupBlocked ? 'alert' : undefined}>
            {popupBlocked ? 'Your browser blocked the sign-in window. Allow pop-ups for Orbit, then try again.' : 'If the app stays empty, use Sign in to app. Sign in once in the window that opens, then close it: the app reloads here.'}
          </p>
          <iframe key={reloads} data-slot="app-frame" className="min-h-0 w-full flex-1 border-0" src={appFrameUrl(workspaceId, app.slug)} title={app.name} />
        </>
      ) : (
        <div className="flex flex-1 flex-col items-center justify-center gap-3 p-6 text-center">
          {app.open_mode === 'embed' ? (
            <>
              <p className="m-0 max-w-sm text-muted-foreground">This browser can block sign-in inside Orbit, so {app.name} opens in its own tab.</p>
              {openInTab(`Open ${app.name}`, 'max-w-full truncate', 'default', 'default')}
              <Button variant="ghost" size="sm" type="button" onClick={() => {
                chooseFrame(workspaceId, app.slug)
                setFrameChosen(true)
              }}>Try inside Orbit</Button>
            </>
          ) : (
            <>
              <p className="m-0 text-muted-foreground">This app opens in its own tab.</p>
              {openInTab('Open in a new tab')}
            </>
          )}
        </div>
      )}
    </Pane>
  )
}
