import { test, expect, inventaris } from '../fixtures';
import { state, NAMA } from '../state';

/**
 * Grup Downstream (spec smoke-test bagian 2): G2 create→get→update→list, lalu G5
 * pratinjau→nonaktifkan→aktifkan. Butuh `state.upstreamPoolId` dari 03-upstream.spec.ts
 * (`upstream_pool_ids`: tepat satu di F0).
 */
test.describe('@smoke downstream', () => {
  test.use({ peran: 'platform-admin' });

  test('@smoke createDownstreamPool: 201, simpan downstream_pool_id', async ({ sesiApi }) => {
    inventaris('createDownstreamPool', 'platform-admin');
    const res = await sesiApi.post('/admin/v1/downstream-pools', {
      data: { nama: NAMA.downstream, upstream_pool_ids: [state.upstreamPoolId] },
    });
    expect(res.status()).toBe(201);
    const body = (await res.json()) as { downstream_pool_id: string };
    expect(body.downstream_pool_id).toBeTruthy();
    state.downstreamPoolId = body.downstream_pool_id;
  });

  test('@smoke getDownstreamPool: 200, nama sesuai', async ({ sesiApi }) => {
    inventaris('getDownstreamPool', 'platform-admin');
    const res = await sesiApi.get(`/admin/v1/downstream-pools/${state.downstreamPoolId}`);
    expect(res.status()).toBe(200);
    expect((await res.json()) as { nama: string }).toMatchObject({ nama: NAMA.downstream });
  });

  test('@smoke updateDownstreamPool: 200', async ({ sesiApi }) => {
    inventaris('updateDownstreamPool', 'platform-admin');
    const res = await sesiApi.put(`/admin/v1/downstream-pools/${state.downstreamPoolId}`, {
      data: {
        nama: NAMA.downstream,
        upstream_pool_ids: [state.upstreamPoolId],
        alasan: 'Smoke ARC-378: update downstream pool sintetis.',
      },
    });
    expect(res.status()).toBe(200);
  });

  test('@smoke listDownstreamPools: 200, memuat downstream_pool_id G2', async ({ sesiApi }) => {
    inventaris('listDownstreamPools', 'platform-admin');
    const res = await sesiApi.get('/admin/v1/downstream-pools');
    expect(res.status()).toBe(200);
    const list = (await res.json()) as Array<{ downstream_pool_id: string }>;
    expect(list.some((d) => d.downstream_pool_id === state.downstreamPoolId)).toBe(true);
  });

  test('@smoke pratinjauNonaktifkanDownstreamPool: 200', async ({ sesiApi }) => {
    inventaris('pratinjauNonaktifkanDownstreamPool', 'platform-admin');
    const res = await sesiApi.get(
      `/admin/v1/downstream-pools/${state.downstreamPoolId}/pratinjau-nonaktif`,
    );
    expect(res.status()).toBe(200);
    const pratinjau = (await res.json()) as Record<string, unknown>;
    expect(pratinjau).toHaveProperty('akan_cancelled');
    expect(pratinjau).toHaveProperty('akan_failed');
  });

  test('@smoke nonaktifkanDownstreamPool: 200', async ({ sesiApi }) => {
    inventaris('nonaktifkanDownstreamPool', 'platform-admin');
    const res = await sesiApi.post(
      `/admin/v1/downstream-pools/${state.downstreamPoolId}/nonaktifkan`,
      { data: { alasan: 'Smoke ARC-378: nonaktifkan downstream pool sintetis sementara.' } },
    );
    expect(res.status()).toBe(200);
  });

  test('@smoke aktifkanDownstreamPool: 200', async ({ sesiApi }) => {
    inventaris('aktifkanDownstreamPool', 'platform-admin');
    const res = await sesiApi.post(`/admin/v1/downstream-pools/${state.downstreamPoolId}/aktifkan`);
    expect(res.status()).toBe(200);
  });
});
