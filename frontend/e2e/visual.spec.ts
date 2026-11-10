import { expect, stubSesi, test } from './fixtures';
import type { Page } from '@playwright/test';

/**
 * Visual evidence at five widths (handoff spec section 4 / AC 1).
 *
 * Widths: 1440 (expanded), 1024 (drawer modal sejak UI v2), 768 (medium), 375 (compact), 320 (compact). Every
 * screenshot lands in `e2e/screenshots/<layar>@<lebar>.png`; the file name is the contract, so a
 * reviewer can open the folder and check a screen at a glance.
 *
 * The role is pinned with `?peran=` — the seam exists only in a build made for this suite
 * (`vite build --mode playwright`), which is what `make e2e` builds.
 *
 * The CSS custom property check after each shot is deliberate: a screenshot of an unstyled page is
 * worthless evidence, and its `text-align` (`start`) cannot collide with anything sensible.
 */
const WIDTHS = [1440, 1024, 768, 375, 320] as const;

const LOG_PAGE = {
  baris: [
    {
      kode_transaksi: 'TRX-2026-0042',
      client_id: 'c1',
      client_nama: 'Dukcapil Demo',
      queue_id: 'a3f1c2d4-0000-4000-8000-000000000001',
      nik: '3175********0001',
      status_akses: 'ok',
      requested_at: '2026-11-03T07:30:00.000Z',
    },
    {
      kode_transaksi: 'TRX-2026-0043',
      client_id: 'c1',
      client_nama: 'Dukcapil Demo',
      queue_id: 'a3f1c2d4-0000-4000-8000-000000000002',
      nik: '3175********0002',
      status_akses: 'timeout',
      requested_at: '2026-11-03T07:31:00.000Z',
    },
  ],
  kursor: null,
  di_luar_retensi_online: false,
};

const PAYLOAD = {
  request_id: '0b6f2a1e-0000-4000-8000-000000000000',
  kode_transaksi: 'TRX-2026-0042',
  status_akses: 'ok',
  requested_at: '2026-11-03T07:30:00.000Z',
  input: { nik: '3175000000000001' },
  metadata: { client_id: 'c1' },
  hasil: { status: 'sesuai' },
};

const AUDIT_PAGE = {
  baris: [
    {
      audit_id: 'a1',
      dilakukan_pada: '2026-11-03T07:30:00.000Z',
      pengguna_id: '8f1c2d4e-0000-4000-8000-000000000001',
      aksi: 'lihat_payload',
      hasil: 'berhasil',
      alasan: 'Cek aduan W-2026-07',
    },
  ],
  kursor: null,
};

const QUOTA = [
  {
    upstream_pool_id: '9c1f2a3b-0000-4000-8000-000000000001',
    kuota_harian: 5000,
    kuota_efektif: 4800,
    sisa_periode: 3200,
    terpakai_periode: 1600,
    terpakai_jam_berjalan: 120,
    terpakai_sync_jam_berjalan: 40,
    porsi_sync_maks: 200,
    rate_limit_per_jam: 300,
    periode_mulai: '2026-11-03T00:00:00.000Z',
    periode_selesai: '2026-11-03T23:59:59.000Z',
    dihitung_pada: '2026-11-03T07:30:00.000Z',
  },
];

async function stub(page: Page, logPage: unknown = LOG_PAGE) {
  await page.route('**/admin/v1/**', async (route) => {
    const url = route.request().url();
    // R-16: the payload answer must state `no-store`; the client rejects one that omits it.
    const json = (body: unknown, headers: Record<string, string> = {}) =>
      route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify(body),
        headers,
      });

    if (url.includes('/payload')) return json(PAYLOAD, { 'cache-control': 'no-store' });
    if (url.includes('/audit')) return json(AUDIT_PAGE);
    if (url.includes('/kuota')) return json(QUOTA);
    if (url.includes('/upstream')) return json([]);
    return json(logPage);
  });
  await stubSesi(page);
}

/** Assert the stylesheet is live, then freeze the animations so the shot is deterministic. */
async function settle(page: Page) {
  // No inline <style>: the production CSP forbids it, and the screenshot call disables animations.
  const align = await page.evaluate(() =>
    getComputedStyle(document.documentElement).getPropertyValue('--md-sys-color-primary'),
  );
  expect(align.trim(), 'stylesheet M3 tidak termuat').not.toBe('');
  await page.evaluate(() => document.fonts.ready);
}

async function shoot(page: Page, screen: string, width: number) {
  await page.setViewportSize({ width, height: 900 });
  await settle(page);
  // Section 4: no width scrolls the page itself. A table that needs more room scrolls inside its
  // own region; the document staying at the viewport is what keeps "Diminta pada" and the payload
  // action on screen at 1024.
  const overflow = await page.evaluate(
    () => document.documentElement.scrollWidth - document.documentElement.clientWidth,
  );
  expect(overflow, `dokumen melebar ${overflow}px pada ${width}px`).toBeLessThanOrEqual(0);
  // Medan rentang tanggal hanya boleh membungkus pada lebar compact; di 1024 ke atas satu baris 56 px
  // dan seluruh isinya (termasuk tombol kalender 48 px) berada di dalam batas medan.
  if (width >= 1024) {
    const medan = await page.evaluate(() =>
      Array.from(document.querySelectorAll('[data-testid="date-range-field"]')).map((el) => {
        const g = el.getBoundingClientRect();
        const t = el.querySelector('button')?.getBoundingClientRect();
        return {
          tinggi: Math.round(g.height),
          tombolKanan: t ? t.right : Infinity,
          grupKanan: g.right,
          luap: el.scrollWidth - el.clientWidth,
        };
      }),
    );
    if (screen.startsWith('log-') || screen === 'audit-akses') {
      expect(medan.length, `tidak ada medan rentang pada ${screen}@${width}`).toBeGreaterThan(0);
    }
    for (const m of medan) {
      expect(m.tinggi, `tinggi medan rentang pada ${width}px`).toBe(56);
      expect(m.tombolKanan, `tombol kalender keluar medan pada ${width}px`).toBeLessThanOrEqual(
        m.grupKanan,
      );
      expect(m.luap, `isi medan rentang meluap ${m.luap}px pada ${width}px`).toBeLessThanOrEqual(0);
    }
  }
  await page.screenshot({
    path: `e2e/screenshots/${screen}@${width}.png`,
    fullPage: true,
    animations: 'disabled',
  });
}

for (const width of WIDTHS) {
  test.describe(`bukti visual @${width}`, () => {
    test.describe.configure({ mode: 'serial' });

    test(`S11 log transaksi (operator) @${width}`, async ({ page }) => {
      await stub(page);
      await page.setViewportSize({ width, height: 900 });
      await page.goto('/?peran=operator');
      await expect(page.getByRole('heading', { level: 1 })).toBeVisible();
      await expect(page.getByRole('link', { name: 'TRX-2026-0042' })).toBeVisible();
      await shoot(page, 'log-operator', width);
    });

    test(`S11 log transaksi (auditor) @${width}`, async ({ page }) => {
      await stub(page);
      await page.setViewportSize({ width, height: 900 });
      await page.goto('/?peran=auditor');
      await expect(page.getByRole('link', { name: 'TRX-2026-0042' })).toBeVisible();
      await shoot(page, 'log-auditor', width);
    });

    test(`dialog payload @${width}`, async ({ page }) => {
      await stub(page);
      await page.setViewportSize({ width, height: 900 });
      await page.goto('/?peran=auditor');
      await page.getByRole('button', { name: 'Lihat payload — TRX-2026-0042' }).click();
      const dialog = page.getByRole('dialog', { name: 'Alasan membuka payload' });
      await expect(dialog.getByRole('heading', { name: 'Alasan membuka payload' })).toBeVisible();
      await dialog.getByLabel('Alasan membuka payload').fill('Cek aduan W-2026-07');
      await dialog.getByRole('button', { name: 'Lihat payload' }).click();
      await expect(
        page
          .getByRole('dialog', { name: 'Payload transaksi' })
          .getByRole('heading', { name: 'Ringkasan transaksi' }),
      ).toBeVisible();
      await shoot(page, 'dialog-payload', width);
    });

    test(`S12 pencarian NIK @${width}`, async ({ page }) => {
      await stub(page);
      await page.setViewportSize({ width, height: 900 });
      await page.goto('/nik?peran=auditor');
      await expect(page.getByRole('heading', { level: 1 })).toBeVisible();
      await shoot(page, 'cari-nik', width);
    });

    test(`S13 audit akses @${width}`, async ({ page }) => {
      await stub(page);
      await page.setViewportSize({ width, height: 900 });
      await page.goto('/audit?peran=auditor');
      await expect(page.getByRole('heading', { level: 1 })).toBeVisible();
      await shoot(page, 'audit-akses', width);
    });

    test(`S06 kuota @${width}`, async ({ page }) => {
      await stub(page);
      await page.setViewportSize({ width, height: 900 });
      await page.goto('/kuota?peran=operator');
      await expect(page.getByRole('heading', { level: 1 })).toBeVisible();
      await shoot(page, 'kuota', width);
    });
  });
}

/**
 * Layar detail provider per tab (S03a Umum, S03 Template, S04 Parameter, S03b Respons; ARC-252).
 * Hanya 1440 dan 375: lebar yang dipakai frame Penpot. Data sintetis; peran platform_admin.
 */
const PROVIDER = {
  provider_id: '00000000-0000-4000-8000-0000000000a1',
  provider_config_version: 3,
  aktif: true,
  nama: 'provider-contoh-a',
  base_url: 'https://provider.example.test/api',
  metode: 'POST',
  tls_trust: 'system',
  log_mode: 'metadata_only',
  config_schema_version: 2,
  body_template: '{"NIK":"{{input.NIK}}"}',
  parameters: [
    { nama: 'NIK', rule: { jenis: 'panjang', panjang_min: 16, panjang_maks: 16 } },
    { nama: 'tanggal_lahir', rule: { jenis: 'format_tanggal', format_tanggal: 'yyyy-MM-dd' } },
    { nama: 'TRESHOLD', nilai: '90' },
  ],
  respons_skema: { status: 'string' },
  data_balikan: { versi: 2 },
};

const PROVIDER_TABS = [
  { tab: 'Umum', berkas: 'provider-umum' },
  { tab: 'Template', berkas: 'provider-template' },
  { tab: 'Parameter', berkas: 'provider-rule' },
  { tab: 'Respons', berkas: 'provider-skema' },
] as const;

for (const width of [1440, 375] as const) {
  test.describe(`bukti visual provider @${width}`, () => {
    test.describe.configure({ mode: 'serial' });

    for (const { tab, berkas } of PROVIDER_TABS) {
      test(`detail provider tab ${tab} @${width}`, async ({ page }) => {
        await stub(page);
        await page.route('**/admin/v1/providers/*', (route) =>
          route.fulfill({
            status: 200,
            headers: { 'content-type': 'application/json', 'cache-control': 'no-store' },
            body: JSON.stringify(PROVIDER),
          }),
        );
        await page.setViewportSize({ width, height: 900 });
        await page.goto(`/provider/${PROVIDER.provider_id}?peran=platform_admin`);
        await expect(page.getByRole('heading', { level: 1 })).toHaveText('provider-contoh-a');
        await page.getByRole('tab', { name: tab }).click();
        await shoot(page, berkas, width);
      });
    }
  });
}
