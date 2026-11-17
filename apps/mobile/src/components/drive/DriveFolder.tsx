import { driveFileKind } from '@orbit/api/drive'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { router } from 'expo-router'
import { useDeferredValue, useState } from 'react'
import { FlatList, Image, Pressable, RefreshControl, View } from 'react-native'
import { TextInput } from '../../ui/scaled'

import { keys } from '../../data/keys'
import { createFolder, driveChildren, driveNode, type DriveNode, type DriveSort, downloadUrl, renameNode, restoreNode, searchNodes, type SortOrder, trashNode, uploadToDrive } from '../../data/drive'
import { fileSize } from '../../data/tasks'
import { pickDriveFiles, type DriveSource } from '../../drive-file'
import { useOrbit } from '../../orbit-context'
import { errorMessage } from '../../session-context'
import { alpha, useTheme } from '../../theme'
import { EmptyState, ErrorView, HeaderTitle, Icon, type IconName, IconButton, LoadingView, PaneHeader, PickerSheet, SafeAreaView, Spinner, SwipeRow, Text } from '../../ui'
import { usePublishPageItem } from '../AskDonna'
import { MenuButton } from '../Drawer'
import { ActionSheet, ConfirmDialog, type ConfirmRequest, PromptDialog, type PromptRequest, type SheetAction } from '../settings/Dialog'
import { openFile } from '../tasks/files'
import { MoveSheet, ShareSheet } from './DriveSheets'

type Menu = { title: string; actions: SheetAction[] }

const SORTS: { key: string; label: string; sort: DriveSort; order: SortOrder }[] = [
  { key: 'name', label: 'Name (A–Z)', sort: 'name', order: 'asc' },
  { key: 'name-desc', label: 'Name (Z–A)', sort: 'name', order: 'desc' },
  { key: 'updated', label: 'Last modified', sort: 'updated_at', order: 'desc' },
  { key: 'size', label: 'Largest first', sort: 'size', order: 'desc' },
]

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
 * system preview; the row menu shares a link, moves, renames or trashes (swipe left trashes too, with Undo); + makes a
 * folder or uploads photos, videos and files. Search looks through all of Drive; the trash is one tap from the top.
 */
export function DriveFolder({ folderId }: { folderId: string | null }) {
  const { colors } = useTheme()
  const { client, workspaceId } = useOrbit()
  const queryClient = useQueryClient()
  const [sortKey, setSortKey] = useState('name')
  const sorting = SORTS.find((item) => item.key === sortKey) ?? SORTS[0]
  const children = useQuery({
    queryKey: [...keys.driveChildren(workspaceId, folderId), sorting.sort, sorting.order],
    queryFn: () => driveChildren(client, workspaceId, folderId, sorting.sort, sorting.order),
  })
  const [searching, setSearching] = useState(false)
  const [search, setSearch] = useState('')
  const query = useDeferredValue(search.trim())
  const found = useQuery({ queryKey: [...keys.drive(workspaceId), 'search', query], queryFn: () => searchNodes(client, workspaceId, query), enabled: searching && query.length > 0, placeholderData: (previous) => previous })
  const [sortOpen, setSortOpen] = useState(false)
  const [moving, setMoving] = useState<DriveNode | null>(null)
  const [sharing, setSharing] = useState<DriveNode | null>(null)
  const [undo, setUndo] = useState<DriveNode | null>(null)
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

  /** Trash at once; the notice offers Undo, and the trash keeps it 30 days. */
  const trashNow = (node: DriveNode) => {
    setNotice(null)
    setUndo(node)
    void trashNode(client, workspaceId, node.id)
      .then(refresh)
      .catch((error: unknown) => {
        setUndo(null)
        failed('Could not move it to the trash.')(error)
      })
  }
  const undoTrash = (node: DriveNode) => {
    setUndo(null)
    void restoreNode(client, workspaceId, node.id)
      .then(refresh)
      .catch(failed('Could not restore it.'))
  }

  const nodeMenu = (node: DriveNode) =>
    setMenu({
      title: node.name,
      actions: [
        ...(node.kind === 'file' ? [{ label: 'Open', icon: 'arrowUpRightSquare' as const, onPress: () => void open(node) }] : []),
        { label: 'Share link…', icon: 'link2', onPress: () => setSharing(node) },
        { label: 'Move to…', icon: 'folder', onPress: () => setMoving(node) },
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
              description: node.kind === 'folder' ? 'Everything in the folder goes with it. You can restore it from the Drive trash for 30 days.' : 'You can restore it from the Drive trash for 30 days.',
              confirmLabel: 'Move to trash',
              danger: true,
              onConfirm: () => trashNow(node),
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
        <IconButton icon="search" label="Search Drive" size={32} color={searching ? colors.primary : undefined} onPress={() => {
            setSearching(!searching)
            setSearch('')
          }}
        />
        <IconButton icon="display" label={`Sort: ${sorting.label}`} size={32} onPress={() => setSortOpen(true)} />
        <IconButton icon="plus" label="Add to Drive" size={32} variant="primary" disabled={uploading !== null} onPress={newMenu} />
      </PaneHeader>
      {searching ? (
        <View className="h-8.5 mx-3 mb-1 flex-row items-center border border-input rounded-lg bg-input/30">
          <View className="w-5.5 items-end">
            <Icon name="search" size={14} color={colors.mutedForeground} />
          </View>
          <TextInput
            accessibilityLabel="Search Drive"
            placeholder="Search all of Drive"
            placeholderTextColor={colors.mutedForeground}
            selectionColor={colors.primary}
            autoFocus
            value={search}
            onChangeText={setSearch}
            returnKeyType="search"
            autoCorrect={false}
            className="flex-1 h-8 pl-2 pr-2.5 font-noto text-[16px] text-foreground"
          />
        </View>
      ) : null}
      {undo ? (
        <View accessibilityRole="alert" className="mx-3 mb-1 p-2.5 rounded-md bg-muted flex-row items-center gap-2">
          <Text numberOfLines={1} className="text-[13px] flex-1">
            Moved “{undo.name}” to the trash.
          </Text>
          <Pressable accessibilityRole="button" onPress={() => undoTrash(undo)} hitSlop={8}>
            <Text className="font-noto-semibold text-[13px] text-primary">Undo</Text>
          </Pressable>
          <IconButton icon="close" label="Dismiss" size={24} iconSize={12} color={muted} onPress={() => setUndo(null)} />
        </View>
      ) : null}
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
      {searching && query ? (
        !found.data ? (
          found.error ? <ErrorView error={found.error} onRetry={() => void found.refetch()} /> : <LoadingView />
        ) : found.data.length === 0 ? (
          <EmptyState icon="search" title="No results" description={`Nothing in Drive is called “${query}”.`} />
        ) : (
          <FlatList
            data={found.data}
            keyExtractor={(entry) => entry.node.id}
            keyboardShouldPersistTaps="handled"
            contentContainerClassName="px-2 pt-1 pb-4"
            renderItem={({ item }) => <DriveRow node={item.node} detail={item.folder_path || 'Drive'} opening={opening === item.node.id} onPress={() => void open(item.node)} onMenu={() => nodeMenu(item.node)} />}
          />
        )
      ) : !children.data ? (
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
          renderItem={({ item }) => (
            <SwipeRow right={{ label: 'Trash', icon: 'trash', tone: 'destructive', onSwipe: () => trashNow(item) }}>
              <DriveRow node={item} opening={opening === item.id} onPress={() => void open(item)} onMenu={() => nodeMenu(item)} />
            </SwipeRow>
          )}
        />
      )}
      {/* the trash as a row at the bottom of the top level, as in Docs */}
      {folderId === null ? (
        <Pressable accessibilityRole="link" accessibilityLabel="Trash" onPress={() => router.push('/drive/trash')} className="h-[49px] flex-row items-center gap-2.5 px-4 border-t border-border active:bg-muted">
          <Icon name="trash" size={15} color={colors.mutedForeground} />
          <Text className="font-noto-medium text-[13px] text-foreground">Trash</Text>
        </Pressable>
      ) : null}

      <ActionSheet visible={menu !== null} title={menu?.title} actions={menu?.actions ?? []} onClose={() => setMenu(null)} />
      <ConfirmDialog request={confirm} onClose={() => setConfirm(null)} />
      <PromptDialog request={prompt} onClose={() => setPrompt(null)} />
      <PickerSheet
        visible={sortOpen}
        title="Sort by"
        options={SORTS.map((item) => ({ key: item.key, label: item.label, selected: item.key === sorting.key }))}
        onSelect={(key) => {
          setSortOpen(false)
          setSortKey(key)
        }}
        onClose={() => setSortOpen(false)}
      />
      <MoveSheet node={moving} onClose={() => setMoving(null)} onMoved={() => setUndo(null)} />
      <ShareSheet node={sharing} onClose={() => setSharing(null)} />
    </SafeAreaView>
  )
}

function DriveRow({ node, detail: where, opening, onPress, onMenu }: { node: DriveNode; detail?: string; opening: boolean; onPress: () => void; onMenu: () => void }) {
  const { colors } = useTheme()
  const muted = alpha(colors.mutedForeground, 0.7)
  const kind = driveFileKind(node)
  const detail = [where, node.kind === 'folder' ? 'Folder' : fileSize(node.size ?? 0)].filter(Boolean).join(' · ')
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
