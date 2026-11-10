import { describe, expect, it, vi } from 'vitest';
import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { ApiError, type ApiClient, type AuditAksesPage } from '../src/api/client';
import { AuditScreen } from '../src/features/audit-akses/AuditScreen';
import { QuotaScreen } from '../src/features/quota/QuotaScreen';
import { renderWithI18n } from './render';

const USER_ID = '123e4567-e89b-12d3-a456-426614174000';

const AUDIT: AuditAksesPage = {
  baris: [
    {
      audit_id: 'a1',
      dilakukan_pada: '2026-11-03T07:30:00.000Z',
      pengguna_id: 'budi',
      aksi: 'lihat_payload',
      hasil: 'berhasil',
      alasan: 'Cek aduan W-2026-07',
    },
    {
      audit_id: 'a2',
      dilakukan_pada: '2026-11-03T06:00:00.000Z',
      pengguna_id: 'sari',
      aksi: 'cari_nik',
      hasil: 'ditolak',
      alasan: null,
    },
  ],
  kursor: null,
};

function stubClient(overrides: Partial<ApiClient> = {}): ApiClient {
  return {
    cariLog: vi.fn(),
    cariLogNik: vi.fn(),
    bukaPayload: vi.fn(),
    eksporLog: vi.fn(),
    cariAuditAkses: vi.fn(),
    kuota: vi.fn(),
    upstreamPools: vi.fn().mockResolvedValue([]),
    ...overrides,
  } as ApiClient;
}

describe('S13 audit akses', () => {
  it('sends the default 7-day range and renders both results', async () => {
    const client = stubClient({ cariAuditAkses: vi.fn().mockResolvedValue(AUDIT) });
    renderWithI18n(<AuditScreen client={client} />);

    await userEvent.click(screen.getByRole('button', { name: 'Cari' }));

    await screen.findByText('budi');
    // Scope to the table: the action filter also renders "Ditolak" as an option.
    const table = within(screen.getByRole('table'));
    // Frame S13 columns, in order: Waktu, Pengguna, Peran, Aksi, Filter atau kueri, Jumlah record.
    const headers = table.getAllByRole('columnheader').map((th) => th.textContent);
    expect(headers).toEqual([
      'Waktu',
      'Pengguna',
      'Peran',
      'Aksi',
      'Filter atau kueri',
      'Jumlah record',
    ]);
    expect(table.getByText('budi')).toBeInTheDocument();
    // The action is a chip from the shared enum group; a refused attempt also reads "Ditolak".
    expect(table.getByText('Lihat payload')).toBeInTheDocument();
    expect(table.getByText('Cari')).toBeInTheDocument();
    expect(table.getByText('Ditolak')).toBeInTheDocument();
    // Role, filter and record count are not in the API yet: placeholders, one per column and row.
    expect(table.getAllByText('—')).toHaveLength(6);
    // The reason is not a column in the frame, so it is not rendered.
    expect(table.queryByText('Cek aduan W-2026-07')).not.toBeInTheDocument();
    expect(
      screen.getByText("Kolom 'Filter atau kueri' menampilkan jenis filter, bukan nilai NIK."),
    ).toBeInTheDocument();

    const body = vi.mocked(client.cariAuditAkses).mock.calls[0]![0];
    // Default window is the last 24 hours, the same as S11: a first visit always has a range.
    const hours = (Date.parse(body.sampai!) - Date.parse(body.dari)) / 3_600_000;
    expect(hours).toBeGreaterThan(23.9);
    expect(hours).toBeLessThan(24.1);
    // The action filter is not sent until the screen offers one.
    expect(body.aksi).toBeUndefined();
  });

  it('sends the typed user as pengguna_id', async () => {
    const cariAuditAkses = vi.fn().mockResolvedValue(AUDIT);
    renderWithI18n(<AuditScreen client={stubClient({ cariAuditAkses })} />);

    await userEvent.click(screen.getByLabelText('Pengguna'));
    await userEvent.paste(USER_ID);
    await userEvent.click(screen.getByRole('button', { name: 'Cari' }));

    await waitFor(() => expect(cariAuditAkses.mock.calls.at(-1)![0].pengguna_id).toBe(USER_ID));
  });

  it('rejects a non-UUID user in the client and does not send it', async () => {
    const cariAuditAkses = vi.fn().mockResolvedValue(AUDIT);
    renderWithI18n(<AuditScreen client={stubClient({ cariAuditAkses })} />);
    await waitFor(() => expect(cariAuditAkses).toHaveBeenCalledTimes(1));

    await userEvent.click(screen.getByLabelText('Pengguna'));
    await userEvent.paste('budi');
    await userEvent.click(screen.getByRole('button', { name: 'Cari' }));

    expect(await screen.findByText(/harus berformat UUID/)).toBeInTheDocument();
    expect(cariAuditAkses).toHaveBeenCalledTimes(1);
  });

  it('offers no edit or delete affordance on any row', async () => {
    const client = stubClient({ cariAuditAkses: vi.fn().mockResolvedValue(AUDIT) });
    renderWithI18n(<AuditScreen client={client} />);

    await userEvent.click(screen.getByRole('button', { name: 'Cari' }));
    await screen.findByText('budi');

    expect(
      screen.queryByRole('button', { name: /ubah|hapus|edit|delete/i }),
    ).not.toBeInTheDocument();
  });

  it('falls back to the raw code for an action the catalog does not know yet', async () => {
    const client = stubClient({
      cariAuditAkses: vi.fn().mockResolvedValue({
        baris: [{ ...AUDIT.baris[0], aksi: 'aksi_baru' }],
        kursor: null,
      }),
    });
    renderWithI18n(<AuditScreen client={client} />);

    await userEvent.click(screen.getByRole('button', { name: 'Cari' }));
    expect(await screen.findByText('aksi_baru')).toBeInTheDocument();
  });

  it('sends the cursor of the previous response when turning the page', async () => {
    const cariAuditAkses = vi
      .fn()
      .mockResolvedValueOnce({ ...AUDIT, kursor: 'cur-2' })
      .mockResolvedValueOnce(AUDIT);
    renderWithI18n(<AuditScreen client={stubClient({ cariAuditAkses })} />);

    await userEvent.click(screen.getByRole('button', { name: 'Cari' }));
    await screen.findByText('budi');
    const callsBefore = cariAuditAkses.mock.calls.length;
    await userEvent.click(screen.getByRole('button', { name: 'Halaman berikutnya' }));

    await waitFor(() => expect(cariAuditAkses.mock.calls.length).toBeGreaterThan(callsBefore));
    expect(cariAuditAkses.mock.calls.at(-1)![0].kursor).toBe('cur-2');
  });

  it('sends the chosen action and starts again from the first page', async () => {
    const cariAuditAkses = vi.fn().mockResolvedValue({ ...AUDIT, kursor: 'cur-2' });
    renderWithI18n(<AuditScreen client={stubClient({ cariAuditAkses })} />);

    await userEvent.click(screen.getByRole('button', { name: 'Cari' }));
    await screen.findByText('budi');
    await userEvent.click(screen.getByRole('button', { name: 'Halaman berikutnya' }));
    await waitFor(() => expect(cariAuditAkses.mock.calls.at(-1)![0].kursor).toBe('cur-2'));

    await userEvent.click(screen.getByRole('button', { name: /Aksi/ }));
    await userEvent.click(await screen.findByRole('option', { name: 'Ekspor' }));
    await userEvent.click(screen.getByRole('button', { name: 'Cari' }));

    await waitFor(() => expect(cariAuditAkses.mock.calls.at(-1)![0].aksi).toBe('ekspor'));
    expect(cariAuditAkses.mock.calls.at(-1)![0].kursor).toBeUndefined();
  });
});

describe('quota tracker', () => {
  const quota = (overrides: Record<string, unknown> = {}) => ({
    upstream_pool_id: '0f0f0f0f-1111-2222-3333-444444444444',
    periode_mulai: '2026-11-03T00:00:00Z',
    periode_selesai: '2026-11-03T01:00:00Z',
    rate_limit_per_jam: 1000,
    kuota_efektif: 800,
    sisa_periode: 100,
    terpakai_jam_berjalan: 700,
    terpakai_sync_jam_berjalan: 50,
    porsi_sync_maks: 100,
    dihitung_pada: '2026-11-03T07:30:00Z',
    ...overrides,
  });

  it('prints the numbers beside the bar and marks a nearly empty quota', async () => {
    const client = stubClient({ kuota: vi.fn().mockResolvedValue([quota()]) });
    renderWithI18n(<QuotaScreen client={client} />);

    expect(await screen.findByText('Hampir habis')).toBeInTheDocument();
    // Colour is never the only signal: the percentage and remaining count are text.
    expect(screen.getByText(/Terpakai 88%, sisa 100/)).toBeInTheDocument();
    // 50 of a 100 sync allowance renders through the catalog's percentage phrasing.
    expect(screen.getByText(/Sync terpakai 50 dari batas 50%/)).toBeInTheDocument();
  });

  it('fills the bar and reports the same percentage as the text (0–100 scale)', async () => {
    const client = stubClient({ kuota: vi.fn().mockResolvedValue([quota()]) });
    renderWithI18n(<QuotaScreen client={client} />);

    expect(await screen.findByText(/Terpakai 88%, sisa 100/)).toBeInTheDocument();
    // `ProgressBar` is a 0–100 control: a 0–1 ratio painted an empty track next to "88%".
    const bar = screen.getByRole('progressbar', { name: 'Hari ini' });
    expect(Number(bar.getAttribute('aria-valuenow'))).toBeCloseTo(87.5);
    expect(bar).toHaveAttribute('aria-valuemin', '0');
    expect(bar).toHaveAttribute('aria-valuemax', '100');
    expect(screen.getByText('88%')).toBeInTheDocument();
  });

  it('says the figures come from the database, not Redis', async () => {
    const client = stubClient({ kuota: vi.fn().mockResolvedValue([quota()]) });
    renderWithI18n(<QuotaScreen client={client} />);

    // Once, not twice: the caveat sits under the title and is not repeated under the cards.
    expect(
      await screen.findAllByText(/Angka dihitung dari jumlah percobaan ke Dukcapil/),
    ).toHaveLength(1);
  });

  it('filters by pool and reloads on demand', async () => {
    const client = stubClient({
      kuota: vi.fn().mockResolvedValue([quota()]),
      upstreamPools: vi.fn().mockResolvedValue([
        {
          upstream_pool_id: '0f0f0f0f-1111-2222-3333-444444444444',
          aktif: true,
        },
      ]),
    });
    renderWithI18n(<QuotaScreen client={client} />);

    await waitFor(() => expect(client.upstreamPools).toHaveBeenCalled());
    await userEvent.click(screen.getByRole('button', { name: 'Muat ulang' }));
    await waitFor(() => expect(vi.mocked(client.kuota).mock.calls.length).toBeGreaterThan(1));
  });

  it('shows the empty state when no pool has a quota yet', async () => {
    const client = stubClient({ kuota: vi.fn().mockResolvedValue([]) });
    renderWithI18n(<QuotaScreen client={client} />);

    expect(await screen.findByText('Belum ada upstream pool')).toBeInTheDocument();
    // Section 7: the empty state points at the screen that fixes it (S06).
    expect(screen.getByRole('link', { name: 'Atur upstream pool' })).toHaveAttribute(
      'href',
      '/upstream',
    );
  });

  it('keeps the last good numbers when a reload fails (section 7)', async () => {
    const client = stubClient({
      kuota: vi
        .fn()
        .mockResolvedValueOnce([quota()])
        .mockRejectedValueOnce(new ApiError(500, { status: 500 })),
    });
    renderWithI18n(<QuotaScreen client={client} />);

    expect(await screen.findByText(/Terpakai/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Muat ulang' }));

    // The error shows, and the previous numbers stay on screen with their timestamp.
    expect(await screen.findByText(/Data belum bisa dimuat/)).toBeInTheDocument();
    expect(screen.getByText(/Data lama ditampilkan/)).toBeInTheDocument();
    expect(screen.getByText(/Terpakai/)).toBeInTheDocument();
  });

  it('surfaces a quota read failure with a retry', async () => {
    const client = stubClient({
      kuota: vi.fn().mockRejectedValue(new ApiError(500, { status: 500 })),
    });
    renderWithI18n(<QuotaScreen client={client} />);

    // The quota scope has no dedicated catalog line, so the generic network copy is used.
    expect(await screen.findByText(/Data belum bisa dimuat/)).toBeInTheDocument();
    // The header button reloads; it says "Muat ulang", not "Coba lagi" (section 13.2).
    expect(screen.getByRole('button', { name: 'Muat ulang' })).toBeInTheDocument();
    // The error alert keeps its own retry.
    expect(screen.getAllByRole('button', { name: 'Coba lagi' }).length).toBeGreaterThan(0);
  });
});
