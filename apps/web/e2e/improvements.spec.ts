import { execFileSync } from 'node:child_process'
import { mkdirSync } from 'node:fs'
import { expect, test, type Page } from '@playwright/test'

// Needs a running server with an admin and a member account, and the `orbit` CLI on the same database (on SQLite the
// CLI needs the server stopped, so use Postgres):
// ORBIT_IMPROVEMENTS_E2E=1 ORBIT_E2E_URL=… ORBIT_E2E_ADMIN=email:password ORBIT_E2E_MEMBER=email:password \
// ORBIT_IMPROVEMENTS_CLI=/path/to/orbit-with-env bunx playwright test -c e2e/real-server.config.ts improvements
const shots = process.env.ORBIT_E2E_SHOTS ?? '/tmp/improvements-shots'
const account = (value: string | undefined) => {
  const [email, ...password] = (value ?? '').split(':')
  return { email, password: password.join(':') }
}

/** `orbit improvements …` on the server's database. Orbit refuses unknown `ORBIT_*` settings, such as this test's. */
function cli(...args: string[]): string {
  const env = Object.fromEntries(Object.entries(process.env).filter(([name]) => !name.startsWith('ORBIT_')))
  return execFileSync(process.env.ORBIT_IMPROVEMENTS_CLI ?? 'orbit', ['improvements', ...args], { encoding: 'utf8', env })
}

interface CliRequest { id: string; status: string; text: string; note: string | null; link: string | null }

async function signIn(page: Page, who: { email: string; password: string }) {
  await page.goto('/login')
  await page.getByLabel('Email').fill(who.email)
  await page.getByLabel('Password', { exact: true }).fill(who.password)
  await page.getByRole('button', { name: 'Sign in' }).click()
  await expect(page).not.toHaveURL(/\/login/)
}

async function shot(page: Page, name: string) {
  // dialogs and menus fade in: wait until no animation runs, or the picture shows a half-drawn frame
  await page.waitForFunction(() => document.getAnimations().every((animation) => animation.playState !== 'running'))
  await page.screenshot({ path: `${shots}/${name}.png` })
}

async function suggest(page: Page, text: string, screenshots = false) {
  await page.getByRole('button', { name: /^Account menu for / }).click()
  if (screenshots) await shot(page, '03-admin-menu')
  await page.getByRole('menuitem', { name: 'Suggest a change' }).click()
  const dialog = page.getByRole('dialog', { name: 'Suggest a change' })
  await expect(dialog.getByRole('button', { name: 'Send' })).toBeDisabled()
  if (screenshots) await shot(page, '04-dialog-empty')
  await dialog.getByLabel('What should change?').fill(text)
  if (screenshots) await shot(page, '05-dialog-filled')
  return dialog
}

test.describe('Suggest a change', () => {
  test.skip(!process.env.ORBIT_IMPROVEMENTS_E2E, 'needs its own server: see the top of this file')
  test.beforeAll(() => mkdirSync(shots, { recursive: true }))

  test('an admin turns it on and sends a request, the agent reports back, a member never sees it', async ({ page, browser }) => {
    const admin = account(process.env.ORBIT_E2E_ADMIN)
    await signIn(page, admin)
    const accountMenu = page.getByRole('button', { name: /^Account menu for / })
    // Start from the default (an earlier run may have left it on); texts carry this run's id.
    const off = await page.request.put('/api/v1/admin/improvements/settings', { data: { enabled: false }, headers: { Origin: new URL(page.url()).origin } })
    expect(off.ok()).toBe(true)
    await page.reload()
    const run = Date.now().toString(36)
    const tagged = (text: string) => `${text} (${run})`
    const rowOf = (text: string) => page.locator('[data-slot="settings-row"]', { hasText: tagged(text) })

    // Off by default: no menu item, no page.
    await accountMenu.click()
    await expect(page.getByRole('menuitem', { name: 'Appearance' })).toBeVisible()
    await expect(page.getByRole('menuitem', { name: 'Suggest a change' })).toHaveCount(0)
    await page.keyboard.press('Escape')
    await page.goto('/admin')
    const toggle = page.getByRole('switch', { name: 'Suggest a change' })
    await expect(toggle).not.toBeChecked()
    await shot(page, '01-admin-switch-off')
    await toggle.click()
    await expect(toggle).toBeChecked()
    await expect(page.getByRole('link', { name: 'Feedback & fixes' })).toBeVisible()
    await shot(page, '02-admin-switch-on')

    // The dialog; the text is untrusted and stays plain text.
    await page.goto('/tasks')
    const text = tagged('Show the due date on the cards of the board view. <b>not bold</b>')
    let dialog = await suggest(page, text, true)
    await dialog.getByRole('button', { name: 'Send' }).click()
    await expect(dialog).toHaveCount(0)
    await expect(page.getByText('Request sent')).toBeVisible()

    // A second one that the admin withdraws, and a third that stays new.
    dialog = await suggest(page, tagged('Make the chat Send button easier to see in dark mode.'))
    await dialog.getByRole('button', { name: 'Send' }).click()
    await expect(dialog).toHaveCount(0)
    dialog = await suggest(page, tagged('Add a keyboard shortcut to archive a task.'))
    await dialog.getByRole('button', { name: 'Send' }).click()
    await expect(dialog).toHaveCount(0)

    await page.goto('/admin/feedback')
    const row = page.locator('[data-slot="settings-row"]', { hasText: run }).filter({ hasText: 'due date on the cards' })
    await expect(row).toContainText(text)
    await expect(row.locator('b')).toHaveCount(0)
    await expect(row).toContainText('New')
    await shot(page, '06-page-new')

    const chat = rowOf('Make the chat Send button easier to see in dark mode.')
    await chat.getByRole('button', { name: 'Withdraw' }).click()
    await page.getByRole('dialog', { name: 'Withdraw this request?' }).getByRole('button', { name: 'Withdraw' }).click()
    await expect(chat).toContainText('Declined')
    await expect(chat).toContainText('Withdrawn')
    await expect(chat.getByRole('button', { name: 'Withdraw' })).toHaveCount(0)

    // The agent picks it up with the CLI; the page shows each status and its note.
    const fresh = JSON.parse(cli('list', '--status', 'new', '--json')) as CliRequest[]
    const id = fresh.find((request) => request.text === text)?.id
    expect(id).toBeTruthy()
    expect(fresh.map((request) => request.text)).not.toContain(tagged('Make the chat Send button easier to see in dark mode.'))
    const steps: [string, string, string, string][] = [
      ['building', 'Building', 'Working on it on a branch.', '07-page-building'],
      ['review', 'In review', 'Built. Screenshots sent to the owner.', '08-page-review'],
      ['live', 'Live', 'Deployed in 7c1d2e9.', '09-page-live'],
    ]
    for (const [status, label, note, name] of steps) {
      const updated = JSON.parse(cli('update', id!, '--status', status, '--note', note, '--link', 'feat/board-due-dates', '--json')) as CliRequest
      expect(updated).toMatchObject({ id, status, note, link: 'feat/board-due-dates' })
      await page.reload()
      await expect(row).toContainText(label)
      await expect(row).toContainText(`Note: ${note}`)
      await expect(row).toContainText('feat/board-due-dates')
      await expect(row.getByRole('button', { name: 'Withdraw' })).toHaveCount(0)
      await shot(page, name)
    }
    // Every status at once: two more requests that the agent has in progress.
    await page.goto('/tasks')
    for (const extra of ['Group the activity list by day.', 'Let me pin a page to the sidebar.']) {
      dialog = await suggest(page, tagged(extra))
      await dialog.getByRole('button', { name: 'Send' }).click()
      await expect(dialog).toHaveCount(0)
    }
    const extras = JSON.parse(cli('list', '--status', 'new', '--json')) as CliRequest[]
    const byText = (start: string) => extras.find((request) => request.text.startsWith(start) && request.text.endsWith(`(${run})`))!.id
    cli('update', byText('Group the activity'), '--status', 'building', '--note', 'Started.')
    cli('update', byText('Let me pin'), '--status', 'review', '--note', 'Ready: see the screenshots.', '--link', 'feat/pinned-pages')
    await page.goto('/admin/feedback')
    for (const label of ['New', 'Building', 'In review', 'Live', 'Declined']) {
      await expect(page.locator('[data-slot="settings-row"]', { hasText: run }).locator('[data-slot="badge"]', { hasText: new RegExp(`^${label}$`) })).toHaveCount(1)
    }
    await shot(page, '10-page-all-statuses')

    // A member: no menu item, no page.
    const memberContext = await browser.newContext({ baseURL: test.info().project.use.baseURL, viewport: { width: 1500, height: 950 } })
    const memberPage = await memberContext.newPage()
    await signIn(memberPage, account(process.env.ORBIT_E2E_MEMBER))
    await memberPage.getByRole('button', { name: /^Account menu for / }).click()
    await expect(memberPage.getByRole('menuitem', { name: 'Appearance' })).toBeVisible()
    await expect(memberPage.getByRole('menuitem', { name: 'Suggest a change' })).toHaveCount(0)
    await shot(memberPage, '11-member-menu')
    await memberPage.keyboard.press('Escape')
    await memberPage.goto('/admin/feedback')
    await expect(memberPage).toHaveURL(/\/tasks/)
    const status = await memberPage.evaluate(async () => (await fetch('/api/v1/admin/improvements')).status)
    expect(status).toBe(403)
    await memberContext.close()
  })

  test('on a phone: send from the drawer, then the switch off hides everything', async ({ browser }) => {
    const context = await browser.newContext({ baseURL: test.info().project.use.baseURL, viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true })
    const page = await context.newPage()
    await signIn(page, account(process.env.ORBIT_E2E_ADMIN))
    const origin = { Origin: new URL(page.url()).origin }
    expect((await page.request.put('/api/v1/admin/improvements/settings', { data: { enabled: true }, headers: origin })).ok()).toBe(true)
    await page.goto('/tasks')
    const run = Date.now().toString(36)
    const text = `Bigger tap targets in the task list. (${run})`

    await page.getByRole('button', { name: 'Menu', exact: true }).click()
    await page.getByRole('button', { name: /^Account menu for / }).click()
    await shot(page, '12-phone-menu')
    await page.getByRole('menuitem', { name: 'Suggest a change' }).click()
    const dialog = page.getByRole('dialog', { name: 'Suggest a change' })
    await dialog.getByLabel('What should change?').fill(text)
    await shot(page, '13-phone-dialog')
    await dialog.getByRole('button', { name: 'Send' }).click()
    await expect(dialog).toHaveCount(0)

    await page.goto('/admin/feedback')
    await expect(page.locator('[data-slot="settings-row"]', { hasText: text })).toContainText('New')
    await expect(page.locator('header nav').first()).toContainText('Feedback & fixes')
    await shot(page, '14-phone-page')

    // Switch off: no menu item, no page, and sending answers 404.
    await page.goto('/admin')
    const toggle = page.getByRole('switch', { name: 'Suggest a change' })
    await toggle.click()
    await expect(toggle).not.toBeChecked()
    await shot(page, '15-phone-switch-off')
    await page.goto('/admin/feedback')
    await expect(page).toHaveURL(/\/admin$/)
    await page.goto('/tasks')
    await page.getByRole('button', { name: 'Menu', exact: true }).click()
    await page.getByRole('button', { name: /^Account menu for / }).click()
    await expect(page.getByRole('menuitem', { name: 'Appearance' })).toBeVisible()
    await expect(page.getByRole('menuitem', { name: 'Suggest a change' })).toHaveCount(0)
    await shot(page, '16-phone-menu-switch-off')
    const send = await page.request.post('/api/v1/admin/improvements', {
      data: { text: 'Off', context: { path: '/tasks', viewport: '390x844', revision: 'development' } },
      headers: origin,
    })
    expect(send.status()).toBe(404)
    // The list stays for the admins.
    expect((await page.request.get('/api/v1/admin/improvements')).status()).toBe(200)
    await context.close()
  })
})
