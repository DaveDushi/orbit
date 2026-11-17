import { daysUntilPurge } from '@orbit/api/trash'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { router } from 'expo-router'
import { useState } from 'react'
import { Alert, Pressable, RefreshControl, ScrollView, View } from 'react-native'

import { ActionSheet, ConfirmDialog, type ConfirmRequest, type SheetAction } from '../../../components/settings/Dialog'
import { driveTrash, type DriveNode, emptyTrash, purgeNode, restoreNode } from '../../../data/drive'
import { keys } from '../../../data/keys'
import { fileSize } from '../../../data/tasks'
import { useOrbit } from '../../../orbit-context'
import { errorMessage } from '../../../session-context'
import { alpha, useTheme } from '../../../theme'
import { Button, EmptyState, ErrorView, Icon, IconButton, LoadingView, PaneHeader, SafeAreaView, SwipeRow, Text } from '../../../ui'

/**
 * The web's Drive trash: what was trashed directly (a folder's items come back with it), newest first, deleted forever
 * after 30 days. Swipe right restores; owners and admins (`drive.purge`) swipe left to delete forever.
 */
export default function DriveTrashScreen() {
  const { colors } = useTheme()
  const { client, workspaceId, workspace } = useOrbit()
  const queryClient = useQueryClient()
  const trashKey = [...keys.drive(workspaceId), 'trash'] as const
  const trash = useQuery({ queryKey: trashKey, queryFn: () => driveTrash(client, workspaceId) })
  const [menu, setMenu] = useState<{ title: string; actions: SheetAction[] } | null>(null)
  const [confirm, setConfirm] = useState<ConfirmRequest | null>(null)
  const canPurge = workspace.permissions.includes('drive.purge')

  /** Off the list at once; an error puts it back. Every Drive list refreshes after. */
  const write = useMutation({
    mutationFn: async ({ node, action }: { node: DriveNode; action: 'restore' | 'purge' }) => {
      if (action === 'restore') await restoreNode(client, workspaceId, node.id)
      else await purgeNode(client, workspaceId, node.id)
    },
    onMutate: async ({ node }) => {
      await queryClient.cancelQueries({ queryKey: trashKey })
      const previous = queryClient.getQueryData<DriveNode[]>(trashKey)
      queryClient.setQueryData<DriveNode[]>(trashKey, (list) => list?.filter((item) => item.id !== node.id))
      return previous
    },
    onError: (error, { action }, previous) => {
      if (previous) queryClient.setQueryData(trashKey, previous)
      Alert.alert(action === 'restore' ? 'Could not restore it' : 'Could not delete it', errorMessage(error))
    },
    onSettled: () => void queryClient.invalidateQueries({ queryKey: keys.drive(workspaceId) }),
  })
  const empty = useMutation({
    mutationFn: () => emptyTrash(client, workspaceId),
    onSuccess: () => queryClient.setQueryData<DriveNode[]>(trashKey, []),
    onError: (error) => Alert.alert('Could not empty the trash', errorMessage(error)),
    onSettled: () => void queryClient.invalidateQueries({ queryKey: keys.drive(workspaceId) }),
  })
  const purge = (node: DriveNode) =>
    setConfirm({
      title: `Delete “${node.name}” forever?`,
      description: node.kind === 'folder' ? 'The folder and everything in it are deleted permanently. This can’t be undone.' : 'The file is deleted permanently. This can’t be undone.',
      confirmLabel: 'Delete forever',
      danger: true,
      onConfirm: () => write.mutate({ node, action: 'purge' }),
    })
  const items = trash.data ?? []

  return (
    <SafeAreaView edges={['top', 'left', 'right']} className="flex-1 bg-background">
      <PaneHeader className="px-3">
        <IconButton icon="back" label="Back to Drive" color={alpha(colors.mutedForeground, 0.7)} onPress={() => (router.canGoBack() ? router.back() : router.navigate('/drive'))} />
        <Icon name="trash" size={15} color={colors.mutedForeground} />
        <Text className="font-noto-semibold text-[13px] flex-1">Drive trash</Text>
        {canPurge && items.length > 0 ? (
          <Button
            label="Empty trash"
            size="sm"
            variant="ghost"
            disabled={empty.isPending}
            onPress={() => setConfirm({ title: 'Empty the Drive trash?', description: 'Everything in the trash is deleted permanently. This can’t be undone.', confirmLabel: 'Empty trash', danger: true, onConfirm: () => empty.mutate() })}
          />
        ) : null}
      </PaneHeader>
      {!trash.data ? (
        trash.error ? (
          <ErrorView error={trash.error} onRetry={() => void trash.refetch()} />
        ) : (
          <LoadingView />
        )
      ) : items.length === 0 ? (
        <EmptyState icon="trash" title="Trash is empty" description="Files and folders you move to the trash show here for 30 days." />
      ) : (
        <ScrollView contentContainerClassName="pb-6" refreshControl={<RefreshControl refreshing={trash.isRefetching} onRefresh={() => void trash.refetch()} tintColor={colors.mutedForeground} />}>
          {items.map((node) => {
            const left = node.trashed_at ? daysUntilPurge(node.trashed_at) : null
            const detail = [node.kind === 'folder' ? 'Folder' : fileSize(node.size ?? 0), left === null ? null : left === 0 ? 'deleted today' : `deleted forever in ${left} ${left === 1 ? 'day' : 'days'}`].filter(Boolean).join(' · ')
            const actions: SheetAction[] = [
              { label: 'Restore', icon: 'rotateLeft', onPress: () => write.mutate({ node, action: 'restore' }) },
              ...(canPurge ? [{ label: 'Delete forever', icon: 'trash' as const, destructive: true, onPress: () => purge(node) }] : []),
            ]
            return (
              <SwipeRow
                key={node.id}
                left={{ label: 'Restore', icon: 'rotateLeft', tone: 'primary', onSwipe: () => write.mutate({ node, action: 'restore' }) }}
                right={canPurge ? { label: 'Delete', icon: 'trash', tone: 'destructive', onSwipe: () => purge(node) } : undefined}
              >
                <Pressable
                  accessibilityRole="button"
                  accessibilityLabel={node.name}
                  accessibilityHint="Opens restore and delete"
                  onPress={() => setMenu({ title: node.name, actions })}
                  className="min-h-14 flex-row items-center gap-3 px-4 py-2 border-b border-border active:bg-muted"
                >
                  <View className="size-8 rounded-md items-center justify-center bg-muted">
                    <Icon name={node.kind === 'folder' ? 'folder' : 'documentDownload'} size={16} color={node.kind === 'folder' ? colors.primary : colors.mutedForeground} />
                  </View>
                  <View className="flex-1 min-w-0">
                    <Text numberOfLines={1} className="font-noto-medium text-[14px]">
                      {node.name}
                    </Text>
                    <Text numberOfLines={1} className="text-[12px] text-muted-foreground">
                      {detail}
                    </Text>
                  </View>
                  <IconButton icon="rotateLeft" label={`Restore ${node.name}`} size={32} color={colors.mutedForeground} onPress={() => write.mutate({ node, action: 'restore' })} />
                </Pressable>
              </SwipeRow>
            )
          })}
        </ScrollView>
      )}
      <ActionSheet visible={menu !== null} title={menu?.title} actions={menu?.actions ?? []} onClose={() => setMenu(null)} />
      <ConfirmDialog request={confirm} onClose={() => setConfirm(null)} />
    </SafeAreaView>
  )
}
