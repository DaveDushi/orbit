import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { openURL } from 'expo-linking'
import { router, useLocalSearchParams } from 'expo-router'
import { type ReactNode, useMemo, useState } from 'react'
import { Alert, KeyboardAvoidingView, Platform, Pressable, RefreshControl, ScrollView, TextInput, View } from 'react-native'
import { SafeAreaView } from 'react-native-safe-area-context'

import { Attachments, type ShownFile } from '../../../components/tasks/Attachments'
import { type FileSource, pickFiles } from '../../../components/tasks/files'
import {
  DueDateChoices,
  LabelChoices,
  LabelPill,
  MenuChoices,
  PeopleChoices,
  PriorityChoices,
  SourceForm,
  StatusChoices,
  TaskChoices,
} from '../../../components/tasks/pickers'
import { DuplicateBanner, RelationsSection, SubIssuesSection } from '../../../components/tasks/sections'
import { Sheet, useSheet } from '../../../components/tasks/Sheet'
import { activityRows, buildFeed, type ActivityRow, type CommentThread, foldRun } from '../../../data/activity'
import { relativeAgo } from '../../../data/format'
import { keys } from '../../../data/keys'
import { markdownToText } from '../../../data/markdown'
import * as api from '../../../data/orbit'
import * as taskApi from '../../../data/task-api'
import {
  attachmentPath,
  bodyWithFiles,
  dueDateLabel,
  identifier,
  isImageType,
  PRIORITY_LABEL,
  priorityOf,
  projectStatuses,
  refIdentifier,
  RELATION_OPTIONS,
  relatedTaskIds,
  type RelationKind,
  relationPickerTitle,
  sourceHost,
  splitInlineFiles,
  subIssueBody,
} from '../../../data/tasks'
import { useAdoptSubIssue, useCommentAttachments, useCreateTask, useRelationWrites, useSubIssues, useTaskAttachments, useTaskRelations, useTaskWrite } from '../../../hooks/tasks'
import { useOrbit } from '../../../orbit-context'
import { useLabels, usePeople, useProjects, useStatuses } from '../../../queries'
import { errorMessage } from '../../../session-context'
import { alpha, type, useTheme } from '../../../theme'
import { Avatar, Button, ErrorView, Icon, type IconName, IconButton, LoadingView, PaneHeader, PriorityIcon, StatusIcon, Text } from '../../../ui'

/** The one sheet of the page and what it shows. */
type TaskSheet =
  | { kind: 'status' | 'priority' | 'assignees' | 'labels' | 'due' | 'source' | 'parent-menu' | 'relation-menu' | 'attach' }
  | { kind: 'pick-task'; purpose: 'parent' | 'sub-issue' | RelationKind }

function sheetTitle(sheet: TaskSheet | null, subject: string): string {
  if (!sheet) return ''
  switch (sheet.kind) {
    case 'status':
      return 'Status'
    case 'priority':
      return 'Priority'
    case 'assignees':
      return 'Assignees'
    case 'labels':
      return 'Labels'
    case 'due':
      return 'Due date'
    case 'source':
      return 'Source'
    case 'parent-menu':
      return 'Parent'
    case 'relation-menu':
      return 'Add relation'
    case 'attach':
      return 'Attach'
    case 'pick-task':
      if (sheet.purpose === 'parent') return `Set parent of ${subject}…`
      if (sheet.purpose === 'sub-issue') return `Add a sub-issue to ${subject}…`
      return relationPickerTitle(sheet.purpose, subject)
  }
}

/** A file of the comment being written: uploading (no `path` yet) or uploaded. */
type DraftFile = ShownFile & { image: boolean }

let draftFileCount = 0

export default function TaskScreen() {
  const { id } = useLocalSearchParams<{ id: string }>()
  const { client, workspaceId, user } = useOrbit()
  const queryClient = useQueryClient()
  const { colors } = useTheme()
  const projects = useProjects()
  const statuses = useStatuses()
  const labels = useLabels()
  const people = usePeople()
  const sheets = useSheet<TaskSheet>()
  const [draft, setDraft] = useState('')
  const [draftFiles, setDraftFiles] = useState<readonly DraftFile[]>([])
  const [unfolded, setUnfolded] = useState<readonly string[]>([])
  const [composingSubIssue, setComposingSubIssue] = useState(false)

  const task = useQuery({
    queryKey: keys.task(workspaceId, id),
    queryFn: () => api.task(client, workspaceId, id),
    // the list row the user tapped, until the full task arrives
    placeholderData: () => queryClient.getQueryData<api.TaskRecord[]>(keys.tasks(workspaceId))?.find((item) => item.id === id),
  })
  const comments = useQuery({ queryKey: keys.comments(workspaceId, id), queryFn: () => api.comments(client, workspaceId, id) })
  const activity = useQuery({ queryKey: keys.taskActivity(workspaceId, id), queryFn: () => api.taskActivity(client, workspaceId, id) })
  const relations = useTaskRelations(id)
  const subIssues = useSubIssues(id)
  const attachments = useTaskAttachments(id)
  const commentFiles = useCommentAttachments(id, comments.data ?? [])
  const write = useTaskWrite(id)
  const relationWrites = useRelationWrites(id)
  const createSubIssue = useCreateTask()
  const adopt = useAdoptSubIssue(id)

  const sendComment = useMutation({
    mutationFn: (body: string) => api.addComment(client, workspaceId, id, body),
    onSuccess: (comment) => {
      setDraft('')
      setDraftFiles([])
      queryClient.setQueryData<api.CommentRecord[]>(keys.comments(workspaceId, id), (list) => (list ? [...list, comment] : [comment]))
    },
    onError: (error) => Alert.alert('Could not add the comment', errorMessage(error)),
  })

  const trash = useMutation({
    mutationFn: (current: api.TaskRecord) => api.trashTask(client, workspaceId, current),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: keys.tasks(workspaceId) })
      close()
    },
    onError: (error) => Alert.alert('Task deletion failed', errorMessage(error)),
  })

  const close = () => (router.canGoBack() ? router.back() : router.navigate('/tasks'))
  const nameOf = (userId: string) => people.find((person) => person.id === userId)?.name ?? 'Unknown'
  const current = task.data
  const project = projects.data?.find((item) => item.id === current?.project_id)

  const feed = useMemo(() => {
    if (!activity.data || !comments.data) return []
    const known = queryClient.getQueryData<api.TaskRecord[]>(keys.tasks(workspaceId)) ?? []
    const rows = activityRows(activity.data, {
      statuses: statuses.data ?? [],
      people,
      labels: labels.data ?? [],
      projects: projects.data ?? [],
      identifierOf: (taskId) => {
        const other = known.find((item) => item.id === taskId)
        return other ? identifier(other, projects.data ?? []) : 'another task'
      },
    })
    return buildFeed(rows, comments.data)
  }, [activity.data, comments.data, labels.data, people, projects.data, queryClient, statuses.data, workspaceId])

  /** Uploads each picked file at once, as the web composer does; the comment links them when it is sent. */
  const attach = async (source: FileSource) => {
    let picked: Awaited<ReturnType<typeof pickFiles>>
    try {
      picked = await pickFiles(source)
    } catch (error) {
      sheets.close()
      return Alert.alert('Could not open the picker', errorMessage(error))
    }
    sheets.close()
    for (const file of picked) {
      const key = `draft-${(draftFileCount += 1)}`
      setDraftFiles((list) => [...list, { key, name: file.name, image: isImageType(file.mimeType) }])
      taskApi.uploadInline(client, workspaceId, id, file).then(
        (record) => setDraftFiles((list) => list.map((item) => (item.key === key ? { ...item, path: attachmentPath(record) } : item))),
        (error: unknown) => {
          setDraftFiles((list) => list.filter((item) => item.key !== key))
          Alert.alert(`Could not upload “${file.name}”`, errorMessage(error))
        },
      )
    }
  }

  const header = (
    <PaneHeader style={{ paddingHorizontal: 12 }}>
      <IconButton icon="back" label="Back to tasks" color={alpha(colors.mutedForeground, 0.7)} onPress={close} />
      <View style={{ flex: 1, flexDirection: 'row', alignItems: 'center', gap: 6, minWidth: 0 }}>
        {project ? (
          <Pressable accessibilityRole="link" onPress={() => router.navigate('/tasks')} style={{ flexDirection: 'row', alignItems: 'center', gap: 6, flexShrink: 1 }}>
            <View style={{ width: 6, height: 6, borderRadius: 3, backgroundColor: project.color }} />
            <Text numberOfLines={1} style={[type(12, 'medium'), { lineHeight: 16, flexShrink: 1 }]}>
              {project.name}
            </Text>
          </Pressable>
        ) : null}
        {project ? <Icon name="chevronRight" size={14} color={colors.mutedForeground} /> : null}
        {(current?.ancestors ?? []).slice(-2).map((ancestor) => (
          <Pressable key={ancestor.id} accessibilityRole="link" accessibilityLabel={`${refIdentifier(ancestor)} ${ancestor.title}`} onPress={() => router.push(`/tasks/${ancestor.id}`)} style={{ flexDirection: 'row', alignItems: 'center', gap: 6 }}>
            <Text style={[type(12), { lineHeight: 16, color: colors.mutedForeground }]}>{refIdentifier(ancestor)}</Text>
            <Icon name="chevronRight" size={14} color={colors.mutedForeground} />
          </Pressable>
        ))}
        <Text style={[type(12), { lineHeight: 16, color: colors.mutedForeground }]}>{current ? identifier(current, projects.data ?? []) : 'Task'}</Text>
      </View>
      {current ? (
        <Button
          label="Delete"
          variant="destructive"
          disabled={trash.isPending}
          onPress={() =>
            Alert.alert('Move to trash?', current.sub_issue_count > 0 ? 'Its sub-issues go to the trash with it. You can restore them from the trash.' : 'You can restore it from the trash.', [
              { text: 'Cancel', style: 'cancel' },
              { text: 'Delete', style: 'destructive', onPress: () => trash.mutate(current) },
            ])
          }
        />
      ) : null}
    </PaneHeader>
  )

  if (!current) {
    return (
      <SafeAreaView edges={['top', 'left', 'right']} style={{ flex: 1, backgroundColor: colors.background }}>
        {header}
        {task.isError ? <ErrorView error={task.error} onRetry={() => void task.refetch()} /> : <LoadingView />}
      </SafeAreaView>
    )
  }

  const subject = identifier(current, projects.data ?? [])
  const status = statuses.data?.find((item) => item.id === current.status_id)
  const ownStatuses = statuses.data ? projectStatuses(statuses.data, current.project_id) : []
  const description = splitInlineFiles(current.description)
  const descriptionText = markdownToText(description.text)
  const taskFiles: ShownFile[] = [
    ...description.files.map((file) => ({ key: file.path, ...file })),
    ...(attachments.data ?? []).map((record) => ({ key: record.id, name: record.display_name, path: attachmentPath(record), image: isImageType(record.media_type), size: record.byte_size })),
  ]
  const taskLabels = (labels.data ?? []).filter((label) => current.label_ids.includes(label.id))
  const children = subIssues.data ?? []
  // a parent is never the task or a loaded sub-issue (deeper ones: the server's parent_cycle); an added sub-issue is
  // never the task, an ancestor or a sub-issue it already has
  const pickExcluded = (purpose: 'parent' | 'sub-issue' | RelationKind) => {
    if (purpose === 'parent') return [current.id, ...children.map((child) => child.id)]
    if (purpose === 'sub-issue') return [current.id, ...(current.ancestors ?? []).map((ref) => ref.id), ...children.map((child) => child.id)]
    if (purpose === 'duplicate') return [current.id]
    return [current.id, ...relatedTaskIds(relations.data ?? [])]
  }
  const choose = (purpose: 'parent' | 'sub-issue' | RelationKind, target: api.TaskRecord) => {
    sheets.close()
    if (purpose === 'parent') write.mutate({ parent_task_id: target.id })
    else if (purpose === 'sub-issue') adopt.mutate(target)
    else if (purpose === 'duplicate') write.mutate({ duplicate_of_id: target.id })
    else relationWrites.add.mutate({ type: purpose, task_id: target.id })
  }
  const filesOf = (comment: api.CommentRecord): ShownFile[] => [
    ...splitInlineFiles(comment.body).files.map((file) => ({ key: file.path, ...file })),
    ...(commentFiles.get(comment.id) ?? []).map((record) => ({ key: record.id, name: record.display_name, path: attachmentPath(record), image: isImageType(record.media_type), size: record.byte_size })),
  ]
  const uploading = draftFiles.some((file) => !file.path)
  const readyFiles = draftFiles.flatMap((file) => (file.path ? [{ name: file.name, path: file.path, image: file.image }] : []))
  const sheet = sheets.sheet

  return (
    <SafeAreaView edges={['top', 'left', 'right']} style={{ flex: 1, backgroundColor: colors.background }}>
      {header}
      <KeyboardAvoidingView style={{ flex: 1 }} behavior={Platform.OS === 'ios' ? 'padding' : undefined}>
        <ScrollView
          contentContainerStyle={{ paddingHorizontal: 14, paddingTop: 16, paddingBottom: 28, gap: 14 }}
          keyboardShouldPersistTaps="handled"
          refreshControl={
            <RefreshControl
              refreshing={task.isRefetching}
              onRefresh={() => void Promise.all([task.refetch(), comments.refetch(), activity.refetch(), relations.refetch(), subIssues.refetch(), attachments.refetch()])}
              tintColor={colors.mutedForeground}
            />
          }
        >
          <View>
            <DuplicateBanner task={current} status={status} pending={write.isPending} onUnmark={() => write.mutate({ duplicate_of_id: null })} />
            <Text selectable style={[type(20, 'semibold'), { lineHeight: 26 }]}>
              {current.title || 'Untitled'}
            </Text>
            {descriptionText ? (
              <Text selectable style={{ ...type(16), lineHeight: 24, marginTop: 19 }}>
                {descriptionText}
              </Text>
            ) : null}
            <Attachments files={taskFiles} />
            <View style={{ flexDirection: 'row', marginTop: descriptionText || taskFiles.length > 0 ? 43 : 14 }}>
              <QuietAction icon="relation" label="Add relation" onPress={() => sheets.open({ kind: 'relation-menu' })} />
              {current.sub_issue_count === 0 && children.length === 0 && !composingSubIssue ? <QuietAction icon="subIssue" label="Add sub-issue" onPress={() => setComposingSubIssue(true)} /> : null}
            </View>
          </View>

          <SubIssuesSection
            tasks={children}
            statuses={statuses.data ?? []}
            projectId={current.project_id}
            composing={composingSubIssue}
            creating={createSubIssue.isPending}
            failed={createSubIssue.isError}
            onCompose={() => setComposingSubIssue(true)}
            onAddExisting={() => sheets.open({ kind: 'pick-task', purpose: 'sub-issue' })}
            onCloseComposer={() => setComposingSubIssue(false)}
            onCreate={(title) => {
              const body = subIssueBody(current, title, { statuses: statuses.data ?? [], userId: user.id, siblings: children })
              if (body) createSubIssue.mutate(body)
            }}
          />
          <RelationsSection
            relations={relations.data ?? []}
            statuses={statuses.data ?? []}
            removingId={relationWrites.remove.isPending ? relationWrites.remove.variables : undefined}
            onRemove={(relationId) => relationWrites.remove.mutate(relationId)}
          />

          <View style={{ borderTopWidth: 1, borderBottomWidth: 1, borderColor: colors.border, paddingVertical: 14, flexDirection: 'row', flexWrap: 'wrap', columnGap: 12, rowGap: 14 }}>
            <PropertyGroup title="Properties" full>
              <View style={{ flexDirection: 'row', flexWrap: 'wrap', gap: 4 }}>
                <Chip pull label={`Status: ${status?.name ?? 'None'}`} onPress={ownStatuses.length > 0 ? () => sheets.open({ kind: 'status' }) : undefined}>
                  <StatusIcon status={status} size={16} />
                  <ChipText>{status?.name ?? 'No status'}</ChipText>
                </Chip>
                <Chip pull label={`Priority: ${PRIORITY_LABEL[priorityOf(current)]}`} onPress={() => sheets.open({ kind: 'priority' })}>
                  <PriorityIcon priority={current.priority} size={16} />
                  <ChipText>{priorityOf(current) === 'none' ? 'Set priority' : PRIORITY_LABEL[priorityOf(current)]}</ChipText>
                </Chip>
                <Chip pull label="Assignees" onPress={() => sheets.open({ kind: 'assignees' })}>
                  {current.assignee_ids.length === 0 ? (
                    <>
                      <Icon name="user" size={14} color={colors.mutedForeground} />
                      <ChipText muted>Assignee</ChipText>
                    </>
                  ) : (
                    <>
                      {current.assignee_ids.slice(0, 2).map((userId) => (
                        <Avatar key={userId} id={userId} name={nameOf(userId)} size={16} />
                      ))}
                      <ChipText>{current.assignee_ids.map(nameOf).join(', ')}</ChipText>
                    </>
                  )}
                </Chip>
              </View>
              <View style={{ flexDirection: 'row', marginTop: 4, marginLeft: -8 }}>
                <Chip
                  label={current.parent ? `Parent: ${refIdentifier(current.parent)} ${current.parent.title}` : 'Set parent'}
                  onPress={() => sheets.open(current.parent ? { kind: 'parent-menu' } : { kind: 'pick-task', purpose: 'parent' })}
                >
                  <Icon name="subIssue" size={14} color={colors.mutedForeground} />
                  {current.parent ? (
                    <>
                      <ChipText muted>{refIdentifier(current.parent)}</ChipText>
                      <ChipText>{current.parent.title || 'Untitled'}</ChipText>
                    </>
                  ) : (
                    <ChipText muted>Set parent</ChipText>
                  )}
                </Chip>
              </View>
            </PropertyGroup>
            <PropertyGroup title="Labels">
              {taskLabels.length === 0 ? (
                <View style={{ marginLeft: -8, flexDirection: 'row' }}>
                  <Chip label="Add label" onPress={() => sheets.open({ kind: 'labels' })}>
                    <Icon name="tag" size={14} color={colors.mutedForeground} />
                    <ChipText muted>Add label</ChipText>
                  </Chip>
                </View>
              ) : (
                <Pressable
                  accessibilityRole="button"
                  accessibilityLabel={`Labels: ${taskLabels.map((label) => label.name).join(', ')}`}
                  onPress={() => sheets.open({ kind: 'labels' })}
                  style={{ flexDirection: 'row', flexWrap: 'wrap', gap: 4 }}
                >
                  {taskLabels.map((label) => (
                    <LabelPill key={label.id} label={label} />
                  ))}
                  <View style={{ width: 20, height: 20, alignItems: 'center', justifyContent: 'center' }}>
                    <Icon name="plus" size={12} color={colors.mutedForeground} />
                  </View>
                </Pressable>
              )}
            </PropertyGroup>
            <PropertyGroup title="Project">{project ? <Pill color={project.color} name={project.name} /> : <ChipText muted>—</ChipText>}</PropertyGroup>
            <PropertyGroup title="Source">
              {current.source_url ? (
                <Pressable accessibilityRole="link" accessibilityLabel={`Open source ${current.source_url}`} onPress={() => void openURL(current.source_url!)} style={{ maxWidth: '100%', flexDirection: 'row', alignItems: 'center', gap: 4, height: 28 }}>
                  <Icon name="arrowUpRightSquare" size={14} color={colors.mutedForeground} />
                  <ChipText>{sourceHost(current.source_url)}</ChipText>
                </Pressable>
              ) : null}
              <View style={{ marginLeft: -8, flexDirection: 'row' }}>
                <Chip label={current.source_url ? 'Edit source' : 'Add source'} onPress={() => sheets.open({ kind: 'source' })}>
                  <ChipText muted>{current.source_url ? 'Edit source' : 'Add source'}</ChipText>
                </Chip>
              </View>
            </PropertyGroup>
            <PropertyGroup title="Due date">
              <View style={{ marginLeft: -8, flexDirection: 'row' }}>
                <Chip label="Due date" onPress={() => sheets.open({ kind: 'due' })}>
                  <Icon name="calendar" size={14} color={colors.foreground} />
                  <ChipText>{dueDateLabel(current.due_at, current.due_start_at)}</ChipText>
                </Chip>
              </View>
            </PropertyGroup>
          </View>

          <View>
            <Text style={[type(14, 'semibold'), { lineHeight: 20, marginBottom: 8 }]}>Activity</Text>
            {activity.isError || comments.isError ? (
              <Pressable onPress={() => void Promise.all([activity.refetch(), comments.refetch()])}>
                <Text style={[type(12), { color: colors.destructive }]}>Could not load the activity. Tap to retry.</Text>
              </Pressable>
            ) : (
              feed.map((entry) =>
                entry.kind === 'thread' ? (
                  <Thread key={entry.thread.root.id} thread={entry.thread} nameOf={nameOf} filesOf={filesOf} />
                ) : (
                  <Timeline
                    key={entry.items[0]!.id}
                    run={entry.items}
                    unfolded={unfolded.includes(entry.items[0]!.id)}
                    onToggle={() => {
                      const runId = entry.items[0]!.id
                      setUnfolded((list) => (list.includes(runId) ? list.filter((item) => item !== runId) : [...list, runId]))
                    }}
                    nameOf={nameOf}
                    statusOf={(statusId) => statuses.data?.find((item) => item.id === statusId)}
                  />
                ),
              )
            )}
            <CommentComposer
              value={draft}
              onChangeText={setDraft}
              files={draftFiles}
              onRemoveFile={(key) => setDraftFiles((list) => list.filter((file) => file.key !== key))}
              canSend={(draft.trim().length > 0 || readyFiles.length > 0) && !uploading && !sendComment.isPending}
              sending={sendComment.isPending}
              onSend={() => sendComment.mutate(bodyWithFiles(draft, readyFiles))}
              onAttach={() => sheets.open({ kind: 'attach' })}
            />
          </View>
        </ScrollView>
      </KeyboardAvoidingView>

      <Sheet visible={sheets.visible} title={sheetTitle(sheet, subject)} onClose={sheets.close}>
        {sheet?.kind === 'status' ? (
          <StatusChoices
            statuses={ownStatuses}
            value={current.status_id}
            onSelect={(statusId) => {
              sheets.close()
              if (statusId !== current.status_id) write.mutate({ status_id: statusId })
            }}
          />
        ) : null}
        {sheet?.kind === 'priority' ? (
          <PriorityChoices
            value={priorityOf(current)}
            onSelect={(priority) => {
              sheets.close()
              if (priority !== priorityOf(current)) write.mutate({ priority })
            }}
          />
        ) : null}
        {sheet?.kind === 'assignees' ? <PeopleChoices people={people} value={current.assignee_ids} onChange={(ids) => write.mutate({ assignee_ids: ids })} /> : null}
        {sheet?.kind === 'labels' ? <LabelChoices projectId={current.project_id} value={current.label_ids} onChange={(ids) => write.mutate({ label_ids: ids })} /> : null}
        {sheet?.kind === 'due' ? (
          <DueDateChoices
            dueAt={current.due_at}
            onChange={(due) => {
              sheets.close()
              write.mutate({ due_at: due?.end ?? null, due_start_at: due?.start ?? null })
            }}
          />
        ) : null}
        {sheet?.kind === 'source' ? (
          <SourceForm
            value={current.source_url}
            onSave={(url) => {
              sheets.close()
              if (url !== (current.source_url ?? null)) write.mutate({ source_url: url })
            }}
          />
        ) : null}
        {sheet?.kind === 'parent-menu' && current.parent ? (
          <MenuChoices
            items={[
              { key: 'open', label: 'Open parent', icon: 'arrowRight' },
              { key: 'change', label: 'Change parent…', icon: 'subIssue' },
              { key: 'remove', label: 'Remove parent', icon: 'close' },
            ]}
            onSelect={(key) => {
              if (key === 'change') return sheets.open({ kind: 'pick-task', purpose: 'parent' })
              sheets.close()
              if (key === 'open') router.push(`/tasks/${current.parent!.id}`)
              else write.mutate({ parent_task_id: null })
            }}
          />
        ) : null}
        {sheet?.kind === 'relation-menu' ? (
          <MenuChoices items={RELATION_OPTIONS.map((option) => ({ key: option.kind, label: option.label }))} onSelect={(kind) => sheets.open({ kind: 'pick-task', purpose: kind })} />
        ) : null}
        {sheet?.kind === 'pick-task' ? (
          <TaskChoices key={sheet.purpose} excludeIds={pickExcluded(sheet.purpose)} excludeDuplicates={sheet.purpose === 'duplicate'} onSelect={(target) => choose(sheet.purpose, target)} />
        ) : null}
        {sheet?.kind === 'attach' ? (
          <MenuChoices
            items={[
              { key: 'photos', label: 'Photo library', icon: 'sticker' },
              { key: 'files', label: 'Choose file', icon: 'attach' },
            ]}
            onSelect={(source) => void attach(source)}
          />
        ) : null}
      </Sheet>
    </SafeAreaView>
  )
}

/** "Add relation" / "Add sub-issue": the web's quiet ghost buttons under the description. */
function QuietAction({ icon, label, onPress }: { icon: IconName; label: string; onPress: () => void }) {
  const { colors } = useTheme()
  const color = alpha(colors.mutedForeground, 0.7)
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={label}
      onPress={onPress}
      style={({ pressed }) => ({ height: 32, flexDirection: 'row', alignItems: 'center', gap: 6, paddingHorizontal: 10, borderRadius: 10, backgroundColor: pressed ? colors.muted : 'transparent' })}
    >
      <Icon name={icon} size={14} color={color} />
      <Text style={[type(12, 'medium'), { lineHeight: 16, color }]}>{label}</Text>
    </Pressable>
  )
}

function PropertyGroup({ title, full = false, children }: { title: string; full?: boolean; children: ReactNode }) {
  const { colors } = useTheme()
  return (
    <View style={{ width: full ? '100%' : '47%', flexGrow: full ? 0 : 1, alignItems: 'flex-start' }}>
      <Text style={[type(12, 'medium'), { lineHeight: 16, marginBottom: full ? 10 : 8, color: alpha(colors.mutedForeground, 0.7) }]}>{title}</Text>
      {children}
    </View>
  )
}

/** The web `PropertyButton` at phone size: a 28px ghost button with 12px text. */
function Chip({ label, onPress, pull = false, children }: { label: string; onPress?: () => void; /** The web's `-ml-2`: each chip of a wrapping row starts 8px left. */ pull?: boolean; children: ReactNode }) {
  const { colors } = useTheme()
  return (
    <Pressable
      accessibilityRole={onPress ? 'button' : undefined}
      accessibilityLabel={label}
      disabled={!onPress}
      onPress={onPress}
      style={({ pressed }) => ({ height: 28, maxWidth: '100%', marginLeft: pull ? -8 : 0, flexDirection: 'row', alignItems: 'center', gap: 6, paddingHorizontal: 6, borderRadius: 10, backgroundColor: pressed ? colors.muted : 'transparent' })}
    >
      {children}
    </Pressable>
  )
}

function ChipText({ children, muted = false }: { children: ReactNode; muted?: boolean }) {
  const { colors } = useTheme()
  return (
    <Text numberOfLines={1} style={[type(12, 'medium'), { lineHeight: 16, flexShrink: 1, color: muted ? colors.mutedForeground : colors.foreground }]}>
      {children}
    </Text>
  )
}

/** A project or label: a round outline pill with its color dot. */
function Pill({ color, name }: { color: string; name: string }) {
  const { colors } = useTheme()
  return (
    <View style={{ height: 20, flexDirection: 'row', alignItems: 'center', gap: 4, paddingHorizontal: 8, borderRadius: 10, borderWidth: 1, borderColor: colors.input, backgroundColor: alpha(colors.input, 0.3) }}>
      <View style={{ width: 6, height: 6, borderRadius: 3, backgroundColor: color }} />
      <Text numberOfLines={1} style={[type(12, 'medium'), { lineHeight: 16 }]}>
        {name}
      </Text>
    </View>
  )
}

/** A run of activity rows on a connector line, folded to its first and latest rows like the web. */
function Timeline({
  run,
  unfolded,
  onToggle,
  nameOf,
  statusOf,
}: {
  run: ActivityRow[]
  unfolded: boolean
  onToggle: () => void
  nameOf: (userId: string) => string
  statusOf: (statusId: string) => api.StatusRecord | undefined
}) {
  const { colors } = useTheme()
  const folded = foldRun(run)
  const items = unfolded ? run : folded.items
  const rows: ReactNode[] = []
  items.forEach((item, index) => {
    const status = item.statusId ? statusOf(item.statusId) : undefined
    const actor = item.actorName ?? (item.actorId ? nameOf(item.actorId) : 'Someone')
    rows.push(
      <TimelineRow
        key={item.id}
        first={index === 0}
        last={index === items.length - 1 && folded.hidden === 0}
        glyph={status ? <StatusIcon status={status} size={14} /> : <Avatar id={item.actorId} name={actor} size={14} />}
      >
        <Text numberOfLines={1} style={[type(12), { lineHeight: 16, flex: 1, color: colors.mutedForeground }]}>
          <Text style={[type(12, 'medium'), { lineHeight: 16, color: colors.foreground }]}>{actor}</Text> {item.text} · {relativeAgo(item.createdAt)}
        </Text>
      </TimelineRow>,
    )
    if (folded.hidden > 0 && index === 0) {
      rows.push(
        <TimelineRow key={`${item.id}-fold`} first={false} last={items.length === 1} glyph={null}>
          <Pressable accessibilityRole="button" accessibilityState={{ expanded: unfolded }} onPress={onToggle} style={{ flexDirection: 'row', alignItems: 'center', gap: 8 }}>
            <View style={{ transform: [{ rotate: unfolded ? '180deg' : '0deg' }] }}>
              <Icon name="chevronDown" size={14} color={colors.mutedForeground} />
            </View>
            <Text style={[type(12, 'medium'), { lineHeight: 16, color: colors.mutedForeground }]}>{unfolded ? 'Show less' : `Show ${folded.hidden} more`}</Text>
          </Pressable>
        </TimelineRow>,
      )
    }
  })
  return <View>{rows}</View>
}

/** One timeline row: the 1px connector runs above and below the 14px glyph, the first and last drop their outer half. */
function TimelineRow({ first, last, glyph, children }: { first: boolean; last: boolean; glyph: ReactNode; children: ReactNode }) {
  const { colors } = useTheme()
  const line = { position: 'absolute' as const, left: 12.5, width: 1, backgroundColor: colors.border }
  return (
    <View style={{ minHeight: 30, flexDirection: 'row', alignItems: 'center', gap: 10, paddingVertical: 7, paddingLeft: 6 }}>
      {first ? null : <View style={[line, { top: 0, height: 7 }]} />}
      {last ? null : <View style={[line, { bottom: 0, height: 7 }]} />}
      <View style={{ width: 14, height: 14, alignItems: 'center', justifyContent: 'center' }}>{glyph}</View>
      {children}
    </View>
  )
}

/** A comment and its replies in one card. */
function Thread({ thread, nameOf, filesOf }: { thread: CommentThread; nameOf: (userId: string) => string; filesOf: (comment: api.CommentRecord) => ShownFile[] }) {
  const { colors } = useTheme()
  return (
    <View style={{ marginVertical: 12, borderRadius: 10, borderWidth: 1, borderColor: colors.border, backgroundColor: colors.card, overflow: 'hidden' }}>
      {[thread.root, ...thread.replies].map((comment, index) => (
        <View key={comment.id} style={{ flexDirection: 'row', gap: 8, paddingHorizontal: 14, paddingVertical: 10, borderTopWidth: index === 0 ? 0 : 1, borderTopColor: colors.border }}>
          <Avatar id={comment.author_id} name={nameOf(comment.author_id)} size={20} />
          <View style={{ flex: 1, minWidth: 0 }}>
            <View style={{ minHeight: 20, flexDirection: 'row', alignItems: 'center', gap: 6 }}>
              <Text numberOfLines={1} style={[type(13, 'semibold'), { flexShrink: 1 }]}>
                {nameOf(comment.author_id)}
              </Text>
              <Text style={{ fontSize: 11, color: alpha(colors.mutedForeground, 0.7) }}>·</Text>
              <Text style={[type(11, 'medium'), { color: colors.mutedForeground }]}>
                {relativeAgo(comment.created_at)}
                {comment.updated_at !== comment.created_at ? ' (edited)' : ''}
              </Text>
            </View>
            <CommentBody comment={comment} files={filesOf(comment)} />
          </View>
        </View>
      ))}
    </View>
  )
}

/** A comment's text, then its files (inline ones linked from the text, then attached ones). */
function CommentBody({ comment, files }: { comment: api.CommentRecord; files: ShownFile[] }) {
  const text = markdownToText(splitInlineFiles(comment.body).text)
  return (
    <>
      {text ? (
        <Text selectable style={[type(13), { lineHeight: 19, marginTop: 2 }]}>
          {text}
        </Text>
      ) : null}
      <Attachments files={files} />
    </>
  )
}

/** The web comment box: a card with the field on top, the files to send, and attach + a round send button below. */
function CommentComposer({
  value,
  onChangeText,
  files,
  onRemoveFile,
  canSend,
  sending,
  onSend,
  onAttach,
}: {
  value: string
  onChangeText: (text: string) => void
  files: readonly ShownFile[]
  onRemoveFile: (key: string) => void
  canSend: boolean
  sending: boolean
  onSend: () => void
  onAttach: () => void
}) {
  const { colors, scheme } = useTheme()
  const [focused, setFocused] = useState(false)
  return (
    <View style={{ marginTop: 12, borderRadius: 10, borderWidth: 1, borderColor: focused ? colors.ring : colors.border, backgroundColor: colors.card }}>
      <TextInput
        value={value}
        onChangeText={onChangeText}
        onFocus={() => setFocused(true)}
        onBlur={() => setFocused(false)}
        placeholder="Leave a comment… Use @ to mention"
        placeholderTextColor={colors.mutedForeground}
        selectionColor={colors.primary}
        keyboardAppearance={scheme}
        multiline
        numberOfLines={1}
        accessibilityLabel="Comment"
        style={{ minHeight: 40, maxHeight: 160, paddingHorizontal: 12, paddingTop: 10, paddingBottom: 0, fontSize: 16, lineHeight: 24, fontFamily: type(16).fontFamily, color: colors.foreground }}
      />
      {files.length > 0 ? (
        <View style={{ paddingHorizontal: 12 }}>
          <Attachments files={files} onRemove={onRemoveFile} />
        </View>
      ) : null}
      <View style={{ flexDirection: 'row', justifyContent: 'flex-end', alignItems: 'center', gap: 4, padding: 6 }}>
        <IconButton icon="attach" label="Attach" color={colors.mutedForeground} onPress={onAttach} />
        <Pressable
          accessibilityRole="button"
          accessibilityLabel={sending ? 'Sending…' : 'Send'}
          accessibilityState={{ disabled: !canSend }}
          disabled={!canSend}
          onPress={onSend}
          style={{ width: 28, height: 28, borderRadius: 14, alignItems: 'center', justifyContent: 'center', backgroundColor: colors.primary, opacity: canSend ? 1 : 0.5 }}
        >
          <Icon name="arrowUp" size={16} color={colors.primaryForeground} />
        </Pressable>
      </View>
    </View>
  )
}
