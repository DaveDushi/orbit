import { expect, test, type Page } from '@playwright/test'
import { testWorkspace } from '../src/test/workspace'

/** Where to save screenshots (e.g. before/after a change); none are taken when unset. */
const shots = process.env.LABEL_OVERFLOW_SHOTS

const names = [
  'GitHub', 'owner:raymond', 'needs-input:raymond', 'ready-for-human', 'wayfinder:task', 'wayfinder:grilling',
  'needs-input:david-d', 'needs-input:david-s', 'ready-for-agent', 'owner:david-s',
  'a-very-long-label-name-that-should-never-push-the-row-wider-than-the-list',
]
const labels = names.map((name, index) => ({ id: `label-${index}`, workspace_id: 'alpha', project_id: 'project-1', name, color: '#5e6ad2', version: 1 }))

const task = (id: number, title: string, labelIds: string[]) => ({
  id: `task-${id}`, workspace_id: 'alpha', project_id: 'project-1', status_id: 'todo', identifier: `SLA-${id}`, number: id,
  title, description: '', priority: 'none', position: id, assignee_ids: [], label_ids: labelIds, creator_id: 'user-1',
  version: 1, blocked: false, duplicate_of: null, collab_epoch: '1', description_json: null,
  created_at: '2026-09-01T12:00:00Z', updated_at: '2026-09-01T12:00:00Z',
})

const tasks = [
  task(1, 'Ship the admin permissions dashboard under confirmed visibility', labels.map((label) => label.id)),
  task(2, 'Short task', ['label-0']),
  task(3, 'Task with one huge label', ['label-10']),
  task(4, 'Task without labels', []),
]

function preference(layout: 'list' | 'board') {
  return {
    page_key: 'all',
    state: {
      filter: { op: 'and', children: [] },
      display: {
        layout, group_by: 'none', sub_group_by: 'none', order_by: 'manual', order_direction: 'asc',
        properties: ['id', 'status', 'assignee', 'priority', 'project', 'due_date', 'labels'], show_completed: 'all', show_empty_groups: false,
      },
    },
    state_error: null,
    updated_at: '2026-09-01T12:00:00Z',
  }
}

async function mockApi(page: Page, layout: 'list' | 'board' = 'list') {
  await page.route('**/api/v1/**', async (route) => {
    const path = new URL(route.request().url()).pathname
    // the shell's favorites sidebar lists saved views on every page
    if (/\/views\/[^/]+\/favorite$|\/view-favorites\/order$/.test(path)) { await route.fulfill({ status: 204 }); return }
    let body: unknown = { items: [], next_cursor: null }
    if (path.endsWith('/views')) body = []
    if (path.endsWith('/setup/status')) body = { complete: true }
    if (path.endsWith('/auth/me')) body = { id: 'user-1', display_name: 'Test User', email: 'test@example.com' }
    if (path === '/api/v1/workspaces') body = [testWorkspace('owner', { id: 'alpha', name: 'Alpha' })]
    if (path.endsWith('/projects')) body = { items: [{ id: 'project-1', name: 'Slate', key: 'SLA', color: '#a78bfa', version: 1 }], next_cursor: null }
    if (path.endsWith('/statuses')) body = { items: [{ id: 'todo', project_id: 'project-1', name: 'Todo', category: 'unstarted', color: '#888888', position: 0, version: 1 }], next_cursor: null }
    if (path.endsWith('/labels')) body = { items: labels, next_cursor: null }
    if (path.endsWith('/tasks') || path.endsWith('/tasks/query')) body = { items: tasks, next_cursor: null }
    if (path.endsWith('/view-preferences/all')) body = preference(layout)
    if (path.endsWith('/tasks/task-1')) body = tasks[0]
    if (path.endsWith('/github-links') || path.endsWith('/relations') || path.endsWith('/followers')) body = []
    await route.fulfill({ json: body })
  })
}

async function shot(page: Page, name: string) {
  if (shots) await page.screenshot({ path: `${shots}/label-overflow-${name}.png` })
}

async function expectAlignedList(page: Page) {
  const rows = page.locator('[data-task-row]')
  await expect(rows).toHaveCount(tasks.length)
  // nothing in the list scrolls sideways
  const scroller = await page.evaluate(() => [...document.querySelectorAll('[data-task-row]')].map((row) => {
    let element: HTMLElement | null = row as HTMLElement
    const overflow: string[] = []
    while (element) {
      if (element.scrollWidth > element.clientWidth + 1 && getComputedStyle(element).overflowX !== 'visible') overflow.push(element.className)
      element = element.parentElement
    }
    return { row: (row as HTMLElement).scrollWidth <= (row as HTMLElement).clientWidth + 1, overflow }
  }))
  for (const row of scroller) expect(row).toEqual({ row: true, overflow: [] })
  // the title keeps room, and the right-hand columns line up in every row
  const title = rows.first().getByText('Ship the admin permissions', { exact: false })
  expect((await title.boundingBox())!.width).toBeGreaterThan(60)
  const dueX = await rows.evaluateAll((items) => items.map((row) => Math.round(row.lastElementChild!.getBoundingClientRect().right)))
  expect(new Set(dueX).size).toBe(1)
  const rowRight = await rows.evaluateAll((items) => items.map((row) => Math.round(row.getBoundingClientRect().right)))
  for (const right of dueX) expect(right).toBeLessThanOrEqual(rowRight[0]!)
}

test('many labels do not widen list rows; +N lists them all', async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 800 })
  await mockApi(page)
  await page.goto('/tasks?workspace=alpha')
  // the first spec pays for Vite's cold start
  await expect(page.locator('[data-task-row]')).toHaveCount(tasks.length, { timeout: 30_000 })
  await shot(page, 'list-wide')
  await expectAlignedList(page)

  const more = page.locator('[data-task-row]').first().getByRole('button', { name: /more labels/ })
  await more.hover()
  const popup = page.locator('[data-slot="popover-content"]')
  for (const name of names) await expect(popup.getByText(name, { exact: true })).toBeVisible()
  await shot(page, 'plus-n-open')
  // a click on the chip does not open the task
  await more.click()
  await expect(page).toHaveURL(/\/tasks\?workspace=alpha$/)
})

// labels hide below 1100px; 1150px is the tightest width that still shows them
for (const width of [800, 1150]) {
  test(`narrow window (${width}px): no sideways scroll, columns line up`, async ({ page }) => {
    await page.setViewportSize({ width, height: 700 })
    await mockApi(page)
    await page.goto('/tasks?workspace=alpha')
    await expect(page.locator('[data-task-row]')).toHaveCount(tasks.length, { timeout: 30_000 })
    await shot(page, `list-narrow-${width}`)
    await expectAlignedList(page)
  })
}

test('board cards show a few labels and +N', async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 800 })
  await mockApi(page, 'board')
  await page.goto('/tasks?workspace=alpha')
  const card = page.locator('article').filter({ hasText: 'Ship the admin permissions' })
  await expect(card).toBeVisible()
  await shot(page, 'board')
  const pills = card.locator('[data-slot="label-pill"]')
  expect(await pills.count()).toBeLessThanOrEqual(3)
  await expect(card.getByRole('button', { name: /more labels/ })).toBeVisible()
  for (const article of await page.locator('article').all()) {
    expect(await article.evaluate((element) => element.scrollWidth <= element.clientWidth + 1)).toBe(true)
  }
})

test('task detail: long labels stay inside the side panel', async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 800 })
  await mockApi(page)
  await page.goto('/tasks/task-1?workspace=alpha')
  const button = page.getByRole('button', { name: /^Labels:/ })
  await expect(button).toBeVisible()
  await shot(page, 'detail')
  const box = (await button.boundingBox())!
  for (const pill of await button.locator('[data-slot="label-pill"]').all()) {
    const pillBox = (await pill.boundingBox())!
    expect(pillBox.x + pillBox.width).toBeLessThanOrEqual(box.x + box.width + 1)
  }
})
