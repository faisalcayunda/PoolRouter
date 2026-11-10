import { test as base, expect } from '../e2e/fixtures';

/**
 * Fixture `@api-asli` (ARC-251, spec ui-v2-layar AC-4): pengguna uji dibuat lewat SQL fixture oleh
 * `scripts/e2e-api-asli.sh` (baris `admin_session` untuk pengguna seed platform admin, MFA sudah
 * terverifikasi), bukan lewat mock. Token sesi datang dari E2E_SESSION_TOKEN, tidak pernah dari repo.
 * Tanpa token tes jatuh ke layar login, jadi tes gagal pada asersinya (bukan error koneksi).
 */
export const COOKIE_SESI = '__Host-pr_session';

export const test = base.extend({
  context: async ({ context, baseURL }, use) => {
    const token = process.env.E2E_SESSION_TOKEN;
    if (token && baseURL) {
      // `__Host-` mewajibkan Secure, Path=/ dan host-only. Bentuk `url` tidak dipakai: Playwright 1.64
      // menimpa `secure` menjadi (protocol === 'https:'), jadi cookie ini jatuh pada http://127.0.0.1.
      // Bentuk domain/path tidak menimpa `secure`. Apakah Chromium menerimanya di loopback http
      // dibuktikan oleh tes /auth/me pada run sungguhan (belum dijalankan; lihat catatan ARC-251).
      await context.addCookies([
        {
          name: COOKIE_SESI,
          value: token,
          domain: new URL(baseURL).hostname,
          path: '/',
          httpOnly: true,
          secure: true,
          sameSite: 'Strict',
        },
      ]);
    }
    await use(context);
  },
});

export { expect };
