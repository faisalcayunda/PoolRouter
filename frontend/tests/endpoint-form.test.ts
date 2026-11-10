import { describe, expect, it } from 'vitest';
import type { Endpoint } from '../src/api/client';
import {
  draftFromEndpoint,
  emptyDraft,
  emptyRow,
  parseParameterLiteral,
  serverErrors,
  toEndpointConfig,
  type EndpointDraft,
} from '../src/features/endpoint/endpointForm';

/** Model form endpoint dan kontrak metadata (ARC-103). Data sintetis. */
const PROVIDER_ID = '00000000-0000-4000-8000-0000000000a1';
const POOL_ID = '00000000-0000-4000-8000-0000000000b1';

const ENDPOINT: Endpoint = {
  endpoint_id: '00000000-0000-4000-8000-0000000000c1',
  provider_id: PROVIDER_ID,
  downstream_pool_id: POOL_ID,
  parameter_literal: { TRESHOLD: '90', USER_ID: 'a=b' },
  log_mode: 'metadata_only',
  metadata_fields: [
    {
      nama_field: 'status_penerima_manfaat',
      tipe: 'enum',
      wajib: true,
      klasifikasi: 'Internal',
      kirim_ke_data_balikan: true,
      deskripsi: 'Status penerima',
      nilai_diizinkan: ['aktif', 'nonaktif'],
      aturan_validasi: 'rule-status',
    },
  ],
  config_schema_version: 1,
  endpoint_config_version: 3,
  aktif: true,
};

function draftWith(change: Partial<EndpointDraft>): EndpointDraft {
  return { ...emptyDraft(), provider_id: PROVIDER_ID, downstream_pool_id: POOL_ID, ...change };
}

describe('parseParameterLiteral', () => {
  it('splits on the first = only and skips blank lines', () => {
    expect(parseParameterLiteral('TRESHOLD=90\n\n  USER_ID = a=b  \n')).toEqual({
      ok: true,
      value: { TRESHOLD: '90', USER_ID: 'a=b' },
    });
  });

  it('rejects a line without = or with a blank name', () => {
    expect(parseParameterLiteral('TRESHOLD 90')).toMatchObject({
      ok: false,
      kode: 'parameter_format',
    });
    expect(parseParameterLiteral('=90')).toMatchObject({ ok: false, kode: 'parameter_format' });
  });

  it('rejects a repeated name instead of silently keeping the last', () => {
    expect(parseParameterLiteral('A=1\nA=2')).toMatchObject({
      ok: false,
      kode: 'parameter_duplikat',
      nama: 'A',
    });
  });
});

describe('toEndpointConfig', () => {
  it('builds a create body with schema version 1 and no reason', () => {
    const row = { ...emptyRow(), nama_field: 'kode_wilayah', deskripsi: 'Kode' };
    const built = toEndpointConfig(
      draftWith({ parameter_literal: 'TRESHOLD=90', metadata_fields: [row] }),
      'create',
    );
    expect(built).toEqual({
      ok: true,
      body: {
        provider_id: PROVIDER_ID,
        downstream_pool_id: POOL_ID,
        parameter_literal: { TRESHOLD: '90' },
        log_mode: 'metadata_only',
        metadata_fields: [
          {
            nama_field: 'kode_wilayah',
            tipe: 'string',
            wajib: false,
            klasifikasi: 'Internal',
            kirim_ke_data_balikan: false,
            deskripsi: 'Kode',
          },
        ],
        config_schema_version: 1,
      },
    });
  });

  it('sends allowed values only for enum rows and omits a blank validation rule', () => {
    const enumRow = {
      ...emptyRow(),
      nama_field: 'status',
      tipe: 'enum' as const,
      nilai_diizinkan: ' aktif, ,nonaktif ',
      aturan_validasi: '  ',
    };
    const textRow = { ...emptyRow(), nama_field: 'catatan', nilai_diizinkan: 'sisa-isian' };
    const built = toEndpointConfig(draftWith({ metadata_fields: [enumRow, textRow] }), 'create');
    if (!built.ok) throw new Error('expected ok');
    expect(built.body.metadata_fields[0]).toMatchObject({ nilai_diizinkan: ['aktif', 'nonaktif'] });
    expect(built.body.metadata_fields[0]).not.toHaveProperty('aturan_validasi');
    expect(built.body.metadata_fields[1]).not.toHaveProperty('nilai_diizinkan');
  });

  it('never sends a Rahasia field to data balikan, even if the box was ticked first', () => {
    const row = {
      ...emptyRow(),
      nama_field: 'nilai_sensitif',
      klasifikasi: 'Rahasia' as const,
      kirim_ke_data_balikan: true,
    };
    const built = toEndpointConfig(draftWith({ metadata_fields: [row] }), 'create');
    if (!built.ok) throw new Error('expected ok');
    expect(built.body.metadata_fields[0]?.kirim_ke_data_balikan).toBe(false);
  });

  it('requires provider, pool and, on update, a 10..500 character reason', () => {
    const built = toEndpointConfig(emptyDraft(), 'update');
    expect(built.ok).toBe(false);
    if (built.ok) return;
    expect(Object.keys(built.errors.fields).sort()).toEqual([
      'alasan',
      'downstream_pool_id',
      'provider_id',
    ]);
    const short = toEndpointConfig(draftWith({ alasan: 'terlalu' }), 'update');
    expect(short.ok).toBe(false);
    const fine = toEndpointConfig(draftWith({ alasan: 'Sesuai surat perubahan.' }), 'update');
    expect(fine).toMatchObject({ ok: true, body: { alasan: 'Sesuai surat perubahan.' } });
  });

  it('refuses a log mode looser than the provider before calling the server', () => {
    const built = toEndpointConfig(
      draftWith({ log_mode: 'full_payload' }),
      'create',
      'metadata_only',
    );
    expect(built).toMatchObject({
      ok: false,
      errors: { fields: { log_mode: { kode: 'log_mode_lebih_longgar' } } },
    });
    expect(
      toEndpointConfig(draftWith({ log_mode: 'full_payload' }), 'create', 'full_payload').ok,
    ).toBe(true);
  });

  it('flags a bad, repeated or enum-without-values field name on the right row', () => {
    const bad = { ...emptyRow(), nama_field: 'Status-Baru' };
    const first = { ...emptyRow(), nama_field: 'status' };
    const again = { ...emptyRow(), nama_field: 'status' };
    const enumEmpty = { ...emptyRow(), nama_field: 'pilihan', tipe: 'enum' as const };
    const built = toEndpointConfig(
      draftWith({ metadata_fields: [bad, first, again, enumEmpty] }),
      'create',
    );
    if (built.ok) throw new Error('expected errors');
    expect(built.errors.rows[bad.key]?.nama_field?.kode).toBe('nama_field_format');
    expect(built.errors.rows[first.key]).toBeUndefined();
    expect(built.errors.rows[again.key]?.nama_field?.kode).toBe('nama_field_duplikat');
    expect(built.errors.rows[enumEmpty.key]?.nilai_diizinkan?.kode).toBe('nilai_diizinkan_wajib');
  });

  it('round-trips a read endpoint without losing any contract field (PUT is a full replacement)', () => {
    const built = toEndpointConfig(draftFromEndpoint(ENDPOINT), 'create');
    if (!built.ok) throw new Error('expected ok');
    expect(built.body).toEqual({
      provider_id: ENDPOINT.provider_id,
      downstream_pool_id: ENDPOINT.downstream_pool_id,
      parameter_literal: ENDPOINT.parameter_literal,
      log_mode: ENDPOINT.log_mode,
      metadata_fields: ENDPOINT.metadata_fields,
      config_schema_version: 1,
    });
  });
});

describe('serverErrors', () => {
  it('puts a pointer under metadata_fields/<n>/<field> on that row', () => {
    const rows = [emptyRow(), emptyRow()];
    const mapped = serverErrors(
      {
        kode: 'validasi_gagal',
        errors: [{ field: '/metadata_fields/1/nama_field', kode: 'format', batas: 64 }],
      },
      rows,
    );
    expect(mapped?.errors.rows[rows[1]!.key]?.nama_field).toMatchObject({ kode: 'format' });
    expect(mapped?.errors.rows[rows[0]!.key]).toBeUndefined();
  });

  it('maps the M-R1 problem even when the body carries no error list', () => {
    const mapped = serverErrors({ kode: 'metadata_wajib_hilang' }, []);
    expect(mapped?.errors.fields.metadata_fields).toEqual({ kode: 'wajib_hilang' });
  });

  it('keeps top-level fields and sends unknown pointers to lainnya', () => {
    const mapped = serverErrors(
      {
        kode: 'validasi_gagal',
        errors: [
          { field: '/log_mode', kode: 'lebih_longgar' },
          { field: '/config_schema_version', kode: 'tidak_didukung' },
        ],
      },
      [],
    );
    expect(mapped?.errors.fields.log_mode?.kode).toBe('lebih_longgar');
    expect(mapped?.lainnya.map((i) => i.kode)).toEqual(['tidak_didukung']);
  });

  it('returns null for a problem with neither errors nor the M-R1 code', () => {
    expect(serverErrors({ kode: 'tidak_ditemukan' }, [])).toBeNull();
    expect(serverErrors(null, [])).toBeNull();
  });
});
