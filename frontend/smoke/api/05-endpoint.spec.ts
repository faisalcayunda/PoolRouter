import { test, expect, inventaris } from '../fixtures';
import { state } from '../state';

/**
 * Grup Endpoint (spec smoke-test bagian 2): G2 create→get→update→list, lalu G5
 * pratinjau→nonaktifkan→aktifkan. Butuh `state.providerId` dan `state.downstreamPoolId`.
 * `metadata_fields` memuat `status_penerima_manfaat` (M-R1).
 */
function metadataFields() {
  return [
    {
      nama_field: 'status_penerima_manfaat',
      tipe: 'string',
      wajib: true,
      klasifikasi: 'Internal',
      kirim_ke_data_balikan: false,
      deskripsi: 'Smoke ARC-378: field metadata wajib M-R1.',
    },
  ];
}

test.describe('@smoke endpoint', () => {
  test.use({ peran: 'platform-admin' });

  test(
    '@smoke createEndpoint: 201, simpan endpoint_id',
    { annotation: inventaris('createEndpoint', 'platform-admin') },
    async ({ sesiApi }) => {
      const res = await sesiApi.post('/admin/v1/endpoints', {
        data: {
          provider_id: state.providerId,
          downstream_pool_id: state.downstreamPoolId,
          parameter_literal: { TRESHOLD: '1' },
          log_mode: 'metadata_only',
          metadata_fields: metadataFields(),
          config_schema_version: 1,
        },
      });
      expect(res.status()).toBe(201);
      const body = (await res.json()) as { endpoint_id: string };
      expect(body.endpoint_id).toBeTruthy();
      state.endpointId = body.endpoint_id;
    },
  );

  test(
    '@smoke getEndpoint: 200, provider_id sesuai',
    { annotation: inventaris('getEndpoint', 'platform-admin') },
    async ({ sesiApi }) => {
      const res = await sesiApi.get(`/admin/v1/endpoints/${state.endpointId}`);
      expect(res.status()).toBe(200);
      expect((await res.json()) as { provider_id: string }).toMatchObject({
        provider_id: state.providerId,
      });
    },
  );

  test(
    '@smoke updateEndpoint: 200',
    { annotation: inventaris('updateEndpoint', 'platform-admin') },
    async ({ sesiApi }) => {
      const res = await sesiApi.put(`/admin/v1/endpoints/${state.endpointId}`, {
        data: {
          provider_id: state.providerId,
          downstream_pool_id: state.downstreamPoolId,
          parameter_literal: { TRESHOLD: '2' },
          log_mode: 'metadata_only',
          metadata_fields: metadataFields(),
          config_schema_version: 1,
          alasan: 'Smoke ARC-378: update endpoint sintetis.',
        },
      });
      expect(res.status()).toBe(200);
    },
  );

  test(
    '@smoke listEndpoints: 200, memuat endpoint_id G2',
    { annotation: inventaris('listEndpoints', 'platform-admin') },
    async ({ sesiApi }) => {
      const res = await sesiApi.get('/admin/v1/endpoints');
      expect(res.status()).toBe(200);
      const list = (await res.json()) as Array<{ endpoint_id: string }>;
      expect(list.some((e) => e.endpoint_id === state.endpointId)).toBe(true);
    },
  );

  test(
    '@smoke pratinjauNonaktifkanEndpoint: 200',
    { annotation: inventaris('pratinjauNonaktifkanEndpoint', 'platform-admin') },
    async ({ sesiApi }) => {
      const res = await sesiApi.get(`/admin/v1/endpoints/${state.endpointId}/pratinjau-nonaktif`);
      expect(res.status()).toBe(200);
      const pratinjau = (await res.json()) as Record<string, unknown>;
      expect(pratinjau).toHaveProperty('akan_cancelled');
      expect(pratinjau).toHaveProperty('akan_failed');
    },
  );

  test(
    '@smoke nonaktifkanEndpoint: 200',
    { annotation: inventaris('nonaktifkanEndpoint', 'platform-admin') },
    async ({ sesiApi }) => {
      const res = await sesiApi.post(`/admin/v1/endpoints/${state.endpointId}/nonaktifkan`, {
        data: { alasan: 'Smoke ARC-378: nonaktifkan endpoint sintetis sementara.' },
      });
      expect(res.status()).toBe(200);
    },
  );

  test(
    '@smoke aktifkanEndpoint: 200',
    { annotation: inventaris('aktifkanEndpoint', 'platform-admin') },
    async ({ sesiApi }) => {
      const res = await sesiApi.post(`/admin/v1/endpoints/${state.endpointId}/aktifkan`);
      expect(res.status()).toBe(200);
    },
  );
});
