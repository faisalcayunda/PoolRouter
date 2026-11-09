import type { Problem, ValidasiErrorAdmin } from '../../api/client';

export type FieldErrors = Record<string, string>;

function normalizeField(field: string): string {
  return field.replace(/^\/+/, '').split('/').pop() ?? field;
}

function jumlahKelebihan(problem: Problem, err?: ValidasiErrorAdmin): number | null {
  if (typeof err?.batas === 'number') return err.batas;
  if (typeof problem.detail === 'string') {
    const m = problem.detail.match(/(\d+)/);
    if (m) return Number(m[1]);
  }
  return null;
}

export function problemToFieldErrors(problem: Problem | null | undefined): FieldErrors {
  const errors: FieldErrors = {};
  if (!problem) return errors;

  const isKelebihan =
    problem.kode === 'alokasi_kelebihan' ||
    (problem.kode ?? '').toLowerCase().includes('kelebihan') ||
    (problem.title ?? '').toLowerCase().includes('kelebihan');

  for (const e of problem.errors ?? []) {
    const field = normalizeField(e.field);
    if (isKelebihan) {
      const jumlah = jumlahKelebihan(problem, e);
      if (jumlah !== null) {
        errors[field] = `Total alokasi melebihi kuota efektif sebanyak ${jumlah} permintaan`;
      } else if (problem.detail) {
        errors[field] = problem.detail;
      } else {
        errors[field] = e.kode;
      }
    } else {
      errors[field] = e.kode;
    }
  }
  return errors;
}
