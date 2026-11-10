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
      // `__Host-` mewajibkan Secure, Path=/ dan tanpa Domain; Chromium menerima Secure pada loopback.
      await context.addCookies([
        {
          name: COOKIE_SESI,
          value: token,
          url: baseURL,
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
