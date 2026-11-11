import { expect, stubSesi, test } from './fixtures';
import type { Page } from '@playwright/test';

/**
 * Audit layout ui-v3 (spec ARC-363 §6, AC6). Setiap rute `routes.tsx` plus Login dibuka pada tiga
 * lebar yang mewakili tiga kelas aturan: 1440 (≥1200), 768 (600–1199), 390 (<600).
 *
 * - L1: padding horizontal konten 36/24/16 dan lebar isi maksimal 1280.
 * - L4: grid field (`[data-layout="field-grid"]`) 3/2/1 kolom; label tiap field sejajar dengan label
 *   field satu kolom di atasnya (x sama) dan berada di atas kontrolnya.
 * - L6: trigger select tinggi 40 dengan nilai satu baris; NIK tersamar tidak patah baris.
 * - Tanpa overflow horizontal dokumen di lebar mana pun.
 *
 * Data sintetis; peran disematkan lewat seam `?peran=` (build `--mode playwright`).
 */
const WIDTHS = [1440, 768, 390] as const;

const PROVIDER_ID = '00000000-0000-4000-8000-0000000000a1';
const UP_ID = '00000000-0000-4000-8000-0000000000b1';
const DOWN_ID = '00000000-0000-4000-8000-0000000000c1';
const ENDPOINT_ID = '00000000-0000-4000-8000-0000000000c5';
const CLIENT_ID = '00000000-0000-4000-8000-0000000000e0';

const LOG_PAGE = {
  baris: [
    {
      kode_transaksi: 'TRX-2026-0042',
      client_id: CLIENT_ID,
      client_nama: 'Aplikasi Contoh A',
      queue_id: 'a3f1c2d4-0000-4000-8000-000000000001',
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
    upstream_pool_id: UP_ID,
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

const PROVIDER = {
  provider_id: PROVIDER_ID,
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

const UPSTREAM = {
  upstream_pool_id: UP_ID,
  provider_id: PROVIDER_ID,
  aktif: true,
  ambang_berdampingan: 4,
  batas_percobaan: 3,
  jeda_min: 200,
  jeda_maks: 800,
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
  provider_id: PROVIDER_ID,
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

const SECRETS = [{ nama: 'USER_ID', tersamar: true, dipakai_oleh_provider_count: 1 }];

const CLIENT = {
  client_id: CLIENT_ID,
  nama: 'Aplikasi Contoh A',
  aktif: true,
  limiter_penolakan_burst: 20,
  limiter_penolakan_isi_ulang_per_detik: 1,
};

const CREDS = [
  {
    credential_id: '00000000-0000-4000-8000-0000000000d1',
    jenis: 'oauth2',
    status: 'aktif',
    kedaluwarsa_at: '2027-01-05T03:00:00Z',
  },
];

const WEBHOOK = { aktif: true, webhook_url: 'https://client.example.test/hook', secret_versi: 2 };

const WEBHOOK_LOG = [
  {
    request_id: '00000000-0000-4000-8000-0000000000f1',
    sent_at: '2026-11-03T07:30:00.000Z',
    hasil: 'gagal_http',
    http_status: 503,
    error_class: 'provider_sementara',
    secret_versi: 2,
  },
];

/** Satu stub untuk semua path admin; jawaban dipilih dari path, bukan urutan pemanggilan. */
async function stubSemua(page: Page, peran: string): Promise<void> {
  await page.route('**/admin/v1/**', (route) => {
    const path = new URL(route.request().url()).pathname.replace('/admin/v1', '');
    const json = (body: unknown) =>
      route.fulfill({
        status: 200,
        headers: { 'content-type': 'application/json', 'cache-control': 'no-store' },
        body: JSON.stringify(body),
      });
    if (path.startsWith('/auditor/audit-akses')) return json(AUDIT_PAGE);
    if (path.startsWith('/kuota')) return json(QUOTA);
    if (/^\/upstream-pools\/[^/]+$/.test(path)) return json(UPSTREAM);
    if (path.startsWith('/upstream-pools')) return json([UPSTREAM]);
    if (/^\/downstream-pools\/[^/]+$/.test(path)) return json(DOWNSTREAM);
    if (path.startsWith('/downstream-pools')) return json([DOWNSTREAM]);
    if (/^\/providers\/[^/]+$/.test(path)) return json(PROVIDER);
    if (path.startsWith('/providers')) return json([PROVIDER]);
    if (/^\/endpoints\/[^/]+$/.test(path)) return json(ENDPOINT);
    if (path.startsWith('/endpoints')) return json([ENDPOINT]);
    if (path.startsWith('/secrets')) return json(SECRETS);
    if (/^\/clients\/[^/]+\/credentials$/.test(path)) return json(CREDS);
    if (/^\/clients\/[^/]+\/webhook\/log$/.test(path)) return json(WEBHOOK_LOG);
    if (/^\/clients\/[^/]+\/webhook$/.test(path)) return json(WEBHOOK);
    if (/^\/clients\/[^/]+$/.test(path)) return json(CLIENT);
    if (path.startsWith('/clients')) return json([CLIENT]);
    return json(LOG_PAGE);
  });
  await stubSesi(page, { peran });
}

interface Rute {
  path: string;
  peran: string;
  /** Rute form: wajib punya minimal satu grid field L4. */
  form?: true;
}

const ADMIN = 'platform_admin';
const RUTE: Rute[] = [
  { path: '/', peran: 'operator' },
  { path: '/log/TRX-2026-0042', peran: 'operator' },
  { path: '/nik', peran: 'auditor' },
  { path: '/audit', peran: 'auditor' },
  { path: '/kuota', peran: 'operator' },
  { path: '/provider', peran: ADMIN },
  { path: `/provider/${PROVIDER_ID}`, peran: ADMIN, form: true },
  { path: '/provider/baru', peran: ADMIN, form: true },
  { path: '/endpoint', peran: ADMIN },
  { path: `/endpoint/${ENDPOINT_ID}`, peran: ADMIN, form: true },
  { path: '/endpoint/baru', peran: ADMIN, form: true },
  { path: '/secret', peran: ADMIN },
  { path: '/client', peran: ADMIN },
  { path: `/client/${CLIENT_ID}`, peran: ADMIN, form: true },
  { path: `/client/${CLIENT_ID}/kredensial`, peran: ADMIN },
  { path: `/client/${CLIENT_ID}/webhook`, peran: ADMIN, form: true },
  { path: '/upstream', peran: ADMIN },
  { path: `/upstream/${UP_ID}`, peran: ADMIN, form: true },
  { path: '/upstream/baru', peran: ADMIN, form: true },
  { path: '/downstream', peran: ADMIN },
  { path: `/downstream/${DOWN_ID}`, peran: ADMIN, form: true },
  { path: '/downstream/baru', peran: ADMIN, form: true },
  { path: '/halaman-tidak-ada', peran: ADMIN },
];

/** Aturan L1/L4 per lebar viewport. */
function aturan(width: number) {
  if (width >= 1200) return { padding: 36, kolom: 3 };
  if (width >= 600) return { padding: 24, kolom: 2 };
  return { padding: 16, kolom: 1 };
}

async function siap(page: Page): Promise<void> {
  await expect(page.getByRole('heading', { level: 1 })).toBeVisible();
  await expect(page.getByText(/^(Memuat…|Loading…)$/).filter({ visible: true })).toHaveCount(0);
  await page.evaluate(() => document.fonts.ready);
}

async function cekOverflow(page: Page, width: number): Promise<void> {
  const luap = await page.evaluate(
    () => document.documentElement.scrollWidth - document.documentElement.clientWidth,
  );
  expect(luap, `dokumen melebar ${luap}px pada ${width}px`).toBeLessThanOrEqual(0);
}

/** L6: trigger select 40 px dan teks nilainya satu baris (tidak membungkus di dalam tombol). */
async function cekSelect(page: Page, label: string): Promise<void> {
  const select = await page.evaluate(() =>
    Array.from(document.querySelectorAll<HTMLElement>('button[aria-haspopup="listbox"]'))
      .filter((b) => b.offsetParent !== null)
      .map((b) => {
        const nilai = b.querySelector<HTMLElement>('span');
        const range = document.createRange();
        if (nilai) range.selectNodeContents(nilai);
        const baris = new Set(Array.from(range.getClientRects()).map((r) => Math.round(r.top)))
          .size;
        return {
          teks: b.textContent ?? '',
          tinggi: Math.round(b.getBoundingClientRect().height),
          baris,
        };
      }),
  );
  for (const s of select) {
    expect(s.tinggi, `tinggi select "${s.teks}" pada ${label}`).toBe(40);
    expect(s.baris, `select "${s.teks}" membungkus pada ${label}`).toBeLessThanOrEqual(1);
  }
}

/** L4: jumlah kolom grid field, label sejajar per kolom, dan label di atas kontrolnya. */
async function cekGrid(page: Page, kolom: number, label: string, wajib: boolean): Promise<void> {
  const grids = await page.evaluate(() =>
    Array.from(document.querySelectorAll<HTMLElement>('[data-layout="field-grid"]'))
      .filter((g) => g.offsetParent !== null)
      .map((g) => {
        const kolomGrid = getComputedStyle(g)
          .gridTemplateColumns.split(' ')
          .filter((v) => v !== '').length;
        const field = Array.from(g.children)
          .filter((c): c is HTMLElement => c instanceof HTMLElement && c.offsetParent !== null)
          .map((c) => {
            // Select menyisipkan <label> native tersembunyi (lebar 1 px) sebelum label yang tampil.
            const lbl = Array.from(c.querySelectorAll('label, span[id]')).find(
              (el) => el.getBoundingClientRect().width > 1,
            );
            const kontrol = c.querySelector('input, textarea, button, [role="group"]');
            const l = lbl?.getBoundingClientRect();
            const k = kontrol?.getBoundingClientRect();
            const span =
              getComputedStyle(c).gridColumnEnd === '-1' ||
              /span/.test(getComputedStyle(c).gridColumnEnd);
            return {
              span,
              adaKontrol: kontrol != null,
              labelX: l ? Math.round(l.left) : null,
              labelBawah: l ? Math.round(l.bottom) : null,
              kontrolAtas: k ? Math.round(k.top) : null,
            };
          });
        return { kolomGrid, field };
      }),
  );
  if (wajib) expect(grids.length, `tidak ada grid field pada ${label}`).toBeGreaterThan(0);
  for (const g of grids) {
    expect(g.kolomGrid, `jumlah kolom grid field pada ${label}`).toBe(kolom);
    // Anak tanpa kontrol (teks bantuan "–" dsb.) bukan field berlabel; hanya field sungguhan yang
    // wajib diperiksa label/sejajar/di-atas.
    const kolomField = g.field.filter((f) => f.adaKontrol);
    const tunggal = kolomField.filter((f) => !f.span);
    // Setiap field di grid wajib punya label terdeteksi (L4): tanpanya, cek sejajar/di-atas tidak
    // berarti apa-apa, jadi field tanpa label harus gagal di sini, bukan dilewati diam-diam.
    for (const f of kolomField) {
      expect(f.labelX, `field tanpa label terdeteksi pada ${label}`).not.toBeNull();
    }
    for (let i = kolom; i < tunggal.length; i += 1) {
      const atas = tunggal[i - kolom];
      const ini = tunggal[i];
      expect(
        Math.abs(ini!.labelX! - atas!.labelX!),
        `x label kolom tidak sejajar pada ${label}`,
      ).toBeLessThanOrEqual(1);
    }
    for (const f of kolomField) {
      expect(f.labelBawah, `label tidak di atas field pada ${label}`).toBeLessThanOrEqual(
        f.kontrolAtas!,
      );
    }
  }
}

for (const width of WIDTHS) {
  test.describe(`layout ui-v3 @${width}`, () => {
    const { padding, kolom } = aturan(width);

    for (const rute of RUTE) {
      test(`${rute.path} @${width}`, async ({ page }) => {
        const label = `${rute.path}@${width}`;
        await stubSemua(page, rute.peran);
        await page.setViewportSize({ width, height: 900 });
        await page.goto(`${rute.path}?peran=${rute.peran}`);
        await siap(page);

        // L1: padding konten dari shell dan lebar isi maksimal 1280.
        const shell = await page.evaluate(() => {
          const main = document.querySelector<HTMLElement>('main#main-content');
          if (!main) return null;
          const s = getComputedStyle(main);
          const isi = main.firstElementChild?.getBoundingClientRect();
          return {
            kiri: parseFloat(s.paddingLeft),
            kanan: parseFloat(s.paddingRight),
            lebarIsi: isi ? Math.round(isi.width) : 0,
          };
        });
        expect(shell, `main#main-content tidak ada pada ${label}`).not.toBeNull();
        expect(shell?.kiri, `padding kiri konten pada ${label}`).toBe(padding);
        expect(shell?.kanan, `padding kanan konten pada ${label}`).toBe(padding);
        expect(shell?.lebarIsi, `lebar isi pada ${label}`).toBeLessThanOrEqual(1280);

        await cekOverflow(page, width);
        await cekGrid(page, kolom, label, rute.form === true);
        await cekSelect(page, label);
      });
    }

    test(`L6 NIK tersamar satu baris @${width}`, async ({ page }) => {
      await stubSemua(page, 'operator');
      await page.setViewportSize({ width, height: 900 });
      await page.goto('/?peran=operator');
      await siap(page);
      // Di compact tabel jadi kartu dan sel membawa label kolom, jadi sel dicari dari teksnya.
      const sel = page.locator('td', { hasText: '3175********' }).first();
      await expect(sel).toBeVisible();
      const baris = await sel.evaluate((td) => {
        const range = document.createRange();
        range.selectNodeContents(td);
        // Kotak elemen dan teks di baris yang sama bisa beda 1 px; baris baru selisih ≥ tinggi baris.
        const atas = Array.from(range.getClientRects())
          .map((r) => r.top)
          .sort((x, y) => x - y);
        return atas.filter((t, i) => i === 0 || t - atas[i - 1]! > 4).length;
      });
      expect(baris, `NIK patah baris pada ${width}px`).toBe(1);
    });

    test(`login @${width}`, async ({ page }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.goto('/login');
      await expect(page.getByRole('heading', { level: 1 })).toBeVisible();
      await page.evaluate(() => document.fonts.ready);
      await cekOverflow(page, width);
    });
  });
}
