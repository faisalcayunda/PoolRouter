import type { Schemas } from '../../api/client';

/**
 * Editor rule terstruktur (S04, ARC-252). Draft provider menyimpan `parameters` sebagai teks JSON
 * (lihat `providerForm.ts`); modul ini membaca teks itu menjadi baris `ParameterRule`, membangun
 * baris dari isian editor, dan menulisnya kembali. Bentuk baris persis `ParameterRule` /
 * `RuleValidasi` di kontrak: kunci yang tidak relevan untuk `jenis` tidak pernah dikirim.
 */

export type ParameterRule = Schemas['ParameterRule'];
export type RuleJenis = Schemas['RuleJenis'];

export const RULE_JENIS: readonly RuleJenis[] = ['regex', 'enum', 'panjang', 'format_tanggal'];

/** Isian editor untuk satu parameter. Angka dan daftar disimpan sebagai teks supaya bisa disunting. */
export interface RuleDraft {
  nama: string;
  nilai: string;
  /** Kosong = parameter tanpa rule. */
  jenis: RuleJenis | '';
  pola: string;
  /** Satu nilai per baris. */
  nilaiDiizinkan: string;
  panjangMin: string;
  panjangMaks: string;
  formatTanggal: string;
}

export type RuleField = keyof RuleDraft;

/** Kode galat per field; kunci pesan ada di `provider.rules.editor.error.*`. */
export type RuleErrors = Partial<Record<RuleField, 'wajib' | 'duplikat' | 'bilangan' | 'rentang'>>;

export type BuildResult = { ok: true; row: ParameterRule } | { ok: false; errors: RuleErrors };

export function emptyRuleDraft(): RuleDraft {
  return {
    nama: '',
    nilai: '',
    jenis: '',
    pola: '',
    nilaiDiizinkan: '',
    panjangMin: '',
    panjangMaks: '',
    formatTanggal: '',
  };
}

export function draftFromParameter(row: ParameterRule): RuleDraft {
  const rule = row.rule;
  return {
    nama: row.nama,
    nilai: row.nilai ?? '',
    jenis: rule?.jenis ?? '',
    pola: rule?.pola ?? '',
    nilaiDiizinkan: (rule?.nilai_diizinkan ?? []).join('\n'),
    panjangMin: rule?.panjang_min == null ? '' : String(rule.panjang_min),
    panjangMaks: rule?.panjang_maks == null ? '' : String(rule.panjang_maks),
    formatTanggal: rule?.format_tanggal ?? '',
  };
}

const INTEGER = /^\d+$/;

/**
 * Validasi murah di sisi klien lalu susun baris. Server tetap penjaga terakhir (mis. sah-tidaknya
 * regex); `nama` dibandingkan apa adanya (disimpan verbatim). `editing` = indeks baris yang sedang
 * diubah, supaya namanya sendiri bukan duplikat.
 */
export function buildParameter(
  draft: RuleDraft,
  existingNames: readonly string[],
  editing: number | null,
): BuildResult {
  const errors: RuleErrors = {};
  const nama = draft.nama.trim();
  if (!nama) errors.nama = 'wajib';
  else if (existingNames.some((other, index) => index !== editing && other === nama)) {
    errors.nama = 'duplikat';
  }

  let rule: ParameterRule['rule'];
  switch (draft.jenis) {
    case 'regex':
      if (!draft.pola) errors.pola = 'wajib';
      else rule = { jenis: 'regex', pola: draft.pola };
      break;
    case 'enum': {
      const nilai = draft.nilaiDiizinkan
        .split('\n')
        .map((line) => line.trim())
        .filter((line) => line !== '');
      if (nilai.length === 0) errors.nilaiDiizinkan = 'wajib';
      else rule = { jenis: 'enum', nilai_diizinkan: nilai };
      break;
    }
    case 'panjang': {
      const min = draft.panjangMin.trim();
      const maks = draft.panjangMaks.trim();
      if (!min && !maks) errors.panjangMin = 'wajib';
      if (min && !INTEGER.test(min)) errors.panjangMin = 'bilangan';
      if (maks && !INTEGER.test(maks)) errors.panjangMaks = 'bilangan';
      if (!errors.panjangMin && !errors.panjangMaks && min && maks && Number(min) > Number(maks)) {
        errors.panjangMaks = 'rentang';
      }
      if (!errors.panjangMin && !errors.panjangMaks) {
        rule = {
          jenis: 'panjang',
          ...(min ? { panjang_min: Number(min) } : {}),
          ...(maks ? { panjang_maks: Number(maks) } : {}),
        };
      }
      break;
    }
    case 'format_tanggal':
      if (!draft.formatTanggal.trim()) errors.formatTanggal = 'wajib';
      else rule = { jenis: 'format_tanggal', format_tanggal: draft.formatTanggal.trim() };
      break;
    case '':
      break;
  }

  if (Object.keys(errors).length > 0) return { ok: false, errors };
  const nilai = draft.nilai.trim();
  return {
    ok: true,
    row: { nama, ...(nilai ? { nilai } : {}), ...(rule ? { rule } : {}) },
  };
}

/** Daftar parameter dari teks JSON draft; `null` bila teks bukan daftar yang sah. */
export function parseParameters(text: string): ParameterRule[] | null {
  try {
    const value: unknown = JSON.parse(text);
    if (!Array.isArray(value)) return null;
    return value.filter(
      (row): row is ParameterRule =>
        typeof row === 'object' && row !== null && typeof (row as ParameterRule).nama === 'string',
    );
  } catch {
    return null;
  }
}

export function serializeParameters(rows: readonly ParameterRule[]): string {
  return JSON.stringify(rows, null, 2);
}
