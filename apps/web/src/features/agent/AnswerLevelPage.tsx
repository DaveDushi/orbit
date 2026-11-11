import type { AnswerLevel } from '@orbit/api/agent'
import { SettingsCard } from '@/components/common/SettingsCard'
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group'
import { useAgentStatus, useAnswerLevel, useSaveAnswerLevel } from './api'

const LEVELS: { value: AnswerLevel; label: string; help: string }[] = [
  { value: 'short', label: 'Short', help: 'Less thinking, a few lines. Fast.' },
  { value: 'normal', label: 'Normal', help: 'The agent’s own setting.' },
  { value: 'thorough', label: 'Thorough', help: 'More thinking, full detail. Slower, and it uses more of your ChatGPT limits.' },
]

/** Agentic → You: how much every agent thinks and writes for this person. */
export function AnswerLevelPage() {
  const status = useAgentStatus().data
  const level = useAnswerLevel()
  const save = useSaveAnswerLevel()
  const name = status?.enabled ? status.name : 'the agent'
  const current = save.isPending ? save.variables : (level.data ?? 'normal')
  return (
    <SettingsCard title="Answer level" description={`How much ${name} and every other agent think and write when you ask them. It applies from your next message.`}>
      <div className="flex flex-col items-start gap-2">
        <ToggleGroup
          aria-label="Answer level"
          variant="outline"
          value={[current]}
          disabled={level.isPending}
          onValueChange={(value: string[]) => {
            const next = value[0] as AnswerLevel | undefined
            if (next && next !== current) save.mutate(next)
          }}
        >
          {LEVELS.map((option) => <ToggleGroupItem key={option.value} value={option.value} className="px-3">{option.label}</ToggleGroupItem>)}
        </ToggleGroup>
        <p className="text-xs text-muted-foreground">{LEVELS.find((option) => option.value === current)?.help}</p>
        {save.isError ? <p role="alert" className="text-xs text-destructive">The answer level could not be saved.</p> : null}
      </div>
    </SettingsCard>
  )
}
