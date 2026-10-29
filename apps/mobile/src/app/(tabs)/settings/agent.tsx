import { Linking, View } from 'react-native'

import { ACCOUNT_NAV, Note, SettingsCard, SettingsShell } from '../../../components/settings/Shell'
import { useAgentStatus, useDisconnectChatgpt, useStartChatgptLogin } from '../../../hooks/agent'
import { Button, Text } from '../../../ui'

const TITLE = 'AI agent'
const DESCRIPTION = 'Mention @Orbit Agent in a chat thread to give it work. It acts with your permissions in Orbit, and your ChatGPT account pays for the runs you start.'

/** The web's Account → AI agent page: connect the person's own ChatGPT account with a device code. */
export default function AgentScreen() {
  const status = useAgentStatus()
  const start = useStartChatgptLogin()
  const disconnect = useDisconnectChatgpt()
  const agent = status.data

  return (
    <SettingsShell title="Account settings" nav={ACCOUNT_NAV} refreshing={status.isRefetching} onRefresh={() => void status.refetch()}>
      {!agent ? (
        <SettingsCard title={TITLE}>
          {status.isError ? <Note error>The agent settings are not available.</Note> : <Note>Loading the agent settings…</Note>}
        </SettingsCard>
      ) : !agent.enabled ? (
        <SettingsCard title={TITLE}>
          <Note>The agent is not set up on this server. Ask your administrator to start it.</Note>
        </SettingsCard>
      ) : (
        <SettingsCard title={TITLE} description={DESCRIPTION}>
          <View className="items-start gap-3">
            {!agent.online ? <Note error>The agent service is offline. Mentions wait until it is back.</Note> : null}
            {agent.chatgpt.connected && !agent.chatgpt.invalid ? (
              <>
                <Text className="text-sm">ChatGPT is connected{agent.chatgpt.account_id ? ` (account ${agent.chatgpt.account_id})` : ''}.</Text>
                <Button label="Disconnect ChatGPT" variant="outline" disabled={disconnect.isPending} onPress={() => disconnect.mutate()} />
              </>
            ) : agent.login?.state === 'pending' && agent.login.user_code ? (
              <>
                <Text className="text-sm">Open this link on any device, sign in to ChatGPT, and enter the code:</Text>
                <Text accessibilityRole="link" onPress={() => void Linking.openURL(agent.login?.verification_uri ?? '')} className="text-sm text-primary">
                  {agent.login.verification_uri}
                </Text>
                <Text accessibilityLabel="Sign-in code" selectable className="rounded-md border border-border px-3 py-2 font-mono text-lg tracking-widest">
                  {agent.login.user_code}
                </Text>
                <Note>This page updates when the sign-in is done.</Note>
              </>
            ) : (
              <>
                <Note>{agent.chatgpt.invalid ? 'Your ChatGPT sign-in stopped working. Connect again to use the agent.' : 'Connect your ChatGPT account (Plus or Pro) to use the agent.'}</Note>
                {!agent.can_connect ? <Note error>The server needs an app key before it can store a sign-in.</Note> : null}
                <Button label="Connect ChatGPT" disabled={!agent.can_connect || !agent.online || start.isPending} onPress={() => start.mutate()} />
                {agent.login?.state === 'failed' ? <Note error>{agent.login.error ?? 'The sign-in failed.'}</Note> : null}
                {start.isError ? <Note error>The sign-in could not start.</Note> : null}
              </>
            )}
          </View>
        </SettingsCard>
      )}
    </SettingsShell>
  )
}
