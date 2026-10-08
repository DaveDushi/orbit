import { afterEach, expect, test } from 'bun:test'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render } from '@testing-library/react'
import { registerConfirmationHandler } from '@/components/common/confirmAction'
import { AdminBackupsPage } from './AdminBackupsPage'

const originalFetch = globalThis.fetch
afterEach(() => { globalThis.fetch = originalFetch })

test('creating a backup refreshes the list with a download link', async () => {
  const stored: object[] = []
  globalThis.fetch = (async (input: RequestInfo | URL) => {
    const request = input as Request
    if (request.method === 'POST') {
      stored.push({ id: 'backup-1', kind: 'snapshot', created_at: Date.UTC(2026, 8, 28, 10), byte_size: 2048, file_count: 3, schema_version: 33, application_version: '0.1.0' })
      return Response.json({ id: 'backup-1' }, { status: 201 })
    }
    return Response.json({ items: stored })
  }) as unknown as typeof fetch
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  const view = render(<QueryClientProvider client={client}><AdminBackupsPage /></QueryClientProvider>)
  await view.findByText('No backups yet.')
  fireEvent.click(view.getByRole('button', { name: 'Create backup now' }))
  const link = await view.findByRole('link', { name: /Download backup from/ })
  expect(link.getAttribute('href')).toBe('/api/v1/admin/backups/backup-1/download')
  expect(view.getByText('2.0 KB · 3 files · Orbit 0.1.0')).toBeTruthy()
})

test('deleting a backup asks first, then removes it from the list', async () => {
  let stored: object[] = [{ id: 'backup-1', kind: 'snapshot', created_at: Date.UTC(2026, 8, 28, 10), byte_size: 2048, file_count: 3, schema_version: 33, application_version: '0.1.0' }]
  const deleted: string[] = []
  globalThis.fetch = (async (input: RequestInfo | URL) => {
    const request = input as Request
    if (request.method === 'DELETE') {
      deleted.push(new URL(request.url).pathname)
      stored = []
      return new Response(null, { status: 204 })
    }
    return Response.json({ items: stored })
  }) as unknown as typeof fetch
  const unregister = registerConfirmationHandler(async () => true)
  try {
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
    const view = render(<QueryClientProvider client={client}><AdminBackupsPage /></QueryClientProvider>)
    fireEvent.click(await view.findByRole('button', { name: /Delete backup from/ }))
    await view.findByText('No backups yet.')
    expect(deleted).toEqual(['/api/v1/admin/backups/backup-1'])
  } finally {
    unregister()
  }
})
