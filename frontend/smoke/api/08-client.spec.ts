import { test, expect, inventaris } from '../fixtures';
import { NAMA, state } from '../state';

/**
 * Grup Client, bagian G2 (spec smoke-test bagian 2): create/update/setClientEndpoints/list/get
 * (PA), listClients/getClient (CO, lewat client seed `Client Sintetis Seed` miliknya sendiri -
 * `seed-client-owner` tidak melihat `smoke-client`), createCredential (oauth2 + api_key),
 * listCredentials, getWebhookConfig/putWebhookConfig/rotateWebhookSecret. `listWebhookLog` (G4,
 * butuh transaksi T) ada di 10-log.spec.ts sesudah grup Downstream API (G3) membuat T.
 */
test.describe('@smoke client', () => {
  test.describe('PA', () => {
    test.use({ peran: 'platform-admin' });

    test(
      '@smoke createClient: 201, simpan client_id',
      { annotation: inventaris('createClient', 'platform-admin') },
      async ({ sesiApi }) => {
        const res = await sesiApi.post('/admin/v1/clients', { data: { nama: NAMA.client } });
        expect(res.status()).toBe(201);
        const body = (await res.json()) as { client_id: string };
        expect(body.client_id).toBeTruthy();
        state.clientId = body.client_id;
      },
    );

    test(
      '@smoke getClient: 200, nama sesuai',
      { annotation: inventaris('getClient', 'platform-admin') },
      async ({ sesiApi }) => {
        const res = await sesiApi.get(`/admin/v1/clients/${state.clientId}`);
        expect(res.status()).toBe(200);
        expect((await res.json()) as { nama: string }).toMatchObject({ nama: NAMA.client });
      },
    );

    test(
      '@smoke updateClient: 200',
      { annotation: inventaris('updateClient', 'platform-admin') },
      async ({ sesiApi }) => {
        const res = await sesiApi.put(`/admin/v1/clients/${state.clientId}`, {
          data: {
            nama: NAMA.client,
            limiter_penolakan_burst: 30,
            alasan: 'Smoke ARC-378: update limiter client sintetis.',
          },
        });
        expect(res.status()).toBe(200);
      },
    );

    test(
      '@smoke setClientEndpoints: 200',
      { annotation: inventaris('setClientEndpoints', 'platform-admin') },
      async ({ sesiApi }) => {
        const res = await sesiApi.put(`/admin/v1/clients/${state.clientId}/endpoints`, {
          data: { endpoint_ids: [state.endpointId] },
        });
        expect(res.status()).toBe(200);
      },
    );

    test(
      '@smoke createCredential: 201 oauth2, 201 api_key',
      { annotation: inventaris('createCredential', 'platform-admin') },
      async ({ sesiApi }) => {
        const resOauth2 = await sesiApi.post(`/admin/v1/clients/${state.clientId}/credentials`, {
          data: { jenis: 'oauth2' },
        });
        expect(resOauth2.status()).toBe(201);
        const oauth2 = (await resOauth2.json()) as { credential_id: string; secret: string };
        state.credentialOauth2Id = oauth2.credential_id;
        state.credentialOauth2Secret = oauth2.secret;

        const resApiKey = await sesiApi.post(`/admin/v1/clients/${state.clientId}/credentials`, {
          data: { jenis: 'api_key' },
        });
        expect(resApiKey.status()).toBe(201);
        const apiKey = (await resApiKey.json()) as { credential_id: string; secret: string };
        state.credentialApiKeyId = apiKey.credential_id;
        state.credentialApiKey = apiKey.secret;
      },
    );

    test(
      '@smoke listCredentials: 200, memuat dua kredensial G2',
      { annotation: inventaris('listCredentials', 'platform-admin') },
      async ({ sesiApi }) => {
        const res = await sesiApi.get(`/admin/v1/clients/${state.clientId}/credentials`);
        expect(res.status()).toBe(200);
        const list = (await res.json()) as Array<{ credential_id: string }>;
        expect(list.some((c) => c.credential_id === state.credentialOauth2Id)).toBe(true);
        expect(list.some((c) => c.credential_id === state.credentialApiKeyId)).toBe(true);
      },
    );

    test(
      '@smoke getWebhookConfig: 200',
      { annotation: inventaris('getWebhookConfig', 'platform-admin') },
      async ({ sesiApi }) => {
        const res = await sesiApi.get(`/admin/v1/clients/${state.clientId}/webhook`);
        expect(res.status()).toBe(200);
        expect((await res.json()) as { aktif: boolean }).toHaveProperty('aktif');
      },
    );

    test(
      '@smoke putWebhookConfig: 200',
      { annotation: inventaris('putWebhookConfig', 'platform-admin') },
      async ({ sesiApi }) => {
        const res = await sesiApi.put(`/admin/v1/clients/${state.clientId}/webhook`, {
          data: { webhook_url: `https://${state.allowlistDomain}/webhook`, aktif: true },
        });
        expect(res.status()).toBe(200);
        expect((await res.json()) as { aktif: boolean }).toMatchObject({ aktif: true });
      },
    );

    test(
      '@smoke rotateWebhookSecret: 200',
      { annotation: inventaris('rotateWebhookSecret', 'platform-admin') },
      async ({ sesiApi }) => {
        const res = await sesiApi.post(`/admin/v1/clients/${state.clientId}/webhook/rotasi-secret`);
        expect(res.status()).toBe(200);
        const body = (await res.json()) as { secret_versi: number; secret: string };
        expect(body.secret_versi).toBeGreaterThanOrEqual(1);
        expect(body.secret).toBeTruthy();
      },
    );
  });

  test.describe('CO (client seed)', () => {
    test.use({ peran: 'client-owner' });

    test(
      '@smoke listClients dan getClient: 200 untuk client owner (client seed sendiri)',
      { annotation: inventaris(['listClients', 'getClient'], 'client-owner') },
      async ({ sesiApi }) => {
        const resList = await sesiApi.get('/admin/v1/clients');
        expect(resList.status()).toBe(200);
        const list = (await resList.json()) as Array<{ client_id: string }>;
        expect(list.length).toBeGreaterThan(0);

        const resGet = await sesiApi.get(`/admin/v1/clients/${list[0]?.client_id}`);
        expect(resGet.status()).toBe(200);
      },
    );
  });
});
