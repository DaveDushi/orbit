import { afterEach, expect, test } from 'bun:test'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render, waitFor, within } from '@testing-library/react'
import type { ReactNode } from 'react'
import { MemoryRouter } from 'react-router'
import { WorkspaceContext } from '@/features/workspaces/workspaceContext'
import type { Task, TaskStatusDef, TaskViewState } from '@/features/tasks/api/models'
import { TaskDetail } from './TaskDetail'
import { testWorkspace } from '@/test/workspace'

const originalFetch = globalThis.fetch
afterEach(() => { globalThis.fetch = originalFetch })

const workspace = testWorkspace()
const project = { id: 'project-1', workspace_id: 'workspace-1', name: 'Launch', key: 'ORB', color: '#e0457b', created_at: '', updated_at: '', version: 1, auto_close_parent: true, auto_close_sub_issues: true, private: false }
const todo: TaskStatusDef = { id: 'todo', projectId: 'project-1', name: 'Todo', description: '', color: '#888', category: 'unstarted', position: 0, version: 1 }
const task: Task = {
  id: 'task-3f2a', identifier: 'ORB-3F2A', title: 'Write the launch post', description: '', statusId: 'todo',
  position: 0, priority: 'none', assigneeIds: [], creatorId: 'user-1', projectId: 'project-1', labels: [], attachments: [],
  dueAt: null, createdAt: '', updatedAt: '', comments: [], activity: [], version: 1, duplicateOf: null, blocked: false,
}
const state: TaskViewState = { currentUserId: 'user-1', users: [], statuses: [todo], labels: [], tasks: [task] }
const summary = (id: string, title: string) => ({ id, parent_id: null, teamspace_id: 'team-1', title, icon: null, position: 0, version: 1, updated_at: '' })
const linked = { id: 'page-plan', title: 'Launch plan', icon: null, private: false, linked_at: '2026-11-14T00:00:00Z' }

type Call = { method: string; path: string; body?: unknown }

function api(calls: Call[]) {
  globalThis.fetch = (async (input: RequestInfo | URL) => {
    const request = input as Request
    const path = new URL(request.url).pathname
    const body = request.method === 'POST' ? await request.json() : undefined
    calls.push({ method: request.method, path, body })
    if (path.endsWith('/linked-docs') && request.method === 'GET') return Response.json([linked])
    if (path.endsWith('/linked-docs') && request.method === 'POST') return Response.json({ ...linked, id: 'page-notes', title: 'Notes' }, { status: 201 })
    if (request.method === 'DELETE') return new Response(null, { status: 204 })
    if (path.endsWith('/github-links') || path.endsWith('/relations')) return Response.json([])
    if (path.endsWith('/pages')) return Response.json({ items: [summary('page-plan', 'Launch plan'), summary('page-notes', 'Notes')] })
    if (path.endsWith('/projects')) return Response.json({ items: [project], next_cursor: null })
    return Response.json({ items: [], next_cursor: null })
  }) as unknown as typeof fetch
}
const writes = (calls: Call[]) => calls.filter((call) => call.method !== 'GET')

function Wrapper({ children }: { children: ReactNode }) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } })
  return <QueryClientProvider client={client}><MemoryRouter><WorkspaceContext.Provider value={{ workspace, workspaces: [workspace], selectWorkspace: () => {} }}>{children}</WorkspaceContext.Provider></MemoryRouter></QueryClientProvider>
}

test('Add doc links the picked page and never offers a page the task already links; × unlinks', async () => {
  const calls: Call[] = []
  api(calls)
  const view = render(<TaskDetail task={task} project={project} state={state} onClose={() => {}} />, { wrapper: Wrapper })
  const section = await view.findByRole('region', { name: 'Docs' })
  expect(within(section).getByRole('link', { name: /Launch plan/ }).getAttribute('href')).toBe('/docs/page-plan')

  fireEvent.click(view.getByRole('button', { name: 'Add doc' }))
  const picker = await view.findByRole('dialog', { name: 'Add doc' })
  const option = await within(picker).findByRole('option', { name: /Notes/ })
  expect(within(picker).queryAllByRole('option', { name: /Launch plan/ })).toHaveLength(0)
  fireEvent.click(option)
  await waitFor(() => expect(writes(calls)).toEqual([
    { method: 'POST', path: '/api/v1/workspaces/workspace-1/tasks/task-3f2a/linked-docs', body: { page_id: 'page-notes' } },
  ]))

  fireEvent.click(within(section).getByRole('button', { name: 'Unlink Launch plan' }))
  await waitFor(() => expect(writes(calls).at(-1)).toEqual(
    { method: 'DELETE', path: '/api/v1/workspaces/workspace-1/tasks/task-3f2a/linked-docs/page-plan', body: undefined },
  ))
}, 20000)
