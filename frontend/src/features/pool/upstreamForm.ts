import type { Problem, UpstreamPool, UpstreamPoolConfig } from '../../api/client';

/**
 * Model form upstream pool (S06b, ARC-102). `PUT /upstream-pools/{id}` adalah penggantian penuh:
 * form memuat SEMUA field `UpstreamPoolConfig` dari bacaan terakhir dan mengirim semuanya kembali,
 * jadi satu field yang tidak tampil tidak pernah tertimpa nilai bawaan server.
 */

export type JendelaKuota = UpstreamPoolConfig['jendela_kuota'];
export const JENDELA_KUOTA: readonly JendelaKuota[] = ['bergulir_24j', 'kalender'];

export const ALASAN_MIN = 10;
export const ALASAN_MAKS = 500;

/** Field bilangan: kunci body, rentang KP-01, dan apakah bilangan bulat. */
interface NumberRule {
  min: number;
  max: number | null;
  integer: boolean;
}

export const NUMBER_RULES = {
  kuota_harian: { min: 1, max: null, integer: true },
  rate_limit_per_jam: { min: 1, max: null, integer: true },
  ambang_berdampingan: { min: 1, max: null, integer: true },
  jeda_min: { min: 0, max: 60_000, integer: true },
  jeda_maks: { min: 0, max: 60_000, integer: true },
  batas_percobaan: { min: 1, max: 5, integer: true },
  retry_backoff_base_detik: { min: 1, max: 300, integer: true },
  retry_backoff_cap_detik: { min: 1, max: 3600, integer: true },
  retry_min_per_jam: { min: 0, max: 10_000, integer: true },
  retry_rasio: { min: 0, max: 0.5, integer: false },
  margin_kuota: { min: 0.001, max: 0.2, integer: false },
  reset_kuota_jam: { min: 0, max: 23, integer: true },
  /** Persen 0..100 di layar; dikirim sebagai pecahan 0..1 (`porsi_sync`). */
  porsi_sync_persen: { min: 0, max: 100, integer: false },
} as const satisfies Record<string, NumberRule>;

export type NumberField = keyof typeof NUMBER_RULES;

export interface UpstreamDraft extends Record<NumberField, string> {
  provider_id: string;
  jendela_kuota: JendelaKuota;
  reset_kuota_zona: string;
  alasan: string;
}

export type DraftField = keyof UpstreamDraft;

/** Urutan field di layar; fokus pindah ke field salah pertama menurut urutan ini. */
export const FIELD_ORDER: readonly DraftField[] = [
  'provider_id',
  'kuota_harian',
  'rate_limit_per_jam',
  'porsi_sync_persen',
  'ambang_berdampingan',
  'jeda_min',
  'jeda_maks',
  'jendela_kuota',
  'reset_kuota_jam',
  'reset_kuota_zona',
  'batas_percobaan',
  'retry_backoff_base_detik',
  'retry_backoff_cap_detik',
  'retry_min_per_jam',
  'retry_rasio',
  'margin_kuota',
  'alasan',
];

export interface FieldIssue {
  /** Kunci pesan di `upstream.form.error.*`. */
  kode: string;
  min?: number;
  max?: number;
  batas?: number;
}

export type DraftErrors = Partial<Record<DraftField, FieldIssue>>;

/** Bawaan KP-01 untuk field yang boleh dikosongkan di pool baru; tujuh field wajib tanpa bawaan. */
export function emptyDraft(): UpstreamDraft {
  return {
    provider_id: '',
    kuota_harian: '',
    rate_limit_per_jam: '',
    porsi_sync_persen: '',
    ambang_berdampingan: '',
    jeda_min: '',
    jeda_maks: '',
    jendela_kuota: 'bergulir_24j',
    reset_kuota_jam: '0',
    reset_kuota_zona: 'Asia/Jakarta',
    batas_percobaan: '3',
    retry_backoff_base_detik: '5',
    retry_backoff_cap_detik: '300',
    retry_min_per_jam: '20',
    retry_rasio: '0.1',
    margin_kuota: '0.01',
    alasan: '',
  };
}

/** Pecahan ke persen tanpa ekor floating point (0.1 * 100 bukan persis 10). */
export function keFraksiPersen(fraksi: number): string {
  return String(Math.round(fraksi * 10_000) / 100);
}

export function draftFromPool(pool: UpstreamPool): UpstreamDraft {
  return {
    provider_id: pool.provider_id,
    kuota_harian: String(pool.kuota_harian),
    rate_limit_per_jam: String(pool.rate_limit_per_jam),
    porsi_sync_persen: keFraksiPersen(pool.porsi_sync),
    ambang_berdampingan: String(pool.ambang_berdampingan),
    jeda_min: String(pool.jeda_min),
    jeda_maks: String(pool.jeda_maks),
    jendela_kuota: pool.jendela_kuota,
    reset_kuota_jam: String(pool.reset_kuota_jam),
    reset_kuota_zona: pool.reset_kuota_zona,
    batas_percobaan: String(pool.batas_percobaan),
    retry_backoff_base_detik: String(pool.retry_backoff_base_detik),
    retry_backoff_cap_detik: String(pool.retry_backoff_cap_detik),
    retry_min_per_jam: String(pool.retry_min_per_jam),
    retry_rasio: String(pool.retry_rasio),
    margin_kuota: String(pool.margin_kuota),
    // Alasan tidak disimpan di entitas; selalu kosong sampai admin menulisnya.
    alasan: '',
  };
}

export type BuildResult =
  { ok: true; body: UpstreamPoolConfig } | { ok: false; errors: DraftErrors };

/** Validasi sisi klien memakai aturan yang sama dengan server (KP-01); server tetap penjaga akhir. */
export function toUpstreamConfig(draft: UpstreamDraft, mode: 'create' | 'update'): BuildResult {
  const errors: DraftErrors = {};
  const num: Record<string, number> = {};

  for (const field of Object.keys(NUMBER_RULES) as NumberField[]) {
    const rule: NumberRule = NUMBER_RULES[field];
    const raw = draft[field].trim().replace(',', '.');
    if (raw === '') {
      errors[field] = { kode: 'wajib' };
      continue;
    }
    const value = Number(raw);
    if (!Number.isFinite(value) || (rule.integer && !Number.isInteger(value))) {
      errors[field] = { kode: rule.integer ? 'bilangan_bulat' : 'bilangan' };
      continue;
    }
    if (value < rule.min || (rule.max !== null && value > rule.max)) {
      errors[field] =
        rule.max === null
          ? { kode: 'minimal', min: rule.min }
          : { kode: 'rentang', min: rule.min, max: rule.max };
      continue;
    }
    num[field] = value;
  }

  if (draft.provider_id === '') errors.provider_id = { kode: 'wajib' };
  if (draft.reset_kuota_zona.trim() === '') errors.reset_kuota_zona = { kode: 'wajib' };

  const { jeda_min, jeda_maks, rate_limit_per_jam } = num;
  if (
    jeda_min !== undefined &&
    jeda_maks !== undefined &&
    jeda_maks < jeda_min &&
    !errors.jeda_maks
  ) {
    errors.jeda_maks = { kode: 'jeda_terbalik' };
  }
  if (rate_limit_per_jam !== undefined) {
    const batas = Math.floor(3_600_000 / rate_limit_per_jam);
    for (const field of ['jeda_min', 'jeda_maks'] as const) {
      const value = num[field];
      if (value !== undefined && value > batas && !errors[field]) {
        errors[field] = { kode: 'jeda_melebihi_rate', batas };
      }
    }
  }
  const { retry_backoff_base_detik: base, retry_backoff_cap_detik: cap } = num;
  if (base !== undefined && cap !== undefined && cap < base && !errors.retry_backoff_cap_detik) {
    errors.retry_backoff_cap_detik = { kode: 'cap_di_bawah_base', batas: base };
  }

  const alasan = draft.alasan.trim();
  const alasanWajib = mode === 'update' || draft.jendela_kuota === 'kalender';
  if (alasanWajib && (alasan.length < ALASAN_MIN || alasan.length > ALASAN_MAKS)) {
    errors.alasan = { kode: 'alasan_rentang', min: ALASAN_MIN, max: ALASAN_MAKS };
  }

  if (Object.keys(errors).length > 0) return { ok: false, errors };

  // Tanpa galat, setiap field bilangan sudah terisi; `?? 0` hanya memuaskan tipe.
  const n = (key: string): number => num[key] ?? 0;
  return {
    ok: true,
    body: {
      provider_id: draft.provider_id,
      kuota_harian: n('kuota_harian'),
      rate_limit_per_jam: n('rate_limit_per_jam'),
      porsi_sync: Math.round(n('porsi_sync_persen') * 100) / 10_000,
      ambang_berdampingan: n('ambang_berdampingan'),
      jeda_min: n('jeda_min'),
      jeda_maks: n('jeda_maks'),
      jendela_kuota: draft.jendela_kuota,
      reset_kuota_jam: n('reset_kuota_jam'),
      reset_kuota_zona: draft.reset_kuota_zona.trim(),
      batas_percobaan: n('batas_percobaan'),
      retry_backoff_base_detik: n('retry_backoff_base_detik'),
      retry_backoff_cap_detik: n('retry_backoff_cap_detik'),
      retry_min_per_jam: n('retry_min_per_jam'),
      retry_rasio: n('retry_rasio'),
      margin_kuota: n('margin_kuota'),
      ...(alasan !== '' && { alasan }),
    },
  };
}

/** Nama field di body ke field draft; `porsi_sync` tampil sebagai persen. */
const SERVER_FIELD: Record<string, DraftField> = { porsi_sync: 'porsi_sync_persen' };

/** Petakan 422 server ke galat per field; yang tidak dikenal masuk `lainnya`. */
export function serverErrors(problem: Problem | null): { fields: DraftErrors; lainnya: number } {
  const fields: DraftErrors = {};
  let lainnya = 0;
  for (const err of problem?.errors ?? []) {
    const name = err.field.replace(/^\/+/, '').split('/').pop() ?? err.field;
    const field = SERVER_FIELD[name] ?? (FIELD_ORDER as readonly string[]).find((f) => f === name);
    if (field === undefined) {
      lainnya += 1;
    } else {
      fields[field as DraftField] = {
        kode: 'server',
        ...(err.batas !== undefined && { batas: err.batas }),
      };
    }
  }
  return { fields, lainnya };
}

export function firstInvalid(errors: DraftErrors): DraftField | undefined {
  return FIELD_ORDER.find((field) => errors[field] !== undefined);
}
