import type { PlaywrightWorkerArgs } from '@playwright/test';
import { test, expect, inventaris, tokenUntuk, ORIGIN, COOKIE_SESI } from '../fixtures';
import { totp } from '../totp';

/**
 * Gelombang G1 (spec smoke-test bagian 2, inventaris.md): satu-satunya tempat yang menguji login
 * + MFA sungguhan lewat HTTP. `seed-operator` tidak wajib MFA (ADR-0006 §9: hanya platform_admin
 * dan auditor); `seed-platform-admin` sudah terdaftar MFA (Keamanan root ARC-376 asumsi) jadi
 * dipakai untuk jalur `langkah=mfa`. `seed-penyetuju` belum terdaftar MFA tapi punya baris
 * `kode_daftar_mfa` dari fixture SQL (Q4) jadi dipakai untuk jalur `langkah=daftar_mfa` +
 * `mfaDaftar` + `mfaKonfirmasi`.
 *
 * Tiap test di sini memakai context request baru tanpa cookie sesi dari fixture (bukan `sesiApi`),
 * karena justru membuktikan cookie yang diterbitkan backend sendiri.
 */
const SEED_PASSWORD = process.env.E2E_SEED_PASSWORD;
const KODE_DAFTAR_MFA = process.env.E2E_KODE_DAFTAR_MFA;

function ctxBaru(playwright: PlaywrightWorkerArgs['playwright']) {
  return playwright.request.newContext({ baseURL: ORIGIN, extraHTTPHeaders: { Origin: ORIGIN } });
}

test.describe('@smoke auth', () => {
  test('@smoke login: 204 untuk peran tanpa MFA wajib (operator)', async ({ playwright }) => {
    inventaris('login', 'operator');
    const ctx = await ctxBaru(playwright);
    const res = await ctx.post('/admin/v1/auth/login', {
      data: { username: 'seed-operator', password: SEED_PASSWORD },
    });
    expect(res.status()).toBe(204);
    const cookie = (res.headers()['set-cookie'] ?? '').toString();
    expect(cookie).toContain(COOKIE_SESI);
    await ctx.dispose();
  });

  test('@smoke login: 200 langkah mfa untuk peran MFA wajib yang sudah terdaftar (platform admin)', async ({
    playwright,
  }) => {
    inventaris('login', 'platform-admin');
    const ctx = await ctxBaru(playwright);
    const res = await ctx.post('/admin/v1/auth/login', {
      data: { username: 'seed-platform-admin', password: SEED_PASSWORD },
    });
    expect(res.status()).toBe(200);
    const body = (await res.json()) as { langkah: string };
    expect(body.langkah).toBe('mfa');
    await ctx.dispose();
  });

  test('@smoke mfaDaftar, mfaKonfirmasi, logout, lalu mfaVerifikasi (penyetuju, satu ctx)', async ({
    playwright,
  }) => {
    // Satu context request (satu cookie jar) dipakai seluruh rantai: cookie pra-sesi
    // __Host-pr_mfa dari login pertama dipakai mfaDaftar/mfaKonfirmasi; sesudah logout, login
    // kedua balik ke langkah=mfa dan dibuktikan lewat mfaVerifikasi dengan TOTP dari rahasia
    // yang sama (spec bagian 3: TOTP dihitung di test dengan node:crypto).
    const ctx = await ctxBaru(playwright);

    const loginRes = await ctx.post('/admin/v1/auth/login', {
      data: { username: 'seed-penyetuju', password: SEED_PASSWORD },
    });
    expect(loginRes.status()).toBe(200);
    expect(((await loginRes.json()) as { langkah: string }).langkah).toBe('daftar_mfa');

    inventaris('mfaDaftar', 'platform-admin');
    const daftarRes = await ctx.post('/admin/v1/auth/mfa/daftar', {
      data: { kode_pendaftaran: KODE_DAFTAR_MFA },
    });
    expect(daftarRes.status()).toBe(200);
    const daftar = (await daftarRes.json()) as { otpauth_uri: string; rahasia_base32: string };
    expect(daftar.rahasia_base32).toBeTruthy();

    inventaris('mfaKonfirmasi', 'platform-admin');
    const konfirmasiRes = await ctx.post('/admin/v1/auth/mfa/daftar/konfirmasi', {
      data: { kode_totp: totp(daftar.rahasia_base32) },
    });
    expect(konfirmasiRes.status()).toBe(200);
    const konfirmasi = (await konfirmasiRes.json()) as { kode_pemulihan: string[] };
    expect(konfirmasi.kode_pemulihan).toHaveLength(10);
    expect((konfirmasiRes.headers()['set-cookie'] ?? '').toString()).toContain(COOKIE_SESI);

    const logoutRes = await ctx.post('/admin/v1/auth/logout');
    expect(logoutRes.status()).toBe(204);

    const loginKeduaRes = await ctx.post('/admin/v1/auth/login', {
      data: { username: 'seed-penyetuju', password: SEED_PASSWORD },
    });
    expect(loginKeduaRes.status()).toBe(200);
    expect(((await loginKeduaRes.json()) as { langkah: string }).langkah).toBe('mfa');

    inventaris('mfaVerifikasi', 'platform-admin');
    const verifikasiRes = await ctx.post('/admin/v1/auth/mfa', {
      data: { kode: totp(daftar.rahasia_base32), kode_pemulihan: null },
    });
    expect(verifikasiRes.status()).toBe(204);
    expect((verifikasiRes.headers()['set-cookie'] ?? '').toString()).toContain(COOKIE_SESI);
    await ctx.dispose();
  });

  test('@smoke getMe: 200 untuk tiap peran seed, peran dan client_id sesuai', async ({
    sesiApi,
  }) => {
    inventaris('getMe', 'operator');
    const res = await sesiApi.get('/admin/v1/auth/me');
    expect(res.status()).toBe(200);
    const me = (await res.json()) as { peran: string[]; mfa: { wajib: boolean; aktif: boolean } };
    expect(me.peran).toContain('operator');
    expect(me.mfa.wajib).toBe(false);
  });

  test('@smoke getSesi: 200, field wajib ada', async ({ sesiApi }) => {
    inventaris('getSesi', 'operator');
    const res = await sesiApi.get('/admin/v1/auth/sesi');
    expect(res.status()).toBe(200);
    const sesi = (await res.json()) as Record<string, unknown>;
    expect(sesi).toHaveProperty('absolut_berakhir_pada');
    expect(sesi).toHaveProperty('idle_berakhir_pada');
    expect(sesi).toHaveProperty('waktu_server');
  });

  test('@smoke perpanjangSesi: 200, idle direset, tanpa Set-Cookie', async ({ sesiApi }) => {
    inventaris('perpanjangSesi', 'operator');
    const res = await sesiApi.post('/admin/v1/auth/sesi/perpanjang');
    expect(res.status()).toBe(200);
    expect(res.headers()['set-cookie']).toBeUndefined();
  });

  test('@smoke updatePreferensi: 200, bahasa_ui tersimpan', async ({ sesiApi }) => {
    inventaris('updatePreferensi', 'operator');
    const res = await sesiApi.patch('/admin/v1/auth/me/preferensi', {
      data: { bahasa_ui: 'id' },
    });
    expect(res.status()).toBe(200);
    expect((await res.json()) as { bahasa_ui: string }).toMatchObject({ bahasa_ui: 'id' });
  });

  test('@smoke logout: 204 lalu getMe 401 (G6)', async ({ playwright }) => {
    inventaris('logout', 'operator');
    const token = tokenUntuk('operator');
    const ctx = await playwright.request.newContext({
      baseURL: ORIGIN,
      extraHTTPHeaders: {
        Origin: ORIGIN,
        ...(token ? { Cookie: `${COOKIE_SESI}=${token}` } : {}),
      },
    });
    const resLogout = await ctx.post('/admin/v1/auth/logout');
    expect(resLogout.status()).toBe(204);
    const resMe = await ctx.get('/admin/v1/auth/me');
    expect(resMe.status()).toBe(401);
    await ctx.dispose();
  });
});
