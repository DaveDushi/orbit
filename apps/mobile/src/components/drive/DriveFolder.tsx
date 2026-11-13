import { driveFileKind } from '@orbit/api/drive'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { router } from 'expo-router'
import { useState } from 'react'
import { FlatList, Image, Pressable, RefreshControl, View } from 'react-native'

import { keys } from '../../data/keys'
import { createFolder, driveChildren, driveNode, type DriveNode, downloadUrl, renameNode, trashNode, uploadToDrive } from '../../data/drive'
import { fileSize } from '../../data/tasks'
import { pickDriveFiles, type DriveSource } from '../../drive-file'
import { useOrbit } from '../../orbit-context'
import { errorMessage } from '../../session-context'
import { alpha, useTheme } from '../../theme'
import { EmptyState, ErrorView, HeaderTitle, Icon, type IconName, IconButton, LoadingView, PaneHeader, SafeAreaView, Spinner, Text } from '../../ui'
import { usePublishPageItem } from '../AskDonna'
import { MenuButton } from '../Drawer'
import { ActionSheet, ConfirmDialog, type ConfirmRequest, PromptDialog, type PromptRequest, type SheetAction } from '../settings/Dialog'
import { openFile } from '../tasks/files'

type Menu = { title: string; actions: SheetAction[] }

const KIND_ICONS: Record<ReturnType<typeof driveFileKind>, IconName> = {
  folder: 'folder',
  image: 'documentDownload',
  video: 'play',
  audio: 'play',
  pdf: 'docs',
  archive: 'archiveBox',
  document: 'docs',
  other: 'documentDownload',
}

/**
 * The web's Drive at phone size: one folder (`null`: the top level) as a list, folders first. A file opens in the
 * system preview; the row menu renames or moves to the trash; + makes a folder or uploads photos, videos and files.
 */
export function DriveFolder({ folderId }: { folderId: string | null }) {
  const { colors } = useTheme()
  const { client, workspaceId } = useOrbit()
  const queryClient = useQueryClient()
  const children = useQuery({ queryKey: keys.driveChildren(workspaceId, folderId), queryFn: () => driveChildren(client, workspaceId, folderId) })
  const folder = useQuery({ queryKey: keys.driveNode(workspaceId, folderId ?? ''), queryFn: () => driveNode(client, workspaceId, folderId ?? ''), enabled: folderId !== null })
  const [menu, setMenu] = useState<Menu | null>(null)
  const [confirm, setConfirm] = useState<ConfirmRequest | null>(null)
  const [prompt, setPrompt] = useState<PromptRequest | null>(null)
  const [notice, setNotice] = useState<string | null>(null)
  const [uploading, setUploading] = useState<{ name: string; percent: number } | null>(null)
  const [opening, setOpening] = useState<string | null>(null)
  // Ask Donna knows the folder on the screen, as on the web
  usePublishPageItem('drive_folder', folder.data?.node.id, folder.data?.node.name)
  const refresh = () => queryClient.invalidateQueries({ queryKey: keys.drive(workspaceId) })
  const failed = (what: string) => (error: unknown) => setNotice(`${what} ${errorMessage(error)}`)
  const title = folderId === null ? 'Drive' : (folder.data?.node.name ?? '')
  const parentPath = folderId === null ? null : folder.data?.node.parent_id ? `/drive/${folder.data.node.parent_id}` : '/drive'

  const upload = async (source: DriveSource) => {
    setNotice(null)
    try {
      const files = await pickDriveFiles(source)
      for (const file of files) {
        setUploading({ name: file.name, percent: 0 })
        await uploadToDrive(client, workspaceId, folderId, file, (sent) => setUploading({ name: file.name, percent: file.size ? Math.round((sent / file.size) * 100) : 100 }))
        await refresh()
      }
    } catch (error) {
      failed('Could not upload the file.')(error)
    } finally {
      setUploading(null)
      void refresh()
    }
  }

  const open = async (node: DriveNode) => {
    if (node.kind === 'folder') {
      router.push(`/drive/${node.id}`)
      return
    }
    setNotice(null)
    setOpening(node.id)
    try {
      await openFile(await downloadUrl(client, workspaceId, node.id), node.name)
    } catch (error) {
      failed('Could not open the file.')(error)
    } finally {
      setOpening(null)
    }
  }

  const newMenu = () =>
    setMenu({
      title: 'Add to Drive',
      actions: [
        {
          label: 'New folder',
          icon: 'folder',
          onPress: () =>
            setPrompt({
              title: 'New folder',
              label: 'Name',
              placeholder: 'Folder name',
              confirmLabel: 'Create',
              onSubmit: async (name) => {
                await createFolder(client, workspaceId, folderId, name.trim())
                await refresh()
              },
            }),
        },
        { label: 'Upload photos or videos', icon: 'import', onPress: () => void upload('photos') },
        { label: 'Upload files', icon: 'attach', onPress: () => void upload('files') },
      ],
    })

  const nodeMenu = (node: DriveNode) =>
    setMenu({
      title: node.name,
      actions: [
        ...(node.kind === 'file' ? [{ label: 'Open', icon: 'arrowUpRightSquare' as const, onPress: () => void open(node) }] : []),
        {
          label: 'Rename',
          icon: 'edit',
          onPress: () =>
            setPrompt({
              title: node.kind === 'folder' ? 'Rename folder' : 'Rename file',
              label: 'Name',
              initial: node.name,
              confirmLabel: 'Save',
              onSubmit: async (name) => {
                await renameNode(client, workspaceId, node.id, name.trim())
                await refresh()
              },
            }),
        },
        {
          label: 'Move to trash',
          icon: 'trash',
          destructive: true,
          onPress: () =>
            setConfirm({
              title: `Move “${node.name}” to the trash?`,
              description: node.kind === 'folder' ? 'Everything in the folder goes with it. You can restore it from the trash on the web.' : 'You can restore it from the trash on the web.',
              confirmLabel: 'Move to trash',
              danger: true,
              onConfirm: () =>
                void trashNode(client, workspaceId, node.id)
                  .then(refresh)
                  .catch(failed('Could not move it to the trash.')),
            }),
        },
      ],
    })

  const muted = alpha(colors.mutedForeground, 0.7)
  const error = children.error ?? folder.error
  return (
    <SafeAreaView edges={['top', 'left', 'right']} className="flex-1 bg-background">
      <PaneHeader>
        {parentPath ? <IconButton icon="back" label="Back" color={muted} onPress={() => (router.canGoBack() ? router.back() : router.navigate(parentPath as never))} /> : <MenuButton />}
        <View className="flex-1">
          <HeaderTitle>{title}</HeaderTitle>
        </View>
        <IconButton icon="plus" label="Add to Drive" size={32} variant="primary" disabled={uploading !== null} onPress={newMenu} />
      </PaneHeader>
      {uploading ? (
        <View accessibilityRole="progressbar" accessibilityLabel={`Uploading ${uploading.name}, ${uploading.percent}%`} className="mx-3 mb-1 p-2.5 rounded-md bg-muted flex-row items-center gap-2">
          <Spinner />
          <Text numberOfLines={1} className="text-[13px] flex-1">
            Uploading {uploading.name}
          </Text>
          <Text className="text-[12px] text-muted-foreground">{uploading.percent}%</Text>
        </View>
      ) : null}
      {notice ? (
        <Pressable accessibilityRole="alert" accessibilityLabel={notice} onPress={() => setNotice(null)} className="mx-3 mb-1 p-2.5 rounded-md bg-destructive/10">
          <Text className="text-[13px] text-destructive">{notice}</Text>
        </Pressable>
      ) : null}
      {!children.data ? (
        error ? (
          <ErrorView error={error} onRetry={() => void refresh()} />
        ) : (
          <LoadingView />
        )
      ) : children.data.length === 0 ? (
        <EmptyState icon="folder" title={folderId === null ? 'Drive is empty' : 'This folder is empty'} description="Upload photos, videos and files, or make a folder, with +." />
      ) : (
        <FlatList
          data={children.data}
          keyExtractor={(node) => node.id}
          contentContainerClassName="px-2 pt-1 pb-4"
          refreshControl={<RefreshControl refreshing={children.isRefetching} onRefresh={() => void refresh()} tintColor={colors.mutedForeground} />}
          renderItem={({ item }) => <DriveRow node={item} opening={opening === item.id} onPress={() => void open(item)} onMenu={() => nodeMenu(item)} />}
        />
      )}

      <ActionSheet visible={menu !== null} title={menu?.title} actions={menu?.actions ?? []} onClose={() => setMenu(null)} />
      <ConfirmDialog request={confirm} onClose={() => setConfirm(null)} />
      <PromptDialog request={prompt} onClose={() => setPrompt(null)} />
    </SafeAreaView>
  )
}

function DriveRow({ node, opening, onPress, onMenu }: { node: DriveNode; opening: boolean; onPress: () => void; onMenu: () => void }) {
  const { colors } = useTheme()
  const muted = alpha(colors.mutedForeground, 0.7)
  const kind = driveFileKind(node)
  const detail = node.kind === 'folder' ? 'Folder' : fileSize(node.size ?? 0)
  return (
    <View className="h-12 flex-row items-center">
      <Pressable accessibilityRole="button" accessibilityLabel={`${node.name}, ${detail}`} onPress={onPress} onLongPress={onMenu} className="flex-1 h-12 flex-row items-center gap-3 px-2 rounded-md active:bg-muted">
        <View className="size-8 rounded-md items-center justify-center bg-muted overflow-hidden">
          {node.thumbnail_url ? <Image source={{ uri: node.thumbnail_url }} className="size-8" /> : <Icon name={KIND_ICONS[kind]} size={16} color={node.kind === 'folder' ? colors.primary : muted} />}
        </View>
        <View className="flex-1">
          <Text numberOfLines={1} className="font-noto-medium text-[13px]">
            {node.name}
          </Text>
          <Text numberOfLines={1} className="text-[11px] text-muted-foreground">
            {detail}
          </Text>
        </View>
        {opening ? <Spinner /> : null}
      </Pressable>
      <IconButton icon="more" label={`${node.name} options`} size={28} iconSize={14} color={muted} onPress={onMenu} />
    </View>
  )
}
