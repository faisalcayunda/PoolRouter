import { describe, expect, it } from 'vitest';
import { normalisasiAlokasi, perkiraanAnggaran, ringkasAlokasi } from '../src/features/pool/alokasi';

describe('alokasi helpers', () => {
  it('normalisasi tanpa alokasi', () => {
    expect(normalisasiAlokasi({ jenis: 'none', nilai: null, sifat: null, boleh_pinjam: null })).toEqual({
      alokasi_jenis: null,
      alokasi_nilai: null,
      sifat: null,
      boleh_pinjam: true,
    });
  });

  it('normalisasi beralokasi default boleh_pinjam false', () => {
    expect(normalisasiAlokasi({ jenis: 'tetap', nilai: '500', sifat: 'reserved', boleh_pinjam: null })).toEqual({
      alokasi_jenis: 'exact',
      alokasi_nilai: 500,
      sifat: 'reserved',
      boleh_pinjam: false,
    });
  });

  it('perkiraan anggaran', () => {
    expect(perkiraanAnggaran({ jenis: 'tetap', nilai: 500 }, 1234)).toBe(500);
    expect(perkiraanAnggaran({ jenis: 'persentase', nilai: 30 }, 1234)).toBe(370);
    expect(perkiraanAnggaran({ jenis: 'persentase', nilai: 100 }, 1234)).toBe(1234);
    expect(perkiraanAnggaran({ jenis: 'persentase', nilai: 30 }, 0)).toBe(0);
    expect(perkiraanAnggaran({ jenis: 'none', nilai: null }, 1234)).toBeNull();
  });

  it('ringkas alokasi', () => {
    expect(ringkasAlokasi({ jenis: 'none', nilai: null, sifat: null, boleh_pinjam: false })).toBe('Tanpa alokasi');
    expect(ringkasAlokasi({ jenis: 'tetap', nilai: 500, sifat: 'reserved', boleh_pinjam: false })).toBe('500 · reserved');
    expect(ringkasAlokasi({ jenis: 'persentase', nilai: 30, sifat: 'shareable', boleh_pinjam: true })).toBe('30% · shareable · boleh pinjam');
  });
});
