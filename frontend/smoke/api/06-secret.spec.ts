import { test, expect, inventaris } from '../fixtures';
import { state } from '../state';

// Nama secret = kunci placeholder `{{secret.<nama>}}`: huruf, angka, `_` saja
// (backend admin/secret.rs periksa_nama); `NAMA.secret` (`smoke-secret`) ditolak 422.
const NAMA_SECRET = 'smoke_secret';

/**
 * Grup Secret (spec smoke-test bagian 2): `putSecret` (204, G2), `listSecrets` (200, tanpa
 * nilai), `deleteSecret` (204, G5). Tanpa `getSecret`/`updateSecret`: tidak ada di kontrak
 * (write-only, nilai tidak pernah dibaca kembali — Keamanan).
 */
test.describe('@smoke secret', () => {
  test.use({ peran: 'platform-admin' });

  test(
    '@smoke putSecret: 204',
    { annotation: inventaris('putSecret', 'platform-admin') },
    async ({ sesiApi }) => {
      const res = await sesiApi.put(`/admin/v1/secrets/${NAMA_SECRET}`, {
        data: { nilai: 'nilai-sintetis-smoke' },
      });
      expect(res.status()).toBe(204);
      state.secretNama = NAMA_SECRET;
    },
  );

  test(
    '@smoke listSecrets: 200, memuat smoke-secret tanpa nilai',
    { annotation: inventaris('listSecrets', 'platform-admin') },
    async ({ sesiApi }) => {
      const res = await sesiApi.get('/admin/v1/secrets');
      expect(res.status()).toBe(200);
      const list = (await res.json()) as Array<Record<string, unknown>>;
      const baris = list.find((s) => s.nama === NAMA_SECRET);
      expect(baris).toBeTruthy();
      expect(baris).not.toHaveProperty('nilai');
    },
  );

  test(
    '@smoke deleteSecret: 204',
    { annotation: inventaris('deleteSecret', 'platform-admin') },
    async ({ sesiApi }) => {
      const res = await sesiApi.delete(`/admin/v1/secrets/${state.secretNama}`);
      expect(res.status()).toBe(204);
    },
  );
});
