import { useState } from 'react'
import { View } from 'react-native'

import { AdminShell, Note, SettingsCard } from '../../../../components/settings/Shell'
import type { AgentIdentity } from '@orbit/api/agent'
import { useAgentIdentity, useSaveAgentName } from '../../../../hooks/agent'
import { errorMessage } from '../../../../session-context'
import { Button, Input } from '../../../../ui'

/** The web's Admin → AI agent: the agent's one name for the installation, which is also its @handle in chat. */
export default function AdminAgentScreen() {
  const identity = useAgentIdentity()
  return (
    <AdminShell refreshing={identity.isRefetching} onRefresh={() => void identity.refetch()}>
      {identity.isPending ? <SettingsCard title="AI agent"><Note>Loading the agent settings…</Note></SettingsCard> : null}
      {identity.isError ? (
        <SettingsCard title="AI agent">
          <View className="flex-row items-center gap-2">
            <Note error>The agent settings could not be loaded.</Note>
            <Button label="Retry" variant="ghost" onPress={() => void identity.refetch()} />
          </View>
        </SettingsCard>
      ) : null}
      {identity.data ? <NameCard identity={identity.data} /> : null}
    </AdminShell>
  )
}

function NameCard({ identity }: { identity: AgentIdentity }) {
  const save = useSaveAgentName()
  const [name, setName] = useState(identity.name)
  const changed = name.trim() !== identity.name
  return (
    <SettingsCard title="AI agent" description="One name for the agent in every workspace. People mention it as @Name in chat, and its messages and earlier mentions show the new name. It cannot be the name of another person in Orbit.">
      <View className="gap-4">
        <Input
          label="Name"
          autoCapitalize="words"
          autoCorrect={false}
          maxLength={identity.max_name_chars}
          value={name}
          onChangeText={(value) => {
            save.reset()
            setName(value)
          }}
          description={`${identity.min_name_chars} to ${identity.max_name_chars} characters: letters, digits, single spaces and - _ . '`}
        />
        {save.isError ? <Note error>{errorMessage(save.error)}</Note> : null}
        {save.isSuccess ? <Note>Agent name saved.</Note> : null}
        <View className="flex-row justify-end">
          <Button label={save.isPending ? 'Saving…' : 'Save name'} disabled={!changed || save.isPending} onPress={() => save.mutate(name.trim(), { onSuccess: (saved) => setName(saved.name) })} />
        </View>
      </View>
    </SettingsCard>
  )
}
