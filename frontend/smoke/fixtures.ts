import {
  test as base,
  expect,
  type APIRequestContext,
  type BrowserContext,
  type TestDetails,
} from '@playwright/test';

/**
 * Fixture `@smoke` (ARC-378, spec KB smoke-test bagian 3, Testing Decisions): sesi per peran dari
 * SQL fixture (`scripts/smoke/fixture.sql`), bukan login UI, kecuali G1 yang menguji login + MFA
 * sungguhan sekali lewat HTTP. Token datang dari env `E2E_SMOKE_TOKEN_<PERAN>` (diisi
 * `scripts/smoke.sh`), tidak pernah dari repo. Header `Origin` disetel di semua request tulis
 * admin (Testing Decisions).
 *
 * Anotasi wajib tiap test: id = operationId/`route:...`/`proc:...` dari inventaris.md — dipakai
 * `scripts/smoke/cakupan.mjs` (cakupan) dan `scripts/smoke/laporan.mjs` (laporan). HARUS lewat
 * detail statis `test(title, details, body)` (`TestDetails.annotation`), BUKAN
 * `test.info().annotations.push(...)` di badan test: `playwright test --list` (dipakai
 * `cakupan.mjs`) tidak pernah menjalankan badan test, jadi anotasi yang ditulis saat runtime tidak
 * pernah muncul di keluaran `--list` (diverifikasi langsung lewat probe `--list` + reporter json).
 * Helper `inventaris(id, peran?)` di bawah mengembalikan `TestDetails` itu untuk argumen kedua
 * `test(...)`; `id` boleh array untuk satu test yang membuktikan beberapa id sekaligus.
 */
export const COOKIE_SESI = '__Host-pr_session';
export const ORIGIN = 'http://127.0.0.1:8080';
/**
 * Downstream API (`/v1/*`, grup klien G3): backend-webserver:8081, dijangkau hanya lewat
 * network compose internal (nginx tidak memproksi `/v1/`, tidak dipublikasikan ke host; spec
 * Testing Decisions). Playwright jalan di netns container frontend (scripts/smoke.sh), jadi nama
 * host compose `backend-webserver` ter-resolve lewat DNS compose-internal.
 */
export const DOWNSTREAM_ORIGIN = 'http://backend-webserver:8081';

export type Peran =
  'platform-admin' | 'penyetuju' | 'operator' | 'auditor' | 'client-owner' | 'admin-auditor';

const ENV_PER_PERAN: Record<Peran, string> = {
  'platform-admin': 'E2E_SMOKE_TOKEN_PLATFORM_ADMIN',
  penyetuju: 'E2E_SMOKE_TOKEN_PENYETUJU',
  operator: 'E2E_SMOKE_TOKEN_OPERATOR',
  auditor: 'E2E_SMOKE_TOKEN_AUDITOR',
  'client-owner': 'E2E_SMOKE_TOKEN_CLIENT_OWNER',
  'admin-auditor': 'E2E_SMOKE_TOKEN_ADMIN_AUDITOR',
};

/** Token sesi mentah dari env; undefined → tes jatuh ke 401, gagal di asersinya (bukan error koneksi). */
export function tokenUntuk(peran: Peran): string | undefined {
  return process.env[ENV_PER_PERAN[peran]];
}

/**
 * Bangun anotasi statis `{type: 'inventaris', description: id}` (satu per id; `id` boleh array
 * untuk satu test yang membuktikan beberapa id sekaligus) dan `{type: 'peran', description: peran}`
 * opsional, untuk argumen `details` test: `test('judul', { annotation: inventaris('opId', 'peran') },
 * async (...) => {...})`.
 */
export function inventaris(id: string | string[], peran?: Peran): TestDetails['annotation'] {
  const anotasi = (Array.isArray(id) ? id : [id]).map((i) => ({
    type: 'inventaris',
    description: i,
  }));
  if (peran) anotasi.push({ type: 'peran', description: peran });
  return anotasi;
}

async function pasangSesi(context: BrowserContext, token: string) {
  await context.addCookies([
    {
      name: COOKIE_SESI,
      value: token,
      domain: new URL(ORIGIN).hostname,
      path: '/',
      httpOnly: true,
      secure: true,
      sameSite: 'Strict',
    },
  ]);
}

export const test = base.extend<{
  peran: Peran | undefined;
  sesiApi: APIRequestContext;
}>({
  peran: [undefined, { option: true }],
  context: async ({ context, peran }, use) => {
    if (peran) {
      const token = tokenUntuk(peran);
      if (token) await pasangSesi(context, token);
    }
    await use(context);
  },
  // Request API murni (tanpa browser) memakai header Origin admin + cookie sesi lewat header
  // Cookie langsung: lebih ringan untuk operasi yang tidak butuh render UI (G2-G6 di S2).
  sesiApi: async ({ playwright, peran }, use) => {
    const token = peran ? tokenUntuk(peran) : undefined;
    const ctx = await playwright.request.newContext({
      baseURL: ORIGIN,
      extraHTTPHeaders: {
        Origin: ORIGIN,
        ...(token ? { Cookie: `${COOKIE_SESI}=${token}` } : {}),
      },
    });
    await use(ctx);
    await ctx.dispose();
  },
});

export { expect };
