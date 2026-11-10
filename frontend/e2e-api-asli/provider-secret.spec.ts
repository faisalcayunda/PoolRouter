import { expect, test } from './fixtures';

/**
 * ARC-101 AC-4 dan AC-15 (spec ui-v2-layar), terhadap backend sungguhan. Semua data sintetis; nama
 * dan nilai secret dibuat per run supaya tidak bentrok dengan data seed.
 */
const SUFIKS = Date.now().toString(36).toUpperCase();
const NAMA_SECRET = `KUNCI_E2E_${SUFIKS}`;
const NILAI_SECRET = `nilai-sintetis-e2e-${SUFIKS.toLowerCase()}`;
const NAMA_PROVIDER = `provider-e2e-${SUFIKS.toLowerCase()}`;
const TEMPLATE = `{"NIK":"{{input.NIK}}","UID":"{{secret.${NAMA_SECRET}}}"}`;

test.describe('@api-asli provider dan secret', () => {
  test('@api-asli sesi uji diterima backend sebagai platform admin', async ({ page }) => {
    const res = await page.request.get('/admin/v1/auth/me');
    expect(res.status()).toBe(200);
    const me = (await res.json()) as { peran?: string[] | string };
    expect(JSON.stringify(me)).toContain('platform_admin');
  });

  test('@api-asli AC-15 buat secret, buat provider yang merujuknya, lalu buka lagi', async ({
    page,
  }) => {
    // 1. Buat secret di S05 dan pastikan nilainya tidak tinggal di DOM (AC-14).
    await page.goto('/secret');
    await page.getByRole('button', { name: 'Tambah secret' }).click();
    const dialog = page.getByRole('dialog');
    await dialog.getByLabel('Nama').fill(NAMA_SECRET);
    await dialog.getByLabel('Nilai').fill(NILAI_SECRET);
    await dialog.getByRole('button', { name: 'Simpan secret' }).click();
    await expect(page.getByText(`Secret ${NAMA_SECRET} tersimpan.`)).toBeVisible();
    await expect(dialog).toBeHidden();
    await expect(page.getByRole('row', { name: new RegExp(NAMA_SECRET) })).toBeVisible();
    await expectNilaiTidakAdaDiHalaman(page, NILAI_SECRET);

    // 2. Buat provider yang template-nya merujuk secret itu.
    await page.goto('/provider/baru');
    await page.getByLabel('Nama provider').fill(NAMA_PROVIDER);
    await page.getByLabel('Alamat endpoint (URL)').fill('https://provider.example.test/api');
    await page.getByRole('tab', { name: 'Template' }).click();
    await page.getByLabel('Template body (JSON)').fill(TEMPLATE);
    await page.getByRole('button', { name: 'Simpan provider' }).click();
    await expect(
      page.getByText(`Provider ${NAMA_PROVIDER} tersimpan sebagai versi 1.`),
    ).toBeVisible();

    // 3. Buka lagi dari daftar: template masih merujuk secret, nilainya tidak ikut terbaca.
    await page.goto('/provider');
    await page.getByRole('link', { name: NAMA_PROVIDER }).first().click();
    await page.getByRole('tab', { name: 'Template' }).click();
    await expect(page.getByLabel('Template body (JSON)')).toHaveValue(TEMPLATE);
    await expectNilaiTidakAdaDiHalaman(page, NILAI_SECRET);
  });
});

/** Nilai secret tidak boleh ada di teks halaman maupun di input/textarea mana pun (AC-14). */
async function expectNilaiTidakAdaDiHalaman(
  page: import('@playwright/test').Page,
  nilai: string,
): Promise<void> {
  expect(await page.content()).not.toContain(nilai);
  const nilaiField = await page
    .locator('input, textarea')
    .evaluateAll((els) => els.map((el) => (el as HTMLInputElement).value));
  expect(nilaiField.filter((v) => v.includes(nilai))).toEqual([]);
}
