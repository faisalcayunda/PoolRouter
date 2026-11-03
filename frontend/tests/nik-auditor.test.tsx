import { describe, expect, it, vi } from 'vitest';
import { screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { ApiError, type ApiClient, type LogPage } from '../src/api/client';
import { NikSearchScreen } from '../src/features/auditor/NikSearchScreen';
import { PayloadDialog } from '../src/features/log/PayloadDialog';
import { renderWithI18n } from './render';

const NIK = '3175000000000001';

const PAGE: LogPage = {
  baris: [
    {
      kode_transaksi: 'TRX-2026-0042',
      client_id: 'c1',
      queue_id: 'q1',
      requested_at: '2026-11-03T07:30:00.000Z',
      client_nama: 'Dukcapil Jabar',
      nik: NIK,
      status_akses: 'ok',
    },
  ],
  kursor: null,
  di_luar_retensi_online: false,
};

function stubClient(overrides: Partial<ApiClient> = {}): ApiClient {
  return {
    cariLog: vi.fn(),
    cariLogNik: vi.fn().mockResolvedValue(PAGE),
    bukaPayload: vi.fn(),
    eksporLog: vi.fn(),
    cariAuditAkses: vi.fn(),
    kuota: vi.fn(),
    upstreamPools: vi.fn(),
    ...overrides,
  } as ApiClient;
}

async function fillNikAndSearch(reason = 'Audit aduan W-2026-07') {
  await userEvent.type(screen.getByLabelText('NIK (16 digit)'), NIK);
  await userEvent.click(screen.getByRole('button', { name: 'Cari' }));

  const dialog = await screen.findByRole('dialog');
  await userEvent.type(within(dialog).getByLabelText('Alasan (minimal 10 karakter)'), reason);
  await userEvent.click(within(dialog).getByRole('button', { name: 'Cari' }));
}

describe('S12 pencarian log per NIK', () => {
  it('refuses a NIK shorter than 16 digits before asking for a reason', async () => {
    const client = stubClient();
    renderWithI18n(<NikSearchScreen client={client} />);

    await userEvent.type(screen.getByLabelText('NIK (16 digit)'), '31750');
    await userEvent.click(screen.getByRole('button', { name: 'Cari' }));

    expect(await screen.findByText('Masukkan NIK 16 digit.')).toBeInTheDocument();
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(client.cariLogNik).not.toHaveBeenCalled();
  });

  it('strips non-digits so a pasted NIK with spaces still validates', async () => {
    renderWithI18n(<NikSearchScreen client={stubClient()} />);

    const field = screen.getByLabelText('NIK (16 digit)');
    await userEvent.click(field);
    await userEvent.paste('3175 0000 0000 0001');

    expect(field).toHaveValue(NIK);
  });

  it('requires a reason before the search leaves the browser', async () => {
    const client = stubClient();
    renderWithI18n(<NikSearchScreen client={client} />);

    await userEvent.type(screen.getByLabelText('NIK (16 digit)'), NIK);
    await userEvent.click(screen.getByRole('button', { name: 'Cari' }));

    const dialog = await screen.findByRole('dialog');
    const confirm = within(dialog).getByRole('button', { name: 'Cari' });
    // Nothing is disabled silently: the confirm stays inert until the reason reaches 10 characters.
    expect(confirm).toBeDisabled();

    await userEvent.type(within(dialog).getByLabelText('Alasan (minimal 10 karakter)'), 'pendek');
    expect(confirm).toBeDisabled();
    expect(client.cariLogNik).not.toHaveBeenCalled();
  });

  it('posts the NIK, range and reason together once the reason is valid', async () => {
    const client = stubClient();
    renderWithI18n(<NikSearchScreen client={client} />);

    await fillNikAndSearch('Audit aduan W-2026-07');

    await screen.findByText('TRX-2026-0042');
    const call = vi.mocked(client.cariLogNik).mock.calls[0];
    if (!call) throw new Error('cariLogNik was not called');
    const [body, options] = call;
    expect(body).toMatchObject({ nik: NIK, alasan: 'Audit aduan W-2026-07' });
    // The reason also rides a header so it never appears in a URL or an access log line.
    expect(options?.alasan).toBe('Audit aduan W-2026-07');
  });
});

describe('payload dialog', () => {
  const payload = {
    request_id: '0b6f2a1e-0000-4000-8000-000000000000',
    kode_transaksi: 'TRX-2026-0042',
    status_akses: 'ok',
    requested_at: '2026-11-03T07:30:00.000Z',
    input: { nik: NIK },
    metadata: null,
    hasil: { status: 'sesuai' },
  };

  it('asks for a reason, then renders the read-only blocks with no copy or download action', async () => {
    const client = stubClient({ bukaPayload: vi.fn().mockResolvedValue(payload) });
    renderWithI18n(
      <PayloadDialog
        isOpen
        kodeTransaksi="TRX-2026-0042"
        initialReason=""
        client={client}
        onClose={() => {}}
      />,
    );

    const dialog = await screen.findByRole('dialog');
    await userEvent.type(
      within(dialog).getByLabelText('Alasan (minimal 10 karakter)'),
      'Cek aduan W-2026-07',
    );
    await userEvent.click(within(dialog).getByRole('button', { name: 'Buka payload' }));

    // GAP: log.payload.summary / .not_available are absent from the catalog; the block heading is
    // the screen title and an empty value renders as an em dash.
    expect(await screen.findByText('Log transaksi')).toBeInTheDocument();
    expect(screen.getByText('—')).toBeInTheDocument(); // empty `metadata`
    expect(screen.queryByRole('button', { name: /salin|unduh/i })).not.toBeInTheDocument();
    expect(vi.mocked(client.bukaPayload).mock.calls[0]![0]).toEqual({
      kode_transaksi: 'TRX-2026-0042',
      alasan: 'Cek aduan W-2026-07',
    });
  });

  it('reports a purged payload as a normal outcome, not a retryable error', async () => {
    const client = stubClient({
      bukaPayload: vi
        .fn()
        .mockRejectedValue(new ApiError(410, { status: 410, kode: 'payload_dihapus' })),
    });
    renderWithI18n(
      <PayloadDialog
        isOpen
        kodeTransaksi="TRX-2026-0042"
        initialReason="Cek aduan W-2026-07"
        client={client}
        onClose={() => {}}
      />,
    );

    const dialog = await screen.findByRole('dialog');
    await userEvent.click(within(dialog).getByRole('button', { name: 'Buka payload' }));

    expect(
      // GAP: log.payload.error_purged — the retention line is the closest catalog copy.
      await screen.findByText(/Data di luar retensi online belum bisa dicari/),
    ).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /coba lagi/i })).not.toBeInTheDocument();
  });
});
