import { useMutation, useQueryClient } from '@tanstack/react-query'
import { router } from 'expo-router'
import { useState } from 'react'
import { KeyboardAvoidingView, Platform, View } from 'react-native'

import { keys } from '../../data/keys'
import * as api from '../../data/orbit'
import { defaultStatusOf, projectStatuses } from '../../data/tasks'
import { useOrbit } from '../../orbit-context'
import { useProjects, useStatuses } from '../../queries'
import { errorMessage } from '../../session-context'
import { useTheme } from '../../theme'
import { Button, EmptyState, ErrorView, Input, ListRow, LoadingView, PickerSheet, Screen, StatusIcon, Text } from '../../ui'

export default function NewTaskScreen() {
  const { client, workspaceId } = useOrbit()
  const queryClient = useQueryClient()
  const { colors, radius, spacing } = useTheme()
  const projects = useProjects()
  const statuses = useStatuses()
  const [title, setTitle] = useState('')
  const [projectId, setProjectId] = useState<string | null>(null)
  const [statusId, setStatusId] = useState<string | null>(null)
  const [picker, setPicker] = useState<'project' | 'status' | null>(null)

  const create = useMutation({
    mutationFn: (input: { title: string; projectId: string; statusId: string }) => api.newTask(client, workspaceId, input),
    onSuccess: (task) => {
      queryClient.setQueryData<api.TaskRecord[]>(keys.tasks(workspaceId), (list) => (list ? [...list, task] : list))
      void queryClient.invalidateQueries({ queryKey: keys.tasks(workspaceId) })
      router.replace(`/task/${task.id}`)
    },
  })

  if (!projects.data || !statuses.data) {
    const error = projects.error ?? statuses.error
    return error ? <ErrorView error={error} onRetry={() => void Promise.all([projects.refetch(), statuses.refetch()])} /> : <LoadingView />
  }
  if (projects.data.length === 0) {
    return <EmptyState icon="tasks" title="No projects" description="Create a project in Orbit on the web first." />
  }

  const project = projects.data.find((item) => item.id === projectId) ?? projects.data[0]!
  const own = projectStatuses(statuses.data, project.id)
  const status = own.find((item) => item.id === statusId) ?? defaultStatusOf(statuses.data, project.id)
  const canCreate = title.trim().length > 0 && status !== undefined

  return (
    <KeyboardAvoidingView style={{ flex: 1 }} behavior={Platform.OS === 'ios' ? 'padding' : undefined}>
      <Screen edges={['bottom', 'left', 'right']} scroll>
        <Input label="Title" placeholder="What needs doing?" autoFocus value={title} onChangeText={setTitle} returnKeyType="done" error={create.isError ? errorMessage(create.error) : undefined} />
        <View style={{ borderWidth: 1, borderColor: colors.border, borderRadius: radius.xl, overflow: 'hidden' }}>
          <ListRow title="Project" subtitle={`${project.name} (${project.key})`} onPress={() => setPicker('project')} separator />
          <ListRow title="Status" subtitle={status?.name ?? 'None'} leading={<StatusIcon status={status} />} onPress={() => setPicker('status')} />
        </View>
        <Button
          label="Create task"
          loading={create.isPending}
          disabled={!canCreate}
          onPress={() => status && create.mutate({ title: title.trim(), projectId: project.id, statusId: status.id })}
        />
        <Text variant="caption" color="muted">
          Set the description, assignees and due date in Orbit on the web.
        </Text>
      </Screen>
      <PickerSheet
        visible={picker === 'project'}
        title="Project"
        options={projects.data.map((item) => ({ key: item.id, label: item.name, subtitle: item.key, selected: item.id === project.id }))}
        onSelect={(id) => {
          setPicker(null)
          setProjectId(id)
          // a status belongs to one project: the new project starts at its default
          setStatusId(null)
        }}
        onClose={() => setPicker(null)}
      />
      <PickerSheet
        visible={picker === 'status'}
        title="Status"
        options={own.map((item) => ({ key: item.id, label: item.name, leading: <StatusIcon status={item} />, selected: item.id === status?.id }))}
        onSelect={(id) => {
          setPicker(null)
          setStatusId(id)
        }}
        onClose={() => setPicker(null)}
      />
    </KeyboardAvoidingView>
  )
}
