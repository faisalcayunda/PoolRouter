import { test, expect, inventaris, DOWNSTREAM_ORIGIN } from '../fixtures';
import { state, NIK_SINTETIS } from '../state';

/**
 * Grup Log, G4 (spec smoke-test bagian 2, inventaris.md): `searchLog`/`getLogEntry` (OP, butuh
 * transaksi T dari `09-downstream-api.spec.ts`), `getQuotaTracker` (PA, data=U).
 * `listWebhookLog`/`requestTerminal` (PA/controller, data="K W T") butuh sebuah pengiriman webhook
 * yang DITOLAK allowlist (AC-29: allowlist diperiksa saat kirim, bukan saat konfigurasi disimpan):
 * `putWebhookConfig` dialihkan sebentar ke domain yang sengaja tidak pernah masuk allowlist
 * (beda dari `state.allowlistDomain` di 07-allowlist.spec.ts/08-client.spec.ts, yang aktif), lalu
 * sebuah `enqueueAsync` baru dibuat supaya terminalnya memicu satu percobaan kirim webhook; baris
 * `webhook_log` hasilnya `ditolak_allowlist` membuktikan `requestTerminal` (controller, tidak
 * punya endpoint HTTP sendiri untuk dipanggil smoke).
 */
test.describe('@smoke log', () => {
  const dari = new Date(Date.now() - 24 * 60 * 60 * 1000).toISOString();
  const sampai = new Date(Date.now() + 60 * 60 * 1000).toISOString();

  test.describe('OP', () => {
    test.use({ peran: 'operator' });

    test('@smoke searchLog: 200, ada baris transaksi G3', async ({ sesiApi }) => {
      inventaris('searchLog', 'operator');
      const res = await sesiApi.post('/admin/v1/log/cari', {
        data: { dari, sampai, client_id: state.clientId },
      });
      expect(res.status()).toBe(200);
      const body = (await res.json()) as {
        baris: Array<{ kode_transaksi: string; queue_id: string }>;
      };
      expect(body.baris.length).toBeGreaterThan(0);
      const baris = body.baris.find((b) => b.queue_id === state.queueIdAsync) ?? body.baris[0];
      state.kodeTransaksiSync = baris?.kode_transaksi;
      expect(state.kodeTransaksiSync).toBeTruthy();
    });

    test('@smoke getLogEntry: 200, kode_transaksi sesuai', async ({ sesiApi }) => {
      inventaris('getLogEntry', 'operator');
      const res = await sesiApi.get(`/admin/v1/log/${state.kodeTransaksiSync}`);
      expect(res.status()).toBe(200);
      const body = (await res.json()) as { kode_transaksi: string };
      expect(body.kode_transaksi).toBe(state.kodeTransaksiSync);
    });
  });

  test.describe('PA', () => {
    test.use({ peran: 'platform-admin' });

    test('@smoke getQuotaTracker: 200', async ({ sesiApi }) => {
      inventaris('getQuotaTracker', 'platform-admin');
      const res = await sesiApi.get(
        `/admin/v1/kuota?upstream_pool_id=${state.upstreamPoolId ?? ''}`,
      );
      expect(res.status()).toBe(200);
      expect(Array.isArray(await res.json())).toBe(true);
    });

    test('@smoke listWebhookLog: 200, ada baris ditolak_allowlist (buktikan requestTerminal)', async ({
      sesiApi,
      playwright,
    }) => {
      inventaris('listWebhookLog', 'platform-admin');
      inventaris('requestTerminal', 'platform-admin');

      // Alihkan webhook ke domain yang sengaja tidak di-allowlist (beda dari state.allowlistDomain,
      // yang aktif sejak 07-allowlist.spec.ts); allowlist diperiksa saat kirim (AC-29).
      const resWebhook = await sesiApi.put(`/admin/v1/clients/${state.clientId}/webhook`, {
        data: { webhook_url: 'https://smoke-ditolak.example.test/webhook', aktif: true },
      });
      expect(resWebhook.status()).toBe(200);

      const ctx = await playwright.request.newContext({ baseURL: DOWNSTREAM_ORIGIN });
      const tokenRes = await ctx.post('/v1/oauth/token', {
        form: {
          grant_type: 'client_credentials',
          client_id: state.credentialOauth2Id ?? '',
          client_secret: state.credentialOauth2Secret ?? '',
        },
      });
      expect(tokenRes.status()).toBe(200);
      const { access_token: token } = (await tokenRes.json()) as { access_token: string };

      const enqueue = await ctx.post(`/v1/endpoints/${state.endpointId}/async`, {
        data: { input: { NIK: NIK_SINTETIS }, metadata: { status_penerima_manfaat: 'sesuai' } },
        headers: { Authorization: `Bearer ${token}` },
      });
      expect(enqueue.status()).toBe(202);
      const { queue_id: queueId } = (await enqueue.json()) as { queue_id: string };

      const terminal = new Set(['done', 'failed', 'expired', 'cancelled']);
      const batas = Date.now() + 60_000;
      let status = '';
      while (Date.now() < batas) {
        const res = await ctx.get(`/v1/requests/${queueId}`, {
          headers: { Authorization: `Bearer ${token}` },
        });
        status = ((await res.json()) as { status: string }).status;
        if (terminal.has(status)) break;
        await new Promise((r) => setTimeout(r, 1000));
      }
      expect(terminal.has(status), `status akhir: ${status}`).toBe(true);
      await ctx.dispose();

      // Webhook dikirim async oleh controller sesudah terminal (bukan dalam respons di atas); beri
      // waktu loop webhook (jadwal.rs INTERVAL_SIKLUS) memprosesnya sebelum membaca log.
      let list: Array<{ hasil: string }> = [];
      const batasLog = Date.now() + 30_000;
      while (Date.now() < batasLog) {
        const res = await sesiApi.get(`/admin/v1/clients/${state.clientId}/webhook/log`);
        expect(res.status()).toBe(200);
        list = (await res.json()) as Array<{ hasil: string }>;
        if (list.some((w) => w.hasil === 'ditolak_allowlist')) break;
        await new Promise((r) => setTimeout(r, 1000));
      }
      expect(list.some((w) => w.hasil === 'ditolak_allowlist')).toBe(true);
    });
  });
});
