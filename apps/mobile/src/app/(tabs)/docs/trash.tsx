import { router } from 'expo-router'
import { useState } from 'react'
import { RefreshControl, ScrollView, View } from 'react-native'
import { SafeAreaView } from 'react-native-safe-area-context'

import { ConfirmDialog, type ConfirmRequest } from '../../../components/settings/Dialog'
import { pageTitle } from '../../../data/docs'
import type { TrashedPage } from '../../../data/docs-api'
import { relativeAgo } from '../../../data/format'
import { usePageTrash, useTrashWrites } from '../../../hooks/docs'
import { useTeamspaces } from '../../../queries'
import { errorMessage } from '../../../session-context'
import { alpha, type, useTheme } from '../../../theme'
import { Button, EmptyState, ErrorView, Icon, IconButton, LoadingView, PaneHeader, Text } from '../../../ui'

/**
 * The web's docs Trash: pages trashed directly (their sub-pages come back with them), newest first, with Restore and
 * Delete forever for the pages the server marks `can_purge`.
 */
export default function DocsTrashScreen() {
  const { colors } = useTheme()
  const trash = usePageTrash()
  const teamspaces = useTeamspaces()
  const { restore, purge, empty } = useTrashWrites()
  const [confirm, setConfirm] = useState<ConfirmRequest | null>(null)
  const busy = restore.isPending || purge.isPending || empty.isPending
  const failure = restore.error ?? purge.error ?? empty.error
  const purgeable = (trash.data ?? []).filter((page) => page.can_purge).length
  const plural = (count: number) => `${count} page${count === 1 ? '' : 's'}`
  const requestEmpty = () => {
    const kept = (trash.data?.length ?? 0) - purgeable
    setConfirm({
      title: 'Empty trash?',
      description:
        `${plural(purgeable)} and the sub-pages trashed with them will be deleted forever. This can’t be undone.` +
        (kept > 0 ? ` ${plural(kept)} in teamspaces stay: only workspace owners and admins can delete those.` : ''),
      confirmLabel: 'Empty trash',
      danger: true,
      onConfirm: () => empty.mutate(),
    })
  }
  const spaceOf = (page: TrashedPage) => (page.private || !page.teamspace_id ? 'Private' : (teamspaces.data?.find((item) => item.id === page.teamspace_id)?.name ?? 'Teamspace'))

  return (
    <SafeAreaView edges={['top', 'left', 'right']} style={{ flex: 1, backgroundColor: colors.background }}>
      <PaneHeader style={{ paddingHorizontal: 12 }}>
        <IconButton icon="back" label="Back to docs" color={alpha(colors.mutedForeground, 0.7)} onPress={() => (router.canGoBack() ? router.back() : router.navigate('/docs'))} />
        <Icon name="trash" size={15} color={colors.mutedForeground} />
        <Text style={[type(13, 'semibold'), { flex: 1 }]}>Trash</Text>
        {purgeable > 0 ? <Button label="Empty trash" size="sm" variant="ghost" disabled={busy} onPress={requestEmpty} /> : null}
      </PaneHeader>
      {failure ? <Text style={[type(13), { marginHorizontal: 16, marginBottom: 8, color: colors.destructive }]}>{errorMessage(failure)}</Text> : null}
      {!trash.data ? (
        trash.isError ? (
          <ErrorView error={trash.error} onRetry={() => void trash.refetch()} />
        ) : (
          <LoadingView />
        )
      ) : trash.data.length === 0 ? (
        <EmptyState icon="trash" title="Trash is empty" description="Pages you move to the trash show here." />
      ) : (
        <ScrollView contentContainerStyle={{ paddingBottom: 24 }} refreshControl={<RefreshControl refreshing={trash.isRefetching} onRefresh={() => void trash.refetch()} tintColor={colors.mutedForeground} />}>
          {trash.data.map((page, index) => (
            <View key={page.id} style={{ flexDirection: 'row', alignItems: 'center', gap: 10, paddingHorizontal: 16, paddingVertical: 10, borderTopWidth: index === 0 ? 0 : 1, borderTopColor: colors.border }}>
              <View style={{ width: 18, alignItems: 'center' }}>{page.icon ? <Text style={type(15)}>{page.icon}</Text> : <Icon name="docs" size={15} color={colors.mutedForeground} />}</View>
              <View style={{ flex: 1, minWidth: 0 }}>
                <Text numberOfLines={1} style={type(13, 'medium')}>
                  {pageTitle(page)}
                </Text>
                <Text numberOfLines={1} style={[type(12), { color: colors.mutedForeground }]}>
                  {`${spaceOf(page)} · deleted ${relativeAgo(page.deleted_at)}`}
                </Text>
              </View>
              <Button label="Restore" icon="rotateLeft" size="sm" variant="outline" disabled={busy} onPress={() => restore.mutate(page)} />
              {page.can_purge ? (
                <IconButton
                  icon="trash"
                  label={`Delete ${pageTitle(page)} forever`}
                  color={colors.destructive}
                  disabled={busy}
                  onPress={() =>
                    setConfirm({
                      title: `Delete “${pageTitle(page)}” forever?`,
                      description: 'The page, the sub-pages trashed with it and their files are deleted permanently. This can’t be undone.',
                      confirmLabel: 'Delete forever',
                      danger: true,
                      onConfirm: () => purge.mutate(page),
                    })
                  }
                />
              ) : null}
            </View>
          ))}
        </ScrollView>
      )}
      <ConfirmDialog request={confirm} onClose={() => setConfirm(null)} />
    </SafeAreaView>
  )
}
