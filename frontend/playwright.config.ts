import { defineConfig, devices } from '@playwright/test';

/**
 * E2E (ADR-0003 section 3.5, scope 1): the built SPA served by `vite preview`.
 *
 * `make e2e` runs this inside the pinned Playwright image, so the browser build
 * matches the image tag. Locally (`pnpm exec playwright test`) the web server is
 * started by Playwright itself; in CI/container the same command is used, so
 * there is one definition of the "build is served" contract.
 */
const PORT = Number(process.env.E2E_PORT ?? 4173);

export default defineConfig({
  testDir: './e2e',
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: 0,
  reporter: process.env.CI ? [['list'], ['html', { open: 'never' }]] : 'list',
  use: {
    baseURL: `http://127.0.0.1:${PORT}`,
    trace: 'retain-on-failure',
  },
  projects: [{ name: 'chromium', use: { ...devices['Desktop Chrome'] } }],
  webServer: {
    // The pinned local binary, so the same command works in the Playwright
    // container (no package manager there) and on the host.
    command: `./node_modules/.bin/vite preview --host 127.0.0.1 --port ${PORT} --strictPort`,
    url: `http://127.0.0.1:${PORT}`,
    reuseExistingServer: !process.env.CI,
    timeout: 60_000,
  },
});
