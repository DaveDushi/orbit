import { router } from 'expo-router'
import { useMemo, useState } from 'react'
import { RefreshControl, SectionList, View } from 'react-native'

import { TaskRow } from '../../components/TaskRow'
import { groupTasksByStatus, identifier } from '../../data/tasks'
import { useOrbit } from '../../orbit-context'
import { usePeople, useProjects, useStatuses, useTasks } from '../../queries'
import { useTheme } from '../../theme'
import { EmptyState, ErrorView, LoadingView, Segmented, StatusIcon, Text } from '../../ui'

type Filter = 'all' | 'mine'
const FILTERS = [
  { value: 'all', label: 'All' },
  { value: 'mine', label: 'Mine' },
] as const

export default function TasksScreen() {
  const { user } = useOrbit()
  const { colors, spacing } = useTheme()
  const [filter, setFilter] = useState<Filter>('all')
  const tasks = useTasks()
  const statuses = useStatuses()
  const projects = useProjects()
  const people = usePeople()
  const now = new Date()

  const sections = useMemo(() => {
    if (!tasks.data || !statuses.data) return []
    const visible = filter === 'mine' ? tasks.data.filter((task) => task.assignee_ids.includes(user.id)) : tasks.data
    return groupTasksByStatus(visible, statuses.data).map((group) => ({ ...group, data: group.tasks }))
  }, [filter, statuses.data, tasks.data, user.id])

  const refresh = () => Promise.all([tasks.refetch(), statuses.refetch(), projects.refetch()])
  const header = (
    <View style={{ padding: spacing[4], paddingBottom: spacing[2] }}>
      <Segmented options={FILTERS} value={filter} onChange={setFilter} />
    </View>
  )

  if (!tasks.data || !statuses.data) {
    const error = tasks.error ?? statuses.error
    return (
      <View style={{ flex: 1 }}>
        {header}
        {error ? <ErrorView error={error} onRetry={() => void refresh()} /> : <LoadingView />}
      </View>
    )
  }

  return (
    <SectionList
      sections={sections}
      keyExtractor={(task) => task.id}
      ListHeaderComponent={header}
      stickySectionHeadersEnabled
      contentContainerStyle={sections.length === 0 ? { flexGrow: 1 } : { paddingBottom: spacing[6] }}
      refreshControl={<RefreshControl refreshing={tasks.isRefetching} onRefresh={() => void refresh()} tintColor={colors.mutedForeground} />}
      ListEmptyComponent={
        <EmptyState icon="tasks" title={filter === 'mine' ? 'Nothing assigned to you' : 'No tasks yet'} description={filter === 'mine' ? 'Tasks assigned to you show here.' : 'Tap + to create the first task.'} />
      }
      renderSectionHeader={({ section }) => (
        <View
          style={{
            flexDirection: 'row',
            alignItems: 'center',
            gap: spacing[2],
            paddingHorizontal: spacing[4],
            paddingVertical: spacing[2],
            backgroundColor: colors.muted,
            borderTopWidth: 1,
            borderBottomWidth: 1,
            borderColor: colors.border,
          }}
        >
          <StatusIcon status={section.status} size={14} />
          <Text variant="label">{section.name}</Text>
          <Text variant="caption" color="muted">
            {section.data.length}
          </Text>
        </View>
      )}
      renderItem={({ item }) => (
        <TaskRow
          task={item}
          status={statuses.data.find((status) => status.id === item.status_id)}
          identifier={identifier(item, projects.data ?? [])}
          assignee={people.find((person) => person.id === item.assignee_ids[0])}
          now={now}
          onPress={() => router.push(`/task/${item.id}`)}
        />
      )}
      ItemSeparatorComponent={() => <View style={{ height: 1, marginLeft: spacing[4] + 26, backgroundColor: colors.border }} />}
    />
  )
}
