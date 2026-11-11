import { useState } from 'react'
import { Alert, View } from 'react-native'
import { type Href, router } from 'expo-router'
import type { AgentMemoryScope, AgentMemorySource } from '@orbit/api/agent'

import { Note, RowText, Segmented, SettingsCard, SettingsRow } from '../settings/Shell'
import { relativeAgo } from '../../data/format'
import { useAgentMemories, useAgentMemoryWake, useAgentProfiles, useAgentStatus, useDeleteAgentMemory, useForgetAgentMemorySummary } from '../../hooks/agent'
import { useOrbit } from '../../orbit-context'
import { errorMessage } from '../../session-context'
import { Button, IconButton, Input, PickerSheet } from '../../ui'

const SCOPES = [
  { value: 'team', label: 'Team' },
  { value: 'private', label: 'Private' },
] as const

/** The app's chat route of a memory's source thread. */
function sourceHref(source: AgentMemorySource | null): string | null {
  if (!source?.conversation_id) return null
  return source.root_message_id ? `/chat/thread/${source.root_message_id}` : `/chat/${source.conversation_id}`
}

/** The picker's key for the default agent, whose memory the API names with no agent. */
const DEFAULT_AGENT = 'default'

/** The web's Agentic → You → Memories: what one agent remembers in this workspace, for the team and for you. Each agent has its own memory. */
export function AgentMemory() {
  const { workspace } = useOrbit()
  const status = useAgentStatus()
  const profiles = useAgentProfiles().data
  const [picked, setPicked] = useState(DEFAULT_AGENT)
  const [picking, setPicking] = useState(false)
  const [scope, setScope] = useState<AgentMemoryScope>('team')
  const [query, setQuery] = useState('')
  const agents = [
    { key: DEFAULT_AGENT, label: profiles?.default_agent.name ?? (status.data?.enabled ? status.data.name : 'Default agent') },
    ...(profiles?.agents ?? []).map((agent) => ({ key: agent.user_id, label: agent.name })),
  ]
  // A deleted agent leaves the list: show the default agent then.
  const current = agents.find((agent) => agent.key === picked) ?? agents[0]
  const agent = current.key === DEFAULT_AGENT ? null : current.key
  const name = current.label
  const memories = useAgentMemories(agent, scope, query.trim())
  const wake = useAgentMemoryWake(agent, scope)
  const remove = useDeleteAgentMemory(agent, scope)
  const forget = useForgetAgentMemorySummary(agent, scope)
  const pages = memories.data?.pages ?? []
  const items = pages.flatMap((page) => page.items)
  const canDelete = pages[0]?.can_delete ?? false
  const summaries = (wake.data?.lines ?? []).filter((line) => line.lo !== line.hi)

  const confirmDelete = (position: number) =>
    Alert.alert('Delete this memory?', 'The agent forgets it. Summaries built from it are written again without it.', [
      { text: 'Cancel', style: 'cancel' },
      { text: 'Delete', style: 'destructive', onPress: () => remove.mutate(position) },
    ])
  const confirmForget = (lo: number, hi: number) =>
    Alert.alert('Delete this summary?', 'The agent writes this summary again from its memories.', [
      { text: 'Cancel', style: 'cancel' },
      { text: 'Delete', style: 'destructive', onPress: () => forget.mutate({ lo, hi }) },
    ])

  if (status.data && !status.data.enabled) {
    return (
    <>
        <SettingsCard title="Agent">
          <Note>The agent is not set up on this server.</Note>
        </SettingsCard>
      </>
    )
  }

  return (
    <>
      <SettingsCard
        title="Memories"
        description={
          scope === 'team'
            ? `What ${name} remembers for everyone in ${workspace.name}. Every member can read it; owners and admins can delete from it. Each agent has its own memory.`
            : `What ${name} remembers about you in ${workspace.name}. Only you can read it, and ${name} reads it only in a direct message with you. Each agent has its own memory.`
        }
        flush
      >
        <View className="gap-3 p-4">
          {agents.length > 1 ? <Button label={`Agent: ${name}`} variant="outline" onPress={() => setPicking(true)} /> : null}
          <Segmented label="Memory" value={scope} options={SCOPES} onChange={setScope} />
          <Input placeholder="Search memories" accessibilityLabel="Search memories" autoCapitalize="none" autoCorrect={false} value={query} onChangeText={setQuery} />
        </View>
        {memories.isPending ? <View className="p-4"><Note>Loading memories…</Note></View> : null}
        {memories.isError ? <View className="p-4"><Note error>Memories could not be loaded.</Note></View> : null}
        {memories.isSuccess && items.length === 0 ? (
          <View className="p-4"><Note>{query.trim() ? 'No memory matches this search.' : `Nothing yet. ${name} notes facts here while it works.`}</Note></View>
        ) : null}
        {items.map((memory) => {
          const href = sourceHref(memory.source)
          return (
            <SettingsRow key={memory.position} onPress={href ? () => router.push(href as Href) : undefined} label={href ? 'Open the source thread' : undefined}>
              <RowText title={memory.body} subtitle={`#${memory.position} · ${memory.author_name ?? 'Former member'} · ${relativeAgo(memory.created_at)}${href ? ' · Source thread' : ''}`} />
              {canDelete ? <IconButton icon="trash" label={`Delete memory #${memory.position}`} onPress={() => confirmDelete(memory.position)} /> : null}
            </SettingsRow>
          )
        })}
        {memories.hasNextPage ? (
          <View className="p-4"><Button label="Show older memories" variant="ghost" disabled={memories.isFetchingNextPage} onPress={() => void memories.fetchNextPage()} /></View>
        ) : null}
        {remove.isError ? <View className="p-4"><Note error>{errorMessage(remove.error)}</Note></View> : null}
      </SettingsCard>
      {summaries.length > 0 ? (
        <SettingsCard title="Summaries" description={`${name} reads older memories as these one-line summaries, which it writes itself. Delete a wrong one and ${name} writes it again from the memories.`} flush>
          {summaries.map((line, index) => (
            <SettingsRow key={`${line.lo}-${line.hi}`} first={index === 0}>
              <RowText title={line.body} subtitle={`Memories #${line.lo}–#${line.hi}`} />
              {canDelete ? <IconButton icon="trash" label={`Delete summary #${line.lo}-${line.hi}`} onPress={() => confirmForget(line.lo, line.hi)} /> : null}
            </SettingsRow>
          ))}
          {forget.isError ? <View className="p-4"><Note error>{errorMessage(forget.error)}</Note></View> : null}
        </SettingsCard>
      ) : null}
      <PickerSheet
        visible={picking}
        title="Agent"
        options={agents.map((option) => ({ ...option, selected: option.key === current.key }))}
        onSelect={(key) => { setPicked(key); setPicking(false) }}
        onClose={() => setPicking(false)}
      />
    </>
  )
}
