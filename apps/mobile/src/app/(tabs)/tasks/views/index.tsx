import { router } from 'expo-router'
import { useMemo, useState } from 'react'
import { Pressable, RefreshControl, SectionList, View } from 'react-native'
import { TextInput } from '../../../../ui/scaled'

import { MenuButton } from '../../../../components/Drawer'
import { ActionSheet, ConfirmDialog, type ConfirmRequest, type SheetAction } from '../../../../components/settings/Dialog'
import { ViewDialog, type ViewDialogRequest } from '../../../../components/tasks/ViewControls'
import { defaultViewState, normalizeViewState, type SavedView, viewSections, viewSummary } from '../../../../data/views'
import { useViews, useViewWrites } from '../../../../hooks/views'
import { useOrbit } from '../../../../orbit-context'
import { useTheme } from '../../../../theme'
import { EmptyState, ErrorView, HeaderTitle, Icon, IconButton, LoadingView, PaneHeader, SafeAreaView, SwipeRow, Text } from '../../../../ui'

/**
 * The web's `/views`: the task views the user can see, favorites first, then their own, then the workspace's. A row
 * opens the view; swipe right stars it, swipe left deletes it; a long press has every action.
 */
export default function ViewsScreen() {
  const { colors } = useTheme()
  const { user } = useOrbit()
  const list = useViews()
  const writes = useViewWrites()
  const [search, setSearch] = useState('')
  const [menu, setMenu] = useState<{ title: string; actions: SheetAction[] } | null>(null)
  const [confirm, setConfirm] = useState<ConfirmRequest | null>(null)
  const [dialog, setDialog] = useState<ViewDialogRequest | null>(null)
  const sections = useMemo(() => viewSections(list.data ?? [], search).map((section) => ({ ...section, data: section.views })), [list.data, search])

  const newView = () =>
    setDialog({
      title: 'New view',
      confirmLabel: 'Create view',
      initial: { name: '', description: '', visibility: 'personal' },
      canChangeVisibility: true,
      onSubmit: async (fields) => {
        const created = await writes.create.mutateAsync({ fields, state: defaultViewState() })
        router.push(`/tasks/views/${created.id}`)
      },
    })
  const remove = (view: SavedView) =>
    setConfirm({
      title: `Delete “${view.name}”?`,
      description: view.visibility === 'workspace' ? 'The view goes for every member. Its tasks stay.' : 'The view goes. Its tasks stay.',
      confirmLabel: 'Delete view',
      danger: true,
      onConfirm: () => writes.remove.mutate(view.id),
    })
  const actions = (view: SavedView): SheetAction[] => [
    { label: view.is_favorite ? 'Remove from favorites' : 'Add to favorites', icon: 'star', onPress: () => writes.favorite.mutate({ viewId: view.id, favorite: !view.is_favorite }) },
    ...(view.can_edit
      ? [
          {
            label: 'Edit name and visibility',
            icon: 'edit' as const,
            onPress: () =>
              setDialog({
                title: 'Edit view',
                confirmLabel: 'Save',
                initial: { name: view.name, description: view.description, visibility: view.visibility },
                canChangeVisibility: view.can_change_visibility,
                onSubmit: (fields) => writes.save.mutateAsync({ view, changes: view.can_change_visibility ? fields : { name: fields.name, description: fields.description } }),
              }),
          },
        ]
      : []),
    {
      label: 'Duplicate',
      icon: 'copy',
      disabled: !view.state,
      onPress: () =>
        setDialog({
          title: 'Duplicate view',
          confirmLabel: 'Duplicate',
          initial: { name: `${view.name} copy`, description: view.description, visibility: 'personal' },
          canChangeVisibility: true,
          onSubmit: async (fields) => {
            const created = await writes.create.mutateAsync({ fields, state: normalizeViewState(view.state) })
            router.push(`/tasks/views/${created.id}`)
          },
        }),
    },
    ...(view.can_edit ? [{ label: 'Delete view', icon: 'trash' as const, destructive: true, onPress: () => remove(view) }] : []),
  ]

  return (
    <SafeAreaView edges={['top', 'left', 'right']} className="flex-1 bg-background">
      <PaneHeader className="flex-wrap">
        <MenuButton />
        <Pressable accessibilityRole="link" onPress={() => router.navigate('/tasks')} className="h-8 justify-center px-1.5 rounded-lg active:bg-muted">
          <Text className="text-[14px] text-muted-foreground">Tasks /</Text>
        </Pressable>
        <View className="flex-1">
          <HeaderTitle>Views</HeaderTitle>
        </View>
        <IconButton icon="plus" label="New view" size={32} variant="primary" onPress={newView} />
        {(list.data?.length ?? 0) > 0 ? (
          <View className="h-8.5 mt-1 basis-full flex-row items-center border border-input rounded-lg bg-input/30">
            <View className="w-5.5 items-end">
              <Icon name="search" size={14} color={colors.mutedForeground} />
            </View>
            <TextInput
              accessibilityLabel="Search views"
              placeholder="Search views"
              placeholderTextColor={colors.mutedForeground}
              selectionColor={colors.primary}
              value={search}
              onChangeText={setSearch}
              returnKeyType="search"
              className="flex-1 h-8 pl-2 pr-2.5 font-noto text-[16px] text-foreground"
            />
          </View>
        ) : null}
      </PaneHeader>
      {!list.data ? (
        list.error ? (
          <ErrorView error={list.error} onRetry={() => void list.refetch()} />
        ) : (
          <LoadingView />
        )
      ) : (
        <SectionList
          sections={sections}
          keyExtractor={(view) => view.id}
          stickySectionHeadersEnabled={false}
          keyboardShouldPersistTaps="handled"
          contentContainerClassName={sections.length === 0 ? 'grow' : 'pb-6'}
          refreshControl={<RefreshControl refreshing={list.isRefetching} onRefresh={() => void list.refetch()} tintColor={colors.mutedForeground} />}
          ListEmptyComponent={
            search ? (
              <EmptyState icon="search" title="No results" description="No view matches your search." />
            ) : (
              <EmptyState icon="layer" title="No views yet" description="A view keeps a filter, a grouping and an order of tasks. Tap + to make one, or use Save as view on the Tasks page." />
            )
          }
          renderSectionHeader={({ section }) => (
            <View className="flex-row items-center gap-1.5 px-4 pt-4 pb-1.5">
              <Icon name={section.key === 'favorites' ? 'star' : section.key === 'mine' ? 'lock' : 'people'} size={12} color={colors.mutedForeground} />
              <Text className="font-noto-medium text-[12px] text-muted-foreground">{section.label}</Text>
            </View>
          )}
          renderItem={({ item }) => (
            <SwipeRow
              left={{ label: item.is_favorite ? 'Unfavorite' : 'Favorite', icon: 'star', tone: 'primary', onSwipe: () => writes.favorite.mutate({ viewId: item.id, favorite: !item.is_favorite }) }}
              right={item.can_edit ? { label: 'Delete', icon: 'trash', tone: 'destructive', onSwipe: () => remove(item) } : undefined}
            >
              <Pressable
                accessibilityRole="button"
                accessibilityLabel={item.name}
                accessibilityHint="Long press for actions"
                onPress={() => router.push(`/tasks/views/${item.id}`)}
                onLongPress={() => setMenu({ title: item.name, actions: actions(item) })}
                className="min-h-14 flex-row items-center gap-3 px-4 py-2 border-b border-border active:bg-muted"
              >
                <View className="size-8 rounded-lg items-center justify-center bg-muted">
                  <Icon name="layer" size={16} color={item.color ?? colors.mutedForeground} />
                </View>
                <View className="flex-1 min-w-0">
                  <Text numberOfLines={1} className="font-noto-medium text-[14px]">
                    {item.name}
                  </Text>
                  <Text numberOfLines={1} className="text-[12px] text-muted-foreground">
                    {[item.description, item.owner.user_id === user.id ? null : item.owner.display_name, viewSummary(item)].filter(Boolean).join(' · ')}
                  </Text>
                </View>
                <IconButton
                  icon="star"
                  label={item.is_favorite ? `Remove ${item.name} from favorites` : `Add ${item.name} to favorites`}
                  size={32}
                  color={item.is_favorite ? colors.primary : colors.mutedForeground}
                  onPress={() => writes.favorite.mutate({ viewId: item.id, favorite: !item.is_favorite })}
                />
              </Pressable>
            </SwipeRow>
          )}
        />
      )}
      <ActionSheet visible={menu !== null} title={menu?.title} actions={menu?.actions ?? []} onClose={() => setMenu(null)} />
      <ConfirmDialog request={confirm} onClose={() => setConfirm(null)} />
      <ViewDialog request={dialog} onClose={() => setDialog(null)} />
    </SafeAreaView>
  )
}
