import { Button } from '@/components/ui/button'
import { SettingsCard } from '@/components/common/SettingsCard'
import { loadFailed } from '@/lib/connection'
import { useAgentStatus, useDisconnectChatgpt, useStartChatgptLogin } from './api'

/** Account → AI agent: connect the person's own ChatGPT account, which pays for the agent runs they start. */
export function AgentPage() {
  const status = useAgentStatus()
  const start = useStartChatgptLogin()
  const disconnect = useDisconnectChatgpt()

  if (loadFailed(status)) return <SettingsCard title="AI agent">The agent settings are not available.</SettingsCard>
  if (status.data === undefined) return <SettingsCard title="AI agent">Loading the agent settings.</SettingsCard>
  const agent = status.data
  if (!agent.enabled) {
    return <SettingsCard title="AI agent">The agent is not set up on this server. Ask your administrator to start it.</SettingsCard>
  }
  const { chatgpt, login } = agent
  const connected = chatgpt.connected && !chatgpt.invalid
  const pending = !connected && login?.state === 'pending' && login.user_code

  return <SettingsCard
    title="AI agent"
    description="Mention @Orbit Agent in a chat thread to give it work. It acts with your permissions in Orbit, and your ChatGPT account pays for the runs you start."
  >
    <div className="flex flex-col items-start gap-3">
      {!agent.online ? <p role="status" className="text-sm text-destructive">The agent service is offline. Mentions wait until it is back.</p> : null}
      {connected ? (
        <>
          <p className="text-sm text-foreground">ChatGPT is connected{chatgpt.account_id ? ` (account ${chatgpt.account_id})` : ''}.</p>
          <Button variant="outline" disabled={disconnect.isPending} onClick={() => disconnect.mutate()}>Disconnect ChatGPT</Button>
        </>
      ) : pending ? (
        <>
          <p className="text-sm text-foreground">Open <a className="font-medium text-primary underline-offset-4 hover:underline" href={login.verification_uri} target="_blank" rel="noreferrer">{login.verification_uri}</a> on any device, sign in to ChatGPT, and enter this code:</p>
          <p aria-label="Sign-in code" className="rounded-md border px-3 py-2 font-mono text-lg tracking-widest">{login.user_code}</p>
          <p className="text-sm text-muted-foreground">This page updates when the sign-in is done.</p>
        </>
      ) : (
        <>
          <p className="text-sm text-muted-foreground">
            {chatgpt.invalid ? 'Your ChatGPT sign-in stopped working. Connect again to use the agent.' : 'Connect your ChatGPT account (Plus or Pro) to use the agent.'}
          </p>
          {!agent.can_connect ? <p role="alert" className="text-xs text-destructive">The server needs an app key before it can store a sign-in.</p> : null}
          <Button disabled={!agent.can_connect || !agent.online || start.isPending} onClick={() => start.mutate()}>Connect ChatGPT</Button>
          {login?.state === 'failed' ? <p role="alert" className="text-xs text-destructive">{login.error ?? 'The sign-in failed.'}</p> : null}
          {start.isError ? <p role="alert" className="text-xs text-destructive">The sign-in could not start.</p> : null}
        </>
      )}
    </div>
  </SettingsCard>
}
