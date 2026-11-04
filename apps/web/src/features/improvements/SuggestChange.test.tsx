import { expect, test } from 'bun:test'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { MemoryRouter } from 'react-router'
import { queryKeys } from '@/api/queryKeys'
import { UserMenu } from '@/app/shell/UserMenu'
import { improvementSettingsKey } from './api'

function renderMenu(installationAdmin: boolean, enabled: boolean) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false, staleTime: Infinity } } })
  client.setQueryData(queryKeys.currentUser, { id: 'user-1', email: 'a@orbit.test', display_name: 'Ada', installation_admin: installationAdmin })
  // also for the member: a cached "on" must not show the item to someone who is not an admin
  client.setQueryData(improvementSettingsKey, { enabled, max_text_chars: 4000 })
  const view = render(
    <QueryClientProvider client={client}>
      <MemoryRouter>
        <UserMenu />
      </MemoryRouter>
    </QueryClientProvider>,
  )
  fireEvent.click(view.getByRole('button', { name: 'Account menu for Ada' }))
  return view
}

test('only an installation admin sees Suggest a change, and only while it is on', () => {
  expect(renderMenu(false, true).queryAllByRole('menuitem', { name: 'Suggest a change' })).toHaveLength(0)
})

test('the switch off hides Suggest a change from admins', () => {
  expect(renderMenu(true, false).queryAllByRole('menuitem', { name: 'Suggest a change' })).toHaveLength(0)
})

test('Send stays off until there is text', async () => {
  const view = renderMenu(true, true)
  await userEvent.click(await view.findByRole('menuitem', { name: 'Suggest a change' }))
  const send = await view.findByRole('button', { name: 'Send' })
  expect(send.hasAttribute('disabled')).toBe(true)
  const text = view.getByRole('textbox', { name: 'What should change?' })
  expect(text.getAttribute('maxlength')).toBe('4000')
  await userEvent.type(text, '   ')
  expect(send.hasAttribute('disabled')).toBe(true)
  await userEvent.type(text, 'Pink buttons')
  expect(send.hasAttribute('disabled')).toBe(false)
})
