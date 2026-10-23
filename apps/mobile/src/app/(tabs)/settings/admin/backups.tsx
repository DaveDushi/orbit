import { View } from 'react-native'

import { AdminShell, Badge, Note, RowIcon, RowText, SettingsCard, SettingsRow } from '../../../../components/settings/Shell'
import { formatBytes } from '../../../../data/account-api'
import { useBackups } from '../../../../hooks/admin'
import { type, useTheme } from '../../../../theme'
import { Text } from '../../../../ui'

/**
 * The web's Admin → Backups, read-only: what snapshots exist. Creating, downloading and deleting a backup is operator
 * work on the web.
 */
export default function AdminBackupsScreen() {
  const { colors } = useTheme()
  const backups = useBackups()
  const list = [...(backups.data ?? [])].sort((a, b) => b.created_at - a.created_at)
  return (
    <AdminShell refreshing={backups.isRefetching} onRefresh={() => void backups.refetch()}>
      <SettingsCard title="Backups" description="Verified snapshots of the database and attachments." flush>
        {backups.isPending ? <View style={{ padding: 16 }}><Note>Loading backups…</Note></View> : null}
        {backups.isError ? <View style={{ padding: 16 }}><Note error>Backups could not be loaded.</Note></View> : null}
        {backups.data && list.length === 0 ? <View style={{ padding: 16 }}><Note>No backups yet.</Note></View> : null}
        {list.map((backup, index) => (
          <SettingsRow key={backup.id} first={index === 0}>
            <RowIcon icon="archiveBox" />
            <RowText
              title={new Date(backup.created_at).toLocaleString()}
              subtitle={`${formatBytes(backup.byte_size)} · ${backup.file_count} ${backup.file_count === 1 ? 'file' : 'files'} · Orbit ${backup.application_version}`}
              badges={
                <>
                  {backup.kind === 'pre_migration' ? <Badge label="Before upgrade" tone="secondary" /> : null}
                  {backup.location === 's3' ? <Badge label="S3" tone="secondary" /> : null}
                </>
              }
            />
          </SettingsRow>
        ))}
      </SettingsCard>
      <Text style={[type(12), { color: colors.mutedForeground }]}>Create, download and delete backups in Orbit in a browser.</Text>
    </AdminShell>
  )
}
