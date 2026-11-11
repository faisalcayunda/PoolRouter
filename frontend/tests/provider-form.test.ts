import { describe, expect, it } from 'vitest';
import type { Provider } from '../src/api/client';
import {
  draftFromProvider,
  emptyDraft,
  firstInvalid,
  serverErrors,
  toProviderConfig,
} from '../src/features/provider/providerForm';

/** Pin sintetis: base64 dari 32 byte. */
const PIN_A = 'AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=';
const PIN_B = 'ICEiIyQlJicoKSorLC0uLzAxMjM0NTY3ODk6Ozw9Pj8=';
const PEM = '-----BEGIN CERTIFICATE-----\nMIIBsynthetic\n-----END CERTIFICATE-----';

/** Provider v2 sintetis lengkap; `data_balikan` dan `parameters` harus kembali utuh saat PUT. */
const PROVIDER_V2 = {
  provider_id: '00000000-0000-4000-8000-0000000000a1',
  provider_config_version: 3,
  aktif: true,
  nama: 'provider-contoh-a',
  base_url: 'https://provider.example.test/api',
  metode: 'POST',
  tls_trust: 'pin_spki',
  tls_pin_spki: [PIN_A, PIN_B],
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
      tls_pin_spki: [PIN_A, PIN_B],
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

describe('bahan TLS per mode (ARC-239)', () => {
  const dasar = {
    ...emptyDraft(),
    nama: 'provider-contoh-a',
    base_url: 'https://provider.example.test/api',
    body_template: '{}',
  };

  it('sends tls_ca_pem only for ca_kustom and omits the pins', () => {
    const built = toProviderConfig(
      { ...dasar, tls_trust: 'ca_kustom', tls_ca_pem: ` ${PEM}\n`, tls_pin_spki: PIN_A },
      'create',
    );
    expect(built.ok).toBe(true);
    if (!built.ok) return;
    expect(built.body.tls_ca_pem).toBe(PEM);
    expect(built.body).not.toHaveProperty('tls_pin_spki');
  });

  it('sends one pin per non-empty line for pin_spki and omits the PEM', () => {
    const built = toProviderConfig(
      {
        ...dasar,
        tls_trust: 'pin_spki',
        tls_ca_pem: PEM,
        tls_pin_spki: `${PIN_A}\r\n\n ${PIN_B} `,
      },
      'create',
    );
    expect(built.ok).toBe(true);
    if (!built.ok) return;
    expect(built.body.tls_pin_spki).toEqual([PIN_A, PIN_B]);
    expect(built.body).not.toHaveProperty('tls_ca_pem');
  });

  it('sends neither for system, even when the draft still holds leftovers', () => {
    const built = toProviderConfig(
      { ...dasar, tls_trust: 'system', tls_ca_pem: PEM, tls_pin_spki: PIN_A },
      'create',
    );
    expect(built.ok).toBe(true);
    if (built.ok) {
      expect(built.body).not.toHaveProperty('tls_ca_pem');
      expect(built.body).not.toHaveProperty('tls_pin_spki');
    }
  });

  it('requires the material of the chosen mode', () => {
    const ca = toProviderConfig({ ...dasar, tls_trust: 'ca_kustom' }, 'create');
    const pin = toProviderConfig(
      { ...dasar, tls_trust: 'pin_spki', tls_pin_spki: ' \n' },
      'create',
    );
    expect(ca.ok || ca.errors.tls_ca_pem?.kode).toBe('wajib');
    expect(pin.ok || pin.errors.tls_pin_spki?.kode).toBe('wajib');
  });

  it('round-trips a ca_kustom provider through the draft', () => {
    const ca = {
      ...PROVIDER_V2,
      tls_trust: 'ca_kustom',
      tls_ca_pem: PEM,
      tls_pin_spki: null,
    } as unknown as Provider;
    const draft = draftFromProvider(ca);
    expect(draft.tls_ca_pem).toBe(PEM);
    expect(draft.tls_pin_spki).toBe('');
  });

  it('maps server 422 codes on the TLS fields, including a pin index', () => {
    const mapped = serverErrors({
      kode: 'validasi_gagal',
      errors: [
        { field: '/tls_ca_pem', kode: 'pem_tidak_sah' },
        { field: '/tls_pin_spki/1', kode: 'pin_tidak_sah' },
      ],
    } as never);
    expect(mapped?.fields.tls_ca_pem?.kode).toBe('pem_tidak_sah');
    expect(mapped?.fields.tls_pin_spki?.kode).toBe('pin_tidak_sah');
    expect(mapped?.lainnya).toEqual([]);
    expect(firstInvalid(mapped?.fields ?? {})).toBe('tls_ca_pem');
  });
});
