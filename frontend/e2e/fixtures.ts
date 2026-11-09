import { test as base, expect, type Page } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';

/**
 * CSP listener (ADR-0003 section 3.5, scope 1).
 *
 * `securitypolicyviolation` fires on `document`; it does not bubble, so the
 * listener is registered in an init script before any app script runs. A
 * violation is collected with its directive and blocked URI, and every test
 * fails at the end if the page produced one — a silent workaround (inline
 * style, data: image, @import) must not slip through as a passing smoke test.
 */
export interface CspViolation {
  directive: string;
  blockedURI: string;
  source: string;
}

declare global {
  interface Window {
    __cspViolations: CspViolation[];
  }
}

export const test = base.extend<{ cspViolations: CspViolation[] }>({
  cspViolations: async ({ page }, use) => {
    await page.addInitScript(() => {
      window.__cspViolations = [];
      document.addEventListener('securitypolicyviolation', (event) => {
        window.__cspViolations.push({
          directive: event.effectiveDirective || event.violatedDirective,
          blockedURI: event.blockedURI,
          source: event.sourceFile ? `${event.sourceFile}:${event.lineNumber}` : '',
        });
      });
    });
    const violations: CspViolation[] = [];
    await use(violations);
    const fromPage = await page.evaluate(() => window.__cspViolations ?? []);
    violations.push(...fromPage);
    expect(violations, 'pelanggaran CSP tidak boleh terjadi').toEqual([]);
  },
});

export { expect };

/** Fails the test with the axe violations, listing selector and rule id. */
export async function expectNoAxeViolations(page: Page) {
  const results = await new AxeBuilder({ page })
    .withTags(['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa'])
    .analyze();
  expect(
    results.violations.map((v) => ({
      id: v.id,
      impact: v.impact,
      nodes: v.nodes.map((n) => n.target),
    })),
  ).toEqual([]);
}

export interface StubSesiOpsi {
  /** Peran yang dikembalikan GET /auth/me. */
  peran?: string;
  /** Sisa waktu idle (ms) dihitung dari saat request dijawab; default jauh di luar jendela peringatan. */
  idleMs?: number;
  /** Sisa waktu batas absolut (ms); default 8 jam. */
  absolutMs?: number;
}

/**
 * Stub identitas dan status sesi. AppRoot memanggil `GET /auth/me` dan `GET /auth/sesi` sebelum
 * menampilkan shell, jadi stub catch-all untuk semua path admin yang menjawab dengan data log akan
 * menjatuhkan halaman ke layar login. Panggil SETELAH route catch-all didaftarkan: route yang
 * didaftarkan belakangan dicocokkan lebih dulu.
 */
export async function stubSesi(page: Page, opsi: StubSesiOpsi = {}): Promise<void> {
  const { peran = 'auditor', idleMs = 25 * 60_000, absolutMs = 8 * 3_600_000 } = opsi;
  const json = (body: unknown) => ({
    status: 200,
    headers: { 'content-type': 'application/json', 'cache-control': 'no-store' },
    body: JSON.stringify(body),
  });
  await page.route('**/admin/v1/auth/me', (route) =>
    route.fulfill(
      json({
        pengguna_id: '00000000-0000-4000-8000-000000000001',
        username: 'a',
        peran: [peran],
        mfa: { wajib: false, aktif: false, sisa_kode_pemulihan: 0 },
      }),
    ),
  );
  await page.route('**/admin/v1/auth/sesi', (route) => {
    const sekarang = Date.now();
    return route.fulfill(
      json({
        idle_berakhir_pada: new Date(sekarang + Math.min(idleMs, absolutMs)).toISOString(),
        absolut_berakhir_pada: new Date(sekarang + absolutMs).toISOString(),
        waktu_server: new Date(sekarang).toISOString(),
      }),
    );
  });
}
