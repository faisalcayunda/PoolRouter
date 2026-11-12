import { test, expect, inventaris } from '../fixtures';

/**
 * Gelombang G6 (spec smoke-test bagian 2): `logout`, dites terakhir (nama file `99-`) karena
 * menghabiskan token sesi operator dari fixture SQL yang dipakai berulang oleh grup G2-G5 lain
 * (satu worker, urutan file menjaga gelombang — spec Testing Decisions).
 */
test.describe('@smoke logout (G6)', () => {
  test.use({ peran: 'operator' });

  test(
    '@smoke logout: 204 lalu getMe 401',
    { annotation: inventaris('logout', 'operator') },
    async ({ sesiApi }) => {
      const resLogout = await sesiApi.post('/admin/v1/auth/logout');
      expect(resLogout.status()).toBe(204);
      const resMe = await sesiApi.get('/admin/v1/auth/me');
      expect(resMe.status()).toBe(401);
    },
  );
});
