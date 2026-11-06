import type { Problem } from '../../api/client';

/** Field-level errors keyed by field name, as returned by `problemToFieldErrors`. */
export type FieldErrors = Record<string, string>;

/**
 * Maps a 422 `validasi_gagal` problem to per-field errors. Any other problem (or a problem
 * without `errors`) yields an empty object; the caller renders those as an `InlineAlert` keyed
 * by `problem.kode`.
 */
export function problemToFieldErrors(problem: Problem | null | undefined): FieldErrors {
  if (!problem || problem.kode !== 'validasi_gagal') return {};
  const out: FieldErrors = {};
  for (const e of problem.errors ?? []) {
    if (e?.field) out[e.field] = e.kode;
  }
  return out;
}

export function firstErrorField(errors: FieldErrors): string | undefined {
  return Object.keys(errors)[0];
}
