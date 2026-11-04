import { defineConfig, devices } from '@playwright/test'

/** For specs that need a real server with its CSP: `ORBIT_E2E_URL=… bunx playwright test -c e2e/real-server.config.ts`. */
export default defineConfig({
  testDir: '.',
  testMatch: ['file-panel.spec.ts', 'improvements.spec.ts'],
  timeout: 120_000,
  use: {
    baseURL: process.env.ORBIT_E2E_URL,
    trace: 'retain-on-failure',
    launchOptions: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE ? { executablePath: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE } : undefined,
  },
  projects: [{ name: 'chromium', use: { ...devices['Desktop Chrome'], viewport: { width: 1500, height: 950 } } }],
})
