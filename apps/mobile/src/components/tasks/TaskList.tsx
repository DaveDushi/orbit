/**
 * The task list of the Tasks page and of a saved view: groups with a header, rows that swipe (right: done or reopen,
 * left: move to the trash) and a long-press menu (the web's right-click menu): open, status, priority, assign to me,
 * done, trash. Every write shows at once and goes back on an error (`useTaskRowWrites`).
 */
import { router } from 'expo-router'
import { type ReactElement, useState } from 'react'
import { Pressable, RefreshControl, ScrollView, SectionList, useWindowDimensions, View } from 'react-native'

import { TaskRow } from '../TaskRow'
import { defaultStatusOf, identifier, isClosed, projectStatuses, PRIORITY_LABEL, priorityOf } from '../../data/tasks'
import type { StatusRecord, TaskRecord } from '../../data/orbit'
import type { Layout, TaskSection } from '../../data/views'
import { useTaskRowWrites } from '../../hooks/views'
import { useOrbit } from '../../orbit-context'
import { usePeople, useProjects, useStatuses } from '../../queries'
import { useTheme } from '../../theme'
import { Avatar, EmptyState, Icon, IconButton, PriorityIcon, StatusIcon, SwipeRow, Text } from '../../ui'
import { PriorityChoices, StatusChoices } from './pickers'
import { Sheet, SheetList, SheetRow, useSheet } from './Sheet'
import { ProjectMark } from '../../ui/ProjectMark'

type RowSheet = { kind: 'menu' | 'status' | 'priority'; task: TaskRecord }

export function TaskList({
  sections,
  grouped,
  layout = 'list',
  refreshing,
  onRefresh,
  onNew,
  empty,
  header,
}: {
  sections: readonly TaskSection[]
  grouped: boolean
  /** `board`: the groups as columns side by side (the web's board); a timeline shows as a list. */
  layout?: Layout
  refreshing: boolean
  onRefresh: () => void
  /** The + of a group header; `undefined` hides it. */
  onNew?: (section: TaskSection) => void
  empty: { title: string; description: string }
  header?: ReactElement
}) {
  const { colors } = useTheme()
  const { user } = useOrbit()
  const statuses = useStatuses().data ?? []
  const projects = useProjects().data ?? []
  const people = usePeople()
  const writes = useTaskRowWrites()
  const sheet = useSheet<RowSheet>()
  const [collapsed, setCollapsed] = useState<ReadonlySet<string>>(new Set())
  const toggle = (key: string) =>
    setCollapsed((current) => {
      const next = new Set(current)
      if (next.has(key)) next.delete(key)
      else next.add(key)
      return next
    })

  const statusOf = (task: TaskRecord) => statuses.find((status) => status.id === task.status_id)
  /** Done for an open task, back to the first open status for a closed one; `undefined` while statuses load. */
  const doneMove = (task: TaskRecord) => {
    const closed = isClosed(statusOf(task))
    const target = closed ? defaultStatusOf(statuses, task.project_id) : projectStatuses(statuses, task.project_id).find((status) => status.category === 'completed')
    return target ? { closed, statusId: target.id } : undefined
  }
  const setStatus = (task: TaskRecord, statusId: string) => writes.change.mutate({ task, patch: { status_id: statusId } })
  const assigned = (task: TaskRecord) => task.assignee_ids.includes(user.id)

  const data = sections.map((section) => ({ ...section, data: collapsed.has(section.key) ? [] : section.tasks }))
  const current = sheet.sheet?.task
  const menuTitle = current ? `${identifier(current, projects)} ${current.title || 'Untitled'}` : ''
  const sheetTitle = sheet.sheet?.kind === 'status' ? 'Status' : sheet.sheet?.kind === 'priority' ? 'Priority' : menuTitle
  const done = current ? doneMove(current) : undefined
  const { width } = useWindowDimensions()
  const column = Math.min(300, width - 56)

  return (
    <>
      {layout === 'board' && grouped ? (
        sections.length === 0 ? (
          <EmptyState icon="tasks" title={empty.title} description={empty.description} />
        ) : (
          <ScrollView horizontal snapToInterval={column + 8} decelerationRate="fast" contentContainerClassName="gap-2 p-2" showsHorizontalScrollIndicator={false}>
            {sections.map((section) => (
              <View key={section.key} className="rounded-lg bg-muted/50 border border-border overflow-hidden" style={{ width: column }}>
                <GroupHeader section={section} collapsed={false} onAdd={onNew ? () => onNew(section) : undefined} />
                <ScrollView contentContainerClassName="gap-1.5 p-1.5 pb-6" refreshControl={<RefreshControl refreshing={refreshing} onRefresh={onRefresh} tintColor={colors.mutedForeground} />}>
                  {section.tasks.map((task) => (
                    <BoardCard
                      key={task.id}
                      task={task}
                      code={identifier(task, projects)}
                      status={statusOf(task)}
                      assignee={people.find((person) => person.id === task.assignee_ids[0])}
                      onPress={() => router.push(`/tasks/${task.id}`)}
                      onLongPress={() => sheet.open({ kind: 'menu', task })}
                    />
                  ))}
                </ScrollView>
              </View>
            ))}
          </ScrollView>
        )
      ) : (
        <SectionList
          sections={data}
          keyExtractor={(task, index) => `${task.id}:${index}`}
          stickySectionHeadersEnabled={false}
          keyboardShouldPersistTaps="handled"
          contentContainerClassName={sections.length === 0 ? 'grow' : 'pb-6'}
          ListHeaderComponent={header}
          refreshControl={<RefreshControl refreshing={refreshing} onRefresh={onRefresh} tintColor={colors.mutedForeground} />}
          ListEmptyComponent={<EmptyState icon="tasks" title={empty.title} description={empty.description} />}
          renderSectionHeader={({ section }) =>
            grouped ? <GroupHeader section={section} collapsed={collapsed.has(section.key)} onToggle={() => toggle(section.key)} onAdd={onNew ? () => onNew(section) : undefined} /> : null
          }
          renderItem={({ item }) => {
            const move = doneMove(item)
            return (
              <SwipeRow
                left={move ? { label: move.closed ? 'Reopen' : 'Done', icon: move.closed ? 'refresh' : 'check', tone: 'primary', onSwipe: () => setStatus(item, move.statusId) } : undefined}
                right={{ label: 'Delete', icon: 'trash', tone: 'destructive', onSwipe: () => writes.trash.mutate(item) }}
              >
                <TaskRow
                  task={item}
                  status={statusOf(item)}
                  assignee={people.find((person) => person.id === item.assignee_ids[0])}
                  onPress={() => router.push(`/tasks/${item.id}`)}
                  onLongPress={() => sheet.open({ kind: 'menu', task: item })}
                />
              </SwipeRow>
            )
          }}
        />
      )}
      <Sheet visible={sheet.visible} title={sheetTitle} onClose={sheet.close}>
        {current && sheet.sheet?.kind === 'menu' ? (
          <SheetList>
            <SheetRow
              label="Open"
              icon="arrowRight"
              onPress={() => {
                sheet.close()
                router.push(`/tasks/${current.id}`)
              }}
            />
            <SheetRow label="Status" subtitle={statusOf(current)?.name} leading={<StatusIcon status={statusOf(current)} />} onPress={() => sheet.open({ kind: 'status', task: current })} />
            <SheetRow label="Priority" subtitle={PRIORITY_LABEL[priorityOf(current)]} leading={<PriorityIcon priority={current.priority} />} onPress={() => sheet.open({ kind: 'priority', task: current })} />
            <SheetRow
              label={assigned(current) ? 'Unassign me' : 'Assign to me'}
              icon="user"
              onPress={() => {
                sheet.close()
                writes.change.mutate({ task: current, patch: { assignee_ids: assigned(current) ? current.assignee_ids.filter((id) => id !== user.id) : [...current.assignee_ids, user.id] } })
              }}
            />
            {done ? (
              <SheetRow
                label={done.closed ? 'Reopen' : 'Mark as done'}
                icon={done.closed ? 'refresh' : 'check'}
                onPress={() => {
                  sheet.close()
                  setStatus(current, done.statusId)
                }}
              />
            ) : null}
            <Pressable
              accessibilityRole="button"
              accessibilityLabel="Move to trash"
              onPress={() => {
                sheet.close()
                writes.trash.mutate(current)
              }}
              className="min-h-12 flex-row items-center gap-3 px-4 active:bg-muted"
            >
              <Icon name="trash" size={18} color={colors.destructive} />
              <Text className="text-[15px] text-destructive">Move to trash</Text>
            </Pressable>
          </SheetList>
        ) : null}
        {current && sheet.sheet?.kind === 'status' ? (
          <StatusChoices
            statuses={projectStatuses(statuses, current.project_id)}
            value={current.status_id}
            onSelect={(statusId) => {
              sheet.close()
              if (statusId !== current.status_id) setStatus(current, statusId)
            }}
          />
        ) : null}
        {current && sheet.sheet?.kind === 'priority' ? (
          <PriorityChoices
            value={current.priority}
            onSelect={(priority) => {
              sheet.close()
              if (priority !== current.priority) writes.change.mutate({ task: current, patch: { priority } })
            }}
          />
        ) : null}
      </Sheet>
    </>
  )
}

/** A board card: identifier, title, and the status, priority and assignee glyphs. A long press opens the row menu. */
function BoardCard({ task, code, status, assignee, onPress, onLongPress }: { task: TaskRecord; code: string; status: StatusRecord | undefined; assignee?: { id: string; name: string }; onPress: () => void; onLongPress: () => void }) {
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={task.title || 'Untitled'}
      accessibilityHint="Long press for actions"
      onPress={onPress}
      onLongPress={onLongPress}
      className="rounded-md border border-border bg-card p-2.5 gap-1.5 active:bg-muted"
    >
      <Text className="text-[11px] text-muted-foreground">{code}</Text>
      <Text numberOfLines={3} className="font-noto-medium text-[13px]">
        {task.title || 'Untitled'}
      </Text>
      <View className="flex-row items-center gap-1.5">
        <StatusIcon status={status} size={14} />
        <PriorityIcon priority={task.priority} size={14} />
        <View className="flex-1" />
        {assignee ? <Avatar id={assignee.id} name={assignee.name} size={18} /> : null}
      </View>
    </Pressable>
  )
}

/** The web group row: chevron, the group's glyph, name, count, and + on the card surface. */
function GroupHeader({ section, collapsed, onToggle, onAdd }: { section: TaskSection; collapsed: boolean; onToggle?: () => void; onAdd?: () => void }) {
  const { colors } = useTheme()
  const glyph = section.status ? (
    <StatusIcon status={section.status} size={14} />
  ) : section.priority ? (
    <PriorityIcon priority={section.priority} size={14} />
  ) : section.userId ? (
    <Avatar id={section.userId} name={section.label} size={16} />
  ) : section.project ? (
    <ProjectMark project={section.project} />
  ) : section.color ? (
    <View className="size-2 rounded-full" style={{ backgroundColor: section.color }} />
  ) : null
  return (
    <View className="h-9 flex-row items-center pl-1.5 pr-2 bg-card border-b border-border">
      {onToggle ? (
        <Pressable
          accessibilityRole="button"
          accessibilityLabel={collapsed ? `Expand ${section.label}` : `Collapse ${section.label}`}
          onPress={onToggle}
          hitSlop={8}
          className="size-5 rounded-md items-center justify-center bg-muted mr-2"
        >
          <View className={collapsed ? '-rotate-90' : undefined}>
            <Icon name="chevronDown" size={12} color={colors.foreground} />
          </View>
        </Pressable>
      ) : null}
      {glyph ? <View className="mr-2">{glyph}</View> : null}
      <Text numberOfLines={1} className="font-noto-semibold text-[12px] leading-[16px] shrink text-muted-foreground">
        {section.label}
      </Text>
      <Text className="text-[12px] leading-[16px] ml-2 flex-1 text-muted-foreground/70">{section.tasks.length}</Text>
      {onAdd ? <IconButton icon="plus" label={`New task in ${section.label}`} size={24} iconSize={12} color={colors.mutedForeground} onPress={onAdd} /> : null}
    </View>
  )
}
