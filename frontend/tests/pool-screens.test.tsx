import { describe, expect, it, vi } from 'vitest';
import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import type { ReactNode } from 'react';
import {
  ApiError,
  type ApiClient,
  type DownstreamPool,
  type PratinjauK20,
  type Provider,
  type UpstreamPool,
} from '../src/api/client';
import { RoleProvider, type AdminRole } from '../src/features/common/RoleContext';
import { DownstreamFormScreen } from '../src/features/pool/DownstreamFormScreen';
import { DownstreamListScreen } from '../src/features/pool/DownstreamListScreen';
import { toDownstreamConfig, emptyDraft } from '../src/features/pool/downstreamForm';
import { UpstreamFormScreen } from '../src/features/pool/UpstreamFormScreen';
import { UpstreamListScreen } from '../src/features/pool/UpstreamListScreen';
import {
  emptyDraft as emptyUpstream,
  keFraksiPersen,
  toUpstreamConfig,
} from '../src/features/pool/upstreamForm';
import { renderWithI18n } from './render';

/** S06, S06b, S07, S14 (ARC-102). Semua id dan nama sintetis. */
const user = userEvent.setup({ delay: null });

const PROVIDER_ID = '00000000-0000-4000-8000-0000000000a1';
const UP_ID = '00000000-0000-4000-8000-0000000000b1';
const UP2_ID = '00000000-0000-4000-8000-0000000000b2';
const DOWN_ID = '00000000-0000-4000-8000-0000000000c1';

const PROVIDERS = [
  { provider_id: PROVIDER_ID, nama: 'provider-contoh-a' },
] as unknown as Provider[];

function upstream(overrides: Partial<UpstreamPool> = {}): UpstreamPool {
  return {
    upstream_pool_id: UP_ID,
    provider_id: PROVIDER_ID,
    aktif: true,
    ambang_berdampingan: 4,
    batas_percobaan: 3,
    jeda_min: 200,
    jeda_maks: 800,
    jendela_kuota: 'bergulir_24j',
    kuota_harian: 500,
    margin_kuota: 0.05,
    porsi_sync: 0.1,
    rate_limit_per_jam: 60,
    reset_kuota_jam: 0,
    reset_kuota_zona: 'Asia/Jakarta',
    retry_backoff_base_detik: 2,
    retry_backoff_cap_detik: 60,
    retry_min_per_jam: 10,
    retry_rasio: 0.5,
    ...overrides,
  };
}

function downstream(overrides: Partial<DownstreamPool> = {}): DownstreamPool {
  return {
    downstream_pool_id: DOWN_ID,
    nama: 'pool-hilir-contoh',
    aktif: true,
    boleh_pinjam: false,
    prioritas: 1,
    ttl_antrean_detik: 86_400,
    upstream_pool_ids: [UP_ID],
    alokasi_jenis: 'persen',
    alokasi_nilai: 25,
    sifat: 'reserved',
    ...overrides,
  };
}

const PREVIEW: PratinjauK20 = {
  akan_cancelled: 7,
  akan_failed: 3,
  dihitung_pada: '2026-11-03T07:30:00.000Z',
  dipakai_oleh_downstream_pool_count: 2,
  upstream_aktif_terakhir_untuk: [{ downstream_pool_id: DOWN_ID, nama: 'pool-hilir-contoh' }],
};

function stubClient(overrides: Partial<ApiClient> = {}): ApiClient {
  return {
    listProviders: vi.fn().mockResolvedValue(PROVIDERS),
    upstreamPools: vi.fn().mockResolvedValue([upstream()]),
    getUpstreamPool: vi.fn().mockResolvedValue(upstream()),
    createUpstreamPool: vi.fn(),
    updateUpstreamPool: vi.fn(),
    aktifkanUpstreamPool: vi.fn().mockResolvedValue(upstream()),
    pratinjauNonaktifUpstreamPool: vi.fn().mockResolvedValue(PREVIEW),
    nonaktifkanUpstreamPool: vi.fn().mockResolvedValue({
      dilakukan_pada: '2026-11-03T07:31:00.000Z',
      jumlah_cancelled: 7,
      jumlah_failed: 3,
    }),
    listDownstreamPools: vi.fn().mockResolvedValue([downstream()]),
    getDownstreamPool: vi.fn().mockResolvedValue(downstream()),
    createDownstreamPool: vi.fn(),
    updateDownstreamPool: vi.fn(),
    aktifkanDownstreamPool: vi.fn().mockResolvedValue(downstream()),
    pratinjauNonaktifDownstreamPool: vi
      .fn()
      .mockResolvedValue({ ...PREVIEW, upstream_aktif_terakhir_untuk: null }),
    nonaktifkanDownstreamPool: vi.fn().mockResolvedValue({
      dilakukan_pada: '2026-11-03T07:31:00.000Z',
      jumlah_cancelled: 1,
      jumlah_failed: 0,
    }),
    ...overrides,
  } as unknown as ApiClient;
}

function asRole(role: AdminRole | null, ui: ReactNode) {
  return renderWithI18n(<RoleProvider current={role}>{ui}</RoleProvider>);
}

async function isi(label: string, text: string) {
  const field = screen.getByLabelText(label);
  await user.clear(field);
  await user.click(field);
  await user.paste(text);
}

const ALASAN_UJI = 'Penyesuaian kuota sesuai kontrak baru.';

describe('S06 daftar upstream pool', () => {
  it('shows the empty state with the add action', async () => {
    asRole(
      'platform_admin',
      <UpstreamListScreen client={stubClient({ upstreamPools: vi.fn().mockResolvedValue([]) })} />,
    );
    expect(await screen.findByRole('heading', { name: 'Belum ada upstream pool' })).toBeVisible();
    expect(screen.getByRole('link', { name: 'Tambah upstream pool' })).toHaveAttribute(
      'href',
      '/upstream/baru',
    );
  });

  it('shows 403 for other roles and never calls the API', () => {
    const client = stubClient();
    asRole('operator', <UpstreamListScreen client={client} />);
    expect(
      screen.getByRole('heading', { name: 'Anda tidak punya akses ke halaman ini' }),
    ).toBeVisible();
    expect(client.upstreamPools).not.toHaveBeenCalled();
  });

  it('lists a pool by its provider name with quota, rate limit and sync share', async () => {
    asRole('platform_admin', <UpstreamListScreen client={stubClient()} />);
    const table = await screen.findByRole('table');
    const row = within(table).getByRole('row', { name: /provider-contoh-a/ });
    expect(within(row).getByText('500 / hari')).toBeVisible();
    expect(within(row).getByText('10%')).toBeVisible();
    expect(within(row).getByText('Aktif')).toBeVisible();
    expect(
      within(row).getByRole('link', { name: 'Ubah upstream pool provider-contoh-a' }),
    ).toHaveAttribute('href', `/upstream/${UP_ID}`);
  });

  it('keeps the row actions on one line like the provider and client tables', async () => {
    asRole('platform_admin', <UpstreamListScreen client={stubClient()} />);
    const row = within(await screen.findByRole('row', { name: /provider-contoh-a/ }));
    const aksi = row.getByRole('link', { name: /Ubah upstream pool/ }).parentElement;
    expect(aksi).toBe(row.getByRole('button', { name: /Nonaktifkan upstream pool/ }).parentElement);
    expect(aksi).toHaveClass('flex-nowrap', 'whitespace-nowrap');
  });

  it('retries a failed read', async () => {
    const upstreamPools = vi
      .fn()
      .mockRejectedValueOnce(new ApiError(500, { status: 500, kode: 'internal' }))
      .mockResolvedValue([upstream()]);
    asRole('platform_admin', <UpstreamListScreen client={stubClient({ upstreamPools })} />);
    await user.click(await screen.findByRole('button', { name: 'Coba lagi' }));
    expect(await screen.findByRole('table')).toBeVisible();
    expect(upstreamPools).toHaveBeenCalledTimes(2);
  });

  it('activates an inactive pool and reloads the list', async () => {
    const upstreamPools = vi.fn().mockResolvedValue([upstream({ aktif: false })]);
    const client = stubClient({ upstreamPools });
    asRole('platform_admin', <UpstreamListScreen client={client} />);
    await user.click(
      await screen.findByRole('button', { name: 'Aktifkan upstream pool provider-contoh-a' }),
    );
    expect(await screen.findByText('Upstream pool provider-contoh-a diaktifkan.')).toBeVisible();
    expect(client.aktifkanUpstreamPool).toHaveBeenCalledWith(UP_ID);
    await waitFor(() => expect(upstreamPools).toHaveBeenCalledTimes(2));
  });
});

describe('S14 NonaktifkanDialog di upstream pool', () => {
  async function openDialog(client: ApiClient) {
    asRole('platform_admin', <UpstreamListScreen client={client} />);
    await user.click(
      await screen.findByRole('button', { name: 'Nonaktifkan upstream pool provider-contoh-a' }),
    );
    return screen.findByRole('dialog', { name: 'Nonaktifkan upstream pool?' });
  }

  it('shows the preview counts and the last-active warning before asking for a reason', async () => {
    const dialog = await openDialog(stubClient());
    expect(await within(dialog).findByText('7 request antre akan dibatalkan.')).toBeVisible();
    expect(
      within(dialog).getByText('3 request yang menunggu retry akan digagalkan.'),
    ).toBeVisible();
    expect(within(dialog).getByText('Dipakai oleh 2 downstream pool.')).toBeVisible();
    const warning = within(dialog).getByText('Upstream aktif terakhir untuk downstream pool ini')
      .parentElement as HTMLElement;
    expect(within(warning).getByText('pool-hilir-contoh')).toBeVisible();
  });

  it('keeps the confirm button disabled until the preview has loaded', async () => {
    let selesai: (value: PratinjauK20) => void = () => undefined;
    const pending = new Promise<PratinjauK20>((resolve) => {
      selesai = resolve;
    });
    const dialog = await openDialog(
      stubClient({ pratinjauNonaktifUpstreamPool: vi.fn().mockReturnValue(pending) }),
    );
    expect(within(dialog).getByRole('button', { name: 'Nonaktifkan' })).toBeDisabled();
    selesai(PREVIEW);
    await waitFor(() =>
      expect(within(dialog).getByRole('button', { name: 'Nonaktifkan' })).toBeEnabled(),
    );
  });

  it('blocks an empty reason and sends nothing', async () => {
    const client = stubClient();
    const dialog = await openDialog(client);
    await within(dialog).findByText('7 request antre akan dibatalkan.');
    await user.click(within(dialog).getByRole('button', { name: 'Nonaktifkan' }));
    expect(await within(dialog).findByText('Tulis alasan 10 sampai 500 karakter.')).toBeVisible();
    expect(client.nonaktifkanUpstreamPool).not.toHaveBeenCalled();
  });

  it('sends the trimmed reason, reports the result and reloads the list', async () => {
    const upstreamPools = vi.fn().mockResolvedValue([upstream()]);
    const client = stubClient({ upstreamPools });
    const dialog = await openDialog(client);
    await within(dialog).findByText('7 request antre akan dibatalkan.');
    await user.click(within(dialog).getByLabelText('Alasan penonaktifan'));
    await user.paste(`  ${ALASAN_UJI}  `);
    await user.click(within(dialog).getByRole('button', { name: 'Nonaktifkan' }));

    expect(client.nonaktifkanUpstreamPool).toHaveBeenCalledWith(UP_ID, ALASAN_UJI);
    expect(
      await screen.findByText(
        'Upstream pool provider-contoh-a dinonaktifkan: 7 dibatalkan, 3 digagalkan.',
      ),
    ).toBeVisible();
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    await waitFor(() => expect(upstreamPools).toHaveBeenCalledTimes(2));
  });

  it('keeps the dialog open and shows the failure when the server refuses', async () => {
    const client = stubClient({
      nonaktifkanUpstreamPool: vi
        .fn()
        .mockRejectedValue(new ApiError(500, { status: 500, kode: 'internal' })),
    });
    const dialog = await openDialog(client);
    await within(dialog).findByText('7 request antre akan dibatalkan.');
    await user.click(within(dialog).getByLabelText('Alasan penonaktifan'));
    await user.paste(ALASAN_UJI);
    await user.click(within(dialog).getByRole('button', { name: 'Nonaktifkan' }));

    expect(
      await within(dialog).findByText('Data belum bisa dimuat. Periksa koneksi, lalu coba lagi.'),
    ).toBeVisible();
    expect(screen.getByRole('dialog')).toBe(dialog);
    // Setelah gagal, admin boleh mencoba lagi dengan alasan yang sama.
    expect(within(dialog).getByRole('button', { name: 'Nonaktifkan' })).toBeEnabled();
  });
});

describe('S06b form upstream pool', () => {
  it('prefills every field from the read, with the sync share as a clean percentage', async () => {
    asRole('platform_admin', <UpstreamFormScreen client={stubClient()} poolId={UP_ID} />);
    expect(
      await screen.findByRole('heading', { name: 'Ubah upstream pool provider-contoh-a' }),
    ).toBeVisible();
    expect(screen.getByLabelText('Porsi sync (%)')).toHaveValue('10');
    expect(screen.getByLabelText('Kuota harian (request)')).toHaveValue('500');
    expect(screen.getByLabelText('Margin kuota (0,001 sampai 0,2)')).toHaveValue('0.05');
    expect(screen.getByLabelText('Zona waktu reset')).toHaveValue('Asia/Jakarta');
  });

  it('requires a reason on update and sends the full body when given one', async () => {
    const updateUpstreamPool = vi.fn().mockResolvedValue(upstream({ kuota_harian: 900 }));
    asRole(
      'platform_admin',
      <UpstreamFormScreen client={stubClient({ updateUpstreamPool })} poolId={UP_ID} />,
    );
    await screen.findByRole('heading', { name: 'Ubah upstream pool provider-contoh-a' });
    await isi('Kuota harian (request)', '900');

    await user.click(screen.getByRole('button', { name: 'Simpan perubahan' }));
    expect(await screen.findByText('Tulis alasan 10 sampai 500 karakter.')).toBeVisible();
    expect(updateUpstreamPool).not.toHaveBeenCalled();

    await isi('Alasan perubahan', ALASAN_UJI);
    await user.click(screen.getByRole('button', { name: 'Simpan perubahan' }));

    await waitFor(() => expect(updateUpstreamPool).toHaveBeenCalledTimes(1));
    expect(updateUpstreamPool).toHaveBeenCalledWith(UP_ID, {
      provider_id: PROVIDER_ID,
      kuota_harian: 900,
      rate_limit_per_jam: 60,
      porsi_sync: 0.1,
      ambang_berdampingan: 4,
      jeda_min: 200,
      jeda_maks: 800,
      jendela_kuota: 'bergulir_24j',
      reset_kuota_jam: 0,
      reset_kuota_zona: 'Asia/Jakarta',
      batas_percobaan: 3,
      retry_backoff_base_detik: 2,
      retry_backoff_cap_detik: 60,
      retry_min_per_jam: 10,
      retry_rasio: 0.5,
      margin_kuota: 0.05,
      alasan: ALASAN_UJI,
    });
    expect(await screen.findByText('Upstream pool disimpan.')).toBeVisible();
  });

  it('rejects a delay above what the hourly rate limit allows', async () => {
    const updateUpstreamPool = vi.fn();
    asRole(
      'platform_admin',
      <UpstreamFormScreen client={stubClient({ updateUpstreamPool })} poolId={UP_ID} />,
    );
    await screen.findByRole('heading', { name: 'Ubah upstream pool provider-contoh-a' });
    await isi('Rate limit per jam (request)', '36000');
    await isi('Alasan perubahan', ALASAN_UJI);
    await user.click(screen.getByRole('button', { name: 'Simpan perubahan' }));

    // 3 600 000 / 36 000 = 100 ms; jeda minimum 200 dan maksimum 800 sama-sama melebihi.
    expect(
      (await screen.findAllByText('Jeda paling lama 100 ms agar rate limit per jam tercapai.'))
        .length,
    ).toBe(2);
    expect(updateUpstreamPool).not.toHaveBeenCalled();
  });

  it('shows the 403 state to other roles', () => {
    asRole('auditor', <UpstreamFormScreen client={stubClient()} poolId={UP_ID} />);
    expect(
      screen.getByRole('heading', { name: 'Anda tidak punya akses ke halaman ini' }),
    ).toBeVisible();
  });
});

describe('upstreamForm', () => {
  it('turns a fraction into a percentage without floating point tails', () => {
    expect(keFraksiPersen(0.1)).toBe('10');
    expect(keFraksiPersen(0.07)).toBe('7');
    expect(keFraksiPersen(0.0025)).toBe('0.25');
  });

  it('asks for a reason with the calendar window even on create', () => {
    const draft = {
      ...emptyUpstream(),
      provider_id: PROVIDER_ID,
      kuota_harian: '100',
      rate_limit_per_jam: '60',
      porsi_sync_persen: '10',
      ambang_berdampingan: '2',
      jeda_min: '0',
      jeda_maks: '100',
      jendela_kuota: 'kalender' as const,
    };
    const built = toUpstreamConfig(draft, 'create');
    expect(built.ok).toBe(false);
    if (!built.ok) expect(built.errors.alasan?.kode).toBe('alasan_rentang');
    const ok = toUpstreamConfig({ ...draft, alasan: ALASAN_UJI }, 'create');
    expect(ok.ok).toBe(true);
  });

  it('rejects a backoff cap below the base', () => {
    const draft = {
      ...emptyUpstream(),
      provider_id: PROVIDER_ID,
      kuota_harian: '100',
      rate_limit_per_jam: '60',
      porsi_sync_persen: '10',
      ambang_berdampingan: '2',
      jeda_min: '0',
      jeda_maks: '100',
      retry_backoff_base_detik: '30',
      retry_backoff_cap_detik: '10',
    };
    const built = toUpstreamConfig(draft, 'create');
    expect(built.ok).toBe(false);
    if (!built.ok) expect(built.errors.retry_backoff_cap_detik?.kode).toBe('cap_di_bawah_base');
  });
});

describe('S07 daftar downstream pool', () => {
  it('shows the upstream provider name, the allocation and the TTL in hours', async () => {
    asRole('platform_admin', <DownstreamListScreen client={stubClient()} />);
    const row = await screen.findByRole('row', { name: /pool-hilir-contoh/ });
    expect(within(row).getByText('provider-contoh-a')).toBeVisible();
    expect(
      within(row).getByText('25% kuota efektif · Reserved: hanya dipakai pemiliknya'),
    ).toBeVisible();
    expect(within(row).getByText('24 jam')).toBeVisible();
  });

  it('shows the empty state and the 403 state', async () => {
    const view = asRole(
      'platform_admin',
      <DownstreamListScreen
        client={stubClient({ listDownstreamPools: vi.fn().mockResolvedValue([]) })}
      />,
    );
    expect(await screen.findByRole('heading', { name: 'Belum ada downstream pool' })).toBeVisible();
    view.unmount();
    const client = stubClient();
    asRole('auditor', <DownstreamListScreen client={client} />);
    expect(
      screen.getByRole('heading', { name: 'Anda tidak punya akses ke halaman ini' }),
    ).toBeVisible();
    expect(client.listDownstreamPools).not.toHaveBeenCalled();
  });

  it('deactivates through the dialog without a last-active warning', async () => {
    const listDownstreamPools = vi.fn().mockResolvedValue([downstream()]);
    const client = stubClient({ listDownstreamPools });
    asRole('platform_admin', <DownstreamListScreen client={client} />);
    await user.click(
      await screen.findByRole('button', { name: 'Nonaktifkan downstream pool pool-hilir-contoh' }),
    );
    const dialog = await screen.findByRole('dialog', { name: 'Nonaktifkan downstream pool?' });
    await within(dialog).findByText('7 request antre akan dibatalkan.');
    expect(
      within(dialog).queryByText('Upstream aktif terakhir untuk downstream pool ini'),
    ).toBeNull();
    await user.click(within(dialog).getByLabelText('Alasan penonaktifan'));
    await user.paste(ALASAN_UJI);
    await user.click(within(dialog).getByRole('button', { name: 'Nonaktifkan' }));

    expect(client.nonaktifkanDownstreamPool).toHaveBeenCalledWith(DOWN_ID, ALASAN_UJI);
    expect(
      await screen.findByText(
        'Downstream pool pool-hilir-contoh dinonaktifkan: 1 dibatalkan, 0 digagalkan.',
      ),
    ).toBeVisible();
    await waitFor(() => expect(listDownstreamPools).toHaveBeenCalledTimes(2));
  });
});

describe('S07 form downstream pool', () => {
  it('prefills from the read and never sends prioritas on update', async () => {
    const updateDownstreamPool = vi.fn().mockResolvedValue(downstream({ alokasi_nilai: 30 }));
    asRole(
      'platform_admin',
      <DownstreamFormScreen client={stubClient({ updateDownstreamPool })} poolId={DOWN_ID} />,
    );
    expect(
      await screen.findByRole('heading', { name: 'Ubah downstream pool pool-hilir-contoh' }),
    ).toBeVisible();
    expect(screen.getByLabelText('TTL antrean (jam)')).toHaveValue('24');
    expect(screen.getByLabelText('Persen kuota efektif (0,01 sampai 100)')).toHaveValue('25');

    await isi('Persen kuota efektif (0,01 sampai 100)', '30');
    await isi('Alasan perubahan', ALASAN_UJI);
    await user.click(screen.getByRole('button', { name: 'Simpan perubahan' }));

    await waitFor(() => expect(updateDownstreamPool).toHaveBeenCalledTimes(1));
    const body = updateDownstreamPool.mock.calls[0]?.[1] as Record<string, unknown>;
    expect(body).toEqual({
      nama: 'pool-hilir-contoh',
      upstream_pool_ids: [UP_ID],
      alokasi_jenis: 'persen',
      alokasi_nilai: 30,
      sifat: 'reserved',
      boleh_pinjam: false,
      ttl_antrean_detik: 86_400,
      alasan: ALASAN_UJI,
    });
    expect(body).not.toHaveProperty('prioritas');
  });

  it('shows the excess from a 422 alokasi_kelebihan on the value field', async () => {
    const updateDownstreamPool = vi.fn().mockRejectedValue(
      new ApiError(422, {
        status: 422,
        kode: 'alokasi_kelebihan',
        errors: [{ field: '/alokasi_nilai', kode: 'alokasi_kelebihan', batas: 120 }],
      }),
    );
    asRole(
      'platform_admin',
      <DownstreamFormScreen client={stubClient({ updateDownstreamPool })} poolId={DOWN_ID} />,
    );
    await screen.findByRole('heading', { name: 'Ubah downstream pool pool-hilir-contoh' });
    await isi('Alasan perubahan', ALASAN_UJI);
    await user.click(screen.getByRole('button', { name: 'Simpan perubahan' }));

    const field = screen.getByLabelText('Persen kuota efektif (0,01 sampai 100)');
    expect(
      await screen.findByText('Total alokasi melebihi kuota efektif sebanyak 120 request.'),
    ).toBeVisible();
    expect(field).toBeInvalid();
  });

  it('creates a pool without allocation, converting TTL hours to seconds', async () => {
    const createDownstreamPool = vi.fn().mockResolvedValue(downstream({ alokasi_jenis: null }));
    asRole(
      'platform_admin',
      <DownstreamFormScreen client={stubClient({ createDownstreamPool })} />,
    );
    await screen.findByRole('heading', { name: 'Tambah downstream pool' });
    await user.click(screen.getByLabelText('Nama'));
    await user.paste('pool-hilir-baru');
    await isi('TTL antrean (jam)', '2,5');
    await user.click(screen.getByRole('button', { name: /Upstream pool/ }));
    await user.click(await screen.findByRole('option', { name: 'provider-contoh-a' }));
    await user.click(screen.getByRole('button', { name: 'Simpan downstream pool' }));

    await waitFor(() => expect(createDownstreamPool).toHaveBeenCalledTimes(1));
    expect(createDownstreamPool).toHaveBeenCalledWith({
      nama: 'pool-hilir-baru',
      upstream_pool_ids: [UP_ID],
      alokasi_jenis: null,
      alokasi_nilai: null,
      sifat: null,
      boleh_pinjam: true,
      ttl_antrean_detik: 9000,
    });
  });
});

describe('downstreamForm', () => {
  it('requires the budget type and a whole number for a fixed allocation', () => {
    const base = { ...emptyDraft(), nama: 'x', upstream_pool_id: UP2_ID, jenis: 'tetap' as const };
    const built = toDownstreamConfig({ ...base, nilai: '2,5' }, 'create');
    expect(built.ok).toBe(false);
    if (!built.ok) {
      expect(built.errors.nilai?.kode).toBe('tetap_bulat');
      expect(built.errors.sifat?.kode).toBe('wajib');
    }
  });
});
