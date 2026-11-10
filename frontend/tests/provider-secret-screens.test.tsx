import { describe, expect, it, vi } from 'vitest';
import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import type { ReactNode } from 'react';
import {
  ApiError,
  type ApiClient,
  type Endpoint,
  type Provider,
  type ProviderConfig,
  type SecretRingkas,
} from '../src/api/client';
import { AppShell } from '../src/app/AppShell';
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
    listEndpoints: vi.fn().mockResolvedValue([]),
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

async function openTab(name: string) {
  await user.click(screen.getByRole('tab', { name }));
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
    // S02: "Tambah provider" ada di kepala halaman dan di kartu kosong.
    const tambah = screen.getAllByRole('link', { name: 'Tambah provider' });
    expect(tambah).toHaveLength(2);
    for (const link of tambah) expect(link).toHaveAttribute('href', '/provider/baru');
    expect(
      screen.getByText(
        'Tambahkan provider untuk mulai menerima request dari aplikasi client. Siapkan dulu secret (USER_ID, PASSWORD, IP_USER) di menu Secret.',
      ),
    ).toBeInTheDocument();
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
      screen.getByRole('heading', { level: 1, name: 'Akses tidak diizinkan' }),
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
    await openTab('Template');
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

  it('names the provider in the breadcrumb instead of its id', async () => {
    asRole(
      'platform_admin',
      <AppShell pathname={`/provider/${PROVIDER.provider_id}`}>
        <ProviderFormScreen client={stubClient()} providerId={PROVIDER.provider_id} />
      </AppShell>,
    );

    const trail = within(screen.getByRole('navigation', { name: 'Jejak halaman' }));
    expect(await trail.findByText('provider-contoh-a')).toHaveAttribute('aria-current', 'page');
    expect(trail.queryByText(PROVIDER.provider_id)).toBeNull();
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
      await screen.findByRole('heading', { level: 1, name: 'provider-contoh-a' }),
    ).toBeInTheDocument();
    await openTab('Respons');
    expect(screen.getByLabelText('Blok data balikan (JSON)')).toHaveValue(
      JSON.stringify(PROVIDER.data_balikan, null, 2),
    );
    await openTab('Umum');
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
    await openTab('Template');
    await paste('Template body (JSON)', '{"TGL":"{{tanggal_lahir}"}');
    await user.click(screen.getByRole('button', { name: 'Simpan provider' }));

    const template = screen.getByLabelText('Template body (JSON)');
    expect(
      await screen.findByText('Kurung kurawal placeholder tidak seimbang di baris 1, kolom 9.'),
    ).toBeInTheDocument();
    expect(template).toHaveAttribute('aria-invalid', 'true');
    const panel = within(screen.getByRole('tabpanel'));
    expect(screen.getByText('Template belum bisa disimpan')).toBeInTheDocument();
    expect(panel.getByText('Baris 1: sintaks error')).toBeInTheDocument();
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
    await openTab('Template');
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
      screen.getByRole('heading', { level: 1, name: 'Akses tidak diizinkan' }),
    ).toBeInTheDocument();
    expect(client.getProvider).not.toHaveBeenCalled();
  });
});

describe('S03 detail provider: tab dan susunan layar Penpot', () => {
  const WITH_RULES = {
    ...PROVIDER,
    parameters: [
      { nama: 'NIK', rule: { jenis: 'panjang', panjang_min: 16, panjang_maks: 16 } },
      { nama: 'TRESHOLD', nilai: '90' },
    ],
  } as unknown as Provider;

  async function renderEdit(provider: Provider = PROVIDER) {
    asRole(
      'platform_admin',
      <ProviderFormScreen
        client={stubClient({ getProvider: vi.fn().mockResolvedValue(provider) })}
        providerId={provider.provider_id}
      />,
    );
    await screen.findByRole('heading', { level: 1, name: 'provider-contoh-a' });
  }

  it('shows the five tabs in design order with Umum selected and a single header save', async () => {
    await renderEdit();

    const tabs = within(screen.getByRole('tablist', { name: 'Bagian konfigurasi provider' }));
    expect(tabs.getAllByRole('tab').map((tab) => tab.textContent)).toEqual([
      'Umum',
      'Template',
      'Parameter',
      'Respons',
      'Endpoint',
    ]);
    expect(tabs.getByRole('tab', { name: 'Umum' })).toHaveAttribute('aria-selected', 'true');
    expect(screen.getAllByRole('button', { name: 'Simpan perubahan' })).toHaveLength(1);
    expect(screen.getByRole('link', { name: 'Batal' })).toHaveAttribute('href', '/provider');
  });

  it('lays out Umum as titled sections, with API-less controls disabled and marked', async () => {
    await renderEdit();

    const panel = within(screen.getByRole('tabpanel'));
    for (const title of [
      'Identitas provider',
      'Metode dan autentikasi',
      'Keamanan dan log',
      'Penanda data kependudukan',
      'Status',
    ]) {
      expect(panel.getByRole('heading', { level: 2, name: title })).toBeInTheDocument();
    }
    const status = within(panel.getByRole('heading', { name: 'Status' }).closest('section')!);
    expect(status.getByText('Aktif')).toBeInTheDocument();
    expect(status.getByRole('button', { name: 'Nonaktifkan provider' })).toBeDisabled();
    expect(panel.getByRole('switch', { name: 'Butuh dasar hukum untuk blend' })).toBeDisabled();
    expect(panel.getAllByText(/Menunggu API/).length).toBeGreaterThanOrEqual(2);
  });

  it('shows the template editor with the literal {{variabel}} hint on the Template tab', async () => {
    await renderEdit();
    await openTab('Template');

    const panel = within(screen.getByRole('tabpanel'));
    expect(
      panel.getByRole('heading', { level: 2, name: 'Editor template JSON request' }),
    ).toBeInTheDocument();
    expect(
      panel.getByText(
        'Variabel secret dan parameter diapit kurung kurawal ganda {{variabel}}. Kunci ditulis persis sesuai dokumen Ditjen Dukcapil.',
      ),
    ).toBeInTheDocument();
    expect(panel.getByLabelText('Template body (JSON)')).toHaveValue(PROVIDER.body_template);
    expect(screen.getByRole('button', { name: 'Simpan template' })).toBeInTheDocument();
  });

  it('lists parameters with rule type and value on the Parameter tab', async () => {
    await renderEdit(WITH_RULES);
    await openTab('Parameter');

    const panel = within(screen.getByRole('tabpanel'));
    expect(panel.getByText('Rule baru hanya berlaku untuk request baru')).toBeInTheDocument();
    const table = within(panel.getByRole('table', { name: 'Parameter dan rule provider' }));
    const rows = table.getAllByRole('row');
    expect(within(rows[1]!).getByText('NIK')).toBeInTheDocument();
    expect(within(rows[1]!).getByText('Panjang')).toBeInTheDocument();
    expect(within(rows[1]!).getByText('16 sampai 16 karakter')).toBeInTheDocument();
    expect(within(rows[2]!).getByText('TRESHOLD')).toBeInTheDocument();
    expect(within(rows[2]!).getByText('Belum ditetapkan')).toBeInTheDocument();
    expect(panel.getByRole('heading', { level: 2, name: 'Atur jenis rule' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Simpan rule' })).toBeInTheDocument();
  });

  it('shows the response schema and a placeholder for the unavailable code table', async () => {
    await renderEdit();
    await openTab('Respons');

    const panel = within(screen.getByRole('tabpanel'));
    expect(panel.getByLabelText('Skema respons (JSON format)')).toHaveValue(
      JSON.stringify(PROVIDER.respons_skema, null, 2),
    );
    expect(
      panel.getByRole('heading', { level: 2, name: 'Tabel penanganan kode respons' }),
    ).toBeInTheDocument();
    expect(panel.getByRole('button', { name: 'Tambah kode respons' })).toBeDisabled();
    const table = within(panel.getByRole('table', { name: 'Tabel penanganan kode respons' }));
    expect(table.getByRole('columnheader', { name: 'Perlakuan middleware' })).toBeInTheDocument();
    expect(table.getByText('Menunggu API')).toBeInTheDocument();
  });

  it("shows the platform-admin notice and this provider's TRESHOLD constant on the Endpoint tab", async () => {
    const endpoint = {
      endpoint_id: '00000000-0000-4000-8000-0000000000e1',
      provider_id: PROVIDER.provider_id,
      parameter_literal: { TRESHOLD: '80' },
    } as unknown as Endpoint;
    const other = {
      endpoint_id: '00000000-0000-4000-8000-0000000000e2',
      provider_id: '00000000-0000-4000-8000-0000000000b2',
      parameter_literal: { TRESHOLD: '55' },
    } as unknown as Endpoint;
    asRole(
      'platform_admin',
      <ProviderFormScreen
        client={stubClient({ listEndpoints: vi.fn().mockResolvedValue([endpoint, other]) })}
        providerId={PROVIDER.provider_id}
      />,
    );
    await screen.findByRole('heading', { level: 1, name: 'provider-contoh-a' });
    await openTab('Endpoint');

    const panel = within(screen.getByRole('tabpanel'));
    expect(await panel.findByText('Konfigurasi tingkat platform admin')).toBeInTheDocument();
    expect(
      panel.getByRole('heading', {
        level: 2,
        name: 'Parameter konstan endpoint provider-contoh-a',
      }),
    ).toBeInTheDocument();
    const field = panel.getByLabelText('TRESHOLD');
    expect(field).toHaveValue('80');
    expect(field).toHaveAttribute('readonly');
    expect(panel.queryByDisplayValue('55')).not.toBeInTheDocument();
    expect(
      panel.getByText(
        'Nilai TRESHOLD dikirimkan ke Dukcapil langsung dari middleware. Client dilarang mengirimkan nilai ini (ditolak 422).',
      ),
    ).toBeInTheDocument();
    expect(panel.getByRole('link', { name: 'Ubah di layar Endpoint' })).toHaveAttribute(
      'href',
      `/endpoint/${endpoint.endpoint_id}`,
    );
  });

  it('keeps the notice and a link to the endpoint list when the provider has no endpoint yet', async () => {
    await renderEdit();
    await openTab('Endpoint');

    const panel = within(screen.getByRole('tabpanel'));
    expect(await panel.findByText('Konfigurasi tingkat platform admin')).toBeInTheDocument();
    expect(panel.getByRole('link', { name: 'Buka daftar endpoint' })).toHaveAttribute(
      'href',
      '/endpoint',
    );
  });

  it('puts one primary action, Tambah parameter, in the Parameter header', async () => {
    await renderEdit(WITH_RULES);
    await openTab('Parameter');

    const header = within(screen.getByRole('heading', { level: 1 }).parentElement!);
    expect(header.getAllByRole('button')).toHaveLength(1);
    expect(header.getByRole('button', { name: 'Tambah parameter' })).toBeInTheDocument();
    expect(header.queryByRole('link', { name: 'Batal' })).not.toBeInTheDocument();
    await user.click(header.getByRole('button', { name: 'Tambah parameter' }));
    expect(screen.getByLabelText('Parameter dan rule (JSON)')).toHaveFocus();
  });

  it('shows five columns on the Parameter table, with Status pending the API', async () => {
    await renderEdit(WITH_RULES);
    await openTab('Parameter');

    const table = within(screen.getByRole('table', { name: 'Parameter dan rule provider' }));
    expect(table.getAllByRole('columnheader').map((c) => c.textContent)).toEqual([
      'Parameter',
      'Jenis rule',
      'Nilai rule',
      'Status',
      'Aksi',
    ]);
    expect(table.getAllByText('Menunggu API')).toHaveLength(2);
  });

  it('gives the dark template editor a high-contrast focus indicator', async () => {
    await renderEdit();
    await openTab('Template');

    const editor = screen.getByLabelText('Template body (JSON)');
    expect(editor.className).toContain('focus-visible:outline-inverse-primary');
    expect(editor.className).toContain('data-[focus-visible]:outline-inverse-primary');
  });

  it('opens the tab that owns the first invalid field before focusing it', async () => {
    const updateProvider = vi.fn();
    asRole(
      'platform_admin',
      <ProviderFormScreen
        client={stubClient({ updateProvider })}
        providerId={PROVIDER.provider_id}
      />,
    );
    await screen.findByRole('heading', { level: 1, name: 'provider-contoh-a' });
    await openTab('Parameter');
    await user.clear(screen.getByLabelText('Parameter dan rule (JSON)'));
    await user.paste('bukan json');
    await user.click(screen.getByRole('button', { name: 'Simpan rule' }));

    // Alasan kosong ada di tab Umum, tetapi galat JSON di tab Parameter urutannya lebih dulu.
    await waitFor(() => expect(screen.getByLabelText('Parameter dan rule (JSON)')).toHaveFocus());
    expect(screen.getByRole('tab', { name: 'Parameter' })).toHaveAttribute('aria-selected', 'true');
    expect(updateProvider).not.toHaveBeenCalled();
  });

  it('opens Umum when the reason is the only missing field', async () => {
    asRole(
      'platform_admin',
      <ProviderFormScreen client={stubClient()} providerId={PROVIDER.provider_id} />,
    );
    await screen.findByRole('heading', { level: 1, name: 'provider-contoh-a' });
    await openTab('Respons');
    await user.click(screen.getByRole('button', { name: 'Simpan perubahan' }));

    await waitFor(() => expect(screen.getByLabelText('Alasan perubahan')).toHaveFocus());
    expect(screen.getByRole('tab', { name: 'Umum' })).toHaveAttribute('aria-selected', 'true');
  });
});

describe('S05 secret', () => {
  it('lists names only, always masked, with no view or copy action', async () => {
    asRole('platform_admin', <SecretScreen client={stubClient()} />);

    const table = within(await screen.findByRole('table'));
    // Frame S05: the value column is twelve bullets, and the name is plain text, not monospace.
    expect(table.getAllByText('••••••••••••')).toHaveLength(2);
    // Titik hanya visual (aria-hidden); sel punya teks aksesibel "Tersamar".
    for (const dots of table.getAllByText('••••••••••••')) {
      expect(dots).toHaveAttribute('aria-hidden', 'true');
    }
    expect(table.getAllByText('Tersamar')).toHaveLength(2);
    for (const cell of table.getAllByRole('cell', { name: 'Tersamar' })) {
      expect(cell).toHaveAccessibleName('Tersamar');
    }
    expect(table.getByText('USER_ID').className).not.toContain('font-code');
    expect(table.getByText('1 provider')).toBeInTheDocument();
    expect(table.getByText('Tidak dipakai')).toBeInTheDocument();
    // Kolom S05 berurutan sesuai desain; "Diubah" belum punya data di API (celah).
    expect(table.getAllByRole('columnheader').map((th) => th.textContent)).toEqual([
      'Nama',
      'Nilai',
      'Dipakai oleh',
      'Diubah',
      'Aksi',
    ]);
    expect(table.getAllByText('Belum tersedia')).toHaveLength(2);
    expect(screen.queryByRole('button', { name: /lihat|salin|tampilkan/i })).toBeNull();
  });

  it('stores a new secret and keeps the value out of the DOM afterwards (AC-14)', async () => {
    const client = stubClient();
    asRole('platform_admin', <SecretScreen client={client} />);

    await user.click(await screen.findByRole('button', { name: 'Tambah secret' }));
    const dialog = await screen.findByRole('dialog');
    await user.click(within(dialog).getByLabelText('Nama'));
    await user.paste('TOKEN_UJI');
    expect(within(dialog).getByLabelText('Nama')).toHaveAttribute('autocomplete', 'off');
    const nilai = within(dialog).getByLabelText('Nilai');
    expect(nilai).toHaveAttribute('type', 'password');
    expect(nilai).toHaveAttribute('autocomplete', 'new-password');
    await user.click(nilai);
    await user.paste(NILAI);
    await user.click(within(dialog).getByRole('button', { name: 'Simpan secret' }));

    await waitFor(() => expect(client.putSecret).toHaveBeenCalledWith('TOKEN_UJI', NILAI));
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(await screen.findByText('Secret TOKEN_UJI tersimpan.')).toBeInTheDocument();
    expect(document.body.innerHTML).not.toContain(NILAI);
    expect(screen.queryByLabelText('Nilai')).toBeNull();
    expect([...document.querySelectorAll('input')].every((el) => el.value !== NILAI)).toBe(true);
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
    const nilaiBaru = within(dialog).getByLabelText('Nilai baru');
    expect(nilaiBaru).toHaveAttribute('type', 'password');
    expect(nilaiBaru).toHaveAttribute('autocomplete', 'new-password');
    await user.click(nilaiBaru);
    await user.paste(NILAI);
    expect(
      within(dialog).getByText(
        'Nilai tersamar. Setelah disimpan, nilai tidak bisa dilihat atau disalin.',
      ),
    ).toBeInTheDocument();
    expect(within(dialog).getByRole('button', { name: 'Batal' })).toBeInTheDocument();
    await user.click(within(dialog).getByRole('button', { name: 'Simpan' }));

    await waitFor(() => expect(client.putSecret).toHaveBeenCalledWith('USER_ID', NILAI));
    expect(await screen.findByText('Nilai secret USER_ID sudah diganti.')).toBeInTheDocument();
    expect(document.body.innerHTML).not.toContain(NILAI);
    expect(screen.queryByLabelText('Nilai baru')).toBeNull();
    expect([...document.querySelectorAll('input')].every((el) => el.value !== NILAI)).toBe(true);
  });

  it('opens "Secret masih dipakai" straight from Hapus when a provider uses it (S05)', async () => {
    const client = stubClient();
    asRole('platform_admin', <SecretScreen client={client} />);

    await user.click(await screen.findByRole('button', { name: 'Hapus secret USER_ID' }));
    const dialog = await screen.findByRole('dialog', { name: 'Secret masih dipakai' });
    expect(
      within(dialog).getByText(
        'USER_ID masih dipakai oleh provider aktif: 1 provider. Nonaktifkan atau ubah provider itu dulu, lalu hapus secret ini.',
      ),
    ).toBeInTheDocument();
    expect(within(dialog).queryByRole('button', { name: 'Hapus secret' })).toBeNull();
    await user.click(within(dialog).getByRole('button', { name: 'Mengerti' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(client.deleteSecret).not.toHaveBeenCalled();
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

    await user.click(await screen.findByRole('button', { name: 'Hapus secret KUNCI_UJI' }));
    const dialog = await screen.findByRole('dialog', { name: 'Hapus secret KUNCI_UJI?' });
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
      screen.getByRole('heading', { level: 1, name: 'Akses tidak diizinkan' }),
    ).toBeInTheDocument();
    expect(client.listSecrets).not.toHaveBeenCalled();
  });
});
