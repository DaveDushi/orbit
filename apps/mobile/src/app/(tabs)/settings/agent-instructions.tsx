import { useState } from 'react'
import { View } from 'react-native'

import type { TeamInstructions } from '@orbit/api/agent'
import { Note, SettingsCard, SettingsShell, WORKSPACE_NAV } from '../../../components/settings/Shell'
import { useAgentStatus, useSaveTeamInstructions, useTeamInstructions } from '../../../hooks/agent'
import { useOrbit } from '../../../orbit-context'
import { errorMessage } from '../../../session-context'
import { Button, Input, Text } from '../../../ui'

/** Characters as the server counts them (code points, not UTF-16 units). */
const characters = (text: string) => [...text.replace(/\r\n/g, '\n').trim()].length

/** The web's Workspace settings → AI agent: the team's instructions for every agent run in this workspace. */
export default function AgentInstructionsScreen() {
  const { workspaceId } = useOrbit()
  const team = useTeamInstructions()
  const status = useAgentStatus().data
  const name = status?.enabled ? status.name : 'the agent'
  return (
    <SettingsShell title="Settings" nav={WORKSPACE_NAV} refreshing={team.isRefetching} onRefresh={() => void team.refetch()}>
      {team.isPending ? <SettingsCard title="AI agent"><Note>Loading the team instructions…</Note></SettingsCard> : null}
      {team.isError ? (
        <SettingsCard title="AI agent">
          <View className="flex-row items-center gap-2">
            <Note error>The team instructions could not be loaded.</Note>
            <Button label="Retry" variant="ghost" onPress={() => void team.refetch()} />
          </View>
        </SettingsCard>
      ) : null}
      {/* a new field for another workspace */}
      {team.data ? <TeamCard key={workspaceId} team={team.data} agentName={name} /> : null}
    </SettingsShell>
  )
}

function TeamCard({ team, agentName }: { team: TeamInstructions; agentName: string }) {
  const save = useSaveTeamInstructions()
  const [text, setText] = useState(team.instructions)
  const count = characters(text)
  const tooLong = count > team.max_chars
  const changed = text.replace(/\r\n/g, '\n').trim() !== team.instructions
  const description = `Instructions that @${agentName} follows in every run in this workspace, for example your language, tone or conventions. Every member can read them. They cannot override the agent's built-in safety rules.`

  if (!team.can_edit) {
    return (
      <SettingsCard title="AI agent" description={description}>
        <View className="gap-3">
          {team.instructions ? <Text selectable className="text-sm">{team.instructions}</Text> : <Note>This workspace has no team instructions.</Note>}
          <Note>Only owners and admins can change them.</Note>
        </View>
      </SettingsCard>
    )
  }

  const changedBy = team.updated_at ? ` · Last changed by ${team.updated_by_name ?? 'a former member'} on ${new Date(team.updated_at).toLocaleDateString()}` : ''
  return (
    <SettingsCard title="AI agent" description={description}>
      <View className="gap-4">
        <Input
          label="Team instructions"
          multiline
          placeholder="For example: Answer in German. Our sprints start on Mondays."
          value={text}
          onChangeText={(value) => {
            save.reset()
            setText(value)
          }}
          error={tooLong ? `${count.toLocaleString()} / ${team.max_chars.toLocaleString()} characters: shorten the text.` : undefined}
          description={`${count.toLocaleString()} / ${team.max_chars.toLocaleString()} characters${changedBy}`}
        />
        {save.isError ? <Note error>{errorMessage(save.error)}</Note> : null}
        {save.isSuccess ? <Note>Team instructions saved.</Note> : null}
        <View className="flex-row justify-end">
          <Button label={save.isPending ? 'Saving…' : 'Save instructions'} disabled={!changed || tooLong || save.isPending} onPress={() => save.mutate(text, { onSuccess: (saved) => setText(saved.instructions) })} />
        </View>
      </View>
    </SettingsCard>
  )
}
