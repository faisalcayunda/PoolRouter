import { expect, expectNoAxeViolations, stubSesi, test } from './fixtures';
import type { Page } from '@playwright/test';

/**
 * axe coverage for every screen in scope, in both languages (SHAA-203 acceptance criteria).
 *
 * The shell spec only proves the empty shell passes. These specs load each screen with data so the
 * table rows, filters and reason dialogs are on screen when axe runs — an empty table hides most of
 * the violations. The `en` pass is the one that matters for the catalogue: a wrong ARIA name or a
 * missing accessible label shows up once the copy comes from the second catalogue.
 */
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

const AUDIT_PAGE = {
  baris: [
    {
      audit_id: 'a1',
      dilakukan_pada: '2026-11-03T07:30:00.000Z',
      pengguna_id: 'budi',
      aksi: 'lihat_payload',
      hasil: 'berhasil',
      alasan: 'Cek aduan W-2026-07',
    },
  ],
  kursor: null,
};

const KUOTA = [
  {
    upstream_pool_id: '0f0f0f0f-1111-2222-3333-444444444444',
    periode_mulai: '2026-11-03T00:00:00Z',
    periode_selesai: '2026-11-03T01:00:00Z',
    rate_limit_per_jam: 1000,
    kuota_efektif: 800,
    sisa_periode: 100,
    terpakai_jam_berjalan: 700,
    terpakai_sync_jam_berjalan: 50,
    porsi_sync_maks: 100,
    dihitung_pada: '2026-11-03T07:30:00Z',
  },
];

const UPSTREAM_POOLS = [
  {
    upstream_pool_id: '0f0f0f0f-1111-2222-3333-444444444444',
    nama: 'dukcapil-utama',
    base_url: 'https://dukcapil.example.test',
    aktif: true,
    ambang_berdampingan: 4,
    batas_percobaan: 3,
    jeda_min: 200,
    jeda_maks: 800,
  },
];

/** Stub the admin API with data so every table has a row for axe to inspect. */
async function stubScreens(page: Page): Promise<void> {
  await page.route('**/admin/v1/**', async (route) => {
    const url = route.request().url();
    const json = (body: unknown) =>
      route.fulfill({
        status: 200,
        headers: { 'content-type': 'application/json', 'cache-control': 'no-store' },
        body: JSON.stringify(body),
      });

    if (url.includes('/audit-akses')) return json(AUDIT_PAGE);
    if (url.includes('/kuota')) return json(KUOTA);
    if (url.includes('/upstream-pools')) return json(UPSTREAM_POOLS);
    if (url.includes('/auditor/log')) return json(LOG_PAGE);
    if (url.includes('/log')) return json(LOG_PAGE);
    return json(LOG_PAGE);
  });
  await stubSesi(page);
}

const SCREENS = [
  {
    path: '/',
    name: 'S11 log transaksi',
    heading: { id: 'Log transaksi', en: 'Transaction logs' },
    // Sejak ARC-247 layar log memuat data sendiri dan filter berlaku langsung: tidak ada tombol Cari.
    table: true,
  },
  {
    path: '/nik',
    name: 'S12 pencarian per NIK',
    heading: { id: 'Pencarian log per NIK', en: 'Search logs by NIK' },
    search: { id: 'Cari', en: 'Search' },
    // S12 needs a full NIK before the search button does anything.
    nikField: { id: 'NIK (16 digit)', en: 'NIK (16 digits)' },
  },
  {
    path: '/audit',
    name: 'S13 audit akses',
    heading: { id: 'Audit akses', en: 'Access audit' },
    search: { id: 'Cari', en: 'Search' },
  },
  { path: '/kuota', name: 'quota tracker', heading: { id: 'Kuota', en: 'Quota' } },
] as const;

const NIK = '3175000000000001';
const REASON = 'Cek tampilan untuk uji aksesibilitas';

for (const screen of SCREENS) {
  for (const bahasa of ['id', 'en'] as const) {
    test(`${screen.name} passes axe in ${bahasa}`, async ({ page, cspViolations }) => {
      await stubScreens(page);
      // `?peran=auditor` pins the role until the session hook lands; `?bahasa=` picks the
      // catalogue. Neither changes what the stub serves, so both runs exercise the same data.
      await page.goto(`${screen.path}?peran=auditor&bahasa=${bahasa}`);

      await expect(page.getByRole('heading', { level: 1 })).toHaveText(screen.heading[bahasa]);

      if ('search' in screen) {
        if ('nikField' in screen) {
          await page.getByRole('textbox', { name: screen.nikField[bahasa] }).fill(NIK);
        }
        // Screens that record a search ask for a reason first; that dialog is where the audit rule
        // lives, so the search is driven through it rather than around it.
        await page.getByRole('button', { name: screen.search[bahasa] }).click();
        const dialog = page.getByRole('dialog');
        if (await dialog.isVisible().catch(() => false)) {
          await dialog.getByLabel(/Alasan|Reason/).fill(REASON);
          await dialog.getByRole('button', { name: screen.search[bahasa] }).click();
        }
      }

      // Wait for the data surface before axe runs: the tables have a row, the quota tracker has a
      // card. An empty screen hides most violations, so this wait is what makes the run meaningful.
      if ('search' in screen || 'table' in screen) {
        await expect(page.getByRole('cell').first()).toBeVisible();
      } else {
        await expect(page.getByText(/Terpakai|used/).first()).toBeVisible();
      }

      await expectNoAxeViolations(page);
      expect(cspViolations).toEqual([]);
    });
  }
}

/**
 * Layar konfigurasi provider dan secret (ARC-101). Hanya platform admin yang boleh membukanya,
 * jadi peran disematkan ke `platform_admin`; stub mengembalikan satu baris supaya tabel dan form
 * terisi saat axe berjalan. Route ini didaftarkan setelah catch-all sehingga dicocokkan lebih dulu.
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
  parameters: [{ nama: 'TRESHOLD', nilai: '90' }],
  respons_skema: { status: 'string' },
  data_balikan: { versi: 2 },
};

const SECRETS = [
  { nama: 'USER_ID', tersamar: true, dipakai_oleh_provider_count: 1 },
  { nama: 'KUNCI_UJI', tersamar: true, dipakai_oleh_provider_count: 0 },
];

async function stubKonfigurasi(page: Page): Promise<void> {
  await stubScreens(page);
  const json = (body: unknown) => ({
    status: 200,
    headers: { 'content-type': 'application/json', 'cache-control': 'no-store' },
    body: JSON.stringify(body),
  });
  await page.route('**/admin/v1/providers', (route) => route.fulfill(json([PROVIDER])));
  await page.route('**/admin/v1/providers/*', (route) => route.fulfill(json(PROVIDER)));
  await page.route('**/admin/v1/secrets', (route) => route.fulfill(json(SECRETS)));
}

const KONFIGURASI = [
  {
    path: '/provider',
    name: 'S02 daftar provider',
    heading: { id: 'Provider', en: 'Providers' },
    ready: (page: Page) => page.getByRole('cell').first(),
  },
  {
    path: `/provider/${PROVIDER.provider_id}`,
    name: 'S03 form provider',
    heading: { id: 'provider-contoh-a', en: 'provider-contoh-a' },
    ready: (page: Page) => page.getByLabel(/Nama provider|Provider name/),
  },
  {
    path: '/secret',
    name: 'S05 secret',
    heading: { id: 'Secret', en: 'Secrets' },
    ready: (page: Page) => page.getByRole('cell').first(),
  },
] as const;

for (const layar of KONFIGURASI) {
  for (const bahasa of ['id', 'en'] as const) {
    test(`${layar.name} passes axe in ${bahasa}`, async ({ page, cspViolations }) => {
      await stubKonfigurasi(page);
      await page.goto(`${layar.path}?peran=platform_admin&bahasa=${bahasa}`);

      await expect(page.getByRole('heading', { level: 1 })).toHaveText(layar.heading[bahasa]);
      await expect(layar.ready(page)).toBeVisible();

      await expectNoAxeViolations(page);
      expect(cspViolations).toEqual([]);
    });
  }
}

for (const bahasa of ['id', 'en'] as const) {
  test(`S05 add-secret dialog passes axe in ${bahasa}`, async ({ page, cspViolations }) => {
    await stubKonfigurasi(page);
    await page.goto(`/secret?peran=platform_admin&bahasa=${bahasa}`);
    await expect(page.getByRole('cell').first()).toBeVisible();

    await page
      .getByRole('button', { name: bahasa === 'id' ? 'Tambah secret' : 'Add secret' })
      .click();
    await expect(page.getByRole('dialog')).toBeVisible();

    await expectNoAxeViolations(page);
    expect(cspViolations).toEqual([]);
  });
}

/**
 * Login dan dua dialog sesi (ARC-209). Layar login berdiri di luar shell dan tidak memanggil API.
 * Dialog sesi dipicu lewat stub `GET /auth/sesi` yang jatuh di dalam jendela peringatan 2 menit,
 * jadi hook membukanya saat dimuat tanpa menunggu timer.
 */
const JENDELA_MS = 110_000;

for (const bahasa of ['id', 'en'] as const) {
  test(`S01 login passes axe in ${bahasa}`, async ({ page, cspViolations }) => {
    await page.goto(`/login?bahasa=${bahasa}`);
    await expect(page.getByRole('heading', { level: 1 })).toHaveText(
      bahasa === 'id' ? 'Masuk ke akun' : 'Sign in to your account',
    );
    await expectNoAxeViolations(page);
    expect(cspViolations).toEqual([]);
  });

  test(`S01 login with the expired-session notice passes axe in ${bahasa}`, async ({
    page,
    cspViolations,
  }) => {
    await page.goto(`/login?alasan=sesi_berakhir&bahasa=${bahasa}`);
    await expect(page.getByRole('heading', { level: 1 })).toBeVisible();
    await expectNoAxeViolations(page);
    expect(cspViolations).toEqual([]);
  });

  test(`idle session warning dialog passes axe in ${bahasa}`, async ({ page, cspViolations }) => {
    await stubScreens(page);
    await stubSesi(page, { idleMs: JENDELA_MS });
    await page.goto(`/?peran=auditor&bahasa=${bahasa}`);
    const dialog = page.getByRole('alertdialog');
    await expect(dialog).toBeVisible();
    await expect(dialog.getByRole('button').first()).toBeVisible();
    await expectNoAxeViolations(page);
    expect(cspViolations).toEqual([]);
  });

  test(`absolute session warning dialog passes axe in ${bahasa}`, async ({
    page,
    cspViolations,
  }) => {
    await stubScreens(page);
    await stubSesi(page, { absolutMs: JENDELA_MS });
    await page.goto(`/?peran=auditor&bahasa=${bahasa}`);
    await expect(page.getByRole('alertdialog')).toBeVisible();
    await expectNoAxeViolations(page);
    expect(cspViolations).toEqual([]);
  });
}

/**
 * Tiga alur MFA (ARC-216): login dua langkah, pendaftaran, dan kode pemulihan. Backend dijawab stub
 * dengan nilai sintetis yang jelas palsu; axe dijalankan di setiap layar yang tampil.
 */
const MFA_RAHASIA = 'CONTOHSECRETSINTETISBUKANASLI234';
const MFA_KODE_PEMULIHAN = Array.from(
  { length: 10 },
  (_, i) => `CONTOHPM${String(i).padStart(2, '0')}`,
);

async function stubMfa(page: Page, langkah: 'mfa' | 'daftar_mfa'): Promise<void> {
  const json = (body: unknown) => ({
    status: 200,
    headers: { 'content-type': 'application/json', 'cache-control': 'no-store' },
    body: JSON.stringify(body),
  });
  await page.route('**/admin/v1/auth/login', (route) => route.fulfill(json({ langkah })));
  await page.route('**/admin/v1/auth/mfa/daftar/konfirmasi', (route) =>
    route.fulfill(json({ kode_pemulihan: MFA_KODE_PEMULIHAN })),
  );
  await page.route('**/admin/v1/auth/mfa/daftar', (route) =>
    route.fulfill(
      json({
        otpauth_uri: `otpauth://totp/PoolRouter:a?secret=${MFA_RAHASIA}&issuer=PoolRouter`,
        rahasia_base32: MFA_RAHASIA,
      }),
    ),
  );
}

async function masukSampaiMfa(page: Page, bahasa: 'id' | 'en'): Promise<void> {
  await page.goto(`/login?bahasa=${bahasa}`);
  await page.getByRole('textbox').first().fill('a');
  await page.locator('input[type="password"]').fill('b');
  await page.locator('form button[type="submit"]').click();
}

for (const bahasa of ['id', 'en'] as const) {
  test(`S17 login second step passes axe in ${bahasa}`, async ({ page, cspViolations }) => {
    await stubMfa(page, 'mfa');
    await masukSampaiMfa(page, bahasa);
    await expect(page.getByRole('heading', { level: 1 })).toBeVisible();
    await expect(page.locator('input[autocomplete="one-time-code"]')).toBeVisible();
    await expectNoAxeViolations(page);
    expect(cspViolations).toEqual([]);
  });

  test(`S17a-S17b enrollment passes axe in ${bahasa}`, async ({ page, cspViolations }) => {
    await stubMfa(page, 'daftar_mfa');
    await masukSampaiMfa(page, bahasa);
    const kodeDaftar = page.locator('input[autocomplete="off"]');
    await expect(kodeDaftar).toBeVisible();
    await expectNoAxeViolations(page);

    await kodeDaftar.fill('CONTOHDAFTAR');
    await page.getByRole('button').first().click();
    await expect(page.getByRole('img').first()).toBeVisible();
    await expectNoAxeViolations(page);
    expect(cspViolations).toEqual([]);
  });

  test(`S17c recovery codes dialog passes axe in ${bahasa}`, async ({ page, cspViolations }) => {
    await stubMfa(page, 'daftar_mfa');
    await masukSampaiMfa(page, bahasa);
    await page.locator('input[autocomplete="off"]').fill('CONTOHDAFTAR');
    await page.getByRole('button').first().click();
    await page.locator('input[autocomplete="one-time-code"]').fill('123456');
    await page.locator('section button').last().click();
    const dialog = page.getByRole('alertdialog');
    await expect(dialog).toBeVisible();
    await expect(dialog.getByRole('listitem')).toHaveCount(10);
    await expectNoAxeViolations(page);
    expect(cspViolations).toEqual([]);
  });
}

/**
 * ui-v3 (ARC-363 §5, AC1): rute `routes.tsx` yang belum tercakup di atas, dua bahasa. Stub per path
 * supaya tabel dan form terisi saat axe berjalan; data sintetis.
 */
const UP_ID = '00000000-0000-4000-8000-0000000000b1';
const DOWN_ID = '00000000-0000-4000-8000-0000000000c1';
const ENDPOINT_ID = '00000000-0000-4000-8000-0000000000c5';
const CLIENT_ID = '00000000-0000-4000-8000-0000000000e0';

const UPSTREAM = {
  ...UPSTREAM_POOLS[0],
  upstream_pool_id: UP_ID,
  provider_id: PROVIDER.provider_id,
  jendela_kuota: 'bergulir_24j',
  kuota_harian: 500,
  margin_kuota: 0.05,
  porsi_sync: 0.1,
  rate_limit_per_jam: 60,
  reset_kuota_jam: 0,
  reset_kuota_zona: 'Asia/Jakarta',
  retry_backoff_base_detik: 2,
  retry_backoff_cap_detik: 60,
  retry_min_per_jam: 10,
  retry_rasio: 0.5,
};

const DOWNSTREAM = {
  downstream_pool_id: DOWN_ID,
  nama: 'pool-hilir-contoh',
  aktif: true,
  boleh_pinjam: false,
  prioritas: 1,
  ttl_antrean_detik: 86_400,
  upstream_pool_ids: [UP_ID],
  alokasi_jenis: 'persen',
  alokasi_nilai: 25,
  sifat: 'reserved',
};

const ENDPOINT = {
  endpoint_id: ENDPOINT_ID,
  provider_id: PROVIDER.provider_id,
  downstream_pool_id: DOWN_ID,
  parameter_literal: { TRESHOLD: '90' },
  log_mode: 'metadata_only',
  metadata_fields: [
    {
      nama_field: 'status_penerima_manfaat',
      tipe: 'string',
      wajib: true,
      klasifikasi: 'Internal',
      kirim_ke_data_balikan: true,
      deskripsi: 'Status penerima',
    },
  ],
  config_schema_version: 1,
  endpoint_config_version: 3,
  aktif: true,
};

const CLIENT = {
  client_id: CLIENT_ID,
  nama: 'Aplikasi Contoh A',
  aktif: true,
  limiter_penolakan_burst: 20,
  limiter_penolakan_isi_ulang_per_detik: 1,
};

async function stubSemuaRute(page: Page): Promise<void> {
  await page.route('**/admin/v1/**', (route) => {
    const path = new URL(route.request().url()).pathname.replace('/admin/v1', '');
    const json = (body: unknown) =>
      route.fulfill({
        status: 200,
        headers: { 'content-type': 'application/json', 'cache-control': 'no-store' },
        body: JSON.stringify(body),
      });
    if (/^\/upstream-pools\/[^/]+$/.test(path)) return json(UPSTREAM);
    if (path.startsWith('/upstream-pools')) return json([UPSTREAM]);
    if (/^\/downstream-pools\/[^/]+$/.test(path)) return json(DOWNSTREAM);
    if (path.startsWith('/downstream-pools')) return json([DOWNSTREAM]);
    if (/^\/providers\/[^/]+$/.test(path)) return json(PROVIDER);
    if (path.startsWith('/providers')) return json([PROVIDER]);
    if (/^\/endpoints\/[^/]+$/.test(path)) return json(ENDPOINT);
    if (path.startsWith('/endpoints')) return json([ENDPOINT]);
    if (/^\/clients\/[^/]+\/credentials$/.test(path)) {
      return json([
        {
          credential_id: '00000000-0000-4000-8000-0000000000d1',
          jenis: 'oauth2',
          status: 'aktif',
          kedaluwarsa_at: '2027-01-05T03:00:00Z',
        },
      ]);
    }
    if (/^\/clients\/[^/]+\/webhook\/log$/.test(path)) {
      return json([
        {
          request_id: '00000000-0000-4000-8000-0000000000f1',
          sent_at: '2026-11-03T07:30:00.000Z',
          hasil: 'gagal_http',
          http_status: 503,
          error_class: 'provider_sementara',
          secret_versi: 2,
        },
      ]);
    }
    if (/^\/clients\/[^/]+\/webhook$/.test(path)) {
      return json({
        aktif: true,
        webhook_url: 'https://client.example.test/hook',
        secret_versi: 2,
      });
    }
    if (/^\/clients\/[^/]+$/.test(path)) return json(CLIENT);
    if (path.startsWith('/clients')) return json([CLIENT]);
    return json(LOG_PAGE);
  });
  await stubSesi(page, { peran: 'platform_admin' });
}

const RUTE_V3 = [
  { name: 'S11 detail log', path: '/log/TRX-2026-0042', peran: 'operator' },
  { name: 'S03 provider baru', path: '/provider/baru', peran: 'platform_admin' },
  { name: 'S04b daftar endpoint', path: '/endpoint', peran: 'platform_admin' },
  { name: 'S04b form endpoint', path: `/endpoint/${ENDPOINT_ID}`, peran: 'platform_admin' },
  { name: 'S04b endpoint baru', path: '/endpoint/baru', peran: 'platform_admin' },
  { name: 'S08 daftar client', path: '/client', peran: 'platform_admin' },
  { name: 'S08 detail client', path: `/client/${CLIENT_ID}`, peran: 'platform_admin' },
  {
    name: 'S08b kredensial client',
    path: `/client/${CLIENT_ID}/kredensial`,
    peran: 'platform_admin',
  },
  { name: 'S09 webhook client', path: `/client/${CLIENT_ID}/webhook`, peran: 'platform_admin' },
  { name: 'S06 daftar upstream', path: '/upstream', peran: 'platform_admin' },
  { name: 'S06 form upstream', path: `/upstream/${UP_ID}`, peran: 'platform_admin' },
  { name: 'S06 upstream baru', path: '/upstream/baru', peran: 'platform_admin' },
  { name: 'S07 daftar downstream', path: '/downstream', peran: 'platform_admin' },
  { name: 'S07 form downstream', path: `/downstream/${DOWN_ID}`, peran: 'platform_admin' },
  { name: 'S07 downstream baru', path: '/downstream/baru', peran: 'platform_admin' },
  { name: 'not found', path: '/halaman-tidak-ada', peran: 'platform_admin' },
] as const;

for (const rute of RUTE_V3) {
  for (const bahasa of ['id', 'en'] as const) {
    test(`${rute.name} passes axe in ${bahasa}`, async ({ page, cspViolations }) => {
      await stubSemuaRute(page);
      await page.goto(`${rute.path}?peran=${rute.peran}&bahasa=${bahasa}`);
      await expect(page.getByRole('heading', { level: 1 })).toBeVisible();
      await expect(page.getByText(/^(Memuat…|Loading…)$/).filter({ visible: true })).toHaveCount(0);
      await expectNoAxeViolations(page);
      expect(cspViolations).toEqual([]);
    });
  }
}
