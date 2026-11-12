import { test, expect, inventaris } from '../fixtures';
import { state, NIK_SINTETIS } from '../state';

/**
 * Grup Auditor, G4 (spec smoke-test bagian 2, inventaris.md): `searchLogByNik` (NIK persis 16
 * digit, `alasan` wajib 10-500 karakter), `openPayload` (tepat satu dari `request_id`/
 * `kode_transaksi`, `alasan` wajib), `exportLogAuditor` (CSV), `searchAuditAkses` (rentang
 * waktu, data=R: hari ini). Semua peran `auditor`; butuh transaksi T dari
 * `09-downstream-api.spec.ts`/`10-log.spec.ts`.
 */
test.describe('@smoke auditor', () => {
  test.use({ peran: 'auditor' });

  const dari = new Date(Date.now() - 24 * 60 * 60 * 1000).toISOString();
  const sampai = new Date(Date.now() + 60 * 60 * 1000).toISOString();
  const alasan = 'Smoke ARC-378: audit transaksi sintetis.';

  test('@smoke searchLogByNik: 200, ada baris NIK sintetis', async ({ sesiApi }) => {
    inventaris('searchLogByNik', 'auditor');
    const res = await sesiApi.post('/admin/v1/auditor/log/cari-nik', {
      data: { nik: NIK_SINTETIS, dari, sampai, alasan },
    });
    expect(res.status()).toBe(200);
    const body = (await res.json()) as { baris: Array<{ kode_transaksi: string }> };
    expect(body.baris.length).toBeGreaterThan(0);
  });

  test('@smoke openPayload: 200, kode_transaksi sesuai', async ({ sesiApi }) => {
    inventaris('openPayload', 'auditor');
    const res = await sesiApi.post('/admin/v1/auditor/payload', {
      data: { kode_transaksi: state.kodeTransaksiSync, alasan },
    });
    expect(res.status()).toBe(200);
    const body = (await res.json()) as { kode_transaksi: string };
    expect(body.kode_transaksi).toBe(state.kodeTransaksiSync);
  });

  test('@smoke exportLogAuditor: 200, CSV', async ({ sesiApi }) => {
    inventaris('exportLogAuditor', 'auditor');
    const res = await sesiApi.post('/admin/v1/auditor/log/ekspor', {
      data: { nik: NIK_SINTETIS, dari, sampai, alasan },
    });
    expect(res.status()).toBe(200);
    expect(res.headers()['content-type']).toContain('text/csv');
  });

  test('@smoke searchAuditAkses: 200, ada baris aksi auditor', async ({ sesiApi }) => {
    inventaris('searchAuditAkses', 'auditor');
    const res = await sesiApi.post('/admin/v1/auditor/audit-akses/cari', {
      data: { dari, sampai },
    });
    expect(res.status()).toBe(200);
    const body = (await res.json()) as { baris: unknown[] };
    expect(body.baris.length).toBeGreaterThan(0);
  });
});
