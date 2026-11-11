import { defineConfig, devices } from '@playwright/test';

/**
 * Proyek `@api-asli` (ARC-251, spec ui-v2-layar AC-4): UI admin terhadap backend sungguhan di
 * stack compose dev, bukan mock. Tidak ada `webServer`: `make e2e-api-asli` di akar repo yang
 * menyalakan stack, migrasi, seed, dan sesi uji, lalu menjalankan file ini di dalam namespace
 * jaringan container frontend compose. Suite mock (`playwright.config.ts`, `e2e/`) tidak berubah.
 */
export default defineConfig({
  testDir: './e2e-api-asli',
  grep: /@api-asli/,
  // Satu pengguna sesi dan satu database bersama: tes berjalan berurutan.
  fullyParallel: false,
  workers: 1,
  forbidOnly: !!process.env.CI,
  retries: 0,
  reporter: 'list',
  use: {
    baseURL: process.env.E2E_BASE_URL ?? 'http://127.0.0.1:8080',
    trace: 'retain-on-failure',
  },
  projects: [{ name: 'api-asli', use: { ...devices['Desktop Chrome'] } }],
});
