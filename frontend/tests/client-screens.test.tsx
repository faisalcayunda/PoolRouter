import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';
import { cleanup, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import type { i18n } from 'i18next';
import type { ReactNode } from 'react';
import {
  ApiError,
  type ApiClient,
  type Client,
  type Credential,
  type WebhookLog,
} from '../src/api/client';
import { peranShell } from '../src/app/AppRoot';
import { ClientDetailScreen } from '../src/features/client/ClientDetailScreen';
import { ClientListScreen } from '../src/features/client/ClientListScreen';
import { CredentialsScreen } from '../src/features/client/CredentialsScreen';
import { WebhookScreen } from '../src/features/client/WebhookScreen';
import { RoleProvider, type AdminRole } from '../src/features/common/RoleContext';
import { createI18n, loadLanguage } from '../src/i18n/i18n';
import { renderWithI18n } from './render';

/** S08, S08b, S09, S09a (ARC-104). Semua data sintetis; nilai yang tampil sekali berisi teks contoh. */
const user = userEvent.setup({ delay: null });

const CLIENT_ID = '00000000-0000-4000-8000-0000000000c1';
const LAIN_ID = '00000000-0000-4000-8000-0000000000c2';
const CRED_A = '00000000-0000-4000-8000-0000000000d1';
const CRED_B = '00000000-0000-4000-8000-0000000000d2';
const CRED_C = '00000000-0000-4000-8000-0000000000d3';
/** Dipakai di dua dialog "tampil sekali"; sengaja bukan bentuk kunci sungguhan. */
const TAMPIL_SEKALI = 'contoh-tampil-sekali-q7x';

const CLIENT_A: Client = {
  client_id: CLIENT_ID,
  nama: 'Aplikasi Contoh A',
  aktif: true,
  limiter_penolakan_burst: 20,
  limiter_penolakan_isi_ulang_per_detik: 1,
};
const CLIENT_B: Client = {
  ...CLIENT_A,
  client_id: LAIN_ID,
  nama: 'Aplikasi Contoh B',
  aktif: false,
};

const CREDS: Credential[] = [
  {
    credential_id: CRED_A,
    jenis: 'oauth2',
    status: 'aktif',
    kedaluwarsa_at: '2027-01-05T03:00:00Z',
  },
  {
    credential_id: CRED_B,
    jenis: 'api_key',
    status: 'dicabut',
    dicabut_at: '2026-10-01T03:00:00Z',
  },
];

const PRATINJAU = {
  akan_cancelled: 3,
  akan_failed: 1,
  dihitung_pada: '2026-11-10T03:00:00Z',
};

function stubClient(overrides: Partial<ApiClient> = {}): ApiClient {
  return {
    listClients: vi.fn().mockResolvedValue([CLIENT_A, CLIENT_B]),
    getClient: vi.fn().mockResolvedValue(CLIENT_A),
    addClient: vi.fn().mockResolvedValue(CLIENT_A),
    updateClient: vi.fn().mockResolvedValue(CLIENT_A),
    aktifkanClient: vi.fn().mockResolvedValue({ ...CLIENT_A, aktif: true }),
    pratinjauNonaktifClient: vi.fn().mockResolvedValue(PRATINJAU),
    nonaktifkanClient: vi.fn().mockResolvedValue({
      jumlah_cancelled: 3,
      jumlah_failed: 1,
      dilakukan_pada: '2026-11-10T03:01:00Z',
    }),
    listCredentials: vi.fn().mockResolvedValue(CREDS),
    createCredential: vi.fn().mockResolvedValue({
      credential_id: CRED_C,
      client_id: CLIENT_ID,
      secret: TAMPIL_SEKALI,
    }),
    pratinjauCabutCredential: vi.fn().mockResolvedValue(PRATINJAU),
    cabutCredential: vi.fn().mockResolvedValue({ ...CREDS[0], status: 'dicabut' }),
    getWebhookConfig: vi.fn().mockResolvedValue({
      aktif: true,
      webhook_url: 'https://client.example.test/hook',
      secret_versi: 2,
    }),
    putWebhookConfig: vi.fn().mockResolvedValue({ aktif: true }),
    listWebhookLog: vi.fn().mockResolvedValue([]),
    rotateWebhookSecret: vi.fn().mockResolvedValue({ secret_versi: 3, secret: TAMPIL_SEKALI }),
    ...overrides,
  } as unknown as ApiClient;
}

let en: i18n;

beforeAll(async () => {
  en = createI18n({ throwOnMissingKey: true });
  await loadLanguage(en, 'en');
});

function asRole(role: AdminRole | null, ui: ReactNode, lng: 'id' | 'en' = 'id') {
  return renderWithI18n(<RoleProvider current={role}>{ui}</RoleProvider>, {
    ...(lng === 'en' ? { instance: en } : {}),
  });
}

async function paste(
  label: string,
  text: string,
  scope: Pick<typeof screen, 'getByLabelText'> = screen,
) {
  const field = scope.getByLabelText(label);
  await user.click(field);
  await user.paste(text);
}

async function clear(label: string) {
  await user.clear(screen.getByLabelText(label));
}

afterEach(cleanup);

describe('peran client_owner di shell', () => {
  it('dikenali sebagai peran admin, bukan dibuang menjadi null', () => {
    expect(peranShell(['client_owner'])).toBe('client_owner');
    expect(peranShell(['tidak_dikenal'])).toBeNull();
  });
});

describe('S08 daftar client', () => {
  it('lists clients with the columns of the Penpot frame and per-row actions', async () => {
    asRole('platform_admin', <ClientListScreen client={stubClient()} />);

    const table = within(await screen.findByRole('table'));
    expect(table.getAllByRole('columnheader').map((kolom) => kolom.textContent?.trim())).toEqual([
      'Client',
      'Jenis kredensial',
      'Client ID',
      'Dibuat',
      'Status',
      'Aksi',
    ]);
    const baris = within(table.getByRole('row', { name: /Aplikasi Contoh A/ }));
    expect(baris.getByText('Aktif')).toBeInTheDocument();
    expect(baris.getByText(CLIENT_ID)).toBeInTheDocument();
    // Jenis kredensial dan waktu buat belum ada di GET /clients: placeholder, bukan nilai karangan.
    expect(baris.getAllByText('\u2014')).toHaveLength(2);
    expect(baris.queryByText(/burst, isi ulang/)).toBeNull();
    expect(table.getByRole('row', { name: /Aplikasi Contoh B/ })).toHaveTextContent('Nonaktif');
    expect(
      baris.getByRole('link', { name: 'Buat kredensial untuk client Aplikasi Contoh A' }),
    ).toHaveAttribute('href', `/client/${CLIENT_ID}/kredensial`);
    expect(baris.getByRole('link', { name: 'Kelola client Aplikasi Contoh A' })).toHaveAttribute(
      'href',
      `/client/${CLIENT_ID}`,
    );
    expect(screen.getByRole('button', { name: 'Tambah client' })).toBeInTheDocument();
    expect(
      screen.getByText(
        'Aplikasi BGN yang memanggil PoolRouter. Secret hanya tampil sekali saat dibuat.',
      ),
    ).toBeInTheDocument();
  });

  it('keeps both row actions on one line, aligned with the provider and upstream tables', async () => {
    asRole('platform_admin', <ClientListScreen client={stubClient()} />);

    const table = within(await screen.findByRole('table'));
    const baris = within(table.getByRole('row', { name: /Aplikasi Contoh A/ }));
    const aksi = baris.getByRole('link', { name: /Kelola client/ }).parentElement;
    expect(aksi).toBe(
      baris.getByRole('link', { name: /Buat kredensial untuk client/ }).parentElement,
    );
    expect(aksi).toHaveClass('flex-nowrap', 'whitespace-nowrap');
  });

  it('shows the empty state to a platform admin with the add action', async () => {
    const client = stubClient({ listClients: vi.fn().mockResolvedValue([]) });
    asRole('platform_admin', <ClientListScreen client={client} />);

    expect(await screen.findByRole('heading', { name: 'Belum ada client' })).toBeVisible();
    expect(screen.getByRole('button', { name: 'Tambah client' })).toBeInTheDocument();
  });

  it('gives client_owner the list without any write action', async () => {
    const client = stubClient({ listClients: vi.fn().mockResolvedValue([CLIENT_A]) });
    asRole('client_owner', <ClientListScreen client={client} />);

    expect(await screen.findByRole('table')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Tambah client' })).toBeNull();
    expect(screen.getByRole('link', { name: 'Kelola client Aplikasi Contoh A' })).toBeVisible();
  });

  it.each(['operator', 'auditor'] as const)('shows 403 to %s without calling the API', (role) => {
    const client = stubClient();
    asRole(role, <ClientListScreen client={client} />);

    expect(
      screen.getByRole('heading', { level: 1, name: 'Anda tidak punya akses ke halaman ini' }),
    ).toBeInTheDocument();
    expect(client.listClients).not.toHaveBeenCalled();
  });

  it('shows an error with a retry that loads again', async () => {
    const listClients = vi
      .fn()
      .mockRejectedValueOnce(new TypeError('network'))
      .mockResolvedValueOnce([CLIENT_A]);
    asRole('platform_admin', <ClientListScreen client={stubClient({ listClients })} />);

    await user.click(await screen.findByRole('button', { name: 'Coba lagi' }));
    expect(await screen.findByText('Aplikasi Contoh A')).toBeInTheDocument();
    expect(listClients).toHaveBeenCalledTimes(2);
  });

  it('validates the form locally and sends defaults as null when limiter is left empty', async () => {
    const client = stubClient();
    asRole('platform_admin', <ClientListScreen client={client} />);
    await user.click(await screen.findByRole('button', { name: 'Tambah client' }));
    const dialog = within(await screen.findByRole('dialog'));

    await user.click(dialog.getByRole('button', { name: 'Simpan client' }));
    expect(await dialog.findByText('Nama client wajib diisi.')).toBeInTheDocument();
    await paste('Nama client', 'Aplikasi Baru', dialog);
    await paste('Burst limiter penolakan', '5000', dialog);
    await user.click(dialog.getByRole('button', { name: 'Simpan client' }));
    expect(
      await dialog.findByText('Burst harus bilangan bulat 1 sampai 1000.'),
    ).toBeInTheDocument();
    expect(client.addClient).not.toHaveBeenCalled();

    await user.clear(dialog.getByLabelText('Burst limiter penolakan'));
    await user.click(dialog.getByRole('button', { name: 'Simpan client' }));
    await waitFor(() => expect(client.addClient).toHaveBeenCalledTimes(1));
    expect(client.addClient).toHaveBeenCalledWith({
      nama: 'Aplikasi Baru',
      limiter_penolakan_burst: null,
      limiter_penolakan_isi_ulang_per_detik: null,
    });
    expect(await screen.findByText('Client tersimpan')).toBeInTheDocument();
    await waitFor(() => expect(client.listClients).toHaveBeenCalledTimes(2));
  });

  it('accepts a decimal comma for the refill rate and rejects out-of-range values', async () => {
    const client = stubClient();
    asRole('platform_admin', <ClientListScreen client={client} />);
    await user.click(await screen.findByRole('button', { name: 'Tambah client' }));
    const dialog = within(await screen.findByRole('dialog'));
    await paste('Nama client', 'Aplikasi Baru', dialog);
    await paste('Isi ulang limiter per detik', '0,05', dialog);
    await user.click(dialog.getByRole('button', { name: 'Simpan client' }));
    expect(await dialog.findByText(/Isi ulang harus 0,10 sampai 100,00/)).toBeInTheDocument();

    await user.clear(dialog.getByLabelText('Isi ulang limiter per detik'));
    await paste('Isi ulang limiter per detik', '2,5', dialog);
    await user.click(dialog.getByRole('button', { name: 'Simpan client' }));
    await waitFor(() => expect(client.addClient).toHaveBeenCalledTimes(1));
    expect(client.addClient).toHaveBeenCalledWith(
      expect.objectContaining({ limiter_penolakan_isi_ulang_per_detik: 2.5 }),
    );
  });

  it('keeps the dialog open and explains a server rejection', async () => {
    const addClient = vi.fn().mockRejectedValue(new ApiError(403, { kode: 'dilarang' }));
    asRole('platform_admin', <ClientListScreen client={stubClient({ addClient })} />);
    await user.click(await screen.findByRole('button', { name: 'Tambah client' }));
    const dialog = within(await screen.findByRole('dialog'));
    await paste('Nama client', 'Aplikasi Baru', dialog);
    await user.click(dialog.getByRole('button', { name: 'Simpan client' }));

    expect(await dialog.findByText('Anda tidak punya akses ke halaman ini')).toBeInTheDocument();
    expect(screen.getByRole('dialog')).toBeInTheDocument();
  });
});

describe('S08 detail client', () => {
  it('asks for a reason only when the limiter changed, and sends it', async () => {
    const client = stubClient();
    asRole('platform_admin', <ClientDetailScreen clientId={CLIENT_ID} client={client} />);

    expect(
      await screen.findByRole('heading', { level: 1, name: 'Aplikasi Contoh A' }),
    ).toBeVisible();
    expect(screen.queryByLabelText('Alasan perubahan limiter')).toBeNull();

    await clear('Burst limiter penolakan');
    await paste('Burst limiter penolakan', '50');
    expect(screen.getByLabelText('Alasan perubahan limiter')).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Simpan' }));
    expect(
      await screen.findByText('Alasan wajib diisi dengan 10–500 karakter.'),
    ).toBeInTheDocument();
    expect(client.updateClient).not.toHaveBeenCalled();

    await paste('Alasan perubahan limiter', 'Lonjakan musiman sesuai surat BGN.');
    await user.click(screen.getByRole('button', { name: 'Simpan' }));
    await waitFor(() => expect(client.updateClient).toHaveBeenCalledTimes(1));
    expect(client.updateClient).toHaveBeenCalledWith(CLIENT_ID, {
      nama: 'Aplikasi Contoh A',
      limiter_penolakan_burst: 50,
      limiter_penolakan_isi_ulang_per_detik: 1,
      alasan: 'Lonjakan musiman sesuai surat BGN.',
    });
    expect(await screen.findByText('Perubahan client tersimpan.')).toBeInTheDocument();
  });

  it('renames without a reason and leaves the limiter untouched', async () => {
    const client = stubClient();
    asRole('platform_admin', <ClientDetailScreen clientId={CLIENT_ID} client={client} />);
    await screen.findByRole('heading', { level: 1, name: 'Aplikasi Contoh A' });

    await clear('Nama client');
    await paste('Nama client', 'Aplikasi Contoh A2');
    await user.click(screen.getByRole('button', { name: 'Simpan' }));

    await waitFor(() => expect(client.updateClient).toHaveBeenCalledTimes(1));
    expect(client.updateClient).toHaveBeenCalledWith(CLIENT_ID, {
      nama: 'Aplikasi Contoh A2',
      limiter_penolakan_burst: 20,
      limiter_penolakan_isi_ulang_per_detik: 1,
    });
  });

  it('shows the K-20 preview, demands a reason, then deactivates and reports the numbers', async () => {
    const client = stubClient();
    asRole('platform_admin', <ClientDetailScreen clientId={CLIENT_ID} client={client} />);
    await user.click(await screen.findByRole('button', { name: 'Nonaktifkan client' }));

    const dialog = within(await screen.findByRole('dialog'));
    expect(await dialog.findByText(/3 request menunggu akan dibatalkan/)).toBeInTheDocument();
    expect(
      dialog.getByText(/1 request yang menunggu percobaan ulang akan ditandai gagal/),
    ).toBeInTheDocument();
    expect(client.pratinjauNonaktifClient).toHaveBeenCalledWith(CLIENT_ID, expect.anything());

    await user.click(dialog.getByRole('button', { name: 'Nonaktifkan client' }));
    expect(client.nonaktifkanClient).not.toHaveBeenCalled();
    expect(
      await dialog.findByText('Alasan wajib diisi dengan 10–500 karakter.'),
    ).toBeInTheDocument();

    await paste('Alasan', 'Kontrak client berakhir.', dialog);
    await user.click(dialog.getByRole('button', { name: 'Nonaktifkan client' }));
    await waitFor(() =>
      expect(client.nonaktifkanClient).toHaveBeenCalledWith(CLIENT_ID, 'Kontrak client berakhir.'),
    );
    expect(
      await screen.findByText('Client dinonaktifkan. 3 request dibatalkan dan 1 ditandai gagal.'),
    ).toBeInTheDocument();
    await waitFor(() => expect(client.getClient).toHaveBeenCalledTimes(2));
  });

  it('blocks confirmation until the preview loads, and retries a failed preview', async () => {
    const pratinjau = vi
      .fn()
      .mockRejectedValueOnce(new TypeError('network'))
      .mockResolvedValueOnce(PRATINJAU);
    const client = stubClient({ pratinjauNonaktifClient: pratinjau });
    asRole('platform_admin', <ClientDetailScreen clientId={CLIENT_ID} client={client} />);
    await user.click(await screen.findByRole('button', { name: 'Nonaktifkan client' }));

    const dialog = within(await screen.findByRole('dialog'));
    await dialog.findByText('Data belum bisa dimuat. Periksa koneksi, lalu coba lagi.');
    expect(dialog.getByRole('button', { name: 'Nonaktifkan client' })).toBeDisabled();
    await user.click(dialog.getByRole('button', { name: 'Coba lagi' }));
    expect(await dialog.findByText(/3 request menunggu akan dibatalkan/)).toBeInTheDocument();
    expect(dialog.getByRole('button', { name: 'Nonaktifkan client' })).toBeEnabled();
  });

  it('says so when nothing is waiting', async () => {
    const client = stubClient({
      pratinjauNonaktifClient: vi
        .fn()
        .mockResolvedValue({ ...PRATINJAU, akan_cancelled: 0, akan_failed: 0 }),
    });
    asRole('platform_admin', <ClientDetailScreen clientId={CLIENT_ID} client={client} />);
    await user.click(await screen.findByRole('button', { name: 'Nonaktifkan client' }));

    expect(
      await within(await screen.findByRole('dialog')).findByText(
        'Tidak ada request yang menunggu.',
      ),
    ).toBeInTheDocument();
  });

  it('activates an inactive client without a K-20 dialog', async () => {
    const client = stubClient({ getClient: vi.fn().mockResolvedValue(CLIENT_B) });
    asRole('platform_admin', <ClientDetailScreen clientId={LAIN_ID} client={client} />);
    await user.click(await screen.findByRole('button', { name: 'Aktifkan client' }));

    await waitFor(() => expect(client.aktifkanClient).toHaveBeenCalledWith(LAIN_ID));
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(await screen.findByText('Client diaktifkan.')).toBeInTheDocument();
  });

  it('shows client_owner a read-only view with no write control', async () => {
    const client = stubClient();
    asRole('client_owner', <ClientDetailScreen clientId={CLIENT_ID} client={client} />);

    expect(await screen.findByText('20 burst, isi ulang 1 per detik')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Simpan' })).toBeNull();
    expect(screen.queryByRole('button', { name: 'Nonaktifkan client' })).toBeNull();
    expect(screen.queryByRole('textbox')).toBeNull();
    expect(screen.getByRole('link', { name: 'Kredensial' })).toHaveAttribute(
      'href',
      `/client/${CLIENT_ID}/kredensial`,
    );
  });

  it('marks the active section in the sub navigation', async () => {
    asRole('platform_admin', <ClientDetailScreen clientId={CLIENT_ID} client={stubClient()} />);
    const nav = within(await screen.findByRole('navigation', { name: 'Bagian client' }));
    expect(nav.getByRole('link', { name: 'Detail client' })).toHaveAttribute(
      'aria-current',
      'page',
    );
    expect(nav.getByRole('link', { name: 'Webhook dan riwayat' })).not.toHaveAttribute(
      'aria-current',
    );
  });
});

describe('S08b kredensial', () => {
  it('lists credentials with status and offers revoke only for active ones', async () => {
    asRole('platform_admin', <CredentialsScreen clientId={CLIENT_ID} client={stubClient()} />);

    const table = within(await screen.findByRole('table'));
    const aktif = table.getByRole('row', { name: new RegExp(CRED_A) });
    expect(within(aktif).getByText('Aktif')).toBeInTheDocument();
    expect(within(aktif).getByText('OAuth2 client credentials')).toBeInTheDocument();
    expect(within(aktif).getByRole('button', { name: `Cabut kredensial ${CRED_A}` })).toBeVisible();
    const dicabut = table.getByRole('row', { name: new RegExp(CRED_B) });
    expect(within(dicabut).getByText('API key')).toBeInTheDocument();
    expect(within(dicabut).getByText('Dicabut')).toBeInTheDocument();
    expect(within(dicabut).queryByRole('button')).toBeNull();
  });

  it('creates a credential, shows the secret once, and leaves no trace after closing', async () => {
    const client = stubClient();
    asRole('platform_admin', <CredentialsScreen clientId={CLIENT_ID} client={client} />);
    await user.click(await screen.findByRole('button', { name: 'Buat kredensial' }));
    const form = within(await screen.findByRole('dialog'));
    await user.click(form.getByRole('button', { name: 'Buat kredensial' }));

    await waitFor(() => expect(client.createCredential).toHaveBeenCalledTimes(1));
    // Umur bawaan 365 hari: tidak ada kedaluwarsa_at yang dikirim, server memakai bawaannya.
    expect(client.createCredential).toHaveBeenCalledWith(CLIENT_ID, { jenis: 'oauth2' });

    const once = await screen.findByRole('alertdialog', {
      name: 'Kredensial baru untuk Aplikasi Contoh A',
    });
    expect(within(once).getByLabelText('Client secret')).toHaveValue(TAMPIL_SEKALI);
    // Client ID kredensial baru tampil di atas secret, sesuai frame Penpot S08.
    expect(within(once).getByLabelText('Client ID')).toHaveValue(CLIENT_ID);
    expect(
      within(once)
        .getAllByRole('button')
        .map((b) => b.textContent),
    ).toEqual(['Salin', 'Sudah saya simpan']);
    // Tidak bisa ditutup dengan Esc, hanya lewat tombol.
    await user.keyboard('{Escape}');
    expect(screen.getByRole('alertdialog')).toBeInTheDocument();

    await user.click(within(once).getByRole('button', { name: 'Sudah saya simpan' }));
    await waitFor(() => expect(screen.queryByRole('alertdialog')).toBeNull());
    expect(document.body.innerHTML).not.toContain(TAMPIL_SEKALI);
    const holders = Array.from(document.querySelectorAll('input, textarea')).filter(
      (el) => (el as HTMLInputElement).value === TAMPIL_SEKALI,
    );
    expect(holders).toEqual([]);
    expect(screen.queryByText(/Secret disalin/)).toBeNull();
    await waitFor(() => expect(client.listCredentials).toHaveBeenCalledTimes(2));
  });

  it('copies the secret to the clipboard on request', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true });
    asRole('platform_admin', <CredentialsScreen clientId={CLIENT_ID} client={stubClient()} />);
    await user.click(await screen.findByRole('button', { name: 'Buat kredensial' }));
    await user.click(
      within(await screen.findByRole('dialog')).getByRole('button', { name: 'Buat kredensial' }),
    );
    const once = within(await screen.findByRole('alertdialog'));
    await user.click(once.getByRole('button', { name: 'Salin' }));

    await waitFor(() => expect(writeText).toHaveBeenCalledWith(TAMPIL_SEKALI));
    expect(await once.findByText('Secret disalin')).toBeInTheDocument();
  });

  it('sends an expiry date for a shorter validity and the chosen kind', async () => {
    const client = stubClient();
    asRole('platform_admin', <CredentialsScreen clientId={CLIENT_ID} client={client} />);
    await user.click(await screen.findByRole('button', { name: 'Buat kredensial' }));
    const form = within(await screen.findByRole('dialog'));
    await user.click(form.getByRole('button', { name: /Jenis kredensial/ }));
    await user.click(await screen.findByRole('option', { name: 'API key' }));
    await user.click(form.getByRole('button', { name: /Masa berlaku/ }));
    await user.click(await screen.findByRole('option', { name: '30 hari' }));
    await user.click(form.getByRole('button', { name: 'Buat kredensial' }));

    await waitFor(() => expect(client.createCredential).toHaveBeenCalledTimes(1));
    const body = vi.mocked(client.createCredential).mock.calls[0]![1];
    expect(body.jenis).toBe('api_key');
    const dalamHari = (new Date(body.kedaluwarsa_at!).getTime() - Date.now()) / 86_400_000;
    expect(dalamHari).toBeGreaterThan(29.9);
    expect(dalamHari).toBeLessThan(30.1);
  });

  it('shows the server refusal when the client already has two active credentials', async () => {
    const createCredential = vi
      .fn()
      .mockRejectedValue(new ApiError(409, { kode: 'batas_tercapai' }));
    asRole(
      'platform_admin',
      <CredentialsScreen clientId={CLIENT_ID} client={stubClient({ createCredential })} />,
    );
    await user.click(await screen.findByRole('button', { name: 'Buat kredensial' }));
    const form = within(await screen.findByRole('dialog'));
    await user.click(form.getByRole('button', { name: 'Buat kredensial' }));

    expect(await form.findByText(/sudah punya 2 kredensial aktif/)).toBeInTheDocument();
    expect(form.queryByText(/Periksa koneksi/)).toBeNull();
    expect(screen.queryByRole('alertdialog')).toBeNull();
  });

  it('disables creation at two active credentials and says why', async () => {
    const dua: Credential[] = [
      CREDS[0]!,
      { credential_id: CRED_C, jenis: 'api_key', status: 'aktif', kedaluwarsa_at: null },
    ];
    const client = stubClient({ listCredentials: vi.fn().mockResolvedValue(dua) });
    asRole('platform_admin', <CredentialsScreen clientId={CLIENT_ID} client={client} />);

    expect(await screen.findByText(/sudah punya 2 kredensial aktif/)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Buat kredensial' })).toBeDisabled();
    expect(screen.getByText('Tanpa batas')).toBeInTheDocument();
  });

  it('revokes through the K-20 dialog with a reason', async () => {
    const client = stubClient();
    asRole('platform_admin', <CredentialsScreen clientId={CLIENT_ID} client={client} />);
    await user.click(await screen.findByRole('button', { name: `Cabut kredensial ${CRED_A}` }));

    const dialog = within(await screen.findByRole('dialog'));
    expect(await dialog.findByText(/3 request menunggu akan dibatalkan/)).toBeInTheDocument();
    expect(client.pratinjauCabutCredential).toHaveBeenCalledWith(CRED_A, expect.anything());
    expect(dialog.getByText(/Pencabutan tidak bisa dibatalkan/)).toBeInTheDocument();
    await user.click(dialog.getByRole('button', { name: 'Cabut kredensial' }));
    expect(client.cabutCredential).not.toHaveBeenCalled();

    await paste('Alasan', 'Kredensial bocor di log aplikasi.', dialog);
    await user.click(dialog.getByRole('button', { name: 'Cabut kredensial' }));
    await waitFor(() =>
      expect(client.cabutCredential).toHaveBeenCalledWith(
        CRED_A,
        'Kredensial bocor di log aplikasi.',
      ),
    );
    expect(await screen.findByText('Kredensial dicabut.')).toBeInTheDocument();
    await waitFor(() => expect(client.listCredentials).toHaveBeenCalledTimes(2));
  });

  it('keeps the revoke dialog open and shows the failure when the server refuses', async () => {
    const cabutCredential = vi
      .fn()
      .mockRejectedValue(new ApiError(422, { kode: 'validasi_gagal' }));
    asRole(
      'platform_admin',
      <CredentialsScreen clientId={CLIENT_ID} client={stubClient({ cabutCredential })} />,
    );
    await user.click(await screen.findByRole('button', { name: `Cabut kredensial ${CRED_A}` }));
    const dialog = within(await screen.findByRole('dialog'));
    await dialog.findByText(/3 request menunggu/);
    await paste('Alasan', 'Kredensial bocor di log aplikasi.', dialog);
    await user.click(dialog.getByRole('button', { name: 'Cabut kredensial' }));

    expect(await dialog.findByText('Aksi belum berhasil. Coba lagi.')).toBeInTheDocument();
    expect(dialog.queryByText(/Periksa koneksi/)).toBeNull();
    expect(screen.queryByText('Kredensial dicabut.')).toBeNull();
  });

  it('lets client_owner manage credentials and gives operator a 403', async () => {
    const client = stubClient();
    asRole('client_owner', <CredentialsScreen clientId={CLIENT_ID} client={client} />);
    expect(await screen.findByRole('button', { name: 'Buat kredensial' })).toBeEnabled();
    cleanup();

    const lain = stubClient();
    asRole('operator', <CredentialsScreen clientId={CLIENT_ID} client={lain} />);
    expect(screen.getByRole('heading', { level: 1 })).toHaveTextContent('Anda tidak punya akses');
    expect(lain.listCredentials).not.toHaveBeenCalled();
  });

  it('shows the 403 problem when a client_owner opens another client', async () => {
    const getClient = vi.fn().mockRejectedValue(new ApiError(403, { kode: 'dilarang' }));
    asRole(
      'client_owner',
      <CredentialsScreen clientId={LAIN_ID} client={stubClient({ getClient })} />,
    );
    expect(await screen.findByText('Anda tidak punya akses ke halaman ini')).toBeInTheDocument();
    expect(screen.queryByRole('table')).toBeNull();
  });
});

const LOG: WebhookLog[] = [
  {
    request_id: '00000000-0000-4000-8000-0000000000e1',
    sent_at: new Date(Date.now() - 3_600_000).toISOString(),
    hasil: 'terkirim',
    http_status: 200,
    secret_versi: 2,
  },
  {
    request_id: '00000000-0000-4000-8000-0000000000e2',
    sent_at: new Date(Date.now() - 7_200_000).toISOString(),
    hasil: 'gagal_http',
    http_status: 503,
    error_class: 'provider_sementara',
    secret_versi: 2,
  },
  {
    request_id: '00000000-0000-4000-8000-0000000000e3',
    sent_at: new Date(Date.now() - 400 * 3_600_000).toISOString(),
    hasil: 'gagal_jaringan',
    error_class: 'timeout',
    secret_versi: 1,
  },
];

describe('S09 webhook', () => {
  it('treats a never-configured webhook (404) as an empty state, not an error', async () => {
    const getWebhookConfig = vi
      .fn()
      .mockRejectedValue(new ApiError(404, { kode: 'tidak_ditemukan' }));
    asRole(
      'platform_admin',
      <WebhookScreen clientId={CLIENT_ID} client={stubClient({ getWebhookConfig })} />,
    );

    expect(await screen.findByText(/Webhook belum diatur untuk client ini/)).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Coba lagi' })).toBeNull();
    expect(screen.getByLabelText('Alamat webhook')).toHaveValue('');
    expect(screen.getByText('Belum ada secret tanda tangan.')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Buat secret' })).toBeInTheDocument();
  });

  it('surfaces any other failure of the config read with a retry', async () => {
    const getWebhookConfig = vi
      .fn()
      .mockRejectedValueOnce(new ApiError(500, { kode: 'galat_server' }))
      .mockResolvedValueOnce({ aktif: true, webhook_url: 'https://client.example.test/hook' });
    asRole(
      'platform_admin',
      <WebhookScreen clientId={CLIENT_ID} client={stubClient({ getWebhookConfig })} />,
    );

    await user.click(await screen.findByRole('button', { name: 'Coba lagi' }));
    expect(await screen.findByLabelText('Alamat webhook')).toHaveValue(
      'https://client.example.test/hook',
    );
  });

  it('refuses a non-https address locally and saves a valid one', async () => {
    const client = stubClient();
    asRole('platform_admin', <WebhookScreen clientId={CLIENT_ID} client={client} />);
    const alamat = await screen.findByLabelText('Alamat webhook');
    expect(alamat).toHaveValue('https://client.example.test/hook');

    await user.clear(alamat);
    await user.click(alamat);
    await user.paste('http://client.example.test/hook');
    await user.click(screen.getByRole('button', { name: 'Simpan' }));
    expect(await screen.findByText('Alamat webhook harus diawali https://.')).toBeInTheDocument();
    expect(client.putWebhookConfig).not.toHaveBeenCalled();

    await user.clear(alamat);
    await user.click(alamat);
    await user.paste('https://baru.example.test/poolrouter');
    await user.click(screen.getByRole('switch', { name: 'Kirim webhook ke alamat ini' }));
    await user.click(screen.getByRole('button', { name: 'Simpan' }));
    await waitFor(() =>
      expect(client.putWebhookConfig).toHaveBeenCalledWith(CLIENT_ID, {
        webhook_url: 'https://baru.example.test/poolrouter',
        aktif: false,
      }),
    );
    expect(await screen.findByText('Webhook tersimpan')).toBeInTheDocument();
  });

  it('explains an allowlist rejection (422) on the address field', async () => {
    const putWebhookConfig = vi
      .fn()
      .mockRejectedValue(new ApiError(422, { kode: 'validasi_gagal' }));
    asRole(
      'platform_admin',
      <WebhookScreen clientId={CLIENT_ID} client={stubClient({ putWebhookConfig })} />,
    );
    await screen.findByLabelText('Alamat webhook');
    await user.click(screen.getByRole('button', { name: 'Simpan' }));

    expect(
      await screen.findByText('Alamat ditolak. Gunakan https dan domain yang ada di allowlist.'),
    ).toBeInTheDocument();
    expect(screen.getByLabelText('Alamat webhook')).toBeInvalid();
  });

  it('rotates the secret only after confirmation and shows the new one once', async () => {
    const client = stubClient();
    asRole('platform_admin', <WebhookScreen clientId={CLIENT_ID} client={client} />);
    expect(await screen.findByText('Secret aktif: versi 2.')).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Rotasi secret' }));

    const konfirmasi = within(
      await screen.findByRole('dialog', { name: 'Rotasi secret webhook?' }),
    );
    expect(client.rotateWebhookSecret).not.toHaveBeenCalled();
    await user.click(konfirmasi.getByRole('button', { name: 'Rotasi secret' }));

    await waitFor(() => expect(client.rotateWebhookSecret).toHaveBeenCalledWith(CLIENT_ID));
    const once = await screen.findByRole('alertdialog', {
      name: 'Secret webhook baru untuk Aplikasi Contoh A',
    });
    expect(within(once).getByLabelText('Secret tanda tangan')).toHaveValue(TAMPIL_SEKALI);
    await user.click(within(once).getByRole('button', { name: 'Sudah saya simpan' }));

    await waitFor(() => expect(screen.queryByRole('alertdialog')).toBeNull());
    expect(document.body.innerHTML).not.toContain(TAMPIL_SEKALI);
    expect(
      Array.from(document.querySelectorAll('input')).filter((el) => el.value === TAMPIL_SEKALI),
    ).toEqual([]);
    await waitFor(() => expect(client.getWebhookConfig).toHaveBeenCalledTimes(2));
  });

  it('cancelling the rotation confirmation changes nothing', async () => {
    const client = stubClient();
    asRole('platform_admin', <WebhookScreen clientId={CLIENT_ID} client={client} />);
    await user.click(await screen.findByRole('button', { name: 'Rotasi secret' }));
    await user.click(
      within(await screen.findByRole('dialog')).getByRole('button', { name: 'Batal' }),
    );

    expect(screen.queryByRole('dialog')).toBeNull();
    expect(client.rotateWebhookSecret).not.toHaveBeenCalled();
  });

  it('creates the first secret directly, without the rotation warning', async () => {
    const getWebhookConfig = vi.fn().mockResolvedValue({
      aktif: true,
      webhook_url: 'https://client.example.test/hook',
      secret_versi: null,
    });
    const client = stubClient({ getWebhookConfig });
    asRole('platform_admin', <WebhookScreen clientId={CLIENT_ID} client={client} />);
    await user.click(await screen.findByRole('button', { name: 'Buat secret' }));

    expect(await screen.findByRole('alertdialog')).toBeInTheDocument();
    expect(client.rotateWebhookSecret).toHaveBeenCalledTimes(1);
  });

  it('shows a failed secret creation inline without opening the one-time dialog', async () => {
    const getWebhookConfig = vi.fn().mockResolvedValue({ aktif: true, secret_versi: null });
    const rotateWebhookSecret = vi.fn().mockRejectedValue(new TypeError('network'));
    asRole(
      'platform_admin',
      <WebhookScreen
        clientId={CLIENT_ID}
        client={stubClient({ getWebhookConfig, rotateWebhookSecret })}
      />,
    );
    await user.click(await screen.findByRole('button', { name: 'Buat secret' }));

    expect(
      await screen.findByText('Data belum bisa dimuat. Periksa koneksi, lalu coba lagi.'),
    ).toBeInTheDocument();
    expect(screen.queryByRole('alertdialog')).toBeNull();
  });

  it('lets client_owner use the screen and gives auditor a 403 without any call', async () => {
    const client = stubClient();
    asRole('client_owner', <WebhookScreen clientId={CLIENT_ID} client={client} />);
    expect(await screen.findByLabelText('Alamat webhook')).toBeInTheDocument();
    cleanup();

    const lain = stubClient();
    asRole('auditor', <WebhookScreen clientId={CLIENT_ID} client={lain} />);
    expect(screen.getByRole('heading', { level: 1 })).toHaveTextContent('Anda tidak punya akses');
    expect(lain.getWebhookConfig).not.toHaveBeenCalled();
    expect(lain.listWebhookLog).not.toHaveBeenCalled();
  });
});

describe('S09a riwayat pengiriman', () => {
  it('lists deliveries with result, reason and the not-resent note', async () => {
    const client = stubClient({ listWebhookLog: vi.fn().mockResolvedValue(LOG) });
    asRole('platform_admin', <WebhookScreen clientId={CLIENT_ID} client={client} />);

    const table = within(await screen.findByRole('table'));
    const terkirim = table.getByRole('row', { name: /0000000000e1/ });
    expect(within(terkirim).getByText('Terkirim')).toBeInTheDocument();
    expect(within(terkirim).queryByText('Tidak dikirim ulang')).toBeNull();
    const gagal = table.getByRole('row', { name: /0000000000e2/ });
    expect(within(gagal).getByText('Gagal: respons HTTP')).toBeInTheDocument();
    expect(within(gagal).getByText('HTTP 503')).toBeInTheDocument();
    expect(within(gagal).getByText('Gangguan sementara di tujuan')).toBeInTheDocument();
    expect(within(gagal).getByText('Tidak dikirim ulang')).toBeInTheDocument();
    expect(table.getByRole('row', { name: /0000000000e3/ })).toHaveTextContent('Waktu habis');
    expect(screen.getByText('Menampilkan halaman pertama riwayat.')).toBeInTheDocument();
  });

  it('counts only failures from the last 24 hours in the warning', async () => {
    const client = stubClient({ listWebhookLog: vi.fn().mockResolvedValue(LOG) });
    asRole('platform_admin', <WebhookScreen clientId={CLIENT_ID} client={client} />);

    const peringatan = await screen.findByText(/webhook gagal dalam 24 jam terakhir/);
    expect(peringatan).toHaveTextContent('1 webhook gagal dalam 24 jam terakhir');
  });

  it('shows the empty state and sends the chosen result filter', async () => {
    const listWebhookLog = vi.fn().mockResolvedValue([]);
    asRole(
      'platform_admin',
      <WebhookScreen clientId={CLIENT_ID} client={stubClient({ listWebhookLog })} />,
    );
    expect(await screen.findByText('Belum ada pengiriman yang cocok.')).toBeInTheDocument();
    expect(listWebhookLog).toHaveBeenLastCalledWith(CLIENT_ID, {}, expect.anything());

    await user.click(screen.getByRole('button', { name: /Hasil pengiriman/ }));
    await user.click(await screen.findByRole('option', { name: 'Semua yang gagal' }));
    await waitFor(() =>
      expect(listWebhookLog).toHaveBeenLastCalledWith(
        CLIENT_ID,
        { hasil: 'gagal' },
        expect.anything(),
      ),
    );
  });

  it('retries a failed log read without touching the config card', async () => {
    const listWebhookLog = vi
      .fn()
      .mockRejectedValueOnce(new TypeError('network'))
      .mockResolvedValueOnce(LOG);
    const client = stubClient({ listWebhookLog });
    asRole('platform_admin', <WebhookScreen clientId={CLIENT_ID} client={client} />);

    expect(await screen.findByLabelText('Alamat webhook')).toBeInTheDocument();
    await user.click(await screen.findByRole('button', { name: 'Coba lagi' }));
    expect(await screen.findByRole('table')).toBeInTheDocument();
    expect(client.getWebhookConfig).toHaveBeenCalledTimes(1);
  });
});

describe('English catalog', () => {
  it('renders every K5 screen and dialog without a missing key', async () => {
    const client = stubClient({ listWebhookLog: vi.fn().mockResolvedValue(LOG) });
    asRole('platform_admin', <ClientListScreen client={client} />, 'en');
    expect(
      await screen.findByRole('heading', { level: 1, name: 'Clients and credentials' }),
    ).toBeVisible();
    expect(screen.getAllByText('20 burst, refill 1 per second')).toHaveLength(2);
    cleanup();

    asRole('platform_admin', <ClientDetailScreen clientId={CLIENT_ID} client={client} />, 'en');
    expect(await screen.findByRole('heading', { name: 'Client settings' })).toBeVisible();
    await user.click(screen.getByRole('button', { name: 'Deactivate client' }));
    const k20 = within(await screen.findByRole('dialog'));
    expect(await k20.findByText(/3 waiting requests will be cancelled/)).toBeInTheDocument();
    cleanup();

    asRole('platform_admin', <CredentialsScreen clientId={CLIENT_ID} client={client} />, 'en');
    expect(
      await screen.findByRole('heading', { level: 1, name: /Credentials · Aplikasi Contoh A/ }),
    ).toBeVisible();
    expect(screen.getByText('Revoked')).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Create credential' }));
    await user.click(
      within(await screen.findByRole('dialog')).getByRole('button', { name: 'Create credential' }),
    );
    expect(await screen.findByRole('alertdialog')).toHaveTextContent('shown only once');
    cleanup();

    asRole('platform_admin', <WebhookScreen clientId={CLIENT_ID} client={client} />, 'en');
    expect(await screen.findByText('Active secret: version 2.')).toBeInTheDocument();
    expect(await screen.findByText('Failed: HTTP response')).toBeInTheDocument();
    expect(screen.getByText('Temporary destination fault')).toBeInTheDocument();
  });
});
