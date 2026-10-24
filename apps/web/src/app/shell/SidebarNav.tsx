import { Kbd } from '@/components/ui/kbd'
import { shortcutText } from '@/shortcuts/shortcutText'
import { useBindings } from '@/shortcuts/useCommand'
import { createContext, Fragment, useContext, type ComponentType, type ReactNode } from 'react'
import { Link, useLocation, useMatch, useResolvedPath, type LinkProps } from 'react-router'
import { cva, type VariantProps } from 'class-variance-authority'
import { Calendar, CalendarTick, Danger, SecurityUser, Home2 as Home, Layer, Notification as Bell, Message as MessageSquare, DocumentText as FileText, Search4 as Search, Setting2 as Settings, Sms as Mail, TaskSquare as SquareCheck, Timer } from 'reicon-react'
import { cn } from 'cn'
import { Button } from '@/components/ui/button'
import { Separator } from '@/components/ui/separator'
import { Tip } from '@/components/common/Tip'
import { chatEnabled, docsHidden } from './productNavigation'

const WORKSPACE_LINKS = [
  { to: '/', label: 'Home', icon: Home, enabled: true },
  { to: '/tasks', label: 'Tasks', icon: SquareCheck, enabled: true },
  { to: '/docs', label: 'Docs', icon: FileText, enabled: true },
  { to: '/chat', label: 'Chat', icon: MessageSquare, enabled: false },
  { to: '/mail', label: 'Mail', icon: Mail, enabled: false },
].filter((link) => !(docsHidden && link.to === '/docs'))

// Coolify tile navigation: the row never fills; the icon sits in a 3D tile and the current row shows a pink tile.
const sidebarNavItemVariants = cva(
  'group/nav-item relative flex h-9 w-full min-w-0 shrink-0 items-center justify-start gap-2.5 overflow-hidden pr-2.5 pl-1 text-left text-[13px] font-medium whitespace-nowrap text-sidebar-nav transition-colors duration-120 outline-none hover:text-sidebar-nav-active focus-visible:text-sidebar-nav-active data-[active=true]:text-sidebar-nav-active group-data-[collapsed=true]/sidebar-nav:justify-center group-data-[collapsed=true]/sidebar-nav:px-0',
  {
    variants: {
      size: {
        default: '',
        // nested under its parent entry while expanded: a label behind a 1px guide line, with a neutral pill when current;
        // a plain tile while collapsed
        sub: 'group-data-[collapsed=false]/sidebar-nav:ml-5 group-data-[collapsed=false]/sidebar-nav:h-7 group-data-[collapsed=false]/sidebar-nav:w-[calc(100%-20px)] group-data-[collapsed=false]/sidebar-nav:rounded-r-md group-data-[collapsed=false]/sidebar-nav:border-l group-data-[collapsed=false]/sidebar-nav:border-black/8 group-data-[collapsed=false]/sidebar-nav:pl-3 group-data-[collapsed=false]/sidebar-nav:data-[active=true]:bg-black/5 dark:group-data-[collapsed=false]/sidebar-nav:border-sidebar-tint/10 dark:group-data-[collapsed=false]/sidebar-nav:data-[active=true]:bg-white/6 group-data-[collapsed=false]/sidebar-nav:[&>[data-slot=sidebar-nav-tile]]:hidden',
      },
    },
    defaultVariants: { size: 'default' },
  },
)

const sidebarNavTileVariants = cva(
  // 30px: a 16px icon, 6px padding and a 1px border, resting on a 2px ledge
  'box-content flex size-4 shrink-0 items-center justify-center rounded-lg border border-black/9 bg-neutral-50 p-1.5 shadow-[0_2px_0_var(--tile-depth)] transition-[translate,box-shadow,border-color,background-color] duration-80 [--tile-depth:rgb(0_0_0/0.22)] group-hover/nav-item:-translate-y-px group-hover/nav-item:border-black/18 group-hover/nav-item:bg-white group-hover/nav-item:shadow-[0_3px_0_var(--tile-depth)] group-active/nav-item:translate-y-0.5 group-active/nav-item:shadow-none group-disabled/nav-item:translate-y-0 group-disabled/nav-item:border-black/9 group-disabled/nav-item:bg-neutral-50 group-disabled/nav-item:shadow-[0_2px_0_var(--tile-depth)] dark:[--tile-depth:rgb(255_255_255/0.05)] [&_svg]:size-4 '
    // current: a pink-edged white tile in light mode, the pink gradient tile in dark mode (no glow: the row clips it)
    + 'group-data-[active=true]/nav-item:border-primary/45 group-data-[active=true]/nav-item:bg-white group-data-[active=true]/nav-item:text-primary group-data-[active=true]/nav-item:[--tile-depth:color-mix(in_oklab,var(--primary)_45%,transparent)] dark:group-data-[active=true]/nav-item:border-[color-mix(in_oklab,var(--primary),white_25%)] dark:group-data-[active=true]/nav-item:bg-primary dark:group-data-[active=true]/nav-item:bg-linear-to-b dark:group-data-[active=true]/nav-item:from-[color-mix(in_oklab,var(--primary),white_6%)] dark:group-data-[active=true]/nav-item:to-[color-mix(in_oklab,var(--primary),black_18%)] dark:group-data-[active=true]/nav-item:text-white dark:group-data-[active=true]/nav-item:[--tile-depth:color-mix(in_oklab,var(--primary),black_40%)]',
  {
    variants: {
      tone: {
        brand: 'dark:group-not-data-[active=true]/nav-item:border-sidebar-tint/13 dark:group-not-data-[active=true]/nav-item:bg-primary/7 dark:group-not-data-[active=true]/nav-item:text-sidebar-tint-foreground dark:group-not-data-[active=true]/nav-item:group-hover/nav-item:border-white/14 dark:group-not-data-[active=true]/nav-item:group-hover/nav-item:bg-white/6 dark:group-not-data-[active=true]/nav-item:group-hover/nav-item:text-white dark:group-disabled/nav-item:border-sidebar-tint/13! dark:group-disabled/nav-item:bg-primary/7! dark:group-disabled/nav-item:text-sidebar-tint-foreground!',
        // Admin is not a workspace app: a neutral tile unless it is the current page
        neutral: 'dark:group-not-data-[active=true]/nav-item:border-white/7 dark:group-not-data-[active=true]/nav-item:bg-white/3 dark:group-not-data-[active=true]/nav-item:text-[oklch(0.72_0_0)] dark:group-not-data-[active=true]/nav-item:group-hover/nav-item:border-white/14 dark:group-not-data-[active=true]/nav-item:group-hover/nav-item:bg-white/6 dark:group-not-data-[active=true]/nav-item:group-hover/nav-item:text-white',
      },
    },
    defaultVariants: { tone: 'brand' },
  },
)

/** The 3D icon tile: it lifts on hover, sinks on press, and turns pink on the current row. */
function SidebarNavTile({ icon: Icon, tone }: { icon: SidebarNavIcon } & VariantProps<typeof sidebarNavTileVariants>) {
  return (
    <span data-slot="sidebar-nav-tile" className={sidebarNavTileVariants({ tone })}>
      <Icon />
    </span>
  )
}

const SidebarNavCollapsed = createContext(false)

type SidebarNavIcon = ComponentType<{ className?: string }>

function SidebarNavLabel({ children }: { children: ReactNode }) {
  return <span data-slot="sidebar-nav-label" className="min-w-0 flex-1 truncate group-data-[collapsed=true]/sidebar-nav:hidden">{children}</span>
}

/** A sidebar link. Active when the route matches `to` (like NavLink), unless `active` says otherwise. */
function SidebarNavItem({
  to,
  end = false,
  icon: Icon,
  label,
  'aria-label': ariaLabel = label,
  active,
  badge = 0,
  unread = false,
  size,
  tone,
  className,
  ...props
}: Omit<LinkProps, 'className' | 'children' | 'title'> & VariantProps<typeof sidebarNavItemVariants> & VariantProps<typeof sidebarNavTileVariants> & {
  className?: string
  end?: boolean
  icon: SidebarNavIcon
  label: string
  active?: boolean
  /** A count of things that wait for the user. Nothing shows for 0. */
  badge?: number
  /** Something waits for the user, but there is no count: a dot shows. */
  unread?: boolean
}) {
  const collapsed = useContext(SidebarNavCollapsed)
  const match = useMatch({ path: useResolvedPath(to).pathname, end })
  const isActive = active ?? match !== null
  const name = badge > 0 ? `${ariaLabel}, ${badge} unread` : unread ? `${ariaLabel}, unread` : ariaLabel
  return (
    <Tip label={collapsed ? name : undefined} side="right">
      <Link
        data-slot="sidebar-nav-item"
        data-active={isActive}
        aria-current={isActive ? 'page' : undefined}
        aria-label={name}
        className={cn(sidebarNavItemVariants({ size }), className)}
        to={to}
        {...props}
      >
        <SidebarNavTile icon={Icon} tone={tone} />
        <SidebarNavLabel>{label}</SidebarNavLabel>
        {badge > 0 ? (
          <span
            data-slot="sidebar-nav-badge"
            aria-hidden="true"
            className="inline-flex h-4 min-w-4 shrink-0 items-center justify-center rounded-full bg-primary px-1 text-[10px] leading-none font-semibold text-primary-foreground tabular-nums group-data-[collapsed=true]/sidebar-nav:absolute group-data-[collapsed=true]/sidebar-nav:top-0.5 group-data-[collapsed=true]/sidebar-nav:right-0.5"
          >
            {badge > 99 ? '99+' : badge}
          </span>
        ) : unread ? (
          <span
            data-slot="sidebar-nav-dot"
            aria-hidden="true"
            className="mr-1 size-2 shrink-0 rounded-full bg-primary group-data-[collapsed=true]/sidebar-nav:absolute group-data-[collapsed=true]/sidebar-nav:top-1 group-data-[collapsed=true]/sidebar-nav:right-1 group-data-[collapsed=true]/sidebar-nav:mr-0"
          />
        ) : null}
      </Link>
    </Tip>
  )
}

/** A product area that is not built yet: shown, but disabled. */
function SidebarNavComingSoon({ icon: Icon, label }: { icon: SidebarNavIcon; label: string }) {
  const collapsed = useContext(SidebarNavCollapsed)
  return (
    <Tip label={collapsed ? `${label} — Coming soon` : undefined} side="right">
      <Button
        data-slot="sidebar-nav-item"
        variant="ghost"
        className={cn(sidebarNavItemVariants(), 'rounded-none border-0 hover:bg-transparent hover:text-sidebar-nav disabled:pointer-events-auto disabled:cursor-not-allowed disabled:opacity-[0.48] dark:hover:bg-transparent')}
        disabled
      >
        <SidebarNavTile icon={Icon} />
        <SidebarNavLabel>{label}</SidebarNavLabel>
        <span className="ml-auto text-[10px] text-muted-foreground/70 group-data-[collapsed=true]/sidebar-nav:hidden">Coming soon</span>
      </Button>
    </Tip>
  )
}

function SidebarSection({ label, collapsed }: { label: string; collapsed: boolean }) {
  if (collapsed) return <Separator className="mx-2.5 my-2 bg-black/8 data-horizontal:w-auto dark:bg-sidebar-tint/10" aria-hidden="true" />
  return (
    <div className="mt-3 px-2.5 py-1 text-[11px] leading-4 font-medium text-sidebar-nav-muted select-none dark:font-mono dark:text-[10.5px] dark:tracking-[0.08em] dark:text-sidebar-tint/55 dark:uppercase">
      {label}
    </div>
  )
}

/** Grouped sidebar navigation — shared by the desktop sidebar and the mobile drawer. */
export function SidebarNav({
  onNavigate,
  collapsed = false,
  chatBadge,
  inboxUnread,
  chatEnabled: chat = chatEnabled,
  isRoot = false,
}: {
  onNavigate?: () => void
  collapsed?: boolean
  /** Unread count on the Chat item. */
  chatBadge?: number
  /** Activity has an unread notification. */
  inboxUnread?: boolean
  /** Chat is a link instead of "Coming soon". Defaults to the build's setting. */
  chatEnabled?: boolean
  /** The root user (installation administrator) also gets the Admin link. */
  isRoot?: boolean
}) {
  const location = useLocation()
  const currentParams = new URLSearchParams(location.search)
  const view = currentParams.get('view')
  const selectedProject = currentParams.get('project')

  const taskViewPath = (nextView?: string) => {
    const params = new URLSearchParams()
    if (selectedProject) params.set('project', selectedProject)
    if (nextView) params.set('view', nextView)
    return `/tasks${params.size > 0 ? `?${params}` : ''}`
  }

  const paletteKeys = useBindings()['palette.open']

  const openSearch = () => {
    onNavigate?.()
    window.dispatchEvent(new CustomEvent('open-command-palette'))
  }

  const activityPath = selectedProject ? `/activity?project=${encodeURIComponent(selectedProject)}` : '/activity'

  // Tasks itself is active on its routes only while no task view is picked
  const tasksActive = useMatch({ path: '/tasks', end: false }) !== null && !view
  const taskViewActive = (name: string) => location.pathname === '/tasks' && view === name

  return (
    <SidebarNavCollapsed.Provider value={collapsed}>
      <div data-slot="sidebar-nav" data-collapsed={collapsed} className="group/sidebar-nav flex min-h-0 flex-1 flex-col">
        <div className="px-1 pb-3">
        <Tip label={collapsed ? 'Search' : undefined} side="right">
          {collapsed ? (
            <button type="button" data-slot="sidebar-nav-item" className={sidebarNavItemVariants()} aria-label="Search" onClick={openSearch}>
              <SidebarNavTile icon={Search} />
            </button>
          ) : (
            // the expanded sidebar shows Search as an input, not a tile
            <button
              type="button"
              className="flex h-8 w-full min-w-0 items-center gap-2.5 rounded-md bg-neutral-100 pr-1.5 pl-2.5 text-[13px] font-medium text-muted-foreground transition-colors outline-none hover:bg-neutral-200 focus-visible:outline-1 focus-visible:outline-primary dark:bg-sidebar-tint/6 dark:inset-ring dark:inset-ring-sidebar-tint/10 dark:hover:bg-sidebar-tint/10"
              aria-label="Search"
              onClick={openSearch}
            >
              <Search className="size-4 shrink-0 opacity-90" />
              <span className="min-w-0 flex-1 truncate text-left">Search</span>
              {paletteKeys ? (
                <Kbd className="h-auto rounded-md border border-neutral-300 bg-neutral-200 px-1.5 py-0.5 text-[11px] font-medium text-muted-foreground dark:border-sidebar-tint/18 dark:bg-white/6">{shortcutText(paletteKeys)}</Kbd>
              ) : null}
            </button>
          )}
        </Tip>
        </div>
        <div className="-mx-1 flex min-h-0 flex-1 flex-col gap-0.5 overflow-y-auto overscroll-contain px-1 pb-2">
          {WORKSPACE_LINKS.map((link) => link.enabled || (link.to === '/chat' && chat) ? (
            <Fragment key={link.to}>
              <SidebarNavItem
                to={link.to === '/tasks' ? taskViewPath() : link.to}
                active={link.to === '/tasks' ? tasksActive : undefined}
                end={link.to === '/'}
                icon={link.icon}
                label={link.label}
                badge={link.to === '/chat' ? chatBadge : undefined}
                onClick={onNavigate}
              />
              {/* saved views belong to Tasks, so they nest under it instead of sitting beside the other apps;
                  always shown so the rows below never move when the route changes */}
              {link.to === '/tasks' ? (
                <SidebarNavItem to="/views" end size="sub" icon={Layer} label="Views" aria-label="Task views" onClick={onNavigate} />
              ) : null}
            </Fragment>
          ) : (
            <SidebarNavComingSoon key={link.to} icon={link.icon} label={link.label} />
          ))}
          <SidebarSection label="Personal" collapsed={collapsed} />
          <SidebarNavItem to={activityPath} icon={Bell} label="Activity" unread={inboxUnread} onClick={onNavigate} />
          <SidebarNavItem to={taskViewPath('mine')} active={taskViewActive('mine')} icon={SquareCheck} label="My tasks" onClick={onNavigate} />
          <SidebarNavItem to={taskViewPath('current_week')} active={taskViewActive('current_week')} icon={Calendar} label="This week" onClick={onNavigate} />
          <SidebarNavItem to={taskViewPath('my_week')} active={taskViewActive('my_week')} icon={CalendarTick} label="My week" onClick={onNavigate} />
          <SidebarNavItem to={taskViewPath('overdue')} active={taskViewActive('overdue')} icon={Danger} label="Overdue" onClick={onNavigate} />
          <SidebarNavItem to={taskViewPath('due_soon')} active={taskViewActive('due_soon')} icon={Timer} label="Due soon" onClick={onNavigate} />
          <SidebarSection label="Manage" collapsed={collapsed} />
          <SidebarNavItem to="/settings" icon={Settings} label="Settings" onClick={onNavigate} />
          {isRoot ? (
            <>
              {/* the installation, not the workspace: it sits apart at the bottom */}
              <div className="flex-1" aria-hidden="true" />
              <SidebarNavItem to="/admin" icon={SecurityUser} tone="neutral" label="Admin" onClick={onNavigate} />
            </>
          ) : null}
        </div>
      </div>
    </SidebarNavCollapsed.Provider>
  )
}
