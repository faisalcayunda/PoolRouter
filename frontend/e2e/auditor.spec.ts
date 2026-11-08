import { expect, expectNoAxeViolations, test } from './fixtures';
import type { Page } from '@playwright/test';

/**
 * Auditor flow (SHAA-203 section 18.3): search by NIK, open a payload, and export the result.
 *
 * The API is stubbed at the network boundary so the spec exercises the real screens, the real
 * catalogue strings and the audit-reason dialogs without a backend. Every request is recorded so
 * the body can be asserted — that is where the audit rules live (reason required, POST not GET).
 */
const NIK = '3175000000000001';

const LOG_PAGE = {
  baris: [
    {
      kode_transaksi: 'TRX-2026-0042',
      client_id: 'c1',
      client_nama: 'Dukcapil Demo',
      queue_id: 'q1',
      nik: '3175********0001',
      status_akses: 'ok',
      requested_at: '2026-11-03T07:30:00.000Z',
    },
  ],
  kursor: null,
  di_luar_retensi_online: false,
};

interface Recorded {
  url: string;
  method: string;
  body: unknown;
}

/** Stub every admin endpoint the auditor screens touch, recording each call. */
async function stubAuditorApi(page: Page): Promise<Recorded[]> {
  const calls: Recorded[] = [];
  await page.route('**/admin/v1/**', async (route) => {
    const request = route.request();
    const url = request.url();
    const body = request.postDataJSON?.() ?? null;
    calls.push({ url, method: request.method(), body });

    // Auth endpoints are stubbed globally; let them through to fixtures.
    if (url.includes('/auth/me')) {
      await route.fulfill({
        status: 200,
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ username: 'test', peran: ['auditor'] }),
      });
      return;
    }
    if (url.includes('/auth/sesi')) {
      await route.fulfill({
        status: 200,
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ batas_idle_detik: 3600, batas_absolut_detik: 28800 }),
      });
      return;
    }

    if (url.includes('/auditor/log/ekspor')) {
      await route.fulfill({
        status: 200,
        headers: {
          'content-type': 'text/csv; charset=utf-8',
          'content-disposition': 'attachment; filename="ekspor-log-nik.csv"',
          'cache-control': 'no-store',
          'repr-digest': 'sha-256=:abc123:',
          'accept-ranges': 'none',
        },
        body: 'kode_transaksi,nik\nTRX-2026-0042,3175********0001\n',
      });
      return;
    }
    if (url.includes('/auditor/payload')) {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        // R-16: the payload answer must state `no-store`; the client rejects one that omits it.
        headers: { 'cache-control': 'no-store' },
        // Shape follows `PayloadView` in the admin OpenAPI: the four identity fields are required,
        // and the screen formats `requested_at`, so an incomplete stub breaks the render.
        body: JSON.stringify({
          request_id: '0190f2c4-0000-7000-8000-000000000001',
          kode_transaksi: 'TRX-2026-0042',
          status_akses: 'ok',
          requested_at: '2026-11-03T07:30:00.000Z',
          input: { nik: '3175000000000001' },
          metadata: {},
          hasil: { status: 'ok' },
        }),
      });
      return;
    }
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify(LOG_PAGE),
    });
  });
  return calls;
}

test.describe('auditor flow', () => {
  test('searches a NIK, opens the payload, and exports the result', async ({ page }) => {
    const calls = await stubAuditorApi(page);
    await page.goto('/nik');

    await expect(page.getByRole('heading', { level: 1 })).toHaveText('Pencarian log per NIK');
    await expectNoAxeViolations(page);

    // 1. Search: the reason dialog gates the request.
    await page.getByRole('textbox', { name: 'NIK (16 digit)' }).fill(NIK);
    await page.getByRole('button', { name: 'Cari' }).click();

    const reasonDialog = page.getByRole('dialog', { name: 'Alasan mencari log NIK' });
    await expect(reasonDialog).toBeVisible();
    await reasonDialog.getByRole('textbox').fill('Cek aduan W-2026-07');
    await reasonDialog.getByRole('button', { name: 'Cari' }).click();

    await expect(page.getByRole('link', { name: 'TRX-2026-0042' })).toBeVisible();
    await expectNoAxeViolations(page);

    // 2. Open the payload: a second, separately audited dialog.
    await page.getByRole('button', { name: 'Lihat payload — TRX-2026-0042' }).click();
    const payloadDialog = page.getByRole('dialog', { name: 'Alasan membuka payload' });
    await expect(payloadDialog).toBeVisible();
    await payloadDialog.getByRole('button', { name: 'Lihat payload' }).click();
    // Section 13.2: once the payload is on screen the dialog is titled "Payload transaksi"; the
    // reason-step title belongs to the step it names.
    const opened = page.getByRole('dialog', { name: 'Payload transaksi' });
    await expect(opened).toBeVisible();
    // The three data-contract blocks render read-only; `metadata` is empty in the stub.
    await expect(opened.getByRole('heading', { name: 'Input' })).toBeVisible();
    await expect(opened.getByText(/3175000000000001/)).toBeVisible();
    await expect(opened.getByRole('heading', { name: 'Metadata' })).toBeVisible();
    await expect(opened.getByText('Tidak tersedia.')).toBeVisible();
    await expectNoAxeViolations(page);
    // The M3 dialog offers "Tutup" twice (header close and footer action); either closes it.
    await opened.getByRole('button', { name: 'Tutup' }).first().click();

    // 3. Export: must be a POST (never a GET, which would log the NIK in the access log).
    const download = page.waitForEvent('download');
    await page.getByRole('button', { name: 'Ekspor hasil' }).click();
    const exportDialog = page.getByRole('dialog', { name: 'Alasan mencari log NIK' });
    await expect(exportDialog).toBeVisible();
    // The reason is pre-filled from the search, so confirm straight away.
    await expect(exportDialog.getByRole('textbox')).toHaveValue('Cek aduan W-2026-07');
    await exportDialog.getByRole('button', { name: 'Ekspor hasil' }).click();
    expect((await download).suggestedFilename()).toBe('ekspor-log-nik.csv');

    const ekspor = calls.filter((c) => c.url.includes('/auditor/log/ekspor'));
    expect(ekspor).toHaveLength(1);
    expect(ekspor[0]!.method).toBe('POST');
    expect(ekspor[0]!.body).toMatchObject({ nik: NIK });
  });

  test('keeps the NIK and drops the code and client in the medium window class', async ({
    page,
  }) => {
    await stubAuditorApi(page);
    // 768 px is `medium` (600–839): section 4 hides Kode transaksi and Client on S13/S12 so the
    // NIK and the time still fit without a sideways scroll.
    await page.setViewportSize({ width: 768, height: 900 });
    await page.goto('/nik');

    await page.getByRole('textbox', { name: 'NIK (16 digit)' }).fill(NIK);
    await page.getByRole('button', { name: 'Cari' }).click();
    const reasonDialog = page.getByRole('dialog', { name: 'Alasan mencari log NIK' });
    await reasonDialog.getByRole('textbox').fill('Cek aduan W-2026-07');
    await reasonDialog.getByRole('button', { name: 'Cari' }).click();

    await expect(page.getByRole('cell', { name: '3175********0001' })).toBeVisible();
    const header = (name: string) => page.getByRole('columnheader', { name, includeHidden: true });
    await expect(header('Kode transaksi')).toBeHidden();
    await expect(header('Client')).toBeHidden();
    await expect(page.getByRole('columnheader', { name: 'NIK' })).toBeVisible();
    await expect(page.getByRole('columnheader', { name: 'Waktu' })).toBeVisible();
  });

  test('refuses a search without a reason and sends no request', async ({ page }) => {
    const calls = await stubAuditorApi(page);
    await page.goto('/nik');

    await page.getByRole('textbox', { name: 'NIK (16 digit)' }).fill(NIK);
    await page.getByRole('button', { name: 'Cari' }).click();

    // Named, because the compact drawer is a dialog too and would make the locator ambiguous.
    const dialog = page.getByRole('dialog', { name: 'Alasan mencari log NIK' });
    await expect(dialog).toBeVisible();
    // Section 8: submit stays enabled and reports the problem instead of being silently inert.
    await expect(dialog.getByRole('button', { name: 'Cari' })).toBeEnabled();
    await dialog.getByRole('button', { name: 'Cari' }).click();
    await expect(dialog.getByText('Alasan wajib diisi dengan 10–500 karakter.')).toBeVisible();
    await dialog.getByRole('button', { name: 'Batal' }).click();

    expect(calls.filter((c) => c.url.includes('/auditor/log'))).toHaveLength(0);
  });
});
