import { test, expect, inventaris } from '../fixtures';
import { NAMA, state } from '../state';

/**
 * Grup Allowlist, bagian G2 (spec smoke-test bagian 2): `putWebhookAllowlistEntry` (200),
 * `listWebhookAllowlist` (200). `deleteWebhookAllowlistEntry` (G5) ada di
 * `frontend/smoke/api/90-g5-sisa.spec.ts` karena domainnya masih dipakai grup Client/Webhook
 * (G2-G4) sesudah file ini.
 */
test.describe('@smoke allowlist', () => {
  test.use({ peran: 'platform-admin' });

  test(
    '@smoke putWebhookAllowlistEntry: 200',
    { annotation: inventaris('putWebhookAllowlistEntry', 'platform-admin') },
    async ({ sesiApi }) => {
      const res = await sesiApi.put(`/admin/v1/webhook-allowlist/${NAMA.allowlistDomain}`, {
        data: { aktif: true },
      });
      expect(res.status()).toBe(200);
      const body = (await res.json()) as { domain: string; aktif: boolean };
      expect(body).toMatchObject({ domain: NAMA.allowlistDomain, aktif: true });
      state.allowlistDomain = NAMA.allowlistDomain;
    },
  );

  test(
    '@smoke listWebhookAllowlist: 200, memuat domain G2',
    { annotation: inventaris('listWebhookAllowlist', 'platform-admin') },
    async ({ sesiApi }) => {
      const res = await sesiApi.get('/admin/v1/webhook-allowlist');
      expect(res.status()).toBe(200);
      const list = (await res.json()) as Array<{ domain: string }>;
      expect(list.some((w) => w.domain === state.allowlistDomain)).toBe(true);
    },
  );
});
