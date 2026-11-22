import { router, useLocalSearchParams } from 'expo-router'
import { useMemo, useState } from 'react'
import { Pressable, View } from 'react-native'
import { TextInput } from '../../../ui/scaled'

import { MenuButton } from '../../../components/Drawer'
import { NewProjectSheet } from '../../../components/NewProjectSheet'
import { NewTaskDialog, type NewTaskDefaults } from '../../../components/NewTaskDialog'
import { TaskList } from '../../../components/tasks/TaskList'
import { DisplaySheet, FilterSheet, ViewDialog, type ViewDialogRequest } from '../../../components/tasks/ViewControls'
import { TASK_PRESETS } from '../../../components/drawerAreas'
import { groupTasksByStatus } from '../../../data/tasks'
import { DEFAULT_DISPLAY, defaultViewState, EMPTY_QUICK, type FilterNode, filterFromQuick, groupViewTasks, PRESET_FILTERS, type QuickFilter, quickCount, type TaskSection, type ViewState } from '../../../data/views'
import { useViewTasks, useViewWrites } from '../../../hooks/views'
import { useOrbit } from '../../../orbit-context'
import { useLabels, usePeople, useProjects, useStatuses, useTasks } from '../../../queries'
import { useTheme } from '../../../theme'
import { ErrorView, HeaderTitle, Icon, IconButton, LoadingView, PaneHeader, PickerSheet, SafeAreaView, Text } from '../../../ui'
import { ProjectMark } from '../../../ui/ProjectMark'

/** The server's task views the drawer links to, with the web page titles. */
const PRESET_TITLE: Record<string, string> = {
  mine: 'My tasks',
  overdue: 'Overdue',
  due_soon: 'Due soon',
  current_week: 'This week',
  my_week: 'My week',
}

type Sheet = 'view' | 'project' | 'new-project' | 'filter' | 'display'

/** The view menu's rows after the presets: the saved views and the trash, so the drawer is not the only way there. */
const SAVED_VIEWS = 'saved-views'
/** The project menu's last row: a new project (also in the drawer). */
const NEW_PROJECT = 'new-project'
const TRASH = 'trash'

const isDefaultDisplay = (display: ViewState['display']) =>
  display.layout === DEFAULT_DISPLAY.layout && display.group_by === DEFAULT_DISPLAY.group_by && display.order_by === DEFAULT_DISPLAY.order_by && display.order_direction === DEFAULT_DISPLAY.order_direction && display.show_completed === DEFAULT_DISPLAY.show_completed

export default function TasksScreen() {
  const { view: viewParam, project: projectParam } = useLocalSearchParams<{ view?: string; project?: string }>()
  const view = viewParam && viewParam in PRESET_TITLE ? viewParam : undefined
  const { user } = useOrbit()
  const { colors } = useTheme()
  const statuses = useStatuses()
  const projects = useProjects()
  const labels = useLabels()
  const people = usePeople()
  // in the URL, so the drawer's project links can pick it
  const projectId = projectParam || null
  const setProjectId = (id: string | null) => router.setParams({ project: id ?? undefined })
  const [quick, setQuick] = useState<QuickFilter>(EMPTY_QUICK)
  const [display, setDisplay] = useState(DEFAULT_DISPLAY)
  const [search, setSearch] = useState('')
  const [sheet, setSheet] = useState<Sheet | null>(null)
  const [creating, setCreating] = useState<NewTaskDefaults | null>(null)
  const [saving, setSaving] = useState<ViewDialogRequest | null>(null)
  const { create } = useViewWrites()
  const project = projects.data?.find((item) => item.id === projectId)

  // the page as a view: its preset, its project and the Filter sheet's rows
  const scope: FilterNode[] = [...(view ? PRESET_FILTERS[view] : []), ...(projectId ? [{ field: 'project' as const, operator: 'is' as const, value: [projectId] }] : [])]
  const state: ViewState = { filter: filterFromQuick(quick, scope), display }
  // with no filter and the default display, the cached task list shows at once; anything else asks the server
  const simple = quickCount(quick) === 0 && isDefaultDisplay(display)
  const listed = useTasks(view)
  const queried = useViewTasks(state, !simple)
  const tasks = simple ? listed : queried

  const sections = useMemo((): TaskSection[] => {
    if (!tasks.data || !statuses.data) return []
    const needle = search.trim().toLowerCase()
    const visible = tasks.data.filter((task) => (!simple || !projectId || task.project_id === projectId) && (!needle || task.title.toLowerCase().includes(needle)))
    if (simple) return groupTasksByStatus(visible, statuses.data).map((group) => ({ key: group.key, label: group.name, status: group.status, tasks: group.tasks }))
    return groupViewTasks(visible, display.group_by, { statuses: statuses.data, people, labels: labels.data ?? [], projects: projects.data ?? [], userId: user.id })
  }, [display.group_by, labels.data, people, projectId, projects.data, search, simple, statuses.data, tasks.data, user.id])

  const refresh = () => Promise.all([tasks.refetch(), statuses.refetch(), projects.refetch()])
  const newTask = (section?: TaskSection) => setCreating({ projectId: projectId ?? undefined, statusId: section?.status && section.status.project_id === projectId ? section.status.id : undefined })
  const filters = quickCount(quick)
  const changed = filters > 0 || !isDefaultDisplay(display)
  const saveAsView = () => {
    const title = view ? PRESET_TITLE[view] : project ? project.name : 'All tasks'
    setSaving({
      title: 'Save as view',
      confirmLabel: 'Save view',
      initial: { name: filters > 0 ? '' : title, description: '', visibility: 'personal' },
      canChangeVisibility: true,
      onSubmit: async (fields) => {
        const saved = await create.mutateAsync({ fields, state: { filter: state.filter, display: { ...defaultViewState().display, ...display } } })
        router.push(`/tasks/views/${saved.id}`)
      },
    })
  }

  const header = (
    <PaneHeader className="flex-wrap">
      <MenuButton />
      <Pressable
        accessibilityRole="button"
        accessibilityLabel="Select view"
        accessibilityValue={{ text: view ? PRESET_TITLE[view] : 'All tasks' }}
        onPress={() => setSheet('view')}
        className="h-8 flex-1 min-w-0 flex-row items-center gap-1 px-1 rounded-lg active:bg-muted"
      >
        <HeaderTitle>{view ? PRESET_TITLE[view] : 'All tasks'}</HeaderTitle>
        <Icon name="chevronDown" size={14} color={colors.mutedForeground} />
      </Pressable>
      <IconButton icon="filter" label={filters > 0 ? `Filter, ${filters} set` : 'Filter'} size={32} onPress={() => setSheet('filter')} color={filters > 0 ? colors.primary : undefined} />
      <IconButton icon="display" label="Display" size={32} onPress={() => setSheet('display')} color={isDefaultDisplay(display) ? undefined : colors.primary} />
      {project ? <IconButton icon="settings" label={`${project.name} settings`} size={32} onPress={() => router.push(`/tasks/project/${project.id}`)} /> : null}
      <IconButton icon="plus" label="New task" size={32} variant="primary" onPress={() => newTask()} />
      {/* the project beside the search, so the title keeps its room */}
      <View className="mt-1 basis-full flex-row items-center gap-2">
        <Pressable
          accessibilityRole="button"
          accessibilityLabel="Select project"
          accessibilityValue={{ text: project?.name ?? 'All projects' }}
          onPress={() => setSheet('project')}
          className="h-8.5 max-w-[45%] flex-row items-center gap-[7px] px-2.5 rounded-lg border border-input active:bg-muted"
        >
          {project ? <ProjectMark project={project} /> : null}
          <Text numberOfLines={1} className="text-[14px] leading-[20px] shrink text-muted-foreground">
            {project?.name ?? 'All projects'}
          </Text>
          <Icon name="chevronDown" size={14} color={colors.mutedForeground} />
        </Pressable>
        <View className="h-8.5 flex-1 flex-row items-center border border-input rounded-lg bg-input/30">
          <View className="w-5.5 items-end">
            <Icon name="search" size={14} color={colors.mutedForeground} />
          </View>
          <TextInput
            accessibilityLabel="Search tasks"
            placeholder="Search tasks"
            placeholderTextColor={colors.mutedForeground}
            selectionColor={colors.primary}
            value={search}
            onChangeText={setSearch}
            returnKeyType="search"
            className="flex-1 h-8 pl-2 pr-2.5 font-noto text-[16px] text-foreground"
          />
        </View>
      </View>
      {changed ? (
        <View className="basis-full flex-row items-center gap-2 mt-1">
          <Text className="text-[12px] flex-1 text-muted-foreground">{filters > 0 ? `${filters} ${filters === 1 ? 'filter' : 'filters'}` : 'Custom display'}</Text>
          <Pressable
            accessibilityRole="button"
            onPress={() => {
              setQuick(EMPTY_QUICK)
              setDisplay(DEFAULT_DISPLAY)
            }}
            className="h-7 px-2.5 rounded-md justify-center active:bg-muted"
          >
            <Text className="text-[12px] text-muted-foreground">Reset</Text>
          </Pressable>
          <Pressable accessibilityRole="button" onPress={saveAsView} className="h-7 px-2.5 rounded-md flex-row items-center gap-1.5 border border-border active:bg-muted">
            <Icon name="layer" size={12} color={colors.foreground} />
            <Text className="font-noto-medium text-[12px]">Save as view</Text>
          </Pressable>
        </View>
      ) : null}
    </PaneHeader>
  )

  const viewOptions = [
    { key: 'all', label: 'All tasks', selected: !view, leading: <Icon name="tasks" size={16} color={colors.mutedForeground} /> },
    ...TASK_PRESETS.map((preset) => ({ key: preset.view, label: preset.label, selected: view === preset.view, leading: <Icon name={preset.icon} size={16} color={colors.mutedForeground} /> })),
    { key: SAVED_VIEWS, label: 'Saved views', leading: <Icon name="layer" size={16} color={colors.mutedForeground} /> },
    { key: TRASH, label: 'Trash', leading: <Icon name="trash" size={16} color={colors.mutedForeground} /> },
  ]
  const projectOptions = [
    { key: 'all', label: 'All projects', selected: projectId === null },
    ...(projects.data ?? []).map((item) => ({ key: item.id, label: item.name, selected: item.id === projectId, leading: <ProjectMark project={item} /> })),
    { key: NEW_PROJECT, label: 'New project', leading: <Icon name="plus" size={16} color={colors.mutedForeground} /> },
  ]

  return (
    <SafeAreaView edges={['top', 'left', 'right']} className="flex-1 bg-background">
      {header}
      {!tasks.data || !statuses.data ? (
        tasks.error || statuses.error ? (
          <ErrorView error={tasks.error ?? statuses.error} onRetry={() => void refresh()} />
        ) : (
          <LoadingView />
        )
      ) : (
        <TaskList
          sections={sections}
          grouped={simple || display.group_by !== 'none'}
          layout={display.layout}
          refreshing={tasks.isRefetching}
          onRefresh={() => void refresh()}
          onNew={(section) => newTask(section)}
          empty={{ title: 'No tasks', description: search || filters || projectId ? 'No task matches the filter.' : 'Tap + to create the first task.' }}
        />
      )}
      {creating ? <NewTaskDialog defaults={creating} onClose={() => setCreating(null)} /> : null}
      <PickerSheet
        visible={sheet === 'view'}
        title="View"
        options={viewOptions}
        onSelect={(key) => {
          setSheet(null)
          if (key === SAVED_VIEWS) router.push('/tasks/views')
          else if (key === TRASH) router.push('/tasks/trash')
          else router.setParams({ view: key === 'all' ? undefined : key })
        }}
        onClose={() => setSheet(null)}
      />
      <PickerSheet
        visible={sheet === 'project'}
        title="Project"
        options={projectOptions}
        onSelect={(key) => {
          if (key === NEW_PROJECT) return setSheet('new-project')
          setSheet(null)
          setProjectId(key === 'all' ? null : key)
        }}
        onClose={() => setSheet(null)}
      />
      <NewProjectSheet
        visible={sheet === 'new-project'}
        onClose={() => setSheet(null)}
        onCreated={(created) => {
          setSheet(null)
          setProjectId(created.id)
        }}
      />
      <FilterSheet visible={sheet === 'filter'} quick={quick} kept={0} withProject={false} onChange={setQuick} onClose={() => setSheet(null)} />
      <DisplaySheet visible={sheet === 'display'} state={state} onChange={(next) => setDisplay(next.display)} onClose={() => setSheet(null)} />
      <ViewDialog request={saving} onClose={() => setSaving(null)} />
    </SafeAreaView>
  )
}
