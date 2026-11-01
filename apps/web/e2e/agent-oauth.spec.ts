import { createServer, type Server } from 'node:http'
import type { AddressInfo } from 'node:net'
import { expect, test, type Page } from '@playwright/test'

// Needs a server of its own with ORBIT_APP_KEY and ORBIT_AGENT_INTEGRATION_TEST_HOSTS=127.0.0.1, so `just e2e` skips it:
// ORBIT_AGENT_OAUTH_E2E=1 ORBIT_SETUP_URL=… bun x playwright test agent-oauth
const shots = process.env.ORBIT_E2E_SHOTS ?? '/tmp/agent-oauth-shots'
const password = process.env.ORBIT_E2E_PASSWORD ?? 'correct horse battery staple'

/** A local token server and API. Only the client `good-client` gets a token. */
function fakeOAuth() {
  return createServer((request, response) => {
    let body = ''
    request.on('data', (chunk) => { body += chunk })
    request.on('end', () => {
      const send = (status: number, value: unknown) => {
        response.writeHead(status, { 'Content-Type': 'application/json' })
        response.end(JSON.stringify(value))
      }
      if (request.url === '/admin/oauth/access_token') {
        if (new URLSearchParams(body).get('client_id') === 'good-client') send(200, { access_token: 'fake-token', scope: 'read_products', expires_in: 86399 })
        else send(400, { error: 'shop_not_permitted', error_description: 'Client credentials cannot be performed on this shop.' })
      } else if (request.headers['x-shopify-access-token'] === 'fake-token') send(200, { shop: { name: 'Demo' } })
      else send(401, {})
    })
  })
}

async function signIn(page: Page) {
  const setupUrl = process.env.ORBIT_SETUP_URL
  if (!setupUrl) throw new Error('ORBIT_SETUP_URL must be provided')
  await page.goto(setupUrl)
  await page.getByLabel('Your name').fill('Orbit Owner')
  await page.getByLabel('Email').fill('owner@orbit.test')
  await page.getByLabel('Password', { exact: true }).fill(password)
  await page.getByRole('button', { name: 'Create root account' }).click()
  // The root user lands in Admin, with no workspace yet.
  await page.getByRole('link', { name: 'Create workspace' }).click()
  await page.getByLabel('Workspace name').fill('Foundation')
  await page.getByRole('button', { name: 'Create workspace' }).last().click()
  await page.getByLabel('Project name').fill('Launch')
  await page.getByRole('button', { name: 'Create project' }).click()
  await expect(page).toHaveURL(/\/tasks(?:\?|$)/)
}

test.describe('agent tools: OAuth client credentials', () => {
  test.skip(!process.env.ORBIT_AGENT_OAUTH_E2E, 'needs its own server: see the top of this file')
  let fake: Server | undefined
  let origin: string

  test.beforeAll(async () => {
    // A fake server that outlives the test keeps the saved integration usable on a test instance.
    if (process.env.ORBIT_E2E_FAKE_ORIGIN) { origin = process.env.ORBIT_E2E_FAKE_ORIGIN; return }
    fake = fakeOAuth()
    await new Promise<void>((resolve) => fake.listen(0, '127.0.0.1', resolve))
    origin = `http://127.0.0.1:${(fake.address() as AddressInfo).port}`
  })
  test.afterAll(() => { fake?.close() })

  test('setup form, Shopify preset, save and Test', async ({ page }) => {
    await page.setViewportSize({ width: 1280, height: 1500 })
    await signIn(page)
    await page.goto('/settings/agent/tools')
    await page.getByRole('button', { name: 'Add tool' }).click()

    await page.getByRole('combobox', { name: 'Kind' }).click()
    await page.getByRole('option', { name: 'HTTP API' }).click()
    await page.getByRole('combobox', { name: 'Credential' }).click()
    await page.getByRole('option', { name: 'OAuth client credentials' }).click()
    await expect(page.getByLabel('Token URL')).toHaveValue('')
    await page.screenshot({ path: `${shots}/1-empty-oauth-form.png` })

    const baseUrl = page.getByLabel(/Base URL/)
    await page.getByLabel(/Name/).fill('Shopify')
    await baseUrl.fill('https://demo-store.myshopify.com/admin/api/2026-10')
    await expect(page.getByLabel('Token URL')).toHaveValue('https://demo-store.myshopify.com/admin/oauth/access_token')
    await expect(page.getByLabel('Token header')).toHaveValue('X-Shopify-Access-Token')
    await expect(page.getByLabel('Test path')).toHaveValue('shop.json')
    await expect(page.getByLabel('GraphQL path')).toHaveValue('/graphql.json')
    await page.screenshot({ path: `${shots}/2-shopify-preset.png` })

    // The preset leaves with the Shopify URL. The saved integration talks to the fake server.
    await baseUrl.fill(`${origin}/admin/api/2026-10`)
    await expect(page.getByLabel('Token URL')).toHaveValue('')
    await page.getByLabel('Token URL').fill(`${origin}/admin/oauth/access_token`)
    await page.locator('summary', { hasText: 'Advanced' }).click()
    await page.getByLabel('Token header').fill('X-Shopify-Access-Token')
    await page.getByLabel('Test path').fill('shop.json')
    await expect(page.getByLabel('GraphQL path')).toHaveValue('')
    await page.getByLabel('GraphQL path').fill('/graphql.json')
    await page.getByLabel(/Client ID/).fill('good-client')
    await page.getByLabel(/Client secret/).fill('good-secret')
    await page.getByRole('button', { name: 'Add', exact: true }).click()

    const card = page.locator('[data-slot="integration-row"]').filter({ hasText: 'Shopify' })
    const edit = () => page.getByRole('button', { name: 'Edit Shopify' }).click()
    await edit()
    await expect(page.getByText('OAuth client credentials, token in X-Shopify-Access-Token')).toBeVisible()
    await page.getByText('OAuth client credentials, token in X-Shopify-Access-Token').scrollIntoViewIfNeeded()
    await page.screenshot({ path: `${shots}/3-saved-integration.png` })
    await expect(page.locator('body')).not.toContainText('good-secret')

    await card.getByRole('button', { name: 'Test', exact: true }).click()
    await expect(page.getByText('The token request worked. GET of shop.json answered HTTP 200.')).toBeVisible()
    await page.waitForTimeout(600) // the toast's animation
    await page.screenshot({ path: `${shots}/4-test-success.png` })

    await page.getByLabel('Replace client ID').fill('other-client')
    await page.getByLabel('Replace client secret').fill('other-secret')
    await page.getByRole('button', { name: 'Save client' }).click()
    await expect(page.getByText('Client replaced')).toBeVisible()
    await page.reload() // no older toasts over the result
    await edit()
    await card.getByRole('button', { name: 'Test', exact: true }).click()
    await expect(page.getByText(/HTTP 400 \(shop_not_permitted\)/)).toBeVisible()
    await expect(page.locator('body')).not.toContainText('cannot be performed')
    await page.waitForTimeout(600) // the toast's animation
    await page.screenshot({ path: `${shots}/5-test-failure.png` })

    // The preset fills in the path only. The switch stays off until the admin turns it on.
    const graphqlReads = page.getByRole('switch', { name: 'Run GraphQL reads without approval' })
    await expect(graphqlReads).not.toBeChecked()
    await page.reload()
    await edit()
    await expect(graphqlReads).not.toBeChecked()
    await page.screenshot({ path: `${shots}/6-graphql-reads-off.png` })
    await graphqlReads.click()
    await expect(graphqlReads).toBeChecked()
    await page.getByRole('checkbox', { name: 'POST' }).click()
    await expect(page.getByRole('checkbox', { name: 'POST' })).toBeChecked()
    await page.reload()
    await edit()
    await expect(graphqlReads).toBeChecked()
    await expect(page.getByRole('checkbox', { name: 'POST' })).toBeChecked()
    await page.screenshot({ path: `${shots}/7-graphql-reads-on.png` })

    // Ends with the client that works, for a look at a test instance afterwards.
    await page.getByLabel('Replace client ID').fill('good-client')
    await page.getByLabel('Replace client secret').fill('good-secret')
    await page.getByRole('button', { name: 'Save client' }).click()
    await expect(page.getByText('Client replaced')).toBeVisible()
  })
})
