import type { QueryClient } from '@tanstack/react-query'

import { keys } from './data/keys'
import type { TaskRecord } from './data/orbit'
import { replaceTask } from './data/tasks'

const holds = (data: unknown, taskId: string): data is TaskRecord[] =>
  Array.isArray(data) && data.some((item: Partial<TaskRecord> | null) => item?.id === taskId && item.version !== undefined)

/**
 * Shows `task` in its detail and in every task list that holds it: All tasks, the views and the Home cards (all under
 * the `tasks` prefix). A write puts its optimistic task here, and the previous one back when the server refuses it.
 */
export function putTask(queryClient: QueryClient, workspaceId: string, task: TaskRecord) {
  queryClient.setQueryData(keys.task(workspaceId, task.id), task)
  queryClient.setQueriesData<unknown>({ queryKey: keys.tasks(workspaceId) }, (data: unknown) => (holds(data, task.id) ? replaceTask(data, task) : data))
}
