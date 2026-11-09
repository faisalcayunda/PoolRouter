import { describe, expect, it } from 'vitest';
import { problemToFieldErrors } from '../src/features/common/problem';

describe('problemToFieldErrors', () => {
  it('maps prioritas error', () => {
    const r = problemToFieldErrors({
      kode: 'validasi_gagal',
      errors: [{ field: 'prioritas', kode: 'di_atas_maks' }],
    });
    expect(r.prioritas).toBe('di_atas_maks');
  });

  it('maps kelebihan alokasi with jumlah', () => {
    const r = problemToFieldErrors({
      kode: 'alokasi_kelebihan',
      errors: [{ field: 'alokasi_nilai', kode: 'kelebihan', batas: 120 }],
    });
    expect(r.alokasi_nilai).toBe('Total alokasi melebihi kuota efektif sebanyak 120 permintaan');
  });

  it('maps kuota_harian and margin_kuota', () => {
    const r = problemToFieldErrors({
      kode: 'alokasi_kelebihan',
      errors: [
        { field: 'kuota_harian', kode: 'kelebihan', batas: 50 },
        { field: 'margin_kuota', kode: 'kelebihan', batas: 50 },
      ],
    });
    expect(r.kuota_harian).toContain('50');
    expect(r.margin_kuota).toContain('50');
  });
});
