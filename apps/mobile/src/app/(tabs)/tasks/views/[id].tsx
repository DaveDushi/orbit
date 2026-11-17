import { router, useLocalSearchParams } from 'expo-router'
import { useMemo, useState } from 'react'
import { Pressable, View } from 'react-native'

import { NewTaskDialog, type NewTaskDefaults } from '../../../../components/NewTaskDialog'
import { ActionSheet, ConfirmDialog, type ConfirmRequest, type SheetAction } from '../../../../components/settings/Dialog'
import { TaskList } from '../../../../components/tasks/TaskList'
import { DisplaySheet, FilterSheet, ViewDialog, type ViewDialogRequest } from '../../../../components/tasks/ViewControls'
import { countConditions, filterFromQuick, groupViewTasks, normalizeViewState, quickCount, quickFromFilter, type ViewState } from '../../../../data/views'
import { useView, useViewTasks, useViewWrites } from '../../../../hooks/views'
import { useOrbit } from '../../../../orbit-context'
import { useLabels, usePeople, useProjects, useStatuses } from '../../../../queries'
import { errorMessage } from '../../../../session-context'
import { alpha, useTheme } from '../../../../theme'
import { EmptyState, ErrorView, HeaderTitle, Icon, IconButton, LoadingView, PaneHeader, SafeAreaView, Text } from '../../../../ui'

type Sheet = 'filter' | 'display'

/**
 * One saved view (the web's `/views/:viewId`): its tasks, grouped and ordered as the view says. Filter and Display
 * change the list at once; "Save" keeps the change in the view (for those who may edit it), "Reset" drops it.
 */
export default function ViewScreen() {
  const { id } = useLocalSearchParams<{ id: string }>()
  const { colors } = useTheme()
  const { user } = useOrbit()
  const view = useView(id)
  const writes = useViewWrites()
  const statuses = useStatuses()
  const projects = useProjects()
  const labels = useLabels()
  const people = usePeople()
  const [draft, setDraft] = useState<ViewState | null>(null)
  const [sheet, setSheet] = useState<Sheet | null>(null)
  const [menu, setMenu] = useState<SheetAction[] | null>(null)
  const [confirm, setConfirm] = useState<ConfirmRequest | null>(null)
  const [dialog, setDialog] = useState<ViewDialogRequest | null>(null)
  const [creating, setCreating] = useState<NewTaskDefaults | null>(null)
  const saved = useMemo(() => normalizeViewState(view.data?.state), [view.data?.state])
  const state = draft ?? saved
  const tasks = useViewTasks(state, Boolean(view.data?.state))
  const { quick, extra } = quickFromFilter(state.filter)
  const muted = alpha(colors.mutedForeground, 0.7)
  const back = () => (router.canGoBack() ? router.back() : router.navigate('/tasks/views'))

  const sections = useMemo(
    () => (tasks.data && statuses.data ? groupViewTasks(tasks.data, state.display.group_by, { statuses: statuses.data, people, labels: labels.data ?? [], projects: projects.data ?? [], userId: user.id }) : []),
    [labels.data, people, projects.data, state.display.group_by, statuses.data, tasks.data, user.id],
  )

  const current = view.data
  const saveDraft = () => {
    if (!current || !draft) return
    writes.save.mutate({ view: current, changes: { state: draft } }, { onSuccess: () => setDraft(null), onError: (error) => setConfirm({ title: 'Could not save the view', description: errorMessage(error), confirmLabel: 'OK', onConfirm: () => {} }) })
  }
  const actions = (): SheetAction[] =>
    current
      ? [
          { label: current.is_favorite ? 'Remove from favorites' : 'Add to favorites', icon: 'star', onPress: () => writes.favorite.mutate({ viewId: current.id, favorite: !current.is_favorite }) },
          ...(current.can_edit
            ? [
                {
                  label: 'Edit name and visibility',
                  icon: 'edit' as const,
                  onPress: () =>
                    setDialog({
                      title: 'Edit view',
                      confirmLabel: 'Save',
                      initial: { name: current.name, description: current.description, visibility: current.visibility },
                      canChangeVisibility: current.can_change_visibility,
                      onSubmit: (fields) => writes.save.mutateAsync({ view: current, changes: current.can_change_visibility ? fields : { name: fields.name, description: fields.description } }),
                    }),
                },
              ]
            : []),
          {
            label: draft ? 'Save as new view' : 'Duplicate',
            icon: 'copy',
            onPress: () =>
              setDialog({
                title: draft ? 'Save as new view' : 'Duplicate view',
                confirmLabel: 'Save view',
                initial: { name: `${current.name} copy`, description: current.description, visibility: 'personal' },
                canChangeVisibility: true,
                onSubmit: async (fields) => {
                  const created = await writes.create.mutateAsync({ fields, state })
                  setDraft(null)
                  router.replace(`/tasks/views/${created.id}`)
                },
              }),
          },
          ...(current.can_edit
            ? [
                {
                  label: 'Delete view',
                  icon: 'trash' as const,
                  destructive: true,
                  onPress: () =>
                    setConfirm({
                      title: `Delete “${current.name}”?`,
                      description: 'The view goes. Its tasks stay.',
                      confirmLabel: 'Delete view',
                      danger: true,
                      onConfirm: () => writes.remove.mutate(current.id, { onSuccess: back }),
                    }),
                },
              ]
            : []),
        ]
      : []

  const filters = quickCount(quick) + countConditions({ op: 'and', children: extra })
  return (
    <SafeAreaView edges={['top', 'left', 'right']} className="flex-1 bg-background">
      <PaneHeader className="flex-wrap">
        <IconButton icon="back" label="Back to views" color={muted} onPress={back} />
        <View className="flex-1 min-w-0">
          <HeaderTitle>{current?.name ?? ''}</HeaderTitle>
        </View>
        {current ? (
          <>
            <IconButton icon="star" label={current.is_favorite ? 'Remove from favorites' : 'Add to favorites'} size={32} color={current.is_favorite ? colors.primary : muted} onPress={() => writes.favorite.mutate({ viewId: current.id, favorite: !current.is_favorite })} />
            <IconButton icon="filter" label={filters > 0 ? `Filter, ${filters} set` : 'Filter'} size={32} color={filters > 0 ? colors.primary : undefined} disabled={!current.state} onPress={() => setSheet('filter')} />
            <IconButton icon="display" label="Display" size={32} disabled={!current.state} onPress={() => setSheet('display')} />
            <IconButton icon="more" label="View options" size={32} onPress={() => setMenu(actions())} />
            <IconButton icon="plus" label="New task" size={32} variant="primary" onPress={() => setCreating({ projectId: quick.project.length === 1 ? quick.project[0] : undefined })} />
          </>
        ) : null}
        {draft ? (
          <View className="basis-full flex-row items-center gap-2 mt-1">
            <Text className="text-[12px] flex-1 text-muted-foreground">Unsaved changes</Text>
            <Pressable accessibilityRole="button" onPress={() => setDraft(null)} className="h-7 px-2.5 rounded-md justify-center active:bg-muted">
              <Text className="text-[12px] text-muted-foreground">Reset</Text>
            </Pressable>
            {current?.can_edit ? (
              <Pressable accessibilityRole="button" disabled={writes.save.isPending} onPress={saveDraft} className="h-7 px-2.5 rounded-md flex-row items-center gap-1.5 bg-primary active:opacity-80">
                <Icon name="check" size={12} color={colors.primaryForeground} />
                <Text className="font-noto-medium text-[12px] text-primary-foreground">Save</Text>
              </Pressable>
            ) : (
              <Pressable accessibilityRole="button" onPress={() => setMenu(actions())} className="h-7 px-2.5 rounded-md justify-center border border-border active:bg-muted">
                <Text className="font-noto-medium text-[12px]">Save as new view</Text>
              </Pressable>
            )}
          </View>
        ) : null}
      </PaneHeader>
      {!current ? (
        view.error ? (
          <ErrorView error={view.error} onRetry={() => void view.refetch()} />
        ) : (
          <LoadingView />
        )
      ) : !current.state ? (
        <EmptyState icon="warning" title="This view cannot be read" description={current.state_error ?? 'Open it on the web to fix its filter.'} />
      ) : !tasks.data || !statuses.data ? (
        tasks.error ? (
          <ErrorView error={tasks.error} onRetry={() => void tasks.refetch()} />
        ) : (
          <LoadingView />
        )
      ) : (
        <TaskList
          sections={sections}
          grouped={state.display.group_by !== 'none'}
          layout={state.display.layout}
          refreshing={tasks.isRefetching && !tasks.isPlaceholderData}
          onRefresh={() => void tasks.refetch()}
          onNew={(section) => setCreating({ projectId: section.status?.project_id ?? (quick.project.length === 1 ? quick.project[0] : undefined), statusId: section.status?.id })}
          empty={{ title: 'No tasks', description: filters > 0 ? 'No task matches this view’s filter.' : 'This view has no tasks yet.' }}
        />
      )}
      <FilterSheet visible={sheet === 'filter'} quick={quick} kept={countConditions({ op: 'and', children: extra })} withProject onChange={(next) => setDraft({ ...state, filter: filterFromQuick(next, extra) })} onClose={() => setSheet(null)} />
      <DisplaySheet visible={sheet === 'display'} state={state} onChange={setDraft} onClose={() => setSheet(null)} />
      <ActionSheet visible={menu !== null} title={current?.name} actions={menu ?? []} onClose={() => setMenu(null)} />
      <ConfirmDialog request={confirm} onClose={() => setConfirm(null)} />
      <ViewDialog request={dialog} onClose={() => setDialog(null)} />
      {creating ? <NewTaskDialog defaults={creating} onClose={() => setCreating(null)} /> : null}
    </SafeAreaView>
  )
}
