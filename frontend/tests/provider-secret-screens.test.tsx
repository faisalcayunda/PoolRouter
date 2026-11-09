import { describe, expect, it, vi } from 'vitest';
import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import type { ReactNode } from 'react';
import {
  ApiError,
  type ApiClient,
  type Provider,
  type ProviderConfig,
  type SecretRingkas,
} from '../src/api/client';
import { RoleProvider, type AdminRole } from '../src/features/common/RoleContext';
import { ProviderFormScreen } from '../src/features/provider/ProviderFormScreen';
import { ProviderListScreen } from '../src/features/provider/ProviderListScreen';
import { SecretScreen } from '../src/features/secret/SecretScreen';
import { renderWithI18n } from './render';

/** S02, S03–S04, S05, S05b (ARC-101). Data sintetis; nilai secret berisi teks contoh. */
const user = userEvent.setup({ delay: null });

const PROVIDER = {
  provider_id: '00000000-0000-4000-8000-0000000000a1',
  provider_config_version: 3,
  aktif: true,
  nama: 'provider-contoh-a',
  base_url: 'https://provider.example.test/api',
  metode: 'POST',
  tls_trust: 'system',
  log_mode: 'metadata_only',
  config_schema_version: 2,
  body_template: '{"NIK":"{{input.NIK}}"}',
  parameters: [{ nama: 'TRESHOLD', nilai: '90' }],
  respons_skema: { status: 'string' },
  data_balikan: { versi: 2, elemen: ['NAMA_LGKP'] },
} as unknown as Provider;

const SECRETS: SecretRingkas[] = [
  { nama: 'USER_ID', tersamar: true, dipakai_oleh_provider_count: 1 },
  { nama: 'KUNCI_UJI', tersamar: true, dipakai_oleh_provider_count: 0 },
];

const NILAI = 'nilai-uji-xq-9f3';

function stubClient(overrides: Partial<ApiClient> = {}): ApiClient {
  return {
    listProviders: vi.fn().mockResolvedValue([]),
    getProvider: vi.fn().mockResolvedValue(PROVIDER),
    createProvider: vi.fn(),
    updateProvider: vi.fn(),
    listSecrets: vi.fn().mockResolvedValue(SECRETS),
    putSecret: vi.fn().mockResolvedValue(undefined),
    deleteSecret: vi.fn().mockResolvedValue(undefined),
    ...overrides,
  } as unknown as ApiClient;
}

function asRole(role: AdminRole | null, ui: ReactNode) {
  return renderWithI18n(<RoleProvider current={role}>{ui}</RoleProvider>);
}

async function paste(label: string, text: string) {
  const field = screen.getByLabelText(label);
  await user.click(field);
  await user.paste(text);
}

describe('S02 daftar provider', () => {
  it('shows the empty state with both actions', async () => {
    asRole('platform_admin', <ProviderListScreen client={stubClient()} />);

    expect(await screen.findByRole('heading', { name: 'Belum ada provider' })).toBeVisible();
    expect(screen.getByRole('link', { name: 'Tambah provider' })).toHaveAttribute(
      'href',
      '/provider/baru',
    );
    expect(screen.getByRole('link', { name: 'Buka menu Secret' })).toHaveAttribute(
      'href',
      '/secret',
    );
  });

  it('lists providers with their version and an edit link per row', async () => {
    const client = stubClient({ listProviders: vi.fn().mockResolvedValue([PROVIDER]) });
    asRole('platform_admin', <ProviderListScreen client={client} />);

    const table = within(await screen.findByRole('table'));
    expect(table.getByText('provider-contoh-a')).toBeInTheDocument();
    expect(table.getByText('3')).toBeInTheDocument();
    expect(table.getByText('Aktif')).toBeInTheDocument();
    expect(table.getByRole('link', { name: 'Ubah provider provider-contoh-a' })).toHaveAttribute(
      'href',
      `/provider/${PROVIDER.provider_id}`,
    );
  });

  it('shows an error with a retry that loads again', async () => {
    const listProviders = vi
      .fn()
      .mockRejectedValueOnce(new TypeError('network'))
      .mockResolvedValueOnce([PROVIDER]);
    asRole('platform_admin', <ProviderListScreen client={stubClient({ listProviders })} />);

    await user.click(await screen.findByRole('button', { name: 'Coba lagi' }));
    expect(await screen.findByText('provider-contoh-a')).toBeInTheDocument();
    expect(listProviders).toHaveBeenCalledTimes(2);
  });

  it.each(['operator', 'auditor'] as const)('shows 403 to %s without calling the API', (role) => {
    const client = stubClient();
    asRole(role, <ProviderListScreen client={client} />);

    expect(
      screen.getByRole('heading', { level: 1, name: 'Anda tidak punya akses ke halaman ini' }),
    ).toBeInTheDocument();
    expect(client.listProviders).not.toHaveBeenCalled();
  });
});

describe('S03–S04 form provider', () => {
  it('creates a provider from the form and switches to edit mode', async () => {
    const createProvider = vi.fn().mockResolvedValue({
      ...PROVIDER,
      config_schema_version: 1,
      data_balikan: undefined,
      provider_config_version: 1,
    });
    asRole('platform_admin', <ProviderFormScreen client={stubClient({ createProvider })} />);

    await paste('Nama provider', 'provider-contoh-a');
    await paste('Alamat endpoint (URL)', 'https://provider.example.test/api');
    await paste('Template body (JSON)', '{"NIK":"{{input.NIK}}"}');
    await user.click(screen.getByRole('button', { name: 'Simpan provider' }));

    await waitFor(() => expect(createProvider).toHaveBeenCalledTimes(1));
    const body = createProvider.mock.calls[0]![0] as ProviderConfig;
    expect(body).toEqual({
      nama: 'provider-contoh-a',
      base_url: 'https://provider.example.test/api',
      metode: 'POST',
      tls_trust: 'system',
      log_mode: 'metadata_only',
      config_schema_version: 1,
      body_template: '{"NIK":"{{input.NIK}}"}',
      parameters: [],
      respons_skema: {},
    });
    expect(
      await screen.findByText('Provider provider-contoh-a tersimpan sebagai versi 1.'),
    ).toBeInTheDocument();
    // Setelah dibuat, simpan berikutnya adalah PUT yang meminta alasan.
    expect(screen.getByLabelText('Alasan perubahan')).toBeInTheDocument();
  });

  it('PUTs every field back, including data_balikan, and clears the reason', async () => {
    const updateProvider = vi
      .fn()
      .mockResolvedValue({ ...PROVIDER, provider_config_version: 4, base_url: 'https://b.test' });
    asRole(
      'platform_admin',
      <ProviderFormScreen
        client={stubClient({ updateProvider })}
        providerId={PROVIDER.provider_id}
      />,
    );

    expect(
      await screen.findByRole('heading', { level: 1, name: 'Ubah provider provider-contoh-a' }),
    ).toBeInTheDocument();
    expect(screen.getByLabelText('Blok data balikan (JSON)')).toHaveValue(
      JSON.stringify(PROVIDER.data_balikan, null, 2),
    );
    await paste('Alasan perubahan', 'Ganti alamat sesuai surat BGN.');
    await user.click(screen.getByRole('button', { name: 'Simpan perubahan' }));

    await waitFor(() => expect(updateProvider).toHaveBeenCalledTimes(1));
    const [id, body] = updateProvider.mock.calls[0]! as [string, ProviderConfig];
    expect(id).toBe(PROVIDER.provider_id);
    expect(body).toEqual({
      nama: PROVIDER.nama,
      base_url: PROVIDER.base_url,
      metode: PROVIDER.metode,
      tls_trust: PROVIDER.tls_trust,
      log_mode: PROVIDER.log_mode,
      config_schema_version: 2,
      body_template: PROVIDER.body_template,
      parameters: PROVIDER.parameters,
      respons_skema: PROVIDER.respons_skema,
      data_balikan: PROVIDER.data_balikan,
      alasan: 'Ganti alamat sesuai surat BGN.',
    });
    await waitFor(() => expect(screen.getByLabelText('Alasan perubahan')).toHaveValue(''));
    expect(screen.getByLabelText('Alamat endpoint (URL)')).toHaveValue('https://b.test');
  });

  it('shows the server template error on the template field (AC-13) and focuses it', async () => {
    const createProvider = vi.fn().mockRejectedValue(
      new ApiError(422, {
        kode: 'validasi_gagal',
        errors: [
          {
            field: '/body_template',
            kode: 'template_tidak_seimbang',
            posisi: { baris: 1, kolom: 9 },
          },
        ],
      }),
    );
    asRole('platform_admin', <ProviderFormScreen client={stubClient({ createProvider })} />);

    await paste('Nama provider', 'provider-contoh-a');
    await paste('Alamat endpoint (URL)', 'https://provider.example.test/api');
    await paste('Template body (JSON)', '{"TGL":"{{tanggal_lahir}"}');
    await user.click(screen.getByRole('button', { name: 'Simpan provider' }));

    const template = screen.getByLabelText('Template body (JSON)');
    expect(
      await screen.findByText('Kurung kurawal placeholder tidak seimbang di baris 1, kolom 9.'),
    ).toBeInTheDocument();
    expect(template).toHaveAttribute('aria-invalid', 'true');
    await waitFor(() => expect(template).toHaveFocus());
    expect(screen.getByText('Periksa 1 isian yang ditandai.')).toBeInTheDocument();
  });

  it('names the missing secret from a template_secret_tidak_ada error', async () => {
    const createProvider = vi.fn().mockRejectedValue(
      new ApiError(422, {
        kode: 'validasi_gagal',
        errors: [{ field: '/body_template', kode: 'template_secret_tidak_ada', nama: 'PASSWORD' }],
      }),
    );
    asRole('platform_admin', <ProviderFormScreen client={stubClient({ createProvider })} />);

    await paste('Nama provider', 'provider-contoh-a');
    await paste('Alamat endpoint (URL)', 'https://provider.example.test/api');
    await paste('Template body (JSON)', '{"PWD":"{{secret.PASSWORD}}"}');
    await user.click(screen.getByRole('button', { name: 'Simpan provider' }));

    expect(
      await screen.findByText(
        'Secret PASSWORD belum dibuat. Buat di menu Secret, lalu simpan template lagi.',
      ),
    ).toBeInTheDocument();
  });

  it('validates required fields before calling the API and focuses the first one', async () => {
    const client = stubClient();
    asRole('platform_admin', <ProviderFormScreen client={client} />);

    await user.click(screen.getByRole('button', { name: 'Simpan provider' }));

    expect(screen.getByText('Periksa 3 isian yang ditandai.')).toBeInTheDocument();
    await waitFor(() => expect(screen.getByLabelText('Nama provider')).toHaveFocus());
    expect(client.createProvider).not.toHaveBeenCalled();
  });

  it('shows 403 to an operator', () => {
    const client = stubClient();
    asRole('operator', <ProviderFormScreen client={client} providerId={PROVIDER.provider_id} />);
    expect(
      screen.getByRole('heading', { level: 1, name: 'Anda tidak punya akses ke halaman ini' }),
    ).toBeInTheDocument();
    expect(client.getProvider).not.toHaveBeenCalled();
  });
});

describe('S05 secret', () => {
  it('lists names only, always masked, with no view or copy action', async () => {
    asRole('platform_admin', <SecretScreen client={stubClient()} />);

    const table = within(await screen.findByRole('table'));
    expect(table.getAllByText('Tersamar')).toHaveLength(2);
    expect(table.getByText('1 provider')).toBeInTheDocument();
    expect(table.getByText('Tidak dipakai')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /lihat|salin|tampilkan/i })).toBeNull();
  });

  it('stores a new secret and keeps the value out of the DOM afterwards (AC-14)', async () => {
    const client = stubClient();
    asRole('platform_admin', <SecretScreen client={client} />);

    await user.click(await screen.findByRole('button', { name: 'Tambah secret' }));
    const dialog = await screen.findByRole('dialog');
    await user.click(within(dialog).getByLabelText('Nama'));
    await user.paste('TOKEN_UJI');
    const nilai = within(dialog).getByLabelText('Nilai');
    expect(nilai).toHaveAttribute('type', 'password');
    expect(nilai).toHaveAttribute('autocomplete', 'off');
    await user.click(nilai);
    await user.paste(NILAI);
    await user.click(within(dialog).getByRole('button', { name: 'Simpan secret' }));

    await waitFor(() => expect(client.putSecret).toHaveBeenCalledWith('TOKEN_UJI', NILAI));
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(await screen.findByText('Secret TOKEN_UJI tersimpan.')).toBeInTheDocument();
    expect(document.body.innerHTML).not.toContain(NILAI);
    await waitFor(() => expect(client.listSecrets).toHaveBeenCalledTimes(2));
  });

  it('refuses a name that already exists and an invalid name without calling the API', async () => {
    const client = stubClient();
    asRole('platform_admin', <SecretScreen client={client} />);

    await user.click(await screen.findByRole('button', { name: 'Tambah secret' }));
    const dialog = await screen.findByRole('dialog');
    await user.click(within(dialog).getByLabelText('Nama'));
    await user.paste('USER_ID');
    await user.click(within(dialog).getByLabelText('Nilai'));
    await user.paste(NILAI);
    await user.click(within(dialog).getByRole('button', { name: 'Simpan secret' }));

    expect(
      within(dialog).getByText(
        'Secret dengan nama ini sudah ada. Pakai Ganti untuk mengganti nilainya.',
      ),
    ).toBeInTheDocument();

    await user.clear(within(dialog).getByLabelText('Nama'));
    await user.paste('1-salah');
    expect(
      within(dialog).getByText(
        'Nama diawali huruf, lalu huruf, angka, atau garis bawah, paling panjang 64 karakter.',
      ),
    ).toBeInTheDocument();
    expect(client.putSecret).not.toHaveBeenCalled();
  });

  it('replaces a value through S05b', async () => {
    const client = stubClient();
    asRole('platform_admin', <SecretScreen client={client} />);

    await user.click(await screen.findByRole('button', { name: 'Ganti nilai secret USER_ID' }));
    const dialog = await screen.findByRole('dialog', { name: 'Ganti secret USER_ID' });
    await user.click(within(dialog).getByLabelText('Nilai baru'));
    await user.paste(NILAI);
    await user.click(within(dialog).getByRole('button', { name: 'Ganti nilai' }));

    await waitFor(() => expect(client.putSecret).toHaveBeenCalledWith('USER_ID', NILAI));
    expect(await screen.findByText('Nilai secret USER_ID sudah diganti.')).toBeInTheDocument();
    expect(document.body.innerHTML).not.toContain(NILAI);
  });

  it('explains a 409 secret_dipakai and offers no delete button afterwards', async () => {
    const deleteSecret = vi.fn().mockRejectedValue(
      new ApiError(409, {
        kode: 'secret_dipakai',
        title: 'Secret masih dipakai',
        detail: 'Masih dipakai 1 provider aktif.',
      }),
    );
    asRole('platform_admin', <SecretScreen client={stubClient({ deleteSecret })} />);

    await user.click(await screen.findByRole('button', { name: 'Hapus secret USER_ID' }));
    const dialog = await screen.findByRole('dialog', { name: 'Hapus secret USER_ID?' });
    await user.click(within(dialog).getByRole('button', { name: 'Hapus secret' }));

    expect(await within(dialog).findByText('Secret masih dipakai')).toBeInTheDocument();
    expect(within(dialog).getByText('Masih dipakai 1 provider aktif.')).toBeInTheDocument();
    expect(within(dialog).queryByRole('button', { name: 'Hapus secret' })).toBeNull();
  });

  it('deletes an unused secret and reloads the list', async () => {
    const client = stubClient();
    asRole('platform_admin', <SecretScreen client={client} />);

    await user.click(await screen.findByRole('button', { name: 'Hapus secret KUNCI_UJI' }));
    const dialog = await screen.findByRole('dialog');
    await user.click(within(dialog).getByRole('button', { name: 'Hapus secret' }));

    await waitFor(() => expect(client.deleteSecret).toHaveBeenCalledWith('KUNCI_UJI'));
    expect(await screen.findByText('Secret KUNCI_UJI dihapus.')).toBeInTheDocument();
    await waitFor(() => expect(client.listSecrets).toHaveBeenCalledTimes(2));
  });

  it('shows the empty state with an add action', async () => {
    asRole(
      'platform_admin',
      <SecretScreen client={stubClient({ listSecrets: vi.fn().mockResolvedValue([]) })} />,
    );
    expect(await screen.findByRole('heading', { name: 'Belum ada secret' })).toBeVisible();
    expect(screen.getByRole('button', { name: 'Tambah secret' })).toBeInTheDocument();
  });

  it('shows 403 to an auditor without calling the API', () => {
    const client = stubClient();
    asRole('auditor', <SecretScreen client={client} />);
    expect(
      screen.getByRole('heading', { level: 1, name: 'Anda tidak punya akses ke halaman ini' }),
    ).toBeInTheDocument();
    expect(client.listSecrets).not.toHaveBeenCalled();
  });
});
