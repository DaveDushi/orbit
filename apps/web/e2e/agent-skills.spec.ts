import { expect, test, type Page } from '@playwright/test'

// Needs a server of its own with the agent (AGENT_MOCK=1), a fake GitHub (ORBIT_AGENT_SKILLS_GITHUB_API/RAW and
// ORBIT_AGENT_INTEGRATION_TEST_HOSTS=127.0.0.1) and a member account, so `just e2e` skips it:
// ORBIT_AGENT_SKILLS_E2E=1 ORBIT_SETUP_URL=… ORBIT_E2E_SKILL_ZIPS=<dir with brand-voice.zip and evil-skill.zip>
// ORBIT_E2E_MEMBER_EMAIL=… bun x playwright test agent-skills
const shots = process.env.ORBIT_E2E_SHOTS ?? '/tmp/agent-skills-shots'
const password = process.env.ORBIT_E2E_PASSWORD ?? 'correct horse battery staple'
const zips = process.env.ORBIT_E2E_SKILL_ZIPS ?? '/tmp/agent-skills-e2e/files'

async function setUp(page: Page) {
  const setupUrl = process.env.ORBIT_SETUP_URL
  if (!setupUrl) throw new Error('ORBIT_SETUP_URL must be provided')
  await page.goto(setupUrl)
  await page.getByLabel('Your name').fill('Orbit Owner')
  await page.getByLabel('Email').fill('owner@orbit.test')
  await page.getByLabel('Password', { exact: true }).fill(password)
  await page.getByRole('button', { name: 'Create root account' }).click()
  await page.getByRole('link', { name: 'Create workspace' }).click()
  await page.getByLabel('Workspace name').fill('Foundation')
  await page.getByRole('button', { name: 'Create workspace' }).last().click()
  await page.getByLabel('Project name').fill('Launch')
  await page.getByRole('button', { name: 'Create project' }).click()
  await expect(page).toHaveURL(/\/tasks(?:\?|$)/)
}

/** A JSON API call with the page's session. */
async function api(page: Page, method: string, path: string, body?: unknown) {
  return page.evaluate(async ({ method, path, body }) => {
    const response = await fetch(path, { method, headers: body ? { 'Content-Type': 'application/json' } : {}, body: body ? JSON.stringify(body) : undefined })
    return { status: response.status, body: await response.json().catch(() => null) }
  }, { method, path, body })
}

async function addSkill(page: Page, mode: string) {
  await page.getByRole('button', { name: 'Add skill' }).click()
  await page.getByRole('dialog').getByRole('radio', { name: mode }).or(page.getByRole('dialog').getByRole('button', { name: mode })).first().click()
}

test.describe('agent skills', () => {
  test.skip(!process.env.ORBIT_AGENT_SKILLS_E2E, 'needs its own server: see the top of this file')

  test('owners add skills three ways, turn one on for one agent, and the agent loads it', async ({ page }) => {
    test.setTimeout(180_000)
    await page.setViewportSize({ width: 1280, height: 1100 })
    await setUp(page)
    const workspaces = await api(page, 'GET', '/api/v1/workspaces')
    const workspaceId = (workspaces.body as { id: string }[])[0].id

    // The owner's ChatGPT (the mock model's sign-in) pays for the runs they start.
    await page.goto('/settings/agentic')
    await page.getByRole('button', { name: 'Connect ChatGPT' }).click()
    await expect(page.getByText(/ChatGPT is connected/)).toBeVisible({ timeout: 30_000 })
    const writer = await api(page, 'POST', `/api/v1/workspaces/${workspaceId}/agents`, { name: 'Writer', tools: ['read_page', 'create_page'], all_channels: true })
    expect(writer.status).toBe(200)

    await page.goto('/settings/agentic/skills')
    await expect(page.getByText('No skills yet.')).toBeVisible()
    await page.screenshot({ path: `${shots}/01-skills-admin-page.png` })

    // From a (fake) GitHub, by its skills.sh address.
    await addSkill(page, 'GitHub or skills.sh')
    await page.getByLabel('Address').fill('https://skills.sh/acme/skills/pdf-forms')
    await page.screenshot({ path: `${shots}/02-install-from-github-dialog.png` })
    await page.getByRole('button', { name: 'Install' }).click()
    await expect(page.locator('[data-slot="agent-skill"]').filter({ hasText: 'pdf-forms' })).toBeVisible()
    await page.screenshot({ path: `${shots}/03-installed-from-github.png` })

    // A .zip.
    await addSkill(page, 'Upload .zip')
    await page.getByLabel('.zip file').setInputFiles(`${zips}/brand-voice.zip`)
    await page.getByRole('button', { name: 'Upload', exact: true }).click()
    await expect(page.locator('[data-slot="agent-skill"]').filter({ hasText: 'brand-voice' })).toBeVisible()
    await page.screenshot({ path: `${shots}/04-uploaded-zip.png` })

    // A .zip whose path leaves the skill folder is refused whole.
    await addSkill(page, 'Upload .zip')
    await page.getByLabel('.zip file').setInputFiles(`${zips}/evil-skill.zip`)
    await page.getByRole('button', { name: 'Upload', exact: true }).click()
    await expect(page.getByRole('dialog').getByRole('alert')).toContainText('not allowed')
    await page.screenshot({ path: `${shots}/05-rejected-bad-zip.png` })
    await page.getByRole('button', { name: 'Cancel' }).click()

    // Written here.
    await addSkill(page, 'Write')
    await page.getByLabel('Name').fill('weekly-report')
    await page.getByLabel('Description').fill('Write the weekly status report. Use when someone asks for the weekly report.')
    await page.getByLabel('Instructions').fill('# Weekly report\n\n1. Search the tasks done this week.\n2. Group them by project.\n3. End with the risks.')
    await page.screenshot({ path: `${shots}/06-write-skill-dialog.png` })
    await page.getByRole('button', { name: 'Add skill' }).last().click()
    const weekly = page.locator('[data-slot="agent-skill"]').filter({ hasText: 'weekly-report' })
    await expect(weekly).toBeVisible()

    // On for the default agent only.
    await weekly.getByRole('checkbox', { name: 'Orbit Agent' }).click()
    await expect(weekly.getByRole('checkbox', { name: 'Orbit Agent' })).toBeChecked()
    await expect(weekly.getByRole('checkbox', { name: 'Writer' })).not.toBeChecked()
    await page.waitForTimeout(500)
    await page.screenshot({ path: `${shots}/07-enabled-for-one-agent.png`, fullPage: true })

    // The agent sees only names and descriptions, and loads the skill when asked.
    const status = await api(page, 'GET', '/api/v1/agent')
    const bot = (status.body as { bot_user_id: string }).bot_user_id
    const conversations = await api(page, 'GET', `/api/v1/workspaces/${workspaceId}/chat/conversations`)
    const general = (conversations.body as { id: string; is_default: boolean }[]).find((conversation) => conversation.is_default)!
    const sent = await api(page, 'POST', `/api/v1/workspaces/${workspaceId}/chat/conversations/${general.id}/messages`, { body: `<@${bot}> use skill weekly-report`, nonce: `skills-${Date.now()}` })
    expect(sent.status).toBe(200)
    await page.goto(`/chat/${general.id}`)
    await page.getByText(/1 repl/).first().click({ timeout: 60_000 })
    await expect(page.getByText(/Done \(load_skill\)/).first()).toBeVisible({ timeout: 60_000 })
    await page.waitForTimeout(500)
    await page.screenshot({ path: `${shots}/08-agent-loads-skill-in-chat.png` })

    // In a new thread, the system prompt (the mock model shows it) has the skill's name and description, not its body.
    const asked = await api(page, 'POST', `/api/v1/workspaces/${workspaceId}/chat/conversations/${general.id}/messages`, { body: `<@${bot}> show prompt`, nonce: `prompt-${Date.now()}` })
    const root = (asked.body as { result: { id: string } }).result.id
    let prompt = ''
    await expect(async () => {
      const thread = await api(page, 'GET', `/api/v1/workspaces/${workspaceId}/chat/threads/${root}`)
      prompt = ((thread.body as { items: { body: string }[] }).items ?? []).map((item) => item.body).join('\n')
      expect(prompt).toContain('Skills:')
    }).toPass({ timeout: 60_000 })
    expect(prompt).toContain('- weekly-report: Write the weekly status report.')
    expect(prompt).not.toContain('Group them by project')
    expect(prompt).not.toContain('pdf-forms')

  })

  // Second run, after the member account exists (ORBIT_E2E_MEMBER_EMAIL, a member of the workspace).
  test('a member sees no admin controls, and the API refuses them', async ({ page }) => {
    const member = process.env.ORBIT_E2E_MEMBER_EMAIL
    test.skip(!member, 'needs ORBIT_E2E_MEMBER_EMAIL')
    await page.setViewportSize({ width: 1280, height: 900 })
    await page.goto('/login')
    await page.getByLabel('Email').fill(member as string)
    await page.getByLabel('Password', { exact: true }).fill(password)
    await page.getByRole('button', { name: /Sign in/ }).click()
    await expect(page).not.toHaveURL(/\/login/)
    await page.goto('/settings/agentic/skills')
    await expect(page.getByText(/Only owners and admins can see and manage/)).toBeVisible()
    expect(await page.getByRole('button', { name: 'Add skill' }).count()).toBe(0)
    const workspaces = await api(page, 'GET', '/api/v1/workspaces')
    const workspaceId = (workspaces.body as { id: string }[])[0].id
    expect((await api(page, 'GET', `/api/v1/workspaces/${workspaceId}/agent-skills`)).status).toBe(403)
    await page.screenshot({ path: `${shots}/09-member-no-admin-controls.png` })
  })
})
