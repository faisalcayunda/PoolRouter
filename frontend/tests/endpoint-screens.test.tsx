import { beforeAll, describe, expect, it, vi } from 'vitest';
import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import type { i18n } from 'i18next';
import type { ReactNode } from 'react';
import {
  ApiError,
  type ApiClient,
  type DownstreamPool,
  type Endpoint,
  type EndpointConfig,
  type Provider,
} from '../src/api/client';
import { RoleProvider, type AdminRole } from '../src/features/common/RoleContext';
import { EndpointFormScreen } from '../src/features/endpoint/EndpointFormScreen';
import { EndpointListScreen } from '../src/features/endpoint/EndpointListScreen';
import { createI18n, loadLanguage } from '../src/i18n/i18n';
import { renderWithI18n } from './render';

/** S04b dan S07b (ARC-103). Data sintetis. */
const user = userEvent.setup({ delay: null });

const PROVIDER_ID = '00000000-0000-4000-8000-0000000000a1';
const PROVIDER_FULL_ID = '00000000-0000-4000-8000-0000000000a2';
const POOL_ID = '00000000-0000-4000-8000-0000000000b1';

const PROVIDERS = [
  { provider_id: PROVIDER_ID, nama: 'provider-metadata', log_mode: 'metadata_only' },
  { provider_id: PROVIDER_FULL_ID, nama: 'provider-penuh', log_mode: 'full_payload' },
] as unknown as Provider[];
const POOLS = [{ downstream_pool_id: POOL_ID, nama: 'pool-contoh' }] as unknown as DownstreamPool[];

const ENDPOINT: Endpoint = {
  endpoint_id: '00000000-0000-4000-8000-0000000000c1',
  provider_id: PROVIDER_ID,
  downstream_pool_id: POOL_ID,
  parameter_literal: { TRESHOLD: '90' },
  log_mode: 'metadata_only',
  metadata_fields: [
    {
      nama_field: 'status_penerima_manfaat',
      tipe: 'string',
      wajib: true,
      klasifikasi: 'Internal',
      kirim_ke_data_balikan: true,
      deskripsi: 'Status penerima',
    },
    {
      nama_field: 'kode_wilayah',
      tipe: 'string',
      wajib: false,
      klasifikasi: 'Terbatas',
      kirim_ke_data_balikan: false,
      deskripsi: 'Kode wilayah',
    },
  ],
  config_schema_version: 1,
  endpoint_config_version: 3,
  aktif: true,
};
const INACTIVE: Endpoint = {
  ...ENDPOINT,
  endpoint_id: '00000000-0000-4000-8000-0000000000c2',
  provider_id: PROVIDER_FULL_ID,
  aktif: false,
  endpoint_config_version: 1,
  metadata_fields: ENDPOINT.metadata_fields.slice(0, 1),
};

function stubClient(overrides: Partial<ApiClient> = {}): ApiClient {
  return {
    listEndpoints: vi.fn().mockResolvedValue([]),
    listProviders: vi.fn().mockResolvedValue(PROVIDERS),
    listDownstreamPools: vi.fn().mockResolvedValue(POOLS),
    getEndpoint: vi.fn().mockResolvedValue(ENDPOINT),
    createEndpoint: vi.fn(),
    updateEndpoint: vi.fn(),
    aktifkanEndpoint: vi.fn(),
    pratinjauNonaktifkanEndpoint: vi.fn().mockResolvedValue({
      akan_cancelled: 4,
      akan_failed: 2,
      dihitung_pada: '2026-11-10T00:00:00Z',
    }),
    nonaktifkanEndpoint: vi.fn(),
    ...overrides,
  } as unknown as ApiClient;
}

let en: i18n;

beforeAll(async () => {
  en = createI18n({ throwOnMissingKey: true });
  await loadLanguage(en, 'en');
});

function asRole(role: AdminRole | null, ui: ReactNode, instance?: i18n) {
  return renderWithI18n(<RoleProvider current={role}>{ui}</RoleProvider>, { instance });
}

async function choose(selectName: RegExp, optionName: string) {
  await user.click(screen.getByRole('button', { name: selectName }));
  await user.click(await screen.findByRole('option', { name: optionName }));
}

async function paste(
  label: string,
  text: string,
  scope: { getByLabelText: typeof screen.getByLabelText } = screen,
) {
  const field = scope.getByLabelText(label);
  await user.click(field);
  await user.paste(text);
}

describe('S04b daftar endpoint', () => {
  it('shows the empty state with the add action', async () => {
    asRole('platform_admin', <EndpointListScreen client={stubClient()} />);

    expect(await screen.findByRole('heading', { name: 'Belum ada endpoint' })).toBeVisible();
    expect(screen.getByRole('link', { name: 'Tambah endpoint' })).toHaveAttribute(
      'href',
      '/endpoint/baru',
    );
  });

  it('resolves provider and pool names and offers the action that fits each status', async () => {
    const client = stubClient({ listEndpoints: vi.fn().mockResolvedValue([ENDPOINT, INACTIVE]) });
    asRole('platform_admin', <EndpointListScreen client={client} />);

    const table = within(await screen.findByRole('table'));
    const active = within(table.getByText('provider-metadata').closest('tr')!);
    expect(active.getByText('pool-contoh')).toBeInTheDocument();
    expect(active.getByText('2 field')).toBeInTheDocument();
    expect(active.getByText('Aktif')).toBeInTheDocument();
    expect(active.getByRole('link', { name: 'Ubah endpoint provider-metadata' })).toHaveAttribute(
      'href',
      `/endpoint/${ENDPOINT.endpoint_id}`,
    );
    expect(
      active.getByRole('button', { name: 'Nonaktifkan endpoint provider-metadata' }),
    ).toBeVisible();
    expect(active.queryByRole('button', { name: /^Aktifkan endpoint/ })).toBeNull();

    const off = within(table.getByText('provider-penuh').closest('tr')!);
    expect(off.getByText('Nonaktif')).toBeInTheDocument();
    expect(off.getByRole('button', { name: 'Aktifkan endpoint provider-penuh' })).toBeVisible();
    expect(off.queryByRole('button', { name: /^Nonaktifkan endpoint/ })).toBeNull();
  });

  it('activates an inactive endpoint without a dialog and reloads the list', async () => {
    const listEndpoints = vi
      .fn()
      .mockResolvedValueOnce([INACTIVE])
      .mockResolvedValueOnce([{ ...INACTIVE, aktif: true }]);
    const aktifkanEndpoint = vi.fn().mockResolvedValue({ ...INACTIVE, aktif: true });
    asRole(
      'platform_admin',
      <EndpointListScreen client={stubClient({ listEndpoints, aktifkanEndpoint })} />,
    );

    await user.click(
      await screen.findByRole('button', { name: 'Aktifkan endpoint provider-penuh' }),
    );

    await waitFor(() => expect(aktifkanEndpoint).toHaveBeenCalledWith(INACTIVE.endpoint_id));
    expect(
      await screen.findByRole('button', { name: 'Nonaktifkan endpoint provider-penuh' }),
    ).toBeVisible();
    expect(listEndpoints).toHaveBeenCalledTimes(2);
    expect(screen.queryByRole('dialog')).toBeNull();
  });

  it('previews the K-20 impact, insists on a reason, then deactivates with that reason', async () => {
    const listEndpoints = vi
      .fn()
      .mockResolvedValueOnce([ENDPOINT])
      .mockResolvedValueOnce([{ ...ENDPOINT, aktif: false }]);
    const nonaktifkanEndpoint = vi.fn().mockResolvedValue({
      jumlah_cancelled: 4,
      jumlah_failed: 2,
      dilakukan_pada: '2026-11-10T00:00:00Z',
    });
    const client = stubClient({ listEndpoints, nonaktifkanEndpoint });
    asRole('platform_admin', <EndpointListScreen client={client} />);

    await user.click(
      await screen.findByRole('button', { name: 'Nonaktifkan endpoint provider-metadata' }),
    );
    const dialog = within(await screen.findByRole('dialog'));
    expect(await dialog.findByText('4 request akan dibatalkan.')).toBeVisible();
    expect(dialog.getByText('2 request akan digagalkan.')).toBeVisible();
    expect(client.pratinjauNonaktifkanEndpoint).toHaveBeenCalledWith(
      ENDPOINT.endpoint_id,
      expect.anything(),
    );

    await paste('Alasan penonaktifan', 'pendek', dialog);
    await user.click(dialog.getByRole('button', { name: 'Nonaktifkan endpoint' }));
    expect(dialog.getByText('Alasan wajib diisi, 10–500 karakter.')).toBeVisible();
    expect(nonaktifkanEndpoint).not.toHaveBeenCalled();

    await user.click(dialog.getByLabelText('Alasan penonaktifan'));
    await user.paste(' dan kontrak berakhir.');
    await user.click(dialog.getByRole('button', { name: 'Nonaktifkan endpoint' }));

    await waitFor(() =>
      expect(nonaktifkanEndpoint).toHaveBeenCalledWith(
        ENDPOINT.endpoint_id,
        'pendek dan kontrak berakhir.',
      ),
    );
    expect(await screen.findByText('Endpoint provider-metadata dinonaktifkan.')).toBeVisible();
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(await screen.findByText('Nonaktif')).toBeVisible();
  });

  it('offers no confirm button until the preview has loaded', async () => {
    const pratinjauNonaktifkanEndpoint = vi.fn().mockRejectedValue(new TypeError('network'));
    asRole(
      'platform_admin',
      <EndpointListScreen
        client={stubClient({
          listEndpoints: vi.fn().mockResolvedValue([ENDPOINT]),
          pratinjauNonaktifkanEndpoint,
        })}
      />,
    );

    await user.click(
      await screen.findByRole('button', { name: 'Nonaktifkan endpoint provider-metadata' }),
    );
    const dialog = within(await screen.findByRole('dialog'));
    expect(await dialog.findByRole('button', { name: 'Coba lagi' })).toBeVisible();
    expect(dialog.queryByRole('button', { name: 'Nonaktifkan endpoint' })).toBeNull();
  });

  it('shows an error with a retry that loads again', async () => {
    const listEndpoints = vi
      .fn()
      .mockRejectedValueOnce(new TypeError('network'))
      .mockResolvedValueOnce([ENDPOINT]);
    asRole('platform_admin', <EndpointListScreen client={stubClient({ listEndpoints })} />);

    await user.click(await screen.findByRole('button', { name: 'Coba lagi' }));
    expect(await screen.findByText('provider-metadata')).toBeInTheDocument();
    expect(listEndpoints).toHaveBeenCalledTimes(2);
  });

  it.each(['operator', 'auditor'] as const)('shows 403 to %s without calling the API', (role) => {
    const client = stubClient();
    asRole(role, <EndpointListScreen client={client} />);

    expect(
      screen.getByRole('heading', { level: 1, name: 'Anda tidak punya akses ke halaman ini' }),
    ).toBeInTheDocument();
    expect(client.listEndpoints).not.toHaveBeenCalled();
  });

  it('renders in English with a singular field count', async () => {
    const one = { ...ENDPOINT, metadata_fields: ENDPOINT.metadata_fields.slice(0, 1) };
    asRole(
      'platform_admin',
      <EndpointListScreen
        client={stubClient({ listEndpoints: vi.fn().mockResolvedValue([one]) })}
      />,
      en,
    );

    expect(await screen.findByRole('heading', { level: 1, name: 'Endpoints' })).toBeVisible();
    expect(screen.getByText('1 field')).toBeVisible();
    expect(
      screen.getByRole('button', { name: 'Deactivate endpoint provider-metadata' }),
    ).toBeVisible();
  });
});

describe('S04b + S07b form endpoint', () => {
  it('renders the form with its metadata rows in English without a missing key', async () => {
    asRole(
      'platform_admin',
      <EndpointFormScreen client={stubClient()} endpointId={ENDPOINT.endpoint_id} />,
      en,
    );

    expect(await screen.findByRole('heading', { level: 1, name: 'Edit endpoint' })).toBeVisible();
    const group = within(screen.getByRole('group', { name: 'Field 1' }));
    expect(group.getByLabelText('Field name')).toHaveValue('status_penerima_manfaat');
    expect(group.getByRole('checkbox', { name: 'Required from the client' })).toBeChecked();
    expect(screen.getByRole('button', { name: 'Save changes' })).toBeVisible();
  });

  it('creates an endpoint with parameters and a metadata contract, then moves to edit mode', async () => {
    const createEndpoint = vi.fn().mockResolvedValue({ ...ENDPOINT, endpoint_config_version: 1 });
    asRole('platform_admin', <EndpointFormScreen client={stubClient({ createEndpoint })} />);

    await screen.findByRole('heading', { level: 1, name: 'Tambah endpoint' });
    await choose(/Provider/, 'provider-metadata');
    await choose(/Downstream pool/, 'pool-contoh');
    await paste('Parameter literal', 'TRESHOLD=90');
    await user.click(screen.getByRole('button', { name: 'Tambah field' }));
    const group = within(screen.getByRole('group', { name: 'Field ke-1' }));
    await paste('Nama field', 'status_penerima_manfaat', group);
    await paste('Deskripsi', 'Status penerima manfaat', group);
    await user.click(group.getByRole('checkbox', { name: 'Wajib diisi client' }));
    await user.click(group.getByRole('checkbox', { name: 'Kirim ke data balikan' }));
    await user.click(screen.getByRole('button', { name: 'Simpan endpoint' }));

    await waitFor(() => expect(createEndpoint).toHaveBeenCalledTimes(1));
    const body = createEndpoint.mock.calls[0]![0] as EndpointConfig;
    expect(body).toEqual({
      provider_id: PROVIDER_ID,
      downstream_pool_id: POOL_ID,
      parameter_literal: { TRESHOLD: '90' },
      log_mode: 'metadata_only',
      metadata_fields: [
        {
          nama_field: 'status_penerima_manfaat',
          tipe: 'string',
          wajib: true,
          klasifikasi: 'Internal',
          kirim_ke_data_balikan: true,
          deskripsi: 'Status penerima manfaat',
        },
      ],
      config_schema_version: 1,
    });
    expect(await screen.findByText('Endpoint tersimpan sebagai versi 1.')).toBeInTheDocument();
    // Setelah dibuat, simpan berikutnya adalah PUT yang meminta alasan.
    expect(screen.getByLabelText('Alasan perubahan')).toBeInTheDocument();
  });

  it('shows the required-field errors and creates nothing', async () => {
    const createEndpoint = vi.fn();
    asRole('platform_admin', <EndpointFormScreen client={stubClient({ createEndpoint })} />);

    await screen.findByRole('heading', { level: 1, name: 'Tambah endpoint' });
    await user.click(screen.getByRole('button', { name: 'Simpan endpoint' }));

    expect(await screen.findByText('Periksa 2 isian yang ditandai.')).toBeVisible();
    expect(createEndpoint).not.toHaveBeenCalled();
  });

  it('fills the form from the read endpoint and PUTs every field back with the reason', async () => {
    const updateEndpoint = vi.fn().mockResolvedValue({ ...ENDPOINT, endpoint_config_version: 4 });
    asRole(
      'platform_admin',
      <EndpointFormScreen
        client={stubClient({ updateEndpoint })}
        endpointId={ENDPOINT.endpoint_id}
      />,
    );

    expect(await screen.findByRole('heading', { level: 1, name: 'Ubah endpoint' })).toBeVisible();
    expect(screen.getByLabelText('Parameter literal')).toHaveValue('TRESHOLD=90');
    const second = within(screen.getByRole('group', { name: 'Field ke-2' }));
    expect(second.getByLabelText('Nama field')).toHaveValue('kode_wilayah');
    await user.click(second.getByRole('button', { name: 'Hapus field' }));
    expect(screen.queryByRole('group', { name: 'Field ke-2' })).toBeNull();

    await paste('Alasan perubahan', 'Kode wilayah dipindah ke provider.');
    await user.click(screen.getByRole('button', { name: 'Simpan perubahan' }));

    await waitFor(() => expect(updateEndpoint).toHaveBeenCalledTimes(1));
    const [id, body] = updateEndpoint.mock.calls[0]! as [string, EndpointConfig];
    expect(id).toBe(ENDPOINT.endpoint_id);
    expect(body).toEqual({
      provider_id: PROVIDER_ID,
      downstream_pool_id: POOL_ID,
      parameter_literal: { TRESHOLD: '90' },
      log_mode: 'metadata_only',
      metadata_fields: [ENDPOINT.metadata_fields[0]],
      config_schema_version: 1,
      alasan: 'Kode wilayah dipindah ke provider.',
    });
    expect(await screen.findByText('Endpoint tersimpan sebagai versi 4.')).toBeVisible();
    await waitFor(() => expect(screen.getByLabelText('Alasan perubahan')).toHaveValue(''));
  });

  it('blocks a Rahasia field from data balikan and explains why', async () => {
    asRole(
      'platform_admin',
      <EndpointFormScreen client={stubClient()} endpointId={ENDPOINT.endpoint_id} />,
    );

    await screen.findByRole('heading', { level: 1, name: 'Ubah endpoint' });
    const first = within(screen.getByRole('group', { name: 'Field ke-1' }));
    expect(first.getByRole('checkbox', { name: 'Kirim ke data balikan' })).toBeChecked();
    // Kedua baris punya select Klasifikasi; ubah yang di baris pertama saja.
    await user.click(first.getByRole('button', { name: /Klasifikasi/ }));
    await user.click(await screen.findByRole('option', { name: 'Rahasia' }));
    expect(first.getByRole('checkbox', { name: 'Kirim ke data balikan' })).toBeDisabled();
    expect(first.getByRole('checkbox', { name: 'Kirim ke data balikan' })).not.toBeChecked();
    expect(first.getByText('Field Rahasia tidak pernah dikirim ke data balikan.')).toBeVisible();
  });

  it('asks for allowed values only when the type is enum', async () => {
    asRole('platform_admin', <EndpointFormScreen client={stubClient()} />);

    await screen.findByRole('heading', { level: 1, name: 'Tambah endpoint' });
    await user.click(screen.getByRole('button', { name: 'Tambah field' }));
    const group = within(screen.getByRole('group', { name: 'Field ke-1' }));
    expect(group.queryByLabelText('Nilai yang diizinkan')).toBeNull();
    await user.click(group.getByRole('button', { name: /Tipe/ }));
    await user.click(await screen.findByRole('option', { name: 'Pilihan tetap' }));
    expect(group.getByLabelText('Nilai yang diizinkan')).toBeVisible();
  });

  it('catches a log mode looser than the provider before calling the server', async () => {
    const createEndpoint = vi.fn();
    asRole('platform_admin', <EndpointFormScreen client={stubClient({ createEndpoint })} />);

    await screen.findByRole('heading', { level: 1, name: 'Tambah endpoint' });
    await choose(/Provider/, 'provider-metadata');
    await choose(/Downstream pool/, 'pool-contoh');
    await choose(/Mode log/, 'Payload lengkap');
    await user.click(screen.getByRole('button', { name: 'Simpan endpoint' }));

    expect(
      await screen.findByText(/Mode log endpoint tidak boleh lebih longgar dari provider/),
    ).toBeVisible();
    expect(createEndpoint).not.toHaveBeenCalled();
  });

  it('shows the M-R1 rejection on the metadata contract', async () => {
    const createEndpoint = vi.fn().mockRejectedValue(
      new ApiError(422, {
        kode: 'metadata_wajib_hilang',
        // Bentuk asli server (endpoint.rs): pointer tanpa garis miring depan, kode sama dengan problem.
        errors: [
          {
            field: 'metadata_fields',
            kode: 'metadata_wajib_hilang',
            nama: 'status_penerima_manfaat',
          },
        ],
      }),
    );
    asRole('platform_admin', <EndpointFormScreen client={stubClient({ createEndpoint })} />);

    await screen.findByRole('heading', { level: 1, name: 'Tambah endpoint' });
    await choose(/Provider/, 'provider-metadata');
    await choose(/Downstream pool/, 'pool-contoh');
    await user.click(screen.getByRole('button', { name: 'Simpan endpoint' }));

    expect(
      await screen.findByText(
        'Endpoint Dukcapil wajib memuat field status_penerima_manfaat di kontrak metadata.',
      ),
    ).toBeVisible();
  });

  it('blocks a metadata row without a description before calling the server', async () => {
    const createEndpoint = vi.fn();
    asRole('platform_admin', <EndpointFormScreen client={stubClient({ createEndpoint })} />);

    await screen.findByRole('heading', { level: 1, name: 'Tambah endpoint' });
    await choose(/Provider/, 'provider-metadata');
    await choose(/Downstream pool/, 'pool-contoh');
    await user.click(screen.getByRole('button', { name: 'Tambah field' }));
    const group = within(screen.getByRole('group', { name: 'Field ke-1' }));
    await paste('Nama field', 'kode_wilayah', group);
    await user.click(screen.getByRole('button', { name: 'Simpan endpoint' }));

    expect(await group.findByText('Wajib diisi.')).toBeVisible();
    expect(createEndpoint).not.toHaveBeenCalled();
  });

  it('shows a server description error on the description of the row it points at', async () => {
    const updateEndpoint = vi.fn().mockRejectedValue(
      new ApiError(422, {
        kode: 'validasi_gagal',
        errors: [{ field: 'metadata_fields/1/deskripsi', kode: 'wajib' }],
      }),
    );
    asRole(
      'platform_admin',
      <EndpointFormScreen
        client={stubClient({ updateEndpoint })}
        endpointId={ENDPOINT.endpoint_id}
      />,
    );

    await screen.findByRole('heading', { level: 1, name: 'Ubah endpoint' });
    await paste('Alasan perubahan', 'Perbaikan kontrak metadata.');
    await user.click(screen.getByRole('button', { name: 'Simpan perubahan' }));

    const second = within(await screen.findByRole('group', { name: 'Field ke-2' }));
    expect(await second.findByText('Wajib diisi.')).toBeVisible();
    expect(
      within(screen.getByRole('group', { name: 'Field ke-1' })).queryByText('Wajib diisi.'),
    ).toBeNull();
  });

  it('puts a server field error on the row it points at', async () => {
    const updateEndpoint = vi.fn().mockRejectedValue(
      new ApiError(422, {
        kode: 'validasi_gagal',
        errors: [{ field: '/metadata_fields/1/nama_field', kode: 'format', batas: 64 }],
      }),
    );
    asRole(
      'platform_admin',
      <EndpointFormScreen
        client={stubClient({ updateEndpoint })}
        endpointId={ENDPOINT.endpoint_id}
      />,
    );

    await screen.findByRole('heading', { level: 1, name: 'Ubah endpoint' });
    await paste('Alasan perubahan', 'Perbaikan kontrak metadata.');
    await user.click(screen.getByRole('button', { name: 'Simpan perubahan' }));

    const second = within(await screen.findByRole('group', { name: 'Field ke-2' }));
    expect(await second.findByText(/Isian ini belum sah \(kode format\)/)).toBeVisible();
    expect(
      within(screen.getByRole('group', { name: 'Field ke-1' })).queryByText(/belum sah/),
    ).toBeNull();
  });

  it('shows the load error with a retry and never builds the form', async () => {
    const getEndpoint = vi
      .fn()
      .mockRejectedValueOnce(new TypeError('network'))
      .mockResolvedValueOnce(ENDPOINT);
    asRole(
      'platform_admin',
      <EndpointFormScreen client={stubClient({ getEndpoint })} endpointId={ENDPOINT.endpoint_id} />,
    );

    await user.click(await screen.findByRole('button', { name: 'Coba lagi' }));
    expect(await screen.findByRole('heading', { level: 1, name: 'Ubah endpoint' })).toBeVisible();
  });

  it.each(['operator', 'auditor'] as const)('shows 403 to %s without calling the API', (role) => {
    const client = stubClient();
    asRole(role, <EndpointFormScreen client={client} />);

    expect(
      screen.getByRole('heading', { level: 1, name: 'Anda tidak punya akses ke halaman ini' }),
    ).toBeInTheDocument();
    expect(client.listProviders).not.toHaveBeenCalled();
  });
});
