import { expect, expectNoAxeViolations, test } from './fixtures';
import type { Page } from '@playwright/test';

/**
 * S11 Log transaksi for an auditor (SHAA-203 section 18.3).
 *
 * The operator variant of S11 is covered by the unit tests, which can pin
 * `canOpenPayload`. This spec is the one place that proves the affordance is
 * reachable in the assembled app: the auditor searches by transaction code,
 * sees every client, and opens the payload through the reason dialog.
 *
 * The API is stubbed at the network boundary, so the assertions are about what
 * the screen sends — filters in a POST body, never in the URL.
 */
const LOG_PAGE = {
  baris: [
    {
      kode_transaksi: 'TRX-2026-0077',
      client_id: 'c1',
      client_nama: 'Dukcapil Demo',
      queue_id: 'q1',
      nik: '3175********0001',
      status_akses: 'ok',
      requested_at: '2026-11-03T07:30:00.000Z',
    },
    {
      // A second client in the same result set: the auditor is not scoped to one client.
      kode_transaksi: 'TRX-2026-0078',
      client_id: 'c2',
      client_nama: 'Kemensos Demo',
      queue_id: 'q2',
      nik: '3273********0002',
      status_akses: 'berhasil',
      requested_at: '2026-11-03T07:31:00.000Z',
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

async function stubS11(page: Page): Promise<Recorded[]> {
  const calls: Recorded[] = [];
  await page.route('**/admin/v1/**', async (route) => {
    const request = route.request();
    const url = request.url();
    calls.push({
      url,
      method: request.method(),
      body: request.postDataJSON?.() ?? null,
    });

    if (url.includes('/auditor/payload')) {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        // `no-store` is part of the contract (R-16): the payload must not be cached.
        headers: { 'cache-control': 'no-store' },
        body: JSON.stringify({
          request_id: '0190f2c4-0000-7000-8000-000000000002',
          kode_transaksi: 'TRX-2026-0077',
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

test.describe('S11 log transaksi (auditor)', () => {
  test('searches by transaction code and opens the payload through a reason', async ({ page }) => {
    const calls = await stubS11(page);
    // The session hook has not landed (section 18.1), so the assembled app cannot assume a role on
    // its own. `?peran=auditor` pins it for this spec (`src/app/routes.tsx`); the API is still the
    // last guard, so the stub is what makes the data available — not the query parameter.
    await page.goto('/?peran=auditor');

    await expect(page.getByRole('heading', { level: 1 })).toHaveText('Log transaksi');
    await expectNoAxeViolations(page);

    // 1. Search by transaction code. The filter must travel in the POST body.
    await page.getByRole('textbox', { name: 'Kode transaksi' }).fill('TRX-2026-0077');
    await page.getByRole('button', { name: 'Cari' }).click();

    await expect(page.getByRole('cell', { name: 'TRX-2026-0077' })).toBeVisible();
    // Both clients appear: an auditor's S11 is not scoped to one client.
    await expect(page.getByRole('cell', { name: 'Kemensos Demo' })).toBeVisible();
    await expectNoAxeViolations(page);

    const cari = calls.filter((c) => c.url.includes('/log/cari'));
    expect(cari.length).toBeGreaterThan(0);
    expect(cari[0]!.method).toBe('POST');
    expect(cari[0]!.body).toMatchObject({ kode_transaksi: 'TRX-2026-0077' });
    // The transaction code never leaks into the query string (it would reach the access log).
    expect(cari[0]!.url).not.toContain('TRX-2026-0077');

    // 2. The auditor gets the payload affordance, and it opens only through the reason dialog.
    const openButtons = page.getByRole('button', { name: 'Payload' });
    await expect(openButtons).toHaveCount(2);
    await openButtons.first().click();

    const dialog = page.getByRole('dialog');
    await expect(dialog).toBeVisible();
    await expectNoAxeViolations(page);

    // The confirm stays disabled until the minimum reason length is met: the dialog is the guard.
    const confirm = dialog.getByRole('button', { name: 'Buka payload' });
    await expect(confirm).toBeDisabled();
    expect(calls.filter((c) => c.url.includes('/auditor/payload'))).toHaveLength(0);

    await dialog.getByLabel('Alasan (minimal 10 karakter)').fill('Tindak lanjut aduan W-2026-09');
    await expect(confirm).toBeEnabled();
    await confirm.click();

    // The payload renders read-only; the full NIK only shows in the Input block.
    await expect(dialog.getByText('TRX-2026-0077')).toBeVisible();
    await expect(dialog.getByText('3175000000000001')).toBeVisible();
    // No copy or download action, ever (section 18.3).
    await expect(dialog.getByRole('button', { name: /salin|unduh/i })).toHaveCount(0);

    const payloadCalls = calls.filter((c) => c.url.includes('/auditor/payload'));
    expect(payloadCalls).toHaveLength(1);
    expect(payloadCalls[0]!.method).toBe('POST');
    expect(payloadCalls[0]!.body).toMatchObject({ kode_transaksi: 'TRX-2026-0077' });
    // The reason rides the header, not the URL or the access log.
    expect(payloadCalls[0]!.url).not.toContain('Tindak');
  });

  test('keeps the payload out of persistent storage and forgets it on close', async ({ page }) => {
    // Section 18.3: the payload lives in the dialog only. Neither the full NIK nor the payload may
    // survive in localStorage, sessionStorage or IndexedDB, and closing the dialog drops it.
    await stubS11(page);
    await page.goto('/?peran=auditor');
    await page.getByRole('textbox', { name: 'Kode transaksi' }).fill('TRX-2026-0077');
    await page.getByRole('button', { name: 'Cari' }).click();
    await page.getByRole('button', { name: 'Payload' }).first().click();

    const dialog = page.getByRole('dialog');
    await dialog.getByLabel('Alasan (minimal 10 karakter)').fill('Tindak lanjut aduan W-2026-09');
    await dialog.getByRole('button', { name: 'Buka payload' }).click();
    await expect(dialog.getByText('3175000000000001')).toBeVisible();

    const stored = await page.evaluate(async () => {
      const local = JSON.stringify(localStorage);
      const session = JSON.stringify(sessionStorage);
      const dbs = (await indexedDB.databases()).map((db) => db.name);
      return { local, session, dbs };
    });
    expect(stored.local).not.toContain('3175000000000001');
    expect(stored.session).not.toContain('3175000000000001');
    expect(stored.dbs).toEqual([]);

    // Two controls carry the label: the icon close button and the footer button. Take the icon one.
    await dialog.getByRole('button', { name: 'Tutup' }).first().click();
    await expect(dialog).toBeHidden();
    await expect(page.getByText('3175000000000001')).toHaveCount(0);
  });
});
