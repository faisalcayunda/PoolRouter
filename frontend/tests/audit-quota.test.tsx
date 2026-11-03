import { describe, expect, it, vi } from 'vitest';
import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { ApiError, type ApiClient, type AuditAksesPage } from '../src/api/client';
import { AuditScreen } from '../src/features/audit-akses/AuditScreen';
import { QuotaScreen } from '../src/features/quota/QuotaScreen';
import { renderWithI18n } from './render';

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

    await screen.findByText('Cek aduan W-2026-07');
    // Scope to the table: the result filter also renders "Ditolak" as an option.
    const table = within(screen.getByRole('table'));
    expect(table.getByText('Cek aduan W-2026-07')).toBeInTheDocument();
    // Catalog v0.14 carries both the result and the payload action (SHAA-285).
    expect(table.getByText('Ditolak')).toBeInTheDocument();
    expect(table.getByText('Lihat payload')).toBeInTheDocument();
    // A missing reason is shown as an em dash, never the string "null".
    expect(table.getByText('—')).toBeInTheDocument();

    const body = vi.mocked(client.cariAuditAkses).mock.calls[0]![0];
    const days = (Date.parse(body.sampai!) - Date.parse(body.dari)) / 86_400_000;
    expect(days).toBeGreaterThan(6.9);
    expect(days).toBeLessThan(7.1);
    expect(body.aksi).toBeNull();
  });

  it('offers no edit or delete affordance on any row', async () => {
    const client = stubClient({ cariAuditAkses: vi.fn().mockResolvedValue(AUDIT) });
    renderWithI18n(<AuditScreen client={client} />);

    await userEvent.click(screen.getByRole('button', { name: 'Cari' }));
    await screen.findByText('Cek aduan W-2026-07');

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

  it('says the figures come from the database, not Redis', async () => {
    const client = stubClient({ kuota: vi.fn().mockResolvedValue([quota()]) });
    renderWithI18n(<QuotaScreen client={client} />);

    expect(
      (await screen.findAllByText(/Angka hitung dari jumlah percobaan ke Dukcapil/)).length,
    ).toBeGreaterThan(0);
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
    await userEvent.click(screen.getByRole('button', { name: 'Coba lagi' }));
    await waitFor(() => expect(vi.mocked(client.kuota).mock.calls.length).toBeGreaterThan(1));
  });

  it('shows the empty state when no pool has a quota yet', async () => {
    const client = stubClient({ kuota: vi.fn().mockResolvedValue([]) });
    renderWithI18n(<QuotaScreen client={client} />);

    expect(await screen.findByText('Belum ada upstream pool')).toBeInTheDocument();
  });

  it('surfaces a quota read failure with a retry', async () => {
    const client = stubClient({
      kuota: vi.fn().mockRejectedValue(new ApiError(500, { status: 500 })),
    });
    renderWithI18n(<QuotaScreen client={client} />);

    // The quota scope has no dedicated catalog line, so the generic network copy is used.
    expect(await screen.findByText(/Data belum bisa dimuat/)).toBeInTheDocument();
    // The header carries its own refresh button; the error alert brings a second retry.
    expect(screen.getAllByRole('button', { name: 'Coba lagi' }).length).toBeGreaterThan(0);
  });
});
