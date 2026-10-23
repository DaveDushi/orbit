import { router, useLocalSearchParams } from 'expo-router'
import { useMemo, useState } from 'react'
import { Pressable, RefreshControl, SectionList, TextInput, View } from 'react-native'
import { SafeAreaView } from 'react-native-safe-area-context'

import { MenuButton } from '../../../components/Drawer'
import { NewTaskDialog, type NewTaskDefaults } from '../../../components/NewTaskDialog'
import { TaskRow } from '../../../components/TaskRow'
import { groupTasksByStatus, type StatusGroup } from '../../../data/tasks'
import { useOrbit } from '../../../orbit-context'
import { usePeople, useProjects, useStatuses, useTasks } from '../../../queries'
import { alpha, type, useTheme } from '../../../theme'
import { EmptyState, ErrorView, HeaderTitle, Icon, IconButton, LoadingView, PaneHeader, PickerSheet, StatusIcon, Text } from '../../../ui'

/** The server's task views the drawer links to, with the web page titles. */
const PRESET_TITLE: Record<string, string> = {
  mine: 'My tasks',
  overdue: 'Overdue',
  due_soon: 'Due soon',
  current_week: 'This week',
  my_week: 'My week',
}

type Sheet = 'project' | 'filter' | 'display'
type Grouping = 'status' | 'none'

export default function TasksScreen() {
  const { view: viewParam, search: searchParam } = useLocalSearchParams<{ view?: string; search?: string }>()
  const view = viewParam && viewParam in PRESET_TITLE ? viewParam : undefined
  const { user } = useOrbit()
  const { colors } = useTheme()
  const tasks = useTasks(view)
  const statuses = useStatuses()
  const projects = useProjects()
  const people = usePeople()
  const [projectId, setProjectId] = useState<string | null>(null)
  const [assignedToMe, setAssignedToMe] = useState(false)
  const [grouping, setGrouping] = useState<Grouping>('status')
  const [search, setSearch] = useState('')
  const [collapsed, setCollapsed] = useState<ReadonlySet<string>>(new Set())
  const [sheet, setSheet] = useState<Sheet | null>(null)
  const [creating, setCreating] = useState<NewTaskDefaults | null>(null)
  const project = projects.data?.find((item) => item.id === projectId)

  const sections = useMemo(() => {
    if (!tasks.data || !statuses.data) return []
    const needle = search.trim().toLowerCase()
    const visible = tasks.data.filter(
      (task) =>
        (!projectId || task.project_id === projectId) &&
        (!assignedToMe || task.assignee_ids.includes(user.id)) &&
        (!needle || task.title.toLowerCase().includes(needle)),
    )
    const groups: StatusGroup[] = grouping === 'status' ? groupTasksByStatus(visible, statuses.data) : [{ key: 'all', name: 'All', status: undefined, tasks: visible }]
    return groups.filter((group) => group.tasks.length > 0).map((group) => ({ ...group, data: collapsed.has(group.key) ? [] : group.tasks }))
  }, [assignedToMe, collapsed, grouping, projectId, search, statuses.data, tasks.data, user.id])

  const refresh = () => Promise.all([tasks.refetch(), statuses.refetch(), projects.refetch()])
  const toggle = (key: string) =>
    setCollapsed((current) => {
      const next = new Set(current)
      if (next.has(key)) next.delete(key)
      else next.add(key)
      return next
    })
  const newTask = (statusId?: string) => setCreating({ projectId: projectId ?? undefined, statusId })

  const header = (
    <>
      <PaneHeader style={{ minHeight: 44, paddingVertical: 6, flexWrap: 'wrap' }}>
        <MenuButton />
        <Pressable
          accessibilityRole="button"
          accessibilityLabel="Select project"
          onPress={() => setSheet('project')}
          style={({ pressed }) => ({ height: 32, maxWidth: '30%', flexDirection: 'row', alignItems: 'center', gap: 7, paddingHorizontal: 7, borderRadius: 10, backgroundColor: pressed ? colors.muted : 'transparent' })}
        >
          {project ? <View style={{ width: 8, height: 8, borderRadius: 4, backgroundColor: project.color }} /> : null}
          <Text numberOfLines={1} style={[type(14), { lineHeight: 20, flexShrink: 1, color: colors.mutedForeground }]}>
            {project?.name ?? 'All projects'}
          </Text>
          <Icon name="chevronDown" size={14} color={colors.mutedForeground} />
        </Pressable>
        <View style={{ flex: 1, minWidth: 0 }}>
          <HeaderTitle>{view ? PRESET_TITLE[view] : 'All tasks'}</HeaderTitle>
        </View>
        <IconButton icon="filter" label="Filter" size={32} onPress={() => setSheet('filter')} color={assignedToMe ? colors.primary : undefined} />
        <IconButton icon="display" label="Display" size={32} onPress={() => setSheet('display')} />
        <IconButton icon="plus" label="New task" size={32} variant="primary" onPress={() => newTask()} />
        <View
          style={{
            flexBasis: '100%',
            height: 34,
            marginTop: 4,
            flexDirection: 'row',
            alignItems: 'center',
            borderWidth: 1,
            borderColor: colors.input,
            borderRadius: 10,
            backgroundColor: alpha(colors.input, 0.3),
          }}
        >
          <View style={{ width: 22, alignItems: 'flex-end' }}>
            <Icon name="search" size={14} color={colors.mutedForeground} />
          </View>
          <TextInput
            accessibilityLabel="Search tasks"
            placeholder="Search tasks"
            placeholderTextColor={colors.mutedForeground}
            selectionColor={colors.primary}
            autoFocus={searchParam === '1'}
            value={search}
            onChangeText={setSearch}
            returnKeyType="search"
            style={{ flex: 1, height: 32, paddingLeft: 8, paddingRight: 10, fontSize: 16, fontFamily: type(16).fontFamily, color: colors.foreground }}
          />
        </View>
      </PaneHeader>
    </>
  )

  const projectOptions = [{ key: 'all', label: 'All projects', selected: projectId === null }, ...(projects.data ?? []).map((item) => ({ key: item.id, label: item.name, selected: item.id === projectId, leading: <View style={{ width: 8, height: 8, borderRadius: 4, backgroundColor: item.color }} /> }))]

  return (
    <SafeAreaView edges={['top', 'left', 'right']} style={{ flex: 1, backgroundColor: colors.background }}>
      {header}
      {!tasks.data || !statuses.data ? (
        tasks.error || statuses.error ? (
          <ErrorView error={tasks.error ?? statuses.error} onRetry={() => void refresh()} />
        ) : (
          <LoadingView />
        )
      ) : (
        <SectionList
          sections={sections}
          keyExtractor={(task) => task.id}
          stickySectionHeadersEnabled={false}
          keyboardShouldPersistTaps="handled"
          contentContainerStyle={sections.length === 0 ? { flexGrow: 1 } : { paddingBottom: 24 }}
          refreshControl={<RefreshControl refreshing={tasks.isRefetching} onRefresh={() => void refresh()} tintColor={colors.mutedForeground} />}
          ListEmptyComponent={<EmptyState icon="tasks" title="No tasks" description={search || assignedToMe || projectId ? 'No task matches the filter.' : 'Tap + to create the first task.'} />}
          renderSectionHeader={({ section }) =>
            grouping === 'none' ? null : (
              <GroupHeader group={section} count={section.tasks.length} collapsed={collapsed.has(section.key)} onToggle={() => toggle(section.key)} onAdd={() => newTask(section.status?.id)} />
            )
          }
          renderItem={({ item }) => (
            <TaskRow
              task={item}
              status={statuses.data.find((status) => status.id === item.status_id)}
              assignee={people.find((person) => person.id === item.assignee_ids[0])}
              onPress={() => router.push(`/tasks/${item.id}`)}
            />
          )}
        />
      )}
      {creating ? <NewTaskDialog defaults={creating} onClose={() => setCreating(null)} /> : null}
      <PickerSheet
        visible={sheet === 'project'}
        title="Project"
        options={projectOptions}
        onSelect={(key) => {
          setSheet(null)
          setProjectId(key === 'all' ? null : key)
        }}
        onClose={() => setSheet(null)}
      />
      <PickerSheet
        visible={sheet === 'filter'}
        title="Filter"
        options={[
          { key: 'everyone', label: 'All tasks', selected: !assignedToMe },
          { key: 'me', label: 'Assigned to me', selected: assignedToMe },
        ]}
        onSelect={(key) => {
          setSheet(null)
          setAssignedToMe(key === 'me')
        }}
        onClose={() => setSheet(null)}
      />
      <PickerSheet
        visible={sheet === 'display'}
        title="Grouping"
        options={[
          { key: 'status', label: 'Status', selected: grouping === 'status' },
          { key: 'none', label: 'No grouping', selected: grouping === 'none' },
        ]}
        onSelect={(key) => {
          setSheet(null)
          setGrouping(key === 'none' ? 'none' : 'status')
        }}
        onClose={() => setSheet(null)}
      />
    </SafeAreaView>
  )
}

/** The web group row: chevron, status, name, count, and + on the card surface. */
function GroupHeader({ group, count, collapsed, onToggle, onAdd }: { group: StatusGroup; count: number; collapsed: boolean; onToggle: () => void; onAdd: () => void }) {
  const { colors } = useTheme()
  return (
    <View style={{ height: 36, flexDirection: 'row', alignItems: 'center', paddingLeft: 6, paddingRight: 8, backgroundColor: colors.card, borderBottomWidth: 1, borderBottomColor: colors.border }}>
      <Pressable
        accessibilityRole="button"
        accessibilityLabel={collapsed ? `Expand ${group.name}` : `Collapse ${group.name}`}
        onPress={onToggle}
        hitSlop={8}
        style={{ width: 20, height: 20, borderRadius: 8, alignItems: 'center', justifyContent: 'center', backgroundColor: colors.muted }}
      >
        <View style={{ transform: [{ rotate: collapsed ? '-90deg' : '0deg' }] }}>
          <Icon name="chevronDown" size={12} color={colors.foreground} />
        </View>
      </Pressable>
      <View style={{ marginLeft: 8 }}>
        <StatusIcon status={group.status} size={14} />
      </View>
      <Text style={[type(12, 'semibold'), { lineHeight: 16, marginLeft: 8, color: colors.mutedForeground }]}>{group.name}</Text>
      <Text style={[type(12), { lineHeight: 16, marginLeft: 8, flex: 1, color: alpha(colors.mutedForeground, 0.7) }]}>{count}</Text>
      <IconButton icon="plus" label={`New task in ${group.name}`} size={24} iconSize={12} color={colors.mutedForeground} onPress={onAdd} />
    </View>
  )
}
