import { test, expect, inventaris } from '../fixtures';
import { state } from '../state';

/**
 * Grup G5 sisa (spec smoke-test bagian 2, inventaris.md), semua PA: pratinjauNonaktifkanClient →
 * nonaktifkanClient → aktifkanClient (data=K, client `state.clientId`); pratinjauCabutCredential →
 * cabutCredential (data=Ka, kredensial api_key - bukan oauth2, agar kredensial yang dipakai
 * `09-downstream-api.spec.ts`/`10-log.spec.ts` tidak ikut tercabut) → aktifkanCredential (409,
 * selalu ditolak untuk kredensial yang sudah dicabut - U-54b);
 * deleteWebhookAllowlistEntry (204, data=W, `state.allowlistDomain`: webhook client sudah
 * dialihkan ke domain lain di `10-log.spec.ts`, jadi domain ini aman dihapus di sini, terakhir
 * dalam urutan file).
 */
test.describe('@smoke g5-sisa', () => {
  test.use({ peran: 'platform-admin' });

  test(
    '@smoke pratinjauNonaktifkanClient: 200',
    { annotation: inventaris('pratinjauNonaktifkanClient', 'platform-admin') },
    async ({ sesiApi }) => {
      const res = await sesiApi.get(`/admin/v1/clients/${state.clientId}/pratinjau-nonaktif`);
      expect(res.status()).toBe(200);
      expect((await res.json()) as { akan_cancelled: number }).toHaveProperty('akan_cancelled');
    },
  );

  test(
    '@smoke nonaktifkanClient: 200',
    { annotation: inventaris('nonaktifkanClient', 'platform-admin') },
    async ({ sesiApi }) => {
      const res = await sesiApi.post(`/admin/v1/clients/${state.clientId}/nonaktifkan`, {
        data: { alasan: 'Smoke ARC-378: nonaktifkan client sintetis.' },
      });
      expect(res.status()).toBe(200);
      expect((await res.json()) as { jumlah_cancelled: number }).toHaveProperty('jumlah_cancelled');
    },
  );

  test(
    '@smoke aktifkanClient: 200, aktif kembali',
    { annotation: inventaris('aktifkanClient', 'platform-admin') },
    async ({ sesiApi }) => {
      const res = await sesiApi.post(`/admin/v1/clients/${state.clientId}/aktifkan`);
      expect(res.status()).toBe(200);
      expect((await res.json()) as { aktif: boolean }).toMatchObject({ aktif: true });
    },
  );

  test(
    '@smoke pratinjauCabutCredential: 200',
    { annotation: inventaris('pratinjauCabutCredential', 'platform-admin') },
    async ({ sesiApi }) => {
      const res = await sesiApi.get(
        `/admin/v1/credentials/${state.credentialApiKeyId}/pratinjau-cabut`,
      );
      expect(res.status()).toBe(200);
      expect((await res.json()) as { akan_cancelled: number }).toHaveProperty('akan_cancelled');
    },
  );

  test(
    '@smoke cabutCredential: 200, status dicabut',
    { annotation: inventaris('cabutCredential', 'platform-admin') },
    async ({ sesiApi }) => {
      const res = await sesiApi.post(`/admin/v1/credentials/${state.credentialApiKeyId}/cabut`, {
        data: { alasan: 'Smoke ARC-378: cabut kredensial api_key sintetis.' },
      });
      expect(res.status()).toBe(200);
      const body = (await res.json()) as { status: string };
      expect(body.status).toBe('dicabut');
    },
  );

  test(
    '@smoke aktifkanCredential: 409, kredensial dicabut selalu ditolak',
    { annotation: inventaris('aktifkanCredential', 'platform-admin') },
    async ({ sesiApi }) => {
      const res = await sesiApi.post(`/admin/v1/credentials/${state.credentialApiKeyId}/aktifkan`);
      expect(res.status()).toBe(409);
    },
  );

  test(
    '@smoke deleteWebhookAllowlistEntry: 204',
    { annotation: inventaris('deleteWebhookAllowlistEntry', 'platform-admin') },
    async ({ sesiApi }) => {
      const res = await sesiApi.delete(`/admin/v1/webhook-allowlist/${state.allowlistDomain}`);
      expect(res.status()).toBe(204);
    },
  );
});
