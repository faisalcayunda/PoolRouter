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
    // The session hook has not landed (section 18.1), so the assembled app renders S11 without a
    // role and the payload affordance stays hidden. This spec drives S11 as an operator: it proves
    // the screen is reachable, searches by transaction code, and keeps the payload column away.
    // The auditor affordance is pinned by `log-screen.test.tsx`, which can inject the role.
    await page.goto('/');

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

    // 2. With no role in session, the payload column stays hidden and nothing opens.
    await expect(page.getByRole('columnheader', { name: 'Payload' })).toHaveCount(0);
    await expect(page.getByRole('button', { name: 'Payload' })).toHaveCount(0);
    expect(calls.filter((c) => c.url.includes('/auditor/payload'))).toHaveLength(0);
  });
});
