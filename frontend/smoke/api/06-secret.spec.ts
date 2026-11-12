import { test, expect, inventaris } from '../fixtures';
import { NAMA, state } from '../state';

/**
 * Grup Secret (spec smoke-test bagian 2): `putSecret` (204, G2), `listSecrets` (200, tanpa
 * nilai), `deleteSecret` (204, G5). Tanpa `getSecret`/`updateSecret`: tidak ada di kontrak
 * (write-only, nilai tidak pernah dibaca kembali — Keamanan).
 */
test.describe('@smoke secret', () => {
  test.use({ peran: 'platform-admin' });

  test('@smoke putSecret: 204', async ({ sesiApi }) => {
    inventaris('putSecret', 'platform-admin');
    const res = await sesiApi.put(`/admin/v1/secrets/${NAMA.secret}`, {
      data: { nilai: 'nilai-sintetis-smoke' },
    });
    expect(res.status()).toBe(204);
    state.secretNama = NAMA.secret;
  });

  test('@smoke listSecrets: 200, memuat smoke-secret tanpa nilai', async ({ sesiApi }) => {
    inventaris('listSecrets', 'platform-admin');
    const res = await sesiApi.get('/admin/v1/secrets');
    expect(res.status()).toBe(200);
    const list = (await res.json()) as Array<Record<string, unknown>>;
    const baris = list.find((s) => s.nama === NAMA.secret);
    expect(baris).toBeTruthy();
    expect(baris).not.toHaveProperty('nilai');
  });

  test('@smoke deleteSecret: 204', async ({ sesiApi }) => {
    inventaris('deleteSecret', 'platform-admin');
    const res = await sesiApi.delete(`/admin/v1/secrets/${state.secretNama}`);
    expect(res.status()).toBe(204);
  });
});
