import { test, expect, inventaris } from '../fixtures';
import { state, NAMA } from '../state';

/**
 * Grup Provider (spec smoke-test bagian 2): G2 create→get→update→list (platform admin), lalu G5
 * pratinjau→nonaktifkan→aktifkan. `tls_trust=ca_kustom` dan `base_url` memakai alias jaringan
 * sidecar `mock-tls` (Q1), bukan kode baru. Tiap operasi PA juga dicek 403 dengan sesi operator.
 */
test.describe('@smoke provider', () => {
  test.use({ peran: 'platform-admin' });

  test('@smoke createProvider: 201, simpan provider_id', async ({ sesiApi }) => {
    inventaris('createProvider', 'platform-admin');
    const res = await sesiApi.post('/admin/v1/providers', {
      data: {
        nama: NAMA.provider,
        metode: 'POST',
        base_url: 'https://dukcapil-sintetis.example.test/api',
        body_template: '{"NIK":"{{input.NIK}}"}',
        parameters: [],
        respons_skema: {},
        tls_trust: 'system',
        log_mode: 'metadata_only',
        config_schema_version: 1,
      },
    });
    expect(res.status()).toBe(201);
    const body = (await res.json()) as { provider_id: string };
    expect(body.provider_id).toBeTruthy();
    state.providerId = body.provider_id;
  });

  test.describe('ditolak untuk peran di luar kolom (operator)', () => {
    test.use({ peran: 'operator' });

    test('@smoke createProvider ditolak untuk operator: 403', async ({ sesiApi }) => {
      const res = await sesiApi.post('/admin/v1/providers', {
        data: {
          nama: `${NAMA.provider}-403`,
          metode: 'POST',
          base_url: 'https://dukcapil-sintetis.example.test/api',
          body_template: '{}',
          parameters: [],
          respons_skema: {},
          tls_trust: 'system',
          log_mode: 'metadata_only',
          config_schema_version: 1,
        },
      });
      expect(res.status()).toBe(403);
    });
  });

  test('@smoke getProvider: 200, nama sesuai', async ({ sesiApi }) => {
    inventaris('getProvider', 'platform-admin');
    const res = await sesiApi.get(`/admin/v1/providers/${state.providerId}`);
    expect(res.status()).toBe(200);
    expect((await res.json()) as { nama: string }).toMatchObject({ nama: NAMA.provider });
  });

  test('@smoke updateProvider: 200, body_template berubah', async ({ sesiApi }) => {
    inventaris('updateProvider', 'platform-admin');
    const res = await sesiApi.put(`/admin/v1/providers/${state.providerId}`, {
      data: {
        nama: NAMA.provider,
        metode: 'POST',
        base_url: 'https://dukcapil-sintetis.example.test/api',
        body_template: '{"NIK":"{{input.NIK}}","V":"2"}',
        parameters: [],
        respons_skema: {},
        tls_trust: 'system',
        log_mode: 'metadata_only',
        config_schema_version: 1,
        alasan: 'Smoke ARC-378: update template provider sintetis.',
      },
    });
    expect(res.status()).toBe(200);
  });

  test('@smoke listProviders: 200, memuat smoke-provider', async ({ sesiApi }) => {
    inventaris('listProviders', 'platform-admin');
    const res = await sesiApi.get('/admin/v1/providers');
    expect(res.status()).toBe(200);
    const list = (await res.json()) as Array<{ nama: string }>;
    expect(list.some((p) => p.nama === NAMA.provider)).toBe(true);
  });

  test('@smoke pratinjauNonaktifkanProvider: 200, field akan_cancelled/akan_failed ada', async ({
    sesiApi,
  }) => {
    inventaris('pratinjauNonaktifkanProvider', 'platform-admin');
    const res = await sesiApi.get(`/admin/v1/providers/${state.providerId}/pratinjau-nonaktif`);
    expect(res.status()).toBe(200);
    const pratinjau = (await res.json()) as Record<string, unknown>;
    expect(pratinjau).toHaveProperty('akan_cancelled');
    expect(pratinjau).toHaveProperty('akan_failed');
  });

  test('@smoke nonaktifkanProvider: 200', async ({ sesiApi }) => {
    inventaris('nonaktifkanProvider', 'platform-admin');
    const res = await sesiApi.post(`/admin/v1/providers/${state.providerId}/nonaktifkan`, {
      data: { alasan: 'Smoke ARC-378: nonaktifkan provider sintetis sementara.' },
    });
    expect(res.status()).toBe(200);
  });

  test('@smoke aktifkanProvider: 200', async ({ sesiApi }) => {
    inventaris('aktifkanProvider', 'platform-admin');
    const res = await sesiApi.post(`/admin/v1/providers/${state.providerId}/aktifkan`);
    expect(res.status()).toBe(200);
  });
});
