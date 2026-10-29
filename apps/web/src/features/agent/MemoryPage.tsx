import { useState } from 'react'
import { Link } from 'react-router'
import { Search4 as Search, Trash } from 'reicon-react'
import { agentMemorySourcePath, type AgentMemoryLine, type AgentMemoryScope } from '@orbit/api/agent'
import { Button } from '@/components/ui/button'
import { InputGroup, InputGroupAddon, InputGroupInput } from '@/components/ui/input-group'
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group'
import { ConfirmDeleteModal } from '@/components/common/ConfirmDeleteModal'
import { SettingsCard } from '@/components/common/SettingsCard'
import { SettingsRow } from '@/features/settings/components/SettingsParts'
import { useWorkspace } from '@/features/workspaces/workspaceContext'
import { relativeTime } from '@/lib/format'
import { useDebouncedValue } from '@/lib/useDebouncedValue'
import { useAgentMemories, useAgentMemoryWake, useDeleteAgentMemory, useForgetAgentMemorySummary, useAgentStatus } from './api'

type Pending = { kind: 'memory'; position: number } | { kind: 'summary'; lo: number; hi: number }

/** Account → Agent memory: what the agent remembers in the current workspace, for the team and for you. */
export function MemoryPage() {
  const { workspace } = useWorkspace()
  const status = useAgentStatus()
  const [scope, setScope] = useState<AgentMemoryScope>('team')
  const [search, setSearch] = useState('')
  const query = useDebouncedValue(search.trim(), 250)
  const [pending, setPending] = useState<Pending | null>(null)
  const memories = useAgentMemories(workspace.id, scope, query)
  const wake = useAgentMemoryWake(workspace.id, scope)
  const remove = useDeleteAgentMemory(workspace.id, scope)
  const forget = useForgetAgentMemorySummary(workspace.id, scope)

  if (status.data && !status.data.enabled) {
    return <SettingsCard title="Agent memory">The agent is not set up on this server.</SettingsCard>
  }
  const pages = memories.data?.pages ?? []
  const items = pages.flatMap((page) => page.items)
  const canDelete = pages[0]?.can_delete ?? false
  const summaries = (wake.data?.lines ?? []).filter((line) => line.lo !== line.hi)

  return (
    <>
      <SettingsCard
        title="Agent memory"
        description={
          scope === 'team'
            ? `What the agent remembers for everyone in ${workspace.name}. Every member can read it; owners and admins can delete from it.`
            : `What the agent remembers about you in ${workspace.name}. Only you can read it, and the agent reads it only in a direct message with you.`
        }
        actions={
          <ToggleGroup
            aria-label="Memory"
            variant="outline"
            size="sm"
            spacing={0}
            value={[scope]}
            onValueChange={(value: string[]) => {
              const next = value[0]
              if (next === 'team' || next === 'private') setScope(next)
            }}
          >
            <ToggleGroupItem value="team">Team</ToggleGroupItem>
            <ToggleGroupItem value="private">Private</ToggleGroupItem>
          </ToggleGroup>
        }
        flush
      >
        <div className="flex flex-col divide-y">
          <div className="px-4 py-3">
            <InputGroup className="w-64 max-[899px]:w-full">
              <InputGroupAddon align="inline-start">
                <Search className="size-3.5 text-muted-foreground/70" />
              </InputGroupAddon>
              <InputGroupInput type="search" placeholder="Search memories" aria-label="Search memories" value={search} onChange={(event) => setSearch(event.target.value)} />
            </InputGroup>
          </div>
          {memories.isPending ? <SettingsRow>Loading memories…</SettingsRow> : null}
          {memories.isError ? (
            <SettingsRow role="alert">
              Memories could not be loaded. <Button variant="ghost" onClick={() => void memories.refetch()}>Retry</Button>
            </SettingsRow>
          ) : null}
          {memories.isSuccess && items.length === 0 ? (
            <SettingsRow className="text-sm text-muted-foreground">
              {query ? 'No memory matches this search.' : 'Nothing yet. The agent notes facts here while it works.'}
            </SettingsRow>
          ) : null}
          {items.map((memory) => {
            const source = agentMemorySourcePath(memory.source)
            return (
              <SettingsRow key={memory.position} className="flex-nowrap items-start">
                <div className="flex min-w-0 flex-1 flex-col gap-0.5">
                  <p className="text-[13px] break-words text-foreground">{memory.body}</p>
                  <p className="text-xs text-muted-foreground/70">
                    #{memory.position} · {memory.author_name ?? 'Former member'} · {relativeTime(memory.created_at)}
                    {source ? (
                      <>
                        {' · '}
                        <Link className="text-primary underline-offset-4 hover:underline" to={source}>Source thread</Link>
                      </>
                    ) : null}
                  </p>
                </div>
                {canDelete ? (
                  <Button type="button" variant="ghost" size="icon-sm" className="text-muted-foreground" aria-label={`Delete memory #${memory.position}`} onClick={() => setPending({ kind: 'memory', position: memory.position })}>
                    <Trash />
                  </Button>
                ) : null}
              </SettingsRow>
            )
          })}
          {memories.hasNextPage ? (
            <SettingsRow>
              <Button type="button" variant="ghost" disabled={memories.isFetchingNextPage} onClick={() => void memories.fetchNextPage()}>Show older memories</Button>
            </SettingsRow>
          ) : null}
        </div>
      </SettingsCard>
      {summaries.length > 0 ? (
        <SettingsCard
          title="Summaries"
          description="The agent reads older memories as these one-line summaries, which it writes itself. Delete a wrong one and the agent writes it again from the memories."
          flush
        >
          <div className="flex flex-col divide-y">
            {summaries.map((line: AgentMemoryLine) => (
              <SettingsRow key={`${line.lo}-${line.hi}`} className="flex-nowrap items-start">
                <div className="flex min-w-0 flex-1 flex-col gap-0.5">
                  <p className="text-[13px] break-words text-foreground">{line.body}</p>
                  <p className="text-xs text-muted-foreground/70">Memories #{line.lo}–#{line.hi}</p>
                </div>
                {canDelete ? (
                  <Button type="button" variant="ghost" size="icon-sm" className="text-muted-foreground" aria-label={`Delete summary #${line.lo}-${line.hi}`} onClick={() => setPending({ kind: 'summary', lo: line.lo, hi: line.hi })}>
                    <Trash />
                  </Button>
                ) : null}
              </SettingsRow>
            ))}
          </div>
        </SettingsCard>
      ) : null}
      {remove.isError || forget.isError ? <p role="alert" className="text-xs text-destructive">The memory could not be deleted. Try again.</p> : null}
      {pending ? (
        <ConfirmDeleteModal
          title={pending.kind === 'memory' ? 'Delete this memory?' : 'Delete this summary?'}
          description={
            pending.kind === 'memory'
              ? 'The agent forgets it. Summaries built from it are written again without it.'
              : 'The agent writes this summary again from its memories.'
          }
          onClose={() => setPending(null)}
          onConfirm={() => {
            if (pending.kind === 'memory') remove.mutate(pending.position)
            else forget.mutate({ lo: pending.lo, hi: pending.hi })
            setPending(null)
          }}
        />
      ) : null}
    </>
  )
}
