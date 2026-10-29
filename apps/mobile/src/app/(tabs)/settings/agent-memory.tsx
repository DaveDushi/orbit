import { useState } from 'react'
import { Alert, View } from 'react-native'
import { type Href, router } from 'expo-router'
import type { AgentMemoryScope, AgentMemorySource } from '@orbit/api/agent'

import { ACCOUNT_NAV, Note, RowText, Segmented, SettingsCard, SettingsRow, SettingsShell } from '../../../components/settings/Shell'
import { relativeAgo } from '../../../data/format'
import { useAgentMemories, useAgentMemoryWake, useAgentStatus, useDeleteAgentMemory, useForgetAgentMemorySummary } from '../../../hooks/agent'
import { useOrbit } from '../../../orbit-context'
import { errorMessage } from '../../../session-context'
import { Button, IconButton, Input } from '../../../ui'

const SCOPES = [
  { value: 'team', label: 'Team' },
  { value: 'private', label: 'Private' },
] as const

/** The app's chat route of a memory's source thread. */
function sourceHref(source: AgentMemorySource | null): string | null {
  if (!source?.conversation_id) return null
  return source.root_message_id ? `/chat/thread/${source.root_message_id}` : `/chat/${source.conversation_id}`
}

/** The web's Account → Agent memory page: what the agent remembers in this workspace, for the team and for you. */
export default function AgentMemoryScreen() {
  const { workspace } = useOrbit()
  const status = useAgentStatus()
  const [scope, setScope] = useState<AgentMemoryScope>('team')
  const [query, setQuery] = useState('')
  const memories = useAgentMemories(scope, query.trim())
  const wake = useAgentMemoryWake(scope)
  const remove = useDeleteAgentMemory(scope)
  const forget = useForgetAgentMemorySummary(scope)
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
      <SettingsShell title="Account settings" nav={ACCOUNT_NAV}>
        <SettingsCard title="Agent memory">
          <Note>The agent is not set up on this server.</Note>
        </SettingsCard>
      </SettingsShell>
    )
  }

  return (
    <SettingsShell title="Account settings" nav={ACCOUNT_NAV} refreshing={memories.isRefetching} onRefresh={() => void memories.refetch()}>
      <SettingsCard
        title="Agent memory"
        description={
          scope === 'team'
            ? `What the agent remembers for everyone in ${workspace.name}. Every member can read it; owners and admins can delete from it.`
            : `What the agent remembers about you in ${workspace.name}. Only you can read it, and the agent reads it only in a direct message with you.`
        }
        flush
      >
        <View className="gap-3 p-4">
          <Segmented label="Memory" value={scope} options={SCOPES} onChange={setScope} />
          <Input placeholder="Search memories" accessibilityLabel="Search memories" autoCapitalize="none" autoCorrect={false} value={query} onChangeText={setQuery} />
        </View>
        {memories.isPending ? <View className="p-4"><Note>Loading memories…</Note></View> : null}
        {memories.isError ? <View className="p-4"><Note error>Memories could not be loaded.</Note></View> : null}
        {memories.isSuccess && items.length === 0 ? (
          <View className="p-4"><Note>{query.trim() ? 'No memory matches this search.' : 'Nothing yet. The agent notes facts here while it works.'}</Note></View>
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
        <SettingsCard title="Summaries" description="The agent reads older memories as these one-line summaries, which it writes itself. Delete a wrong one and the agent writes it again from the memories." flush>
          {summaries.map((line, index) => (
            <SettingsRow key={`${line.lo}-${line.hi}`} first={index === 0}>
              <RowText title={line.body} subtitle={`Memories #${line.lo}–#${line.hi}`} />
              {canDelete ? <IconButton icon="trash" label={`Delete summary #${line.lo}-${line.hi}`} onPress={() => confirmForget(line.lo, line.hi)} /> : null}
            </SettingsRow>
          ))}
          {forget.isError ? <View className="p-4"><Note error>{errorMessage(forget.error)}</Note></View> : null}
        </SettingsCard>
      ) : null}
    </SettingsShell>
  )
}
