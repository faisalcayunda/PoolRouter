import { defineConfig, devices } from '@playwright/test';

/**
 * Proyek `smoke` (ARC-378/ARC-376, spec smoke-test bagian 3 dan 7): satu smoke per operasi
 * OpenAPI, rute `routes.tsx`, dan proses, terhadap stack compose berlabel sungguhan (bukan
 * mock). `scripts/smoke.sh` menyalakan stack lebih dulu; tidak ada `webServer` di sini, sama
 * alasannya dengan `playwright.api-asli.config.ts`.
 *
 * Project `ui` (frontend/smoke/ui/**, milik S3/ARC-379) bergantung pada project `api`
 * (frontend/smoke/api/**, S2/ARC-378): objek G2 yang dibuat lewat API dipakai rute G2-G5
 * sampai R, satu worker, gelombang lewat urutan test di tiap file (spec Testing Decisions).
 * Project `ui` boleh kosong (direktorinya belum ada) sebelum S3 merge; Playwright tidak
 * menggagalkan project tanpa file test.
 */
export default defineConfig({
  grep: /@smoke/,
  fullyParallel: false,
  workers: 1,
  forbidOnly: !!process.env.CI,
  retries: 0,
  reporter: process.env.PLAYWRIGHT_JSON_OUTPUT_NAME ? [['json'], ['list']] : 'list',
  use: {
    baseURL: process.env.E2E_BASE_URL ?? 'http://127.0.0.1:8080',
    trace: 'retain-on-failure',
  },
  projects: [
    { name: 'api', testDir: './smoke/api', use: { ...devices['Desktop Chrome'] } },
    {
      name: 'ui',
      testDir: './smoke/ui',
      dependencies: ['api'],
      use: { ...devices['Desktop Chrome'] },
    },
  ],
});
