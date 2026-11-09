import { describe, expect, it } from 'vitest';
import type { Provider } from '../src/api/client';
import {
  draftFromProvider,
  emptyDraft,
  firstInvalid,
  serverErrors,
  toProviderConfig,
} from '../src/features/provider/providerForm';

/** Provider v2 sintetis lengkap; `data_balikan` dan `parameters` harus kembali utuh saat PUT. */
const PROVIDER_V2 = {
  provider_id: '00000000-0000-4000-8000-0000000000a1',
  provider_config_version: 3,
  aktif: true,
  nama: 'provider-contoh-a',
  base_url: 'https://provider.example.test/api',
  metode: 'POST',
  tls_trust: 'pin_spki',
  log_mode: 'metadata_only',
  config_schema_version: 2,
  body_template: '{"NIK":"{{input.NIK}}","PWD":"{{secret.PASSWORD}}"}',
  parameters: [{ nama: 'TRESHOLD', nilai: '90', rule: { jenis: 'panjang', panjang_maks: 3 } }],
  respons_skema: { status: 'string' },
  data_balikan: { versi: 2, elemen: ['NAMA_LGKP'] },
} as unknown as Provider;

describe('toProviderConfig (PUT penggantian penuh, ARC-101)', () => {
  it('round-trips every ProviderConfig field of a v2 provider, including data_balikan', () => {
    const draft = { ...draftFromProvider(PROVIDER_V2), alasan: 'Ganti pin TLS sesuai surat BGN.' };
    const built = toProviderConfig(draft, 'update');

    expect(built.ok).toBe(true);
    if (!built.ok) return;
    expect(built.body).toEqual({
      nama: 'provider-contoh-a',
      base_url: 'https://provider.example.test/api',
      metode: 'POST',
      tls_trust: 'pin_spki',
      log_mode: 'metadata_only',
      config_schema_version: 2,
      body_template: PROVIDER_V2.body_template,
      parameters: PROVIDER_V2.parameters,
      respons_skema: PROVIDER_V2.respons_skema,
      data_balikan: PROVIDER_V2.data_balikan,
      alasan: 'Ganti pin TLS sesuai surat BGN.',
    });
    // Field baca-saja tidak ikut: kontrak `ProviderConfig` menolak properti tambahan.
    expect(built.body).not.toHaveProperty('provider_id');
    expect(built.body).not.toHaveProperty('aktif');
    expect(built.body).not.toHaveProperty('provider_config_version');
  });

  it('treats a null data_balikan on a v1 provider as absent and omits it from the PUT', () => {
    const v1 = {
      ...PROVIDER_V2,
      config_schema_version: 1,
      data_balikan: null,
    } as unknown as Provider;
    const draft = { ...draftFromProvider(v1), alasan: 'Perbarui template untuk uji.' };
    expect(draft.data_balikan).toBe('');

    const built = toProviderConfig(draft, 'update');
    expect(built.ok).toBe(true);
    if (built.ok) expect(built.body).not.toHaveProperty('data_balikan');
  });

  it('starts the reason empty and requires 10–500 characters on update only', () => {
    const draft = draftFromProvider(PROVIDER_V2);
    expect(draft.alasan).toBe('');

    const update = toProviderConfig(draft, 'update');
    expect(update.ok).toBe(false);
    if (!update.ok) expect(update.errors.alasan?.kode).toBe('alasan_rentang');

    const create = toProviderConfig({ ...draft, nama: 'baru' }, 'create');
    expect(create.ok).toBe(true);
    if (create.ok) expect(create.body).not.toHaveProperty('alasan');
  });

  it('flags invalid JSON, a non-https URL, and a missing data_balikan for v2', () => {
    const built = toProviderConfig(
      {
        ...emptyDraft(),
        nama: 'provider-contoh-b',
        base_url: 'http://provider.example.test',
        body_template: '{}',
        parameters: '[{',
        respons_skema: '[]',
        config_schema_version: 2,
      },
      'create',
    );

    expect(built.ok).toBe(false);
    if (built.ok) return;
    expect(built.errors).toEqual({
      base_url: { kode: 'https_wajib' },
      parameters: { kode: 'json_tidak_sah' },
      respons_skema: { kode: 'json_bukan_objek' },
      data_balikan: { kode: 'required_untuk_versi_2' },
    });
    expect(firstInvalid(built.errors)).toBe('base_url');
  });
});

describe('serverErrors (422 validasi_gagal)', () => {
  it('maps a JSON pointer to its top-level field and keeps position and name', () => {
    const mapped = serverErrors({
      kode: 'validasi_gagal',
      errors: [
        {
          field: '/body_template',
          kode: 'template_tidak_seimbang',
          posisi: { baris: 1, kolom: 9 },
        },
        { field: '/parameters/0/rule/pola', kode: 'regex_tidak_sah' },
        { field: '/base_url', kode: 'format' },
        { field: '/alasan', kode: 'rentang', batas: 10 },
        { field: '/config_schema_version', kode: 'config_schema_version_tidak_didukung' },
      ],
    });

    expect(mapped?.fields.body_template).toMatchObject({
      kode: 'template_tidak_seimbang',
      posisi: { baris: 1, kolom: 9 },
    });
    expect(mapped?.fields.parameters).toMatchObject({
      kode: 'regex_tidak_sah',
      lokasi: '/parameters/0/rule/pola',
    });
    expect(mapped?.fields.base_url?.kode).toBe('https_wajib');
    expect(mapped?.fields.alasan?.kode).toBe('alasan_rentang');
    // Select tidak punya teks galat sendiri, jadi galatnya masuk ringkasan di atas form.
    expect(mapped?.fields.config_schema_version).toBeUndefined();
    expect(mapped?.lainnya).toEqual([
      {
        kode: 'config_schema_version_tidak_didukung',
        posisi: null,
        nama: null,
        lokasi: '/config_schema_version',
      },
    ]);
  });

  it('returns null for a problem without field errors', () => {
    expect(serverErrors({ kode: 'akses_ditolak' })).toBeNull();
  });
});
