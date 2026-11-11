import { Link, Navigate, Outlet, useLocation, useNavigate } from 'react-router'
import { mcpClientCommands, mcpUrl } from '@orbit/api/mcp'
import { SettingsCard } from '@/components/common/SettingsCard'
import { CopyCodeButton } from '@/components/common/CopyCodeButton'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { AgentNamePage } from '@/features/agent/AgentNamePage'
import { AgentPage } from '@/features/agent/AgentPage'
import { AgentsPage } from '@/features/agent/AgentsPage'
import { AnswerLevelPage } from '@/features/agent/AnswerLevelPage'
import { MemoryPage } from '@/features/agent/MemoryPage'
import { RoutinesPage } from '@/features/agent/RoutinesPage'
import { TeamInstructionsPage } from '@/features/agent/TeamInstructionsPage'
import { useAgentStatus } from '@/features/agent/api'
import { AgentToolsPage } from '@/features/agentIntegrations/AgentToolsPage'
import { useCurrentUser } from '@/features/auth/api'
import { OAuthConnections } from '@/features/auth/components/OAuthConnections'
import { AGENTIC_TABS, agenticTabsView } from '../settingsNav'

/** The page's sections by role, as tabs. One tab needs no bar. Each section keeps the access check it always had. */
export function AgenticTabs() {
  const { pathname, search, hash } = useLocation()
  const navigate = useNavigate()
  const user = useCurrentUser()
  const status = useAgentStatus()
  // Both answers decide the tabs, so a redirect must wait for them.
  if (status.isPending || user.isPending) return null
  const { tabs, current, redirect } = agenticTabsView(AGENTIC_TABS, user.data?.installation_admin === true, status.data?.enabled === true, pathname)
  if (redirect) return <Navigate to={{ pathname: redirect, search, hash }} replace />
  if (tabs.length < 2 || !current || !tabs.includes(current)) return <Outlet />
  return (
    <Tabs className="gap-5" value={current.to} onValueChange={(to) => void navigate(to as string)}>
      <TabsList variant="line" className="min-h-10 w-full justify-start gap-1.5 border-b group-data-horizontal/tabs:h-auto">
        {tabs.map((tab) => <TabsTrigger key={tab.to} value={tab.to} className="h-7 flex-none px-2.5 text-[13px]">{tab.label}</TabsTrigger>)}
      </TabsList>
      <TabsContent value={current.to} className="flex min-w-0 flex-col gap-6"><Outlet /></TabsContent>
    </Tabs>
  )
}

/** What belongs to the signed-in person: the agent's ChatGPT sign-in and memory (when the server runs it), and MCP clients. */
export function AgenticYou() {
  const agent = useAgentStatus().data?.enabled === true
  return (
    <>
      {agent ? <AgentPage /> : null}
      {agent ? <AnswerLevelPage /> : null}
      <McpSetup />
      <OAuthConnections />
      {agent ? <MemoryPage /> : null}
    </>
  )
}

/** The team's instructions, which every member reads, and the tools, which only `integrations.manage` sees. */
export function AgenticWorkspace() {
  return (
    <>
      <TeamInstructionsPage />
      <AgentToolsPage />
    </>
  )
}

/** The workspace's agents besides the default one: every member reads them, owners and admins manage them. */
export function AgenticAgents() {
  return useAgentStatus().data?.enabled === true ? <AgentsPage /> : null
}

/** Routines: the person's own, and every routine of the workspace for owners and admins. */
export function AgenticRoutines() {
  return useAgentStatus().data?.enabled === true ? <RoutinesPage /> : null
}

/** The agent's name and model for the whole instance. */
export function AgenticAdmin() {
  return <AgentNamePage />
}

function McpSetup() {
  const url = mcpUrl(window.location.origin)
  return (
    <>
      <SettingsCard title="MCP server" description="AI clients such as Claude, Codex and Grok can use tasks and read docs and chat through this address. Sign in with your Orbit account and select the workspace, the projects and the permissions.">
        <CodeBlock text={url} />
      </SettingsCard>
      <SettingsCard title="Connect a client" description="Run the command for your client in a terminal. Then sign in to Orbit in the browser, select the projects and permissions, then allow access.">
        <div className="flex flex-col gap-4">
          {mcpClientCommands(url).map((entry) => (
            <div key={entry.client} className="flex flex-col gap-1.5">
              <p className="text-[13px] font-medium text-foreground">{entry.client}</p>
              <CodeBlock text={entry.command} />
              <p className="text-xs text-muted-foreground">{entry.signIn}</p>
            </div>
          ))}
          <p className="text-xs text-muted-foreground">For a remote server, run claude mcp login orbit --no-browser or codex mcp login orbit --no-browser. Open the printed link in your browser, select “Client runs on a remote server”, then copy the callback URL into your terminal.</p>
          <p className="text-xs text-muted-foreground">
            A client that cannot sign in with OAuth can send an API token with selected permissions instead. A workspace admin creates it in{' '}
            <Link to="/settings/api-tokens" className="text-foreground underline underline-offset-2">Settings → API tokens</Link>.
          </p>
        </div>
      </SettingsCard>
    </>
  )
}

function CodeBlock({ text }: { text: string }) {
  return (
    <div className="group relative rounded-md bg-muted/50 py-2.5 pr-20 pl-3">
      <pre className="font-mono text-xs leading-5 break-all whitespace-pre-wrap text-foreground">{text}</pre>
      <CopyCodeButton getText={() => text} className="absolute top-1.5 right-1.5" />
    </div>
  )
}
