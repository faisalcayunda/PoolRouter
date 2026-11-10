import { describe, expect, it } from 'vitest';
import { toProviderConfig, emptyDraft } from '../src/features/provider/providerForm';
import {
  buildParameter,
  draftFromParameter,
  emptyRuleDraft,
  parseParameters,
  serializeParameters,
  type ParameterRule,
  type RuleDraft,
} from '../src/features/provider/ruleEditor';

const base = (patch: Partial<RuleDraft>): RuleDraft => ({
  ...emptyRuleDraft(),
  nama: 'NIK',
  ...patch,
});

describe('buildParameter: editor rule terstruktur menghasilkan bentuk ParameterRule kontrak', () => {
  it('parameter tanpa rule hanya membawa nama dan nilai', () => {
    expect(buildParameter(base({ nilai: '90' }), [], null)).toEqual({
      ok: true,
      row: { nama: 'NIK', nilai: '90' },
    });
    expect(buildParameter(base({}), [], null)).toEqual({ ok: true, row: { nama: 'NIK' } });
  });

  it('regex hanya membawa pola', () => {
    expect(buildParameter(base({ jenis: 'regex', pola: '^\\d{16}$' }), [], null)).toEqual({
      ok: true,
      row: { nama: 'NIK', rule: { jenis: 'regex', pola: '^\\d{16}$' } },
    });
  });

  it('enum memecah satu nilai per baris dan membuang baris kosong', () => {
    expect(buildParameter(base({ jenis: 'enum', nilaiDiizinkan: ' A \n\nB\n' }), [], null)).toEqual(
      { ok: true, row: { nama: 'NIK', rule: { jenis: 'enum', nilai_diizinkan: ['A', 'B'] } } },
    );
  });

  it('panjang memakai bilangan bulat dan boleh sebelah saja', () => {
    expect(
      buildParameter(base({ jenis: 'panjang', panjangMin: '16', panjangMaks: '16' }), [], null),
    ).toEqual({
      ok: true,
      row: { nama: 'NIK', rule: { jenis: 'panjang', panjang_min: 16, panjang_maks: 16 } },
    });
    expect(buildParameter(base({ jenis: 'panjang', panjangMaks: '3' }), [], null)).toEqual({
      ok: true,
      row: { nama: 'NIK', rule: { jenis: 'panjang', panjang_maks: 3 } },
    });
  });

  it('format_tanggal membawa format', () => {
    expect(
      buildParameter(base({ jenis: 'format_tanggal', formatTanggal: 'yyyy-MM-dd' }), [], null),
    ).toEqual({
      ok: true,
      row: { nama: 'NIK', rule: { jenis: 'format_tanggal', format_tanggal: 'yyyy-MM-dd' } },
    });
  });

  it('menolak isian yang tidak sah per field', () => {
    expect(buildParameter(base({ nama: ' ' }), [], null)).toMatchObject({
      ok: false,
      errors: { nama: 'wajib' },
    });
    expect(buildParameter(base({}), ['NIK'], null)).toMatchObject({
      ok: false,
      errors: { nama: 'duplikat' },
    });
    expect(buildParameter(base({}), ['NIK'], 0)).toMatchObject({ ok: true });
    expect(buildParameter(base({ jenis: 'regex' }), [], null)).toMatchObject({
      errors: { pola: 'wajib' },
    });
    expect(buildParameter(base({ jenis: 'enum', nilaiDiizinkan: '\n' }), [], null)).toMatchObject({
      errors: { nilaiDiizinkan: 'wajib' },
    });
    expect(buildParameter(base({ jenis: 'panjang' }), [], null)).toMatchObject({
      errors: { panjangMin: 'wajib' },
    });
    expect(
      buildParameter(base({ jenis: 'panjang', panjangMin: '5', panjangMaks: '2' }), [], null),
    ).toMatchObject({ errors: { panjangMaks: 'rentang' } });
    expect(buildParameter(base({ jenis: 'panjang', panjangMin: '1.5' }), [], null)).toMatchObject({
      errors: { panjangMin: 'bilangan' },
    });
    expect(buildParameter(base({ jenis: 'format_tanggal' }), [], null)).toMatchObject({
      errors: { formatTanggal: 'wajib' },
    });
  });

  it('mengisi ulang editor dari parameter yang ada dan kembali ke bentuk yang sama', () => {
    const row: ParameterRule = {
      nama: 'NIK',
      nilai: '1',
      rule: { jenis: 'panjang', panjang_min: 1, panjang_maks: 9 },
    };
    expect(buildParameter(draftFromParameter(row), [], null)).toEqual({ ok: true, row });
    const en: ParameterRule = { nama: 'K', rule: { jenis: 'enum', nilai_diizinkan: ['x', 'y'] } };
    expect(buildParameter(draftFromParameter(en), [], null)).toEqual({ ok: true, row: en });
  });
});

describe('daftar parameter ke payload ProviderConfig', () => {
  it('serialize lalu toProviderConfig mengirim parameters persis seperti daftar editor', () => {
    const rows = [
      { nama: 'NIK', rule: { jenis: 'regex', pola: '^\\d{16}$' } },
      { nama: 'TRESHOLD', nilai: '90' },
    ] as const;
    const draft = {
      ...emptyDraft(),
      nama: 'p',
      base_url: 'https://host.example',
      body_template: '{}',
      parameters: serializeParameters(rows),
    };
    const built = toProviderConfig(draft, 'create');
    expect(built.ok && built.body.parameters).toEqual(rows);
  });

  it('parseParameters mengembalikan null untuk teks yang bukan daftar', () => {
    expect(parseParameters('[{')).toBeNull();
    expect(parseParameters('{}')).toBeNull();
    expect(parseParameters('[]')).toEqual([]);
  });
});
