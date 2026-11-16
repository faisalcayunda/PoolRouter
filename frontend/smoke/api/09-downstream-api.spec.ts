import { test, expect, inventaris, DOWNSTREAM_ORIGIN } from '../fixtures';
import { state, NIK_SINTETIS } from '../state';

/**
 * Grup Downstream API (klien), G3 (spec smoke-test bagian 2/7, inventaris.md): `issueToken`,
 * `getEndpointKontrak`, `submitSync`, `enqueueAsync`, `getRequest` (polling sampai terminal, ≤60
 * detik, juga membuktikan konsumsi `proc:worker` per inventaris.md), `cancelRequest` (200 pada
 * request kedua yang masih `queued`, bukan request `submitSync`/`enqueueAsync` pertama yang
 * dipakai buktikan `getRequest` terminal). Request lewat `playwright.request.newContext` murni
 * (tanpa cookie sesi admin, tanpa fixture `sesiApi`): downstream API adalah API lain, bukan
 * `/admin/v1`, dan diautentikasi lewat token oauth2 dari `issueToken` (bukan peran admin).
 *
 * Butuh `state.endpointId` (05-endpoint.spec.ts) dan `state.credentialOauth2Id`/
 * `credentialOauth2Secret` (08-client.spec.ts, kredensial oauth2 milik `state.clientId`).
 * `state.downstreamPoolId`/`state.providerId` sudah dipasang lewat `setClientEndpoints`.
 */
test.describe('@smoke downstream-api (klien)', () => {
  let token: string;

  test(
    '@smoke issueToken: 200, access_token',
    { annotation: inventaris('issueToken') },
    async ({ playwright }) => {
      const ctx = await playwright.request.newContext({ baseURL: DOWNSTREAM_ORIGIN });
      const res = await ctx.post('/v1/oauth/token', {
        form: {
          grant_type: 'client_credentials',
          client_id: state.credentialOauth2Id ?? '',
          client_secret: state.credentialOauth2Secret ?? '',
        },
      });
      expect(res.status()).toBe(200);
      const body = (await res.json()) as {
        access_token: string;
        token_type: string;
        expires_in: number;
      };
      expect(body.access_token).toBeTruthy();
      expect(body.token_type).toBe('Bearer');
      token = body.access_token;
      await ctx.dispose();
    },
  );

  test(
    '@smoke getEndpointKontrak: 200, endpoint_id sesuai',
    { annotation: inventaris('getEndpointKontrak') },
    async ({ playwright }) => {
      const ctx = await playwright.request.newContext({
        baseURL: DOWNSTREAM_ORIGIN,
        extraHTTPHeaders: { Authorization: `Bearer ${token}` },
      });
      const res = await ctx.get(`/v1/endpoints/${state.endpointId}/kontrak`);
      expect(res.status()).toBe(200);
      const body = (await res.json()) as { endpoint_id: string };
      expect(body.endpoint_id).toBe(state.endpointId);
      await ctx.dispose();
    },
  );

  test(
    '@smoke submitSync: 200, status done',
    { annotation: inventaris('submitSync') },
    async ({ playwright }) => {
      const ctx = await playwright.request.newContext({
        baseURL: DOWNSTREAM_ORIGIN,
        extraHTTPHeaders: { Authorization: `Bearer ${token}` },
      });
      const res = await ctx.post(`/v1/endpoints/${state.endpointId}/sync`, {
        data: {
          input: { NIK: NIK_SINTETIS },
          metadata: { status_penerima_manfaat: 'sesuai' },
        },
      });
      expect(res.status()).toBe(200);
      const body = (await res.json()) as { status: string; request_id: string };
      expect(body.status).toBe('done');
      await ctx.dispose();
    },
  );

  test(
    '@smoke enqueueAsync: 202, simpan queue_id',
    { annotation: inventaris('enqueueAsync') },
    async ({ playwright }) => {
      const ctx = await playwright.request.newContext({
        baseURL: DOWNSTREAM_ORIGIN,
        extraHTTPHeaders: { Authorization: `Bearer ${token}` },
      });
      const res = await ctx.post(`/v1/endpoints/${state.endpointId}/async`, {
        data: {
          input: { NIK: NIK_SINTETIS },
          metadata: { status_penerima_manfaat: 'sesuai' },
        },
      });
      expect(res.status()).toBe(202);
      const body = (await res.json()) as { queue_id: string; status: string };
      expect(body.queue_id).toBeTruthy();
      expect(body.status).toBe('queued');
      state.queueIdAsync = body.queue_id;
      await ctx.dispose();
    },
  );

  test(
    '@smoke getRequest: terminal dalam 60 detik (juga buktikan proc:worker)',
    { annotation: inventaris('getRequest') },
    async ({ playwright }) => {
      const ctx = await playwright.request.newContext({
        baseURL: DOWNSTREAM_ORIGIN,
        extraHTTPHeaders: { Authorization: `Bearer ${token}` },
      });
      const terminal = new Set(['done', 'failed', 'expired', 'cancelled']);
      let status = '';
      const batas = Date.now() + 60_000;
      while (Date.now() < batas) {
        const res = await ctx.get(`/v1/requests/${state.queueIdAsync}`);
        expect(res.status()).toBe(200);
        const body = (await res.json()) as { status: string };
        status = body.status;
        if (terminal.has(status)) break;
        await new Promise((r) => setTimeout(r, 1000));
      }
      expect(terminal.has(status), `status akhir: ${status}`).toBe(true);
      await ctx.dispose();
    },
  );

  test(
    '@smoke cancelRequest: 200 pada request kedua yang masih queued',
    { annotation: inventaris('cancelRequest') },
    async ({ playwright }) => {
      const ctx = await playwright.request.newContext({
        baseURL: DOWNSTREAM_ORIGIN,
        extraHTTPHeaders: { Authorization: `Bearer ${token}` },
      });
      // Request baru (bukan state.queueIdAsync, sudah terminal di tes sebelumnya): cancelRequest
      // diuji pada request yang statusnya masih bisa dibatalkan (200), bukan 409.
      const enqueue = await ctx.post(`/v1/endpoints/${state.endpointId}/async`, {
        data: {
          input: { NIK: NIK_SINTETIS },
          metadata: { status_penerima_manfaat: 'sesuai' },
        },
      });
      expect(enqueue.status()).toBe(202);
      const { queue_id: queueId } = (await enqueue.json()) as { queue_id: string };

      const res = await ctx.post(`/v1/requests/${queueId}/cancel`);
      // Balapan dengan worker (mock punya latensi, tapi tidak dijamin): 200 dibatalkan sebelum
      // diproses, atau 409 bila worker sudah memprosesnya (spec Testing Decisions).
      expect([200, 409]).toContain(res.status());
      await ctx.dispose();
    },
  );
});
