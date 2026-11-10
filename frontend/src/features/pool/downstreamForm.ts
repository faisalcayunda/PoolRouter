import type { DownstreamPool, DownstreamPoolConfig, Problem } from '../../api/client';
import { normalisasiAlokasi, type JenisAlokasi, type SifatAlokasi } from './alokasi';

/**
 * Model form downstream pool (S07, ARC-102). F0 melayani tepat satu upstream pool per downstream
 * pool; `prioritas` diturunkan server dan tidak pernah dikirim. TTL antrean tampil dalam jam dan
 * dikirim dalam detik; kosong berarti tidak dikirim (bawaan server 24 jam).
 */

export const JENIS_ALOKASI: readonly JenisAlokasi[] = ['none', 'tetap', 'persentase'];
export const SIFAT_ALOKASI: readonly Exclude<SifatAlokasi, null>[] = ['reserved', 'shareable'];

export const ALASAN_MIN = 10;
export const ALASAN_MAKS = 500;

export interface DownstreamDraft {
  nama: string;
  upstream_pool_id: string;
  ttl_jam: string;
  jenis: JenisAlokasi;
  nilai: string;
  sifat: SifatAlokasi;
  boleh_pinjam: boolean;
  alasan: string;
}

export type DraftField = keyof DownstreamDraft;

export const FIELD_ORDER: readonly DraftField[] = [
  'nama',
  'upstream_pool_id',
  'ttl_jam',
  'jenis',
  'nilai',
  'sifat',
  'boleh_pinjam',
  'alasan',
];

export interface FieldIssue {
  /** Kunci pesan di `downstream.form.error.*`. */
  kode: string;
  min?: number;
  max?: number;
  batas?: number;
}

export type DraftErrors = Partial<Record<DraftField, FieldIssue>>;

export function emptyDraft(): DownstreamDraft {
  return {
    nama: '',
    upstream_pool_id: '',
    ttl_jam: '',
    jenis: 'none',
    nilai: '',
    sifat: null,
    boleh_pinjam: true,
    alasan: '',
  };
}

export function draftFromPool(pool: DownstreamPool): DownstreamDraft {
  const jenis: JenisAlokasi =
    pool.alokasi_jenis === 'exact'
      ? 'tetap'
      : pool.alokasi_jenis === 'persen'
        ? 'persentase'
        : 'none';
  return {
    nama: pool.nama,
    upstream_pool_id: pool.upstream_pool_ids[0] ?? '',
    // 86 400 detik = 24 jam; dibulatkan dua desimal supaya detik ganjil tidak jadi ekor panjang.
    ttl_jam: String(Math.round((pool.ttl_antrean_detik / 3600) * 100) / 100),
    jenis,
    nilai:
      pool.alokasi_nilai === null || pool.alokasi_nilai === undefined
        ? ''
        : String(pool.alokasi_nilai),
    sifat: pool.sifat ?? null,
    boleh_pinjam: pool.boleh_pinjam,
    alasan: '',
  };
}

export type BuildResult =
  { ok: true; body: DownstreamPoolConfig } | { ok: false; errors: DraftErrors };

export function toDownstreamConfig(draft: DownstreamDraft, mode: 'create' | 'update'): BuildResult {
  const errors: DraftErrors = {};
  const nama = draft.nama.trim();
  if (nama === '') errors.nama = { kode: 'wajib' };
  if (draft.upstream_pool_id === '') errors.upstream_pool_id = { kode: 'wajib' };

  let ttl: number | null = null;
  if (draft.ttl_jam.trim() !== '') {
    const jam = Number(draft.ttl_jam.trim().replace(',', '.'));
    if (!Number.isFinite(jam) || jam <= 0) errors.ttl_jam = { kode: 'ttl_positif' };
    else ttl = Math.round(jam * 3600);
  }

  if (draft.jenis !== 'none') {
    const nilai = Number(draft.nilai.trim().replace(',', '.'));
    if (draft.nilai.trim() === '' || !Number.isFinite(nilai)) {
      errors.nilai = { kode: 'wajib' };
    } else if (draft.jenis === 'tetap' && (!Number.isInteger(nilai) || nilai < 1)) {
      errors.nilai = { kode: 'tetap_bulat' };
    } else if (draft.jenis === 'persentase' && (nilai < 0.01 || nilai > 100)) {
      errors.nilai = { kode: 'rentang', min: 0.01, max: 100 };
    }
    if (draft.sifat === null) errors.sifat = { kode: 'wajib' };
  }

  const alasan = draft.alasan.trim();
  if (mode === 'update' && (alasan.length < ALASAN_MIN || alasan.length > ALASAN_MAKS)) {
    errors.alasan = { kode: 'alasan_rentang', min: ALASAN_MIN, max: ALASAN_MAKS };
  }

  if (Object.keys(errors).length > 0) return { ok: false, errors };

  const alokasi = normalisasiAlokasi({
    jenis: draft.jenis,
    nilai: draft.nilai.trim().replace(',', '.'),
    sifat: draft.sifat,
    boleh_pinjam: draft.boleh_pinjam,
  });
  return {
    ok: true,
    body: {
      nama,
      upstream_pool_ids: [draft.upstream_pool_id],
      ...alokasi,
      ...(ttl !== null && { ttl_antrean_detik: ttl }),
      ...(alasan !== '' && { alasan }),
    },
  };
}

/** Nama field di body ke field draft. */
const SERVER_FIELD: Record<string, DraftField> = {
  nama: 'nama',
  upstream_pool_ids: 'upstream_pool_id',
  ttl_antrean_detik: 'ttl_jam',
  alokasi_jenis: 'jenis',
  alokasi_nilai: 'nilai',
  sifat: 'sifat',
  boleh_pinjam: 'boleh_pinjam',
  alasan: 'alasan',
};

/** 422 server ke galat per field; `alokasi_kelebihan` membawa jumlah kelebihan di `batas`. */
export function serverErrors(problem: Problem | null): { fields: DraftErrors; lainnya: number } {
  const fields: DraftErrors = {};
  let lainnya = 0;
  for (const err of problem?.errors ?? []) {
    const name = err.field.replace(/^\/+/, '').split('/')[0] ?? err.field;
    const field = SERVER_FIELD[name];
    if (field === undefined) {
      lainnya += 1;
      continue;
    }
    const kelebihan = err.kode === 'alokasi_kelebihan' || problem?.kode === 'alokasi_kelebihan';
    fields[field] = {
      kode: kelebihan ? 'alokasi_kelebihan' : 'server',
      ...(err.batas !== undefined && { batas: err.batas }),
    };
  }
  // Server kadang menjawab `alokasi_kelebihan` tanpa daftar field; tetap tampil di kolom nilai.
  if (problem?.kode === 'alokasi_kelebihan' && Object.keys(fields).length === 0) {
    fields.nilai = { kode: 'alokasi_kelebihan' };
  }
  return { fields, lainnya };
}

export function firstInvalid(errors: DraftErrors): DraftField | undefined {
  return FIELD_ORDER.find((field) => errors[field] !== undefined);
}
