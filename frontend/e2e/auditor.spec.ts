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

    const reasonDialog = page.getByRole('dialog');
    await expect(reasonDialog).toBeVisible();
    await reasonDialog.getByRole('textbox').fill('Cek aduan W-2026-07');
    await reasonDialog.getByRole('button', { name: 'Cari' }).click();

    await expect(page.getByRole('cell', { name: 'TRX-2026-0042' })).toBeVisible();
    await expectNoAxeViolations(page);

    // 2. Open the payload: a second, separately audited dialog.
    await page.getByRole('button', { name: 'Payload' }).click();
    const payloadDialog = page.getByRole('dialog');
    await expect(payloadDialog).toBeVisible();
    await payloadDialog.getByRole('button', { name: 'Buka payload' }).click();
    // The three data-contract blocks render read-only; `metadata` is empty in the stub.
    await expect(payloadDialog.getByRole('heading', { name: 'input' })).toBeVisible();
    await expect(payloadDialog.getByText(/3175000000000001/)).toBeVisible();
    await expect(payloadDialog.getByRole('heading', { name: 'metadata' })).toBeVisible();
    await expect(payloadDialog.getByText('—')).toBeVisible();
    await expectNoAxeViolations(page);
    // The M3 dialog offers "Tutup" twice (header close and footer action); either closes it.
    await payloadDialog.getByRole('button', { name: 'Tutup' }).first().click();

    // 3. Export: must be a POST (never a GET, which would log the NIK in the access log).
    const download = page.waitForEvent('download');
    await page.getByRole('button', { name: 'Ekspor hasil' }).click();
    const exportDialog = page.getByRole('dialog');
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

  test('refuses a search without a reason and sends no request', async ({ page }) => {
    const calls = await stubAuditorApi(page);
    await page.goto('/nik');

    await page.getByRole('textbox', { name: 'NIK (16 digit)' }).fill(NIK);
    await page.getByRole('button', { name: 'Cari' }).click();

    const dialog = page.getByRole('dialog');
    await expect(dialog).toBeVisible();
    // The confirm stays disabled until the minimum reason length is met.
    await expect(dialog.getByRole('button', { name: 'Cari' })).toBeDisabled();
    await dialog.getByRole('button', { name: 'Batal' }).click();

    expect(calls.filter((c) => c.url.includes('/auditor/log'))).toHaveLength(0);
  });
});
