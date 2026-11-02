import { useEffect, useId, useReducer, useRef, useState } from 'react'
import { Link, useNavigate, useParams } from 'react-router'
import { Menu, MoreH as MoreHorizontal, Xmark as X } from 'reicon-react'
import { appFrameUrl, type WorkspaceApp } from '@orbit/api/apps'
import { Pane, PaneHeader, PaneTitle } from '@/components/common/Pane'
import { Button } from '@/components/ui/button'
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from '@/components/ui/dropdown-menu'
import { useAgentStatus } from '@/features/agent/api'
import { useCan } from '@/features/workspaces/permissions'
import { useWorkspace } from '@/features/workspaces/workspaceContext'
import { useApps } from './api'
import { type AppCard, appBarView, initialAppFrame, nextAppFrame, readAppBarClosed, readAppKnownReady, rememberAppBarClosed, rememberAppKnownReady } from './appBar'
import { type AppWatch, startAppFrame, watchAppReady } from './appReady'
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
  return <AppView key={`${workspace.id}:${app.slug}`} app={app} workspaceId={workspace.id} />
}

function AppView({ app, workspaceId }: { app: WorkspaceApp; workspaceId: string }) {
  const navigate = useNavigate()
  const canSetUpTools = useCan('integrations.manage')
  const agent = useAgentStatus().data?.enabled === true
  const showTools = canSetUpTools && agent
  // A new key loads the frame again, and the rule below starts again.
  const [reloads, setReloads] = useState(0)
  const [state, dispatch] = useReducer(nextAppFrame, initialAppFrame)
  const [closed, setClosed] = useState(() => readAppBarClosed(window, workspaceId, app.slug))
  const frameRef = useRef<HTMLIFrameElement>(null)
  const watch = useRef<AppWatch | null>(null)
  const stopWatching = useRef<(() => void) | null>(null)
  useEffect(() => () => stopWatching.current?.(), [])

  const embedded = app.open_mode === 'embed'
  useEffect(() => {
    if (!embedded) return undefined
    // The app runs in the wrapper document's one frame. Read each time: the first signal makes the app known.
    const current = watchAppReady(
      window as never,
      { appOrigin: new URL(app.url).origin, orbitOrigin: window.location.origin, frame: () => frameRef.current?.contentWindow?.frames[0], knownReady: readAppKnownReady(window, workspaceId, app.slug) },
      () => {
        rememberAppKnownReady(window, workspaceId, app.slug)
        dispatch({ type: 'ready' })
      },
      (failure) => dispatch({ type: 'failed', failure }),
    )
    watch.current = current
    return () => {
      current.stop()
      watch.current = null
    }
  }, [embedded, app.url, app.slug, workspaceId, reloads])
  // The wrapper has loaded: listen to its frame, then start it.
  const startFrame = () => startAppFrame(frameRef.current?.contentDocument, { loaded: () => watch.current?.loaded(), blocked: () => watch.current?.blocked() })

  const { bar: showBar, notice, card } = appBarView({ embedded, closed, ...state })
  const closeBar = () => {
    rememberAppBarClosed(window, workspaceId, app.slug)
    setClosed(true)
  }
  const reload = () => {
    dispatch({ type: 'reload' })
    setReloads((count) => count + 1)
  }
  const signIn = () => {
    stopWatching.current?.()
    const popup = openSignInPopup(window, app.url)
    dispatch({ type: 'sign-in', opened: popup !== null })
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
            {showTools ? <Button variant="ghost" size="sm" nativeButton={false} render={<Link to="/agentic/workspace" />}>Agent tools</Button> : null}
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
                  {showTools ? <DropdownMenuItem onClick={() => void navigate('/agentic/workspace')}>Agent tools</DropdownMenuItem> : null}
                </DropdownMenuContent>
              </DropdownMenu>
            ) : null}
          </div>
          {embedded ? (
            <Button type="button" variant="ghost" size="icon-sm" className="shrink-0 text-muted-foreground" aria-label="Close bar" onClick={closeBar}>
              <X className="size-4" />
            </Button>
          ) : null}
        </PaneHeader>
      ) : null}
      {embedded ? (
        <>
          {notice ? (
            <p className="m-0 border-b px-3 py-1.5 text-xs text-muted-foreground" role={notice === 'popup-blocked' ? 'alert' : undefined}>
              {notice === 'popup-blocked' ? 'Your browser blocked the sign-in window. Allow pop-ups for Orbit, then try again.' : 'If the app stays empty, use Sign in to app. Sign in once in the window that opens, then close it: the app reloads here.'}
            </p>
          ) : null}
          <div className="relative flex min-h-0 flex-1 flex-col">
            <iframe ref={frameRef} key={reloads} data-slot="app-frame" className="min-h-0 w-full flex-1 border-0" src={appFrameUrl(workspaceId, app.slug)} title={app.name} onLoad={startFrame} />
            {card ? <SignInCard name={app.name} url={app.url} reason={card} popupBlocked={state.popupBlocked} onSignIn={signIn} onReload={reload} onShowApp={() => dispatch({ type: 'show-app' })} /> : null}
          </div>
          {/* with the bar hidden the actions stay one tap away, and on a phone also the navigation drawer */}
          {showBar ? null : (
            <DropdownMenu>
              <DropdownMenuTrigger render={<Button type="button" variant="outline" size="icon-sm" className="absolute right-5 bottom-3 z-10 bg-background text-foreground shadow-md hover:bg-muted dark:bg-background dark:hover:bg-muted" aria-label="App actions" />}>
                <MoreHorizontal className="size-4" />
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end" side="top" className="min-w-44">
                <DropdownMenuItem className="min-[900px]:hidden" onClick={openMenu}>Menu</DropdownMenuItem>
                <DropdownMenuItem onClick={signIn}>Sign in to app</DropdownMenuItem>
                <DropdownMenuItem onClick={reload}>Reload</DropdownMenuItem>
                <DropdownMenuItem render={<a href={app.url} target="_blank" rel="noopener noreferrer" />}>Open in a new tab</DropdownMenuItem>
                {showTools ? <DropdownMenuItem onClick={() => void navigate('/agentic/workspace')}>Agent tools</DropdownMenuItem> : null}
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

const CARD_REASONS: Record<AppCard, (name: string) => string> = {
  'login-required': (name) => `${name} asked you to sign in, and its login cannot open inside Orbit.`,
  'frame-blocked': (name) => `${name} tried to open a login page that Orbit does not allow in the frame.`,
  silent: (name) => `${name} did not finish loading. It may need you to sign in.`,
}

/** Over the empty frame when the app failed to load: the ways out, with the most likely one first. */
function SignInCard({ name, url, reason, popupBlocked, onSignIn, onReload, onShowApp }: { name: string; url: string; reason: AppCard; popupBlocked: boolean; onSignIn: () => void; onReload: () => void; onShowApp: () => void }) {
  const id = useId()
  return (
    <div data-slot="app-sign-in-card" className="absolute inset-0 z-10 grid place-items-center bg-background/35 p-4">
      <div role="alertdialog" aria-labelledby={`${id}-title`} aria-describedby={`${id}-reason`} className="w-full max-w-[360px] rounded-xl border bg-card p-5 text-center text-card-foreground shadow-xl">
        <h2 id={`${id}-title`} className="m-0 text-[15px] font-semibold">Sign in to {name}</h2>
        <p id={`${id}-reason`} className="m-0 mt-1.5 text-[13px] text-muted-foreground">{CARD_REASONS[reason](name)}</p>
        {popupBlocked ? <p role="alert" className="m-0 mt-2 text-[13px] text-destructive">Your browser blocked the sign-in window. Allow pop-ups for Orbit, then try again.</p> : null}
        <div className="mt-4 flex flex-wrap justify-center gap-2">
          <Button type="button" onClick={onSignIn}>Sign in to app</Button>
          <Button variant="outline" nativeButton={false} render={<a href={url} target="_blank" rel="noopener noreferrer" />}>Open in a new tab</Button>
        </div>
        <div className="mt-2 flex flex-wrap justify-center gap-2">
          <Button type="button" variant="ghost" size="sm" className="text-muted-foreground" onClick={onReload}>Reload</Button>
          <Button type="button" variant="ghost" size="sm" className="text-muted-foreground" onClick={onShowApp}>Show the app</Button>
        </div>
      </div>
    </div>
  )
}
