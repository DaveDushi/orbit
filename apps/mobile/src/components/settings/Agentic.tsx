import { router, useLocalSearchParams } from 'expo-router'
import { mcpSetup } from '@orbit/api/mcp'
import * as Clipboard from 'expo-clipboard'
import * as WebBrowser from 'expo-web-browser'
import { useEffect, useState } from 'react'
import { View } from 'react-native'

import { AgentConnection } from '../agentic/AgentConnection'
import { AgentToolsCard } from '../agentic/AgentTools'
import { AgentMemory } from '../agentic/AgentMemory'
import { AgentsCard } from '../agentic/Agents'
import { DefaultModelCard } from '../agentic/DefaultModel'
import { RoutinesSection } from '../agentic/Routines'
import { SkillsCard } from '../agentic/Skills'
import { TeamInstructions } from '../agentic/TeamInstructions'
import { LineTabs } from './Fields'
import { Note, RowIcon, RowText, SettingsCard, SettingsRow, SettingsShell, WORKSPACE_NAV } from './Shell'
import { agenticTabs, pickedTab } from '../../data/agentic'
import { useDisconnectOAuth, useOAuthConnections } from '../../hooks/account'
import { useAgentStatus } from '../../hooks/agent'
import { useOrbit } from '../../orbit-context'
import { errorMessage } from '../../session-context'
import { Button, Text } from '../../ui'

/**
 * The one Agentic page, the web's with its tabs: You, and with the team agent Workspace, Agents, Routines and Skills
 * (`agenticTabs`). Each tab keeps its own access check; the installation's default model is on Agents, as on the web
 * (there is no Admin tab). `?tab=routines` opens a tab. Until the server says the agent is off, its tabs and cards show
 * (they load, or say they failed, with Retry).
 */
export function AgenticScreen() {
  const agent = useAgentStatus().data?.enabled ?? null
  const connections = useOAuthConnections()
  const { tab } = useLocalSearchParams<{ tab?: string }>()
  const tabs = agenticTabs(agent)
  const section = pickedTab(tabs, tab)

  return (
    <SettingsShell title="Agentic" nav={WORKSPACE_NAV} refreshing={connections.isRefetching} onRefresh={() => void connections.refetch()}>
      {tabs.length > 1 ? <LineTabs label="Agentic" value={section} tabs={tabs} onChange={(next) => router.setParams({ tab: next })} /> : null}
      {section === 'you' ? (
        <>
          {agent !== false ? <AgentConnection /> : null}
          <McpSetup />
          {agent !== false ? <AgentMemory /> : null}
        </>
      ) : null}
      {section === 'workspace' ? (
        <>
          <TeamInstructions />
          <AgentToolsCard />
        </>
      ) : null}
      {section === 'agents' ? (
        <>
          <AgentsCard />
          <DefaultModelCard />
        </>
      ) : null}
      {section === 'routines' ? <RoutinesSection /> : null}
      {section === 'skills' ? <SkillsCard /> : null}
    </SettingsShell>
  )
}

/** The address and the commands run on a computer: each has a Copy button, and a long press selects it. */
function McpSetup() {
  const { session } = useOrbit()
  const connections = useOAuthConnections()
  const disconnect = useDisconnectOAuth()
  const { url, commands, guide } = mcpSetup(session.server, connections.data?.mcp_url)
  const list = connections.data?.items ?? []
  return (
    <>
      <SettingsCard title="MCP server" description="AI clients such as Claude, Codex and Grok can use tasks and read docs and chat through this address. Sign in with your Orbit account and select the workspace, the projects and the permissions.">
        <Code text={url} />
      </SettingsCard>

      <SettingsCard title="Connect a client" description="Run the command for your client in a terminal. Then sign in to Orbit in the browser, select the projects and permissions, then allow access.">
        <View className="gap-4">
          {commands.map((entry) => (
            <View key={entry.client} className="gap-1.5">
              <Text className="text-[13px] font-medium">{entry.client}</Text>
              <Code text={entry.command} />
              <Text className="text-xs text-muted-foreground">{entry.signIn}</Text>
            </View>
          ))}
          {guide ? <Text className="text-xs text-muted-foreground">Step by step: <Text className="text-xs text-foreground underline" onPress={() => void WebBrowser.openBrowserAsync(guide)}>Connect your agent</Text>.</Text> : null}
          <Text className="text-xs text-muted-foreground">For a remote server, run claude mcp login orbit --no-browser or codex mcp login orbit --no-browser. Open the printed link in your browser, select “Client runs on a remote server”, then copy the callback URL into your terminal.</Text>
          <Text className="text-xs text-muted-foreground">A client that cannot sign in with OAuth can send an API token with selected permissions instead. A workspace admin creates it in Settings → API tokens.</Text>
        </View>
      </SettingsCard>

      <SettingsCard title="Connected apps" description="MCP clients you approved. Disconnecting an app stops its access and refresh tokens." flush>
        {connections.isPending ? <View className="p-4"><Note loading>Loading connections…</Note></View> : null}
        {connections.isError ? <View className="p-4"><Note error>Connections could not be loaded.</Note></View> : null}
        {connections.data && list.length === 0 ? <View className="p-4"><Note>No connected apps.</Note></View> : null}
        {list.map((connection, index) => (
          <SettingsRow key={connection.id} first={index === 0}>
            <RowIcon icon="magicStar" />
            <RowText title={connection.client_name} subtitle={`${connection.workspace_name} · ${connection.scope} · ${connection.all_projects ? 'All projects' : connection.project_names.join(', ')} · Connected ${connectedAt(connection.created_at)}`} />
            <Button label="Disconnect" variant="ghost" size="sm" disabled={disconnect.isPending} onPress={() => disconnect.mutate(connection.id)} />
          </SettingsRow>
        ))}
        {disconnect.isError ? <View className="p-4"><Note error>The app could not be disconnected. {errorMessage(disconnect.error)}</Note></View> : null}
      </SettingsCard>
    </>
  )
}

const connectedAt = (millis: number) => new Date(millis).toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' })

function Code({ text }: { text: string }) {
  const [copied, setCopied] = useState(false)
  useEffect(() => {
    if (!copied) return
    const timer = setTimeout(() => setCopied(false), 2000)
    return () => clearTimeout(timer)
  }, [copied])
  return (
    <View className="flex-row items-start gap-2 rounded-md bg-muted py-1.5 pr-1.5 pl-3">
      <Text selectable className="flex-1 min-w-0 py-1 font-mono text-xs leading-5">{text}</Text>
      <Button
        label={copied ? 'Copied' : 'Copy'}
        icon={copied ? 'check' : 'copy'}
        variant="ghost"
        size="sm"
        onPress={() => void Clipboard.setStringAsync(text).then(() => setCopied(true))}
      />
    </View>
  )
}
