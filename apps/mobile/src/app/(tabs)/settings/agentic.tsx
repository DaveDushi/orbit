import { useState } from 'react'
import { mcpClientCommands, mcpUrl } from '@orbit/api/mcp'
import { View } from 'react-native'

import { AdminAgent } from '../../../components/agentic/AdminAgent'
import { AgentConnection } from '../../../components/agentic/AgentConnection'
import { AgentMemory } from '../../../components/agentic/AgentMemory'
import { TeamInstructions } from '../../../components/agentic/TeamInstructions'
import { Note, RowIcon, RowText, Segmented, SettingsCard, SettingsRow, SettingsShell } from '../../../components/settings/Shell'
import { useDisconnectOAuth, useOAuthConnections } from '../../../hooks/account'
import { useAgentEnabled } from '../../../hooks/agent'
import { useOrbit } from '../../../orbit-context'
import { errorMessage } from '../../../session-context'
import { Button, Text } from '../../../ui'

type Section = 'you' | 'workspace' | 'admin'

/**
 * The web's Agentic page: every AI setting, in sections by role. Each section keeps its own access check. Agent tools
 * are set up on the web; this app has no screen for them.
 */
export default function AgenticScreen() {
  const { user } = useOrbit()
  const agent = useAgentEnabled()
  const connections = useOAuthConnections()
  const [picked, setPicked] = useState<Section>('you')
  const sections: { value: Section; label: string }[] = [
    { value: 'you', label: 'You' },
    ...(agent ? [{ value: 'workspace' as const, label: 'Workspace' }] : []),
    ...(agent && user.installation_admin ? [{ value: 'admin' as const, label: 'Admin' }] : []),
  ]
  const section = sections.some((option) => option.value === picked) ? picked : 'you'

  return (
    <SettingsShell title="Agentic" nav={[]} refreshing={connections.isRefetching} onRefresh={() => void connections.refetch()}>
      {sections.length > 1 ? <Segmented label="Section" value={section} options={sections} onChange={setPicked} /> : null}
      {section === 'you' ? (
        <>
          {agent ? <AgentConnection /> : null}
          <McpSetup />
          {agent ? <AgentMemory /> : null}
        </>
      ) : null}
      {section === 'workspace' ? <TeamInstructions /> : null}
      {section === 'admin' ? <AdminAgent /> : null}
    </SettingsShell>
  )
}

/** The commands run on a computer, so a long press selects them; the app has no clipboard module. */
function McpSetup() {
  const { session } = useOrbit()
  const connections = useOAuthConnections()
  const disconnect = useDisconnectOAuth()
  const url = mcpUrl(session.server)
  const list = connections.data ?? []
  return (
    <>
      <SettingsCard title="MCP server" description="AI clients such as Claude, Codex and Grok can use tasks and read docs and chat through this address. Sign in with your Orbit account and select the workspace, the projects and the permissions.">
        <Code text={url} />
      </SettingsCard>

      <SettingsCard title="Connect a client" description="Run the command for your client in a terminal. Then sign in to Orbit in the browser, select the projects and permissions, then allow access.">
        <View className="gap-4">
          {mcpClientCommands(url).map((entry) => (
            <View key={entry.client} className="gap-1.5">
              <Text className="text-[13px] font-medium">{entry.client}</Text>
              <Code text={entry.command} />
              <Text className="text-xs text-muted-foreground">{entry.signIn}</Text>
            </View>
          ))}
          <Text className="text-xs text-muted-foreground">For a remote server, run claude mcp login orbit --no-browser or codex mcp login orbit --no-browser. Open the printed link in your browser, select “Client runs on a remote server”, then copy the callback URL into your terminal.</Text>
          <Text className="text-xs text-muted-foreground">A client that cannot sign in with OAuth can send an API token with selected permissions instead. A workspace admin creates it in the web app, in Settings → API tokens.</Text>
        </View>
      </SettingsCard>

      <SettingsCard title="Connected apps" description="MCP clients you approved. Disconnecting an app stops its access and refresh tokens." flush>
        {connections.isPending ? <View className="p-4"><Note>Loading connections…</Note></View> : null}
        {connections.isError ? <View className="p-4"><Note error>Connections could not be loaded.</Note></View> : null}
        {connections.data && list.length === 0 ? <View className="p-4"><Note>No connected apps.</Note></View> : null}
        {list.map((connection, index) => (
          <SettingsRow key={connection.id} first={index === 0}>
            <RowIcon icon="magicStar" />
            <RowText title={connection.client_name} subtitle={`${connection.workspace_name} · ${connection.scope} · ${connection.project_names.join(', ')}`} />
            <Button label="Disconnect" variant="ghost" size="sm" disabled={disconnect.isPending} onPress={() => disconnect.mutate(connection.id)} />
          </SettingsRow>
        ))}
        {disconnect.isError ? <View className="p-4"><Note error>The app could not be disconnected. {errorMessage(disconnect.error)}</Note></View> : null}
      </SettingsCard>
    </>
  )
}

function Code({ text }: { text: string }) {
  return (
    <View className="rounded-md bg-muted px-3 py-2.5">
      <Text selectable className="font-mono text-xs leading-5">{text}</Text>
    </View>
  )
}
