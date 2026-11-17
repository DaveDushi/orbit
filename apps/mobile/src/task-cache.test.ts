import { expect, test } from 'bun:test'
import { QueryClient } from '@tanstack/react-query'

import type { TaskRecord } from './data/orbit'
import { putTask } from './task-cache'

test('a task write shows in every task list, and its rollback puts the old task back everywhere', () => {
  const queryClient = new QueryClient()
  const open = { id: 't1', version: 1, status_id: 'todo' } as TaskRecord
  const other = { id: 't2', version: 1, status_id: 'todo' } as TaskRecord
  queryClient.setQueryData(['task', 'w', 't1'], open)
  queryClient.setQueryData(['tasks', 'w'], [open, other])
  queryClient.setQueryData(['tasks', 'w', 'home-week'], [open])
  queryClient.setQueryData(['tasks', 'w', 'mine'], [other])
  queryClient.setQueryData(['tasks', 'w', 'search', 'x'], { not: 'a list' })

  putTask(queryClient, 'w', { ...open, status_id: 'done' })
  expect(queryClient.getQueryData<TaskRecord>(['task', 'w', 't1'])?.status_id).toBe('done')
  expect(queryClient.getQueryData<TaskRecord[]>(['tasks', 'w'])?.map((task) => task.status_id)).toEqual(['done', 'todo'])
  expect(queryClient.getQueryData<TaskRecord[]>(['tasks', 'w', 'home-week'])?.[0]?.status_id).toBe('done')
  expect(queryClient.getQueryData<TaskRecord[]>(['tasks', 'w', 'mine'])).toEqual([other])

  putTask(queryClient, 'w', open)
  expect(queryClient.getQueryData<TaskRecord[]>(['tasks', 'w', 'home-week'])).toEqual([open])
  expect(queryClient.getQueryData(['tasks', 'w', 'search', 'x'])).toEqual({ not: 'a list' })
})
