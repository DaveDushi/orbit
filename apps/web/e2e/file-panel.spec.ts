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
