import { Alert, View } from 'react-native'

import { openFile } from '../../../components/tasks/files'
import { Note, RowIcon, RowText, SettingsCard, SettingsRow, SettingsShell, WORKSPACE_NAV } from '../../../components/settings/Shell'
import { formatBytes } from '../../../data/account-api'
import { exportDownloadPath, isActiveExport, type WorkspaceExport } from '../../../data/workspaces'
import { useStartExport, useWorkspaceExports } from '../../../hooks/workspaces'
import { useOrbit } from '../../../orbit-context'
import { errorMessage } from '../../../session-context'
import { Button, Spinner, Text } from '../../../ui'

/**
 * The web's Settings → Export: a ZIP of the whole workspace to download and keep. Owners and admins
 * (`workspace.export`). The download opens in Quick Look, where Share saves it to Files.
 */
export default function ExportScreen() {
  const { workspace } = useOrbit()
  const canExport = workspace.permissions.includes('workspace.export')
  const exports = useWorkspaceExports(canExport)
  const start = useStartExport()
  const active = exports.data?.some(isActiveExport) ?? false
  return (
    <SettingsShell title="Settings" nav={WORKSPACE_NAV} refreshing={exports.isRefetching} onRefresh={canExport ? () => void exports.refetch() : undefined}>
      {!canExport ? (
        <SettingsCard title="Export workspace">
          <Note>Only workspace owners and administrators can export the workspace.</Note>
        </SettingsCard>
      ) : (
        <SettingsCard
          title="Export workspace"
          description="Download everything in this workspace as one ZIP file: tasks with comments, pages as Markdown, chat and all attached files. It includes your private pages and the private channels and direct messages you are in, so only you see and download your exports. Each download is kept for 7 days."
          actions={<Button label={active ? 'Export in progress' : 'Start export'} size="sm" loading={start.isPending} disabled={active || !exports.isSuccess} onPress={() => start.mutate()} />}
          flush
        >
          {start.isError ? <View className="p-4"><Note error>The export could not be started. {errorMessage(start.error)}</Note></View> : null}
          {exports.isPending ? <View className="p-4"><Note loading>Loading exports…</Note></View> : null}
          {exports.isError ? (
            <View className="p-4 flex-row items-center gap-2">
              <Note error>Exports could not be loaded.</Note>
              <Button label="Retry" variant="ghost" size="sm" onPress={() => void exports.refetch()} />
            </View>
          ) : null}
          {exports.data?.length === 0 ? <View className="p-4"><Note>No exports yet.</Note></View> : null}
          {exports.data?.map((item, index) => <ExportRow key={item.id} item={item} first={index === 0} />)}
        </SettingsCard>
      )}
    </SettingsShell>
  )
}

function ExportRow({ item, first }: { item: WorkspaceExport; first: boolean }) {
  const { session, workspaceId } = useOrbit()
  const created = new Date(item.created_at).toLocaleString()
  return (
    <SettingsRow first={first}>
      <RowIcon icon="documentDownload" />
      {item.status === 'ready' ? (
        <RowText title={created} subtitle={`${formatBytes(item.size_bytes ?? 0)} · Deleted on ${new Date(item.expires_at).toLocaleString()}`} />
      ) : item.status === 'failed' ? (
        <View className="flex-1 min-w-0 gap-0.5">
          <Text className="font-noto-medium text-[13px]">{created}</Text>
          <Text className="text-[12px] text-destructive">The export failed. Start a new one.</Text>
        </View>
      ) : (
        <View className="flex-1 min-w-0 gap-0.5">
          <Text className="font-noto-medium text-[13px]">{created}</Text>
          <View accessibilityRole="progressbar" className="flex-row items-center gap-1.5">
            <Spinner />
            <Text className="text-[12px] text-muted-foreground">{item.status === 'queued' ? 'Waiting to start…' : 'Preparing the archive…'}</Text>
          </View>
        </View>
      )}
      {item.status === 'ready' ? (
        <Button
          label="Download"
          icon="download"
          variant="outline"
          size="sm"
          onPress={() => void openFile(`${session.server}${exportDownloadPath(workspaceId, item.id)}`, `orbit-export-${item.created_at.slice(0, 10)}.zip`, session.token).catch((error: unknown) => Alert.alert('The download failed', errorMessage(error)))}
        />
      ) : null}
    </SettingsRow>
  )
}
