import { test, expect, inventaris } from '../fixtures';
import { state } from '../state';

/**
 * Grup Upstream (spec smoke-test bagian 2): G2 create→get→update→list, lalu G5
 * pratinjau→nonaktifkan→aktifkan. Butuh `state.providerId` dari 02-provider.spec.ts (urutan
 * file menjaga gelombang, satu worker — Testing Decisions).
 */
function konfigDasar(alasan?: string) {
  return {
    provider_id: state.providerId,
    kuota_harian: 500,
    rate_limit_per_jam: 60,
    porsi_sync: 0.1,
    margin_kuota: 0.05,
    batas_percobaan: 3,
    retry_backoff_base_detik: 5,
    retry_backoff_cap_detik: 300,
    retry_rasio: 0.1,
    retry_min_per_jam: 20,
    jendela_kuota: 'bergulir_24j',
    reset_kuota_jam: 0,
    reset_kuota_zona: 'Asia/Jakarta',
    jeda_min: 250,
    jeda_maks: 1500,
    ambang_berdampingan: 8,
    ...(alasan ? { alasan } : {}),
  };
}

test.describe('@smoke upstream', () => {
  test.use({ peran: 'platform-admin' });

  test('@smoke createUpstreamPool: 201, simpan upstream_pool_id', async ({ sesiApi }) => {
    inventaris('createUpstreamPool', 'platform-admin');
    const res = await sesiApi.post('/admin/v1/upstream-pools', { data: konfigDasar() });
    expect(res.status()).toBe(201);
    const body = (await res.json()) as { upstream_pool_id: string };
    expect(body.upstream_pool_id).toBeTruthy();
    state.upstreamPoolId = body.upstream_pool_id;
  });

  test('@smoke getUpstreamPool: 200, kuota_harian sesuai', async ({ sesiApi }) => {
    inventaris('getUpstreamPool', 'platform-admin');
    const res = await sesiApi.get(`/admin/v1/upstream-pools/${state.upstreamPoolId}`);
    expect(res.status()).toBe(200);
    expect((await res.json()) as { kuota_harian: number }).toMatchObject({ kuota_harian: 500 });
  });

  test('@smoke updateUpstreamPool: 200', async ({ sesiApi }) => {
    inventaris('updateUpstreamPool', 'platform-admin');
    const res = await sesiApi.put(`/admin/v1/upstream-pools/${state.upstreamPoolId}`, {
      data: konfigDasar('Smoke ARC-378: update upstream pool sintetis.'),
    });
    expect(res.status()).toBe(200);
  });

  test('@smoke listUpstreamPools: 200, memuat upstream_pool_id G2', async ({ sesiApi }) => {
    inventaris('listUpstreamPools', 'platform-admin');
    const res = await sesiApi.get('/admin/v1/upstream-pools');
    expect(res.status()).toBe(200);
    const list = (await res.json()) as Array<{ upstream_pool_id: string }>;
    expect(list.some((u) => u.upstream_pool_id === state.upstreamPoolId)).toBe(true);
  });

  test('@smoke pratinjauNonaktifkanUpstreamPool: 200', async ({ sesiApi }) => {
    inventaris('pratinjauNonaktifkanUpstreamPool', 'platform-admin');
    const res = await sesiApi.get(
      `/admin/v1/upstream-pools/${state.upstreamPoolId}/pratinjau-nonaktif`,
    );
    expect(res.status()).toBe(200);
    const pratinjau = (await res.json()) as Record<string, unknown>;
    expect(pratinjau).toHaveProperty('akan_cancelled');
    expect(pratinjau).toHaveProperty('akan_failed');
  });

  test('@smoke nonaktifkanUpstreamPool: 200', async ({ sesiApi }) => {
    inventaris('nonaktifkanUpstreamPool', 'platform-admin');
    const res = await sesiApi.post(`/admin/v1/upstream-pools/${state.upstreamPoolId}/nonaktifkan`, {
      data: { alasan: 'Smoke ARC-378: nonaktifkan upstream pool sintetis sementara.' },
    });
    expect(res.status()).toBe(200);
  });

  test('@smoke aktifkanUpstreamPool: 200', async ({ sesiApi }) => {
    inventaris('aktifkanUpstreamPool', 'platform-admin');
    const res = await sesiApi.post(`/admin/v1/upstream-pools/${state.upstreamPoolId}/aktifkan`);
    expect(res.status()).toBe(200);
  });
});
