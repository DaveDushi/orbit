import { toast } from 'sonner'
import { ApiProblem } from '@orbit/api/problem'
import { SettingsCard } from '@/components/common/SettingsCard'
import { Switch } from '@/components/ui/switch'
import { SettingsRow } from '@/features/settings/components/SettingsParts'
import { useImprovementSettings, useSaveImprovementSettings } from './api'

/** Admin → General: the "Suggest a change" switch. Off by default. */
export function ImprovementsCard() {
  const settings = useImprovementSettings()
  const save = useSaveImprovementSettings()
  return (
    <SettingsCard title="Suggest a change" description="Change requests from the admins of this Orbit, for an agent on this server." flush>
      <SettingsRow className="flex-nowrap">
        <div className="flex min-w-0 flex-1 flex-col gap-0.5 text-[13px]">
          <span className="font-medium text-foreground">Suggest a change</span>
          <span className="text-xs text-muted-foreground">
            On: instance admins find Suggest a change in their account menu, and the requests under Feedback &amp; fixes. An agent on this server reads them with <code className="font-mono">orbit improvements</code>. Members never see it.
          </span>
        </div>
        <Switch
          aria-label="Suggest a change"
          checked={settings.data?.enabled ?? false}
          disabled={!settings.data || save.isPending}
          onCheckedChange={(checked: boolean) => save.mutate(checked, {
            onSuccess: () => toast(checked ? 'Suggest a change is on' : 'Suggest a change is off'),
            onError: (error) => toast.error(error instanceof ApiProblem ? error.detail : 'The setting could not be changed.'),
          })}
        />
      </SettingsRow>
    </SettingsCard>
  )
}
