export type JenisAlokasi = 'none' | 'tetap' | 'persentase';

export type AlokasiJenis = 'exact' | 'persen' | null;

export type SifatAlokasi = 'reserved' | 'shareable' | null;

export interface AlokasiBody {
  alokasi_jenis: AlokasiJenis;
  alokasi_nilai: number | null;
  sifat: SifatAlokasi;
  boleh_pinjam: boolean;
}

export interface AlokasiForm {
  jenis: JenisAlokasi;
  nilai: string | number | null;
  sifat: SifatAlokasi;
  boleh_pinjam: boolean | null;
}

function toJenis(jenis?: JenisAlokasi, alokasiJenis?: AlokasiJenis): AlokasiJenis {
  if (alokasiJenis !== undefined) return alokasiJenis;
  if (jenis === 'tetap') return 'exact';
  if (jenis === 'persentase') return 'persen';
  return null;
}

export function normalisasiAlokasi(input: AlokasiForm | AlokasiBody | Record<string, unknown>): AlokasiBody {
  const jenisInput = (input as AlokasiForm).jenis;
  const alokasiJenisInput = (input as AlokasiBody).alokasi_jenis;
  const jenis = toJenis(jenisInput, alokasiJenisInput);
  const rawNilai = (input as AlokasiForm).nilai ?? (input as AlokasiBody).alokasi_nilai;
  const sifatInput = (input as AlokasiForm).sifat ?? (input as AlokasiBody).sifat ?? null;
  const bolehPinjamInput = (input as AlokasiForm).boleh_pinjam ?? (input as AlokasiBody).boleh_pinjam;

  if (jenis === null) {
    return {
      alokasi_jenis: null,
      alokasi_nilai: null,
      sifat: null,
      boleh_pinjam: true,
    };
  }

  const nilai = rawNilai === null || rawNilai === '' ? null : Number(rawNilai);
  return {
    alokasi_jenis: jenis,
    alokasi_nilai: nilai,
    sifat: sifatInput,
    boleh_pinjam: bolehPinjamInput === true,
  };
}

export function perkiraanAnggaran(
  alokasi: { jenis?: JenisAlokasi; alokasi_jenis?: AlokasiJenis; nilai?: string | number | null; alokasi_nilai?: number | null },
  kuotaEfektif: number,
): number | null {
  const jenis = toJenis(alokasi.jenis, alokasi.alokasi_jenis);
  const rawNilai = alokasi.nilai ?? alokasi.alokasi_nilai;
  if (jenis === null || rawNilai === null || rawNilai === undefined || rawNilai === '') {
    return null;
  }
  const nilai = Number(rawNilai);
  if (jenis === 'exact') {
    return nilai;
  }
  return Math.floor((nilai / 100) * kuotaEfektif);
}

export function ringkasAlokasi(alokasi: { jenis?: JenisAlokasi; alokasi_jenis?: AlokasiJenis; nilai?: string | number | null; alokasi_nilai?: number | null; sifat?: SifatAlokasi; boleh_pinjam?: boolean | null } | AlokasiBody | AlokasiForm): string {
  const jenis = toJenis((alokasi as AlokasiForm).jenis, (alokasi as AlokasiBody).alokasi_jenis);
  const rawNilai = (alokasi as AlokasiForm).nilai ?? (alokasi as AlokasiBody).alokasi_nilai;
  const sifat = (alokasi as AlokasiForm).sifat ?? (alokasi as AlokasiBody).sifat ?? null;
  const bolehPinjam = (alokasi as AlokasiForm).boleh_pinjam ?? (alokasi as AlokasiBody).boleh_pinjam ?? false;

  if (jenis === null) {
    return 'Tanpa alokasi';
  }
  const nilai = rawNilai === null || rawNilai === undefined ? '' : rawNilai;
  const sifatLabel = sifat ?? '';
  const parts = [`${nilai}${jenis === 'persen' ? '%' : ''}`, sifatLabel];
  if (bolehPinjam) parts.push('boleh pinjam');
  return parts.filter(Boolean).join(' · ');
}
