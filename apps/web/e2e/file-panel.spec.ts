import { expect, test, type Page } from '@playwright/test'

// Runs against a real server (see real-server.config.ts), because the point includes its CSP. The page named
// ORBIT_E2E_PAGE holds the files sample.pdf, sample.xlsx, sample.csv and sample.txt.
const { ORBIT_E2E_URL, ORBIT_E2E_PAGE, ORBIT_E2E_EMAIL, ORBIT_E2E_PASSWORD, ORBIT_E2E_SHOTS } = process.env

test.skip(!ORBIT_E2E_URL, 'needs a real server')

const shot = (page: Page, name: string) => page.screenshot({ path: `${ORBIT_E2E_SHOTS ?? 'test-results'}/${name}.png` })

async function openPage(page: Page) {
  await page.goto('/login')
  await page.getByLabel(/email/i).fill(ORBIT_E2E_EMAIL!)
  await page.getByLabel(/password/i).fill(ORBIT_E2E_PASSWORD!)
  await page.keyboard.press('Enter')
  await page.waitForURL((url) => !url.pathname.startsWith('/login'))
  await page.goto(`/docs/${ORBIT_E2E_PAGE}`)
  await expect(page.locator('.bn-block-content[data-content-type="file"]').first()).toBeVisible()
}

const fileBlock = (page: Page, name: string) => page.locator('.bn-block-content[data-content-type="file"]').filter({ hasText: name }).first()

test('files open beside the page; csv edits save; a stale save overwrites nothing', async ({ page }) => {
  const refused: string[] = []
  page.on('console', (message) => {
    if (/Content Security Policy|unsafe-eval/i.test(message.text())) refused.push(message.text())
  })
  await openPage(page)
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
  await expect(panel.getByRole('button', { name: 'Edit' })).toHaveCount(0)
  await shot(page, '03-xlsx-panel')
  await panel.locator('[data-file-table]').evaluate((element) => element.scrollTo(0, element.scrollHeight))
  await expect(panel.getByText('Item 300', { exact: true })).toBeVisible()
  await panel.getByRole('tab', { name: 'Notes' }).click()
  await expect(panel.getByText('Second sheet', { exact: true })).toBeVisible()
  await shot(page, '04-xlsx-second-sheet')

  // csv: edit and save, then the new version is in the list and the file keeps its delimiter and quotes.
  await fileBlock(page, 'sample.csv').click()
  await expect(panel.getByText('Alpha; Inc', { exact: true })).toBeVisible()
  await panel.getByRole('button', { name: 'Edit' }).click()
  const cell = panel.getByLabel('B2')
  await cell.fill('edited; "value"')
  await shot(page, '05-csv-editing')
  await panel.getByRole('button', { name: 'Save' }).click()
  await expect(panel.getByRole('button', { name: 'Edit' })).toBeVisible()
  await expect(panel.getByText('edited; "value"', { exact: true })).toBeVisible()
  await shot(page, '06-csv-saved')
  const fileUrl = await fileBlock(page, 'sample.csv').getAttribute('data-url')
  const saved = await page.evaluate((url) => fetch(url!).then((response) => response.text()), fileUrl)
  expect(saved).toContain('"edited; ""value"""')
  expect(saved).toContain(';')

  // A save based on an old version: the server refuses, and the panel says so.
  await panel.getByRole('button', { name: 'Edit' }).click()
  await panel.getByLabel('B3').fill('my change')
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
  await shot(page, '07-csv-stale-save')
  await panel.getByRole('button', { name: 'Load their version' }).click()
  await expect(panel.getByLabel('A2')).toHaveValue('someone')
  await panel.getByRole('button', { name: 'Cancel' }).click()

  // Restore the first version.
  await panel.getByRole('button', { name: 'Versions' }).click()
  await expect(page.locator('[data-file-version="1"]')).toBeVisible()
  await shot(page, '08-csv-versions')
  await page.locator('[data-file-version="1"]').getByRole('button', { name: 'Restore' }).click()
  await expect(panel.getByText('Alpha; Inc', { exact: true })).toBeVisible()

  // Text.
  await fileBlock(page, 'sample.txt').click()
  await expect(panel.locator('[data-file-text]')).toContainText('Plain text sample')
  await panel.getByRole('button', { name: 'Edit' }).click()
  await panel.getByLabel('File text').fill('Changed text\n')
  await panel.getByRole('button', { name: 'Save' }).click()
  await expect(panel.locator('[data-file-text]')).toContainText('Changed text')
  await shot(page, '09-text-saved')
  // Back to the sample, so the spec can run again.
  await page.evaluate((url) => fetch(`${url}/versions/1/restore`, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ expected_version: 2 }) }), await fileBlock(page, 'sample.txt').getAttribute('data-url'))

  expect(refused, 'nothing was blocked by the CSP').toEqual([])
})
