import { View } from 'react-native'

import { AdminShell, Note, RowIcon, RowText, SettingsCard, SettingsRow } from '../../../../components/settings/Shell'
import { jobLabel, limitText } from '../../../../data/admin-api'
import { useJobLimits } from '../../../../hooks/admin'
import { Text } from '../../../../ui'

/** The web's Admin → Background jobs, read-only: how many jobs of each kind run at once. */
export default function AdminJobsScreen() {
  const limits = useJobLimits()
  return (
    <AdminShell refreshing={limits.isRefetching} onRefresh={() => void limits.refetch()}>
      <SettingsCard
        title="Background jobs"
        description="How many jobs of each kind run at once. Per workspace applies only while another workspace has a job waiting."
        flush
      >
        {limits.isPending ? <View className="p-4"><Note>Loading limits…</Note></View> : null}
        {limits.isError ? <View className="p-4"><Note error>Limits could not be loaded.</Note></View> : null}
        {limits.data?.map((kind, index) => (
          <SettingsRow key={kind.kind} first={index === 0}>
            <RowIcon icon="cpu" />
            <RowText
              title={jobLabel(kind.kind)}
              subtitle={`${limitText(kind.concurrency)} on all servers, ${limitText(kind.workspace_concurrency)} per workspace\nDefault: ${limitText(kind.default_concurrency)} on all servers, ${limitText(kind.default_workspace_concurrency)} per workspace`}
            />
          </SettingsRow>
        ))}
      </SettingsCard>
      <Text className="text-[12px] text-muted-foreground">Change the limits in Orbit in a browser.</Text>
    </AdminShell>
  )
}
