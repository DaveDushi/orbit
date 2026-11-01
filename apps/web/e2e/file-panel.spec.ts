import { expect, test, type Page } from '@playwright/test'

// Runs against a real server (see real-server.config.ts), because the point includes its CSP. The page named
// ORBIT_E2E_PAGE holds the files sample.pdf, sample.xlsx, sample.csv and sample.txt.
const { ORBIT_E2E_URL, ORBIT_E2E_PAGE, ORBIT_E2E_LOCKED_PAGE, ORBIT_E2E_EMAIL, ORBIT_E2E_PASSWORD, ORBIT_E2E_VIEWER_EMAIL, ORBIT_E2E_VIEWER_PASSWORD, ORBIT_E2E_SHOTS } = process.env

test.skip(!ORBIT_E2E_URL, 'needs a real server')

const shot = (page: Page, name: string) => page.screenshot({ path: `${ORBIT_E2E_SHOTS ?? 'test-results'}/${name}.png` })

async function openPage(page: Page, pageId = ORBIT_E2E_PAGE, email = ORBIT_E2E_EMAIL, password = ORBIT_E2E_PASSWORD) {
  await page.goto('/login')
  await page.getByLabel(/email/i).fill(email!)
  await page.getByLabel(/password/i).first().fill(password!)
  await page.keyboard.press('Enter')
  await page.waitForURL((url) => !url.pathname.startsWith('/login'))
  await page.goto(`/docs/${pageId}`)
  await expect(page.locator('.bn-block-content[data-content-type="file"]').first()).toBeVisible()
}

/** Puts the sample back as it was uploaded, so the spec can run again. */
async function restoreSample(page: Page, name: string) {
  const url = await fileBlock(page, name).getAttribute('data-url')
  await page.evaluate(async (file) => {
    const { file: current } = await fetch(`${file}/content`).then((response) => response.json())
    if (current.version === 1) return
    await fetch(`${file}/versions/1/restore`, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ expected_version: current.version }) })
  }, url)
}

const fileBlock = (page: Page, name: string) => page.locator('.bn-block-content[data-content-type="file"]').filter({ hasText: name }).first()

test('files open beside the page; csv edits save; a stale save overwrites nothing', async ({ page }) => {
  const refused: string[] = []
  page.on('console', (message) => {
    // The editor's inline styles are refused on every page; code must not be.
    if (/Content Security Policy/i.test(message.text()) && /script-src|worker-src|connect-src|font-src|eval/i.test(message.text())) refused.push(message.text())
  })
  await openPage(page)
  for (const name of ['sample.xlsx', 'sample.csv', 'sample.txt']) await restoreSample(page, name)
  await shot(page, '01-page-with-files')

  // PDF: real pages on a canvas that has drawn something.
  await fileBlock(page, 'sample.pdf').click()
  const panel = page.locator('[data-file-panel]')
  await expect(panel.getByText('sample.pdf')).toBeVisible()
  const firstPage = panel.locator('[data-pdf-page="1"]')
  await expect(firstPage).toBeVisible()
  await expect
    .poll(() =>
      firstPage.evaluate((canvas: HTMLCanvasElement) => {
        const pixels = canvas.getContext('2d')?.getImageData(0, 0, canvas.width, canvas.height).data
        return pixels ? pixels.some((value, index) => index % 4 !== 3 && value < 128) : false
      }),
    )
    .toBe(true)
  await expect(panel.locator('[data-pdf-page="2"]')).toBeAttached()
  await shot(page, '02-pdf-panel')

  // xlsx: a table with sheet tabs that loads more when scrolled, and no way to edit.
  await fileBlock(page, 'sample.xlsx').click()
  await expect(panel.getByRole('tab', { name: 'Budget' })).toBeVisible()
  await expect(panel.getByText('Item 1', { exact: true })).toBeVisible()
  await expect(panel.locator('[data-formula-notice]')).toContainText('Formula results shown here can be old')
  await shot(page, '03-xlsx-panel-with-notice')
  // Scroll to the end again each time a part arrives.
  await expect(async () => {
    await panel.locator('[data-file-table]').evaluate((element) => element.scrollTo(0, element.scrollHeight))
    await expect(panel.getByText('Item 300', { exact: true })).toBeVisible({ timeout: 500 })
  }).toPass()
  await panel.getByRole('tab', { name: 'Notes' }).click()
  await expect(panel.getByText('Second sheet', { exact: true })).toBeVisible()
  await shot(page, '04-xlsx-second-sheet')
  await panel.getByRole('tab', { name: 'Budget' }).click()
  await expect(panel.getByText('Item 1', { exact: true })).toBeVisible()

  // xlsx edit: a formula cell stays read-only, a cell changes, and the rest of the file is what it was.
  await panel.getByRole('button', { name: 'Edit' }).click()
  await expect(panel.locator('[data-read-only-cell="D2"]')).toBeVisible()
  await expect(panel.getByLabel('D2', { exact: true })).toHaveCount(0)
  await panel.getByLabel('B2', { exact: true }).fill('999')
  await shot(page, '05-xlsx-editing-formula-read-only')
  await panel.getByRole('button', { name: 'Save' }).click()
  await expect(panel.getByRole('button', { name: 'Edit' })).toBeVisible()
  await expect(panel.getByText('999', { exact: true })).toBeVisible()
  await expect(panel.locator('[data-formula-notice]')).toBeVisible()
  await shot(page, '06-xlsx-saved-with-notice')
  await restoreSample(page, 'sample.xlsx')
  await page.reload()
  await expect(fileBlock(page, 'sample.csv')).toBeVisible()

  // csv: edit and save, then the new version is in the list and the file keeps its delimiter and quotes.
  await fileBlock(page, 'sample.csv').click()
  await expect(panel.getByText('Alpha; Inc', { exact: true })).toBeVisible()
  await panel.getByRole('button', { name: 'Edit' }).click()
  const cell = panel.getByLabel('B2', { exact: true })
  await cell.fill('edited; "value"')
  await shot(page, '07-csv-editing')
  await panel.getByRole('button', { name: 'Save' }).click()
  await expect(panel.getByRole('button', { name: 'Edit' })).toBeVisible()
  await expect(panel.getByText('edited; "value"', { exact: true })).toBeVisible()
  await shot(page, '08-csv-saved')
  const fileUrl = await fileBlock(page, 'sample.csv').getAttribute('data-url')
  const saved = await page.evaluate((url) => fetch(url!).then((response) => response.text()), fileUrl)
  expect(saved).toContain('"edited; ""value"""')
  expect(saved).toContain(';')

  // A save based on an old version: the server refuses, and the panel says so.
  await panel.getByRole('button', { name: 'Edit' }).click()
  await panel.getByLabel('B3', { exact: true }).fill('my change')
  const content = `${fileUrl}/content`
  const current = await page.evaluate((url) => fetch(url).then((response) => response.json()), content)
  const changed = await page.evaluate(
    ([url, version]) =>
      fetch(url as string, { method: 'PUT', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ expected_version: version, text: 'name;note\nsomeone;else\n' }) }).then((response) => response.status),
    [content, current.file.version],
  )
  expect(changed).toBe(200)
  await panel.getByRole('button', { name: 'Save' }).click()
  await expect(panel.getByText('Someone changed this file')).toBeVisible()
  await shot(page, '09-csv-stale-save')
  await panel.getByRole('button', { name: 'Load their version' }).click()
  await expect(panel.getByLabel('A2', { exact: true })).toHaveValue('someone')
  await panel.getByRole('button', { name: 'Cancel' }).click()

  // Restore the first version.
  await panel.getByRole('button', { name: 'Versions' }).click()
  await expect(page.locator('[data-file-version="1"]')).toBeVisible()
  await shot(page, '10-csv-versions')
  await page.locator('[data-file-version="1"]').getByRole('button', { name: 'Restore' }).click()
  await expect(panel.getByText('Alpha; Inc', { exact: true })).toBeVisible()

  // Text.
  await fileBlock(page, 'sample.txt').click()
  await expect(panel.locator('[data-file-text]')).toContainText('Plain text sample')
  await panel.getByRole('button', { name: 'Edit' }).click()
  await panel.getByLabel('File text').fill('Changed text\n')
  await panel.getByRole('button', { name: 'Save' }).click()
  await expect(panel.locator('[data-file-text]')).toContainText('Changed text')
  await shot(page, '11-text-saved')
  await restoreSample(page, 'sample.txt')

  expect(refused, 'no code or font was blocked by the CSP').toEqual([])
})

test('on a locked page a member reads files and cannot edit them', async ({ page }) => {
  await openPage(page, ORBIT_E2E_LOCKED_PAGE, ORBIT_E2E_VIEWER_EMAIL, ORBIT_E2E_VIEWER_PASSWORD)
  const panel = page.locator('[data-file-panel]')
  await fileBlock(page, 'sample.xlsx').click()
  await expect(panel.getByText('Item 1', { exact: true })).toBeVisible()
  await expect(panel.getByRole('button', { name: 'Edit' })).toHaveCount(0)
  await fileBlock(page, 'sample.csv').click()
  await expect(panel.getByText('Alpha; Inc', { exact: true })).toBeVisible()
  await expect(panel.getByRole('button', { name: 'Edit' })).toHaveCount(0)
  await shot(page, '12-locked-page-read-only')
})

test('the divider moves over the full row, keeps edits and scroll, and the size stays', async ({ page }) => {
  await openPage(page)
  await restoreSample(page, 'sample.csv')
  await page.evaluate(() => localStorage.removeItem('orbit:file_panel_share'))
  await page.reload()

  const panel = page.locator('[data-file-panel]')
  const pageColumn = page.locator('[data-print-root]')
  const row = page.locator('[data-file-split]')
  const divider = page.getByRole('separator', { name: 'Resize file panel' })
  const left = async (target = divider) => (await target.boundingBox())!.x
  const dragTo = async (x: number, steps = 8) => {
    const box = (await divider.boundingBox())!
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2)
    await page.mouse.down()
    await page.mouse.move(x, box.y + box.height / 2, { steps })
    await page.mouse.up()
  }
  const rowBox = async () => (await row.boundingBox())!

  // Default: the width from before the divider.
  await fileBlock(page, 'sample.csv').click()
  await expect(panel.getByText('Alpha; Inc', { exact: true })).toBeVisible()
  const startRow = await rowBox()
  expect(Math.abs((await panel.boundingBox())!.width - Math.min(startRow.width * 0.52, 760))).toBeLessThan(2)
  await shot(page, '20-resize-default')

  // An unsaved edit and the page's scroll survive both hidden states.
  await panel.getByRole('button', { name: 'Edit' }).click()
  await panel.getByLabel('B2', { exact: true }).fill('unsaved edit')
  await pageColumn.evaluate((element) => element.scrollTo(0, 100))
  const scrolled = await pageColumn.evaluate((element) => element.scrollTop)
  expect(scrolled).toBeGreaterThan(0)

  // File full width, and no text was selected on the way over the page.
  await dragTo(startRow.x + 100)
  await expect(pageColumn).toBeHidden()
  expect((await panel.boundingBox())!.width).toBeGreaterThan(startRow.width - 12)
  expect(await left()).toBe(startRow.x)
  expect(await page.evaluate(() => window.getSelection()?.toString())).toBe('')
  await expect(panel.getByLabel('B2', { exact: true })).toHaveValue('unsaved edit')
  await shot(page, '21-resize-file-full-width')

  // Page full width: the panel stays mounted and hidden, and only the divider stays at the right edge.
  await dragTo(startRow.x + startRow.width - 50)
  await expect(pageColumn).toBeVisible()
  await expect(panel).toBeHidden()
  await expect(panel).toBeAttached()
  await expect(divider).toBeVisible()
  const edge = (await divider.boundingBox())!
  expect(Math.abs(edge.x + edge.width - (startRow.x + startRow.width))).toBeLessThan(1)
  expect((await pageColumn.boundingBox())!.width).toBeGreaterThan(startRow.width - 12)
  expect(await pageColumn.evaluate((element) => element.scrollTop)).toBe(scrolled)
  await shot(page, '22-resize-page-full-width')

  // Back to the middle: the edit and the scroll are still there.
  await dragTo(startRow.x + startRow.width / 2)
  await expect(panel.getByLabel('B2', { exact: true })).toHaveValue('unsaved edit')
  expect(await pageColumn.evaluate((element) => element.scrollTop)).toBe(scrolled)
  expect(Math.abs((await left()) - (startRow.x + startRow.width / 2))).toBeLessThan(10)

  // A size that hides the file is not used to open one. The file is the same one: its edit stays.
  await dragTo(startRow.x + startRow.width - 50)
  await pageColumn.evaluate((element) => element.scrollTo(0, 0))
  await fileBlock(page, 'sample.csv').click()
  await expect(panel.getByLabel('B2', { exact: true })).toHaveValue('unsaved edit')
  expect(Math.abs((await panel.boundingBox())!.width - Math.min(startRow.width * 0.52, 760))).toBeLessThan(2)

  // The full screen button: an unsaved edit stays, and a second click goes back.
  await dragTo(startRow.x + 400)
  const beforeFull = await left()
  await panel.getByRole('button', { name: 'Full screen', exact: true }).click()
  await expect(pageColumn).toBeHidden()
  expect((await panel.boundingBox())!.width).toBeGreaterThan(startRow.width - 12)
  await expect(panel.getByLabel('B2', { exact: true })).toHaveValue('unsaved edit')
  await shot(page, '27-full-screen-button')
  await panel.getByRole('button', { name: 'Exit full screen' }).click()
  await expect(pageColumn).toBeVisible()
  expect(Math.abs((await left()) - beforeFull)).toBeLessThan(2)
  await expect(panel.getByLabel('B2', { exact: true })).toHaveValue('unsaved edit')
  // After a drag to full width the button shows the same state, and goes to the default size.
  await dragTo(startRow.x)
  await panel.getByRole('button', { name: 'Exit full screen' }).click()
  expect(Math.abs((await panel.boundingBox())!.width - Math.min(startRow.width * 0.52, 760))).toBeLessThan(2)

  // Keys, then a double click for the default.
  const before = await left()
  await divider.focus()
  await page.keyboard.press('ArrowLeft')
  expect(Math.abs((await left()) - (before - 16))).toBeLessThan(2)
  await divider.dblclick()
  expect(Math.abs((await left()) - before)).toBeLessThan(2)

  await panel.getByRole('button', { name: 'Cancel' }).click()

  // The size stays after a reload, and a close keeps it for the next open.
  const wide = startRow.x + 300
  await dragTo(wide)
  expect(Math.abs((await left()) - wide)).toBeLessThan(10)
  await page.reload()
  await fileBlock(page, 'sample.csv').click()
  await expect(panel.getByText('Alpha; Inc', { exact: true })).toBeVisible()
  expect(Math.abs((await left()) - wide)).toBeLessThan(10)
  await panel.getByRole('button', { name: 'Close file' }).click()
  await expect(panel).toHaveCount(0)
  await expect(divider).toHaveCount(0)
  await fileBlock(page, 'sample.csv').click()
  expect(Math.abs((await left()) - wide)).toBeLessThan(10)

  // A reload in full screen stays there, and the second click still goes back to the size from before.
  await panel.getByRole('button', { name: 'Full screen', exact: true }).click()
  await page.reload()
  await fileBlock(page, 'sample.csv').click()
  await expect(pageColumn).toBeHidden()
  await panel.getByRole('button', { name: 'Exit full screen' }).click()
  expect(Math.abs((await left()) - wide)).toBeLessThan(10)

  // Saved full width: a close brings the page back, and a reopen shows the file full width.
  await dragTo(startRow.x)
  await panel.getByRole('button', { name: 'Close file' }).click()
  await expect(pageColumn).toBeVisible()
  await fileBlock(page, 'sample.csv').click()
  await expect(pageColumn).toBeHidden()

  // Print with the page hidden on screen: the page prints, the panel and the divider do not.
  await page.evaluate(() => document.documentElement.setAttribute('data-print-doc', ''))
  await page.emulateMedia({ media: 'print' })
  await expect(pageColumn).toBeVisible()
  await expect(panel).toBeHidden()
  await expect(divider).toBeHidden()
  await page.emulateMedia({ media: 'screen' })
  await page.evaluate(() => document.documentElement.removeAttribute('data-print-doc'))

  // A saved hidden file after a reload opens at the default size.
  await dragTo(startRow.x + startRow.width / 2)
  await dragTo(startRow.x + startRow.width - 50)
  await page.reload()
  await fileBlock(page, 'sample.pdf').click()
  await expect(panel).toBeVisible()
  expect(Math.abs((await panel.boundingBox())!.width - Math.min(startRow.width * 0.52, 760))).toBeLessThan(2)

  // PDF: one jump of the pointer lands on the canvas. Capture keeps the drag, and the canvas does not take it.
  const canvas = panel.locator('[data-pdf-page="1"]')
  await expect(canvas).toBeVisible()
  const canvasBox = (await canvas.boundingBox())!
  const box = (await divider.boundingBox())!
  const y = canvasBox.y + 100
  await page.mouse.move(box.x + box.width / 2, y)
  await page.mouse.down()
  const overCanvas = canvasBox.x + 120
  await page.mouse.move(overCanvas, y)
  expect(Math.abs((await left()) - overCanvas)).toBeLessThan(10)
  await page.mouse.move(overCanvas + 60, y)
  expect(Math.abs((await left()) - (overCanvas + 60))).toBeLessThan(10)
  await page.mouse.up()
  // The redraw waits for the rest, and then fits the new width.
  await expect.poll(() => canvas.evaluate((element: HTMLCanvasElement) => Math.abs(element.width - element.clientWidth * devicePixelRatio) < 4)).toBe(true)
  await shot(page, '23-resize-pdf-after-drag')

  // A workbook in a wider split shows more columns, and its scroll stays through a hide.
  await fileBlock(page, 'sample.xlsx').click()
  await expect(panel.getByText('Item 1', { exact: true })).toBeVisible()
  await dragTo(startRow.x + 280)
  await shot(page, '24-resize-workbook-wider')
  const table = panel.locator('[data-file-table]')
  await table.evaluate((element) => element.scrollTo(0, 200))
  const tableTop = await table.evaluate((element) => element.scrollTop)
  await dragTo(startRow.x + startRow.width - 50)
  await expect(panel).toBeHidden()
  await dragTo(startRow.x + 280)
  expect(await table.evaluate((element) => element.scrollTop)).toBe(tableTop)

  // Comments in the same row: a full-width file does not push them out of view.
  await page.evaluate(() => localStorage.setItem('orbit:file_panel_share', '0.5'))
  await page.reload()
  await fileBlock(page, 'sample.csv').click()
  // The comments button closes the file; a link to a thread opens comments beside it.
  await page.evaluate(() => {
    history.pushState(history.state, '', '?thread=none')
    dispatchEvent(new PopStateEvent('popstate', { state: history.state }))
  })
  const comments = page.locator('[data-comments-panel]')
  await expect(comments).toBeVisible()
  await dragTo(0)
  await expect(pageColumn).toBeHidden()
  const commentsBox = (await comments.boundingBox())!
  expect(commentsBox.x + commentsBox.width).toBeLessThanOrEqual(1500)
  expect(commentsBox.width).toBe(340)
  await expect(panel).toBeVisible()
  await shot(page, '25-resize-with-comments')
  await comments.getByRole('button', { name: 'Close comments' }).click()

  // Below 1100px: the overlay from before, with no divider, and the page is not hidden by a saved full width.
  await page.setViewportSize({ width: 1000, height: 800 })
  await expect(divider).toBeHidden()
  await expect(panel.getByRole('button', { name: /full screen/i })).toBeHidden()
  await expect(pageColumn).toBeVisible()
  await expect.poll(async () => (await panel.boundingBox())!.width).toBe(640)
  await shot(page, '26-resize-narrow-overlay')
})
