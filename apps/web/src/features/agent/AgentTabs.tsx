import { useContext } from 'react'
import { Navigate, Outlet, useLocation, useNavigate } from 'react-router'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { ACCOUNT_AGENT_TABS, WORKSPACE_AGENT_TABS, agentTabsView } from '@/features/settings/settingsNav'
import { WorkspaceContext } from '@/features/workspaces/workspaceContext'
import { useAgentStatus } from './api'

const AREA_TABS = { settings: WORKSPACE_AGENT_TABS, profile: ACCOUNT_AGENT_TABS }

/** The sub-tabs of an Agent settings page. One tab needs no bar, so a person who may open only one sees just that page. */
export function AgentTabs({ area }: { area: keyof typeof AREA_TABS }) {
  const { pathname, search, hash } = useLocation()
  const navigate = useNavigate()
  const permissions = useContext(WorkspaceContext)?.workspace.permissions ?? []
  const status = useAgentStatus()
  if (status.isPending) return null
  const { tabs, current, redirect } = agentTabsView(AREA_TABS[area], permissions, status.data?.enabled === true, pathname)
  if (redirect) return <Navigate to={{ pathname: redirect, search, hash }} replace />
  if (tabs.length < 2 || !current) return <Outlet />
  return (
    <Tabs className="gap-5" value={current.to} onValueChange={(to) => void navigate(to as string)}>
      <TabsList variant="line" className="min-h-10 w-full justify-start gap-1.5 border-b group-data-horizontal/tabs:h-auto">
        {tabs.map((tab) => <TabsTrigger key={tab.to} value={tab.to} className="h-7 flex-none px-2.5 text-[13px]">{tab.label}</TabsTrigger>)}
      </TabsList>
      <TabsContent value={current.to} className="flex min-w-0 flex-col gap-6"><Outlet /></TabsContent>
    </Tabs>
  )
}
