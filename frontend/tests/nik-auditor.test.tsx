import { useState } from 'react';
import { describe, expect, it, vi } from 'vitest';
import { screen, waitFor, within } from '@testing-library/react';
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
    await userEvent.click(within(dialog).getByRole('button', { name: 'Lihat payload' }));

    // Catalog v0.14 supplies the §18.3 payload block copy (SHAA-285).
    expect(await screen.findByText('Ringkasan transaksi')).toBeInTheDocument();
    expect(screen.getByText('Tidak tersedia.')).toBeInTheDocument(); // empty `metadata`
    expect(screen.queryByRole('button', { name: /salin|unduh/i })).not.toBeInTheDocument();
    expect(vi.mocked(client.bukaPayload).mock.calls[0]![0]).toEqual({
      kode_transaksi: 'TRX-2026-0042',
      alasan: 'Cek aduan W-2026-07',
    });
  });

  it('keeps the NIK out of every persistent store and drops the payload on close', async () => {
    // Section 18.3: the payload lives in the dialog only. Nothing about it — and no NIK — may be
    // written to localStorage, sessionStorage or IndexedDB, and closing the dialog must forget it.
    const client = stubClient({ bukaPayload: vi.fn().mockResolvedValue(payload) });
    const { unmount } = renderWithI18n(
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
    await userEvent.click(within(dialog).getByRole('button', { name: 'Lihat payload' }));

    // The NIK did render inside the Input JSON block, so the assertions below are not vacuous.
    expect(await screen.findByText('Ringkasan transaksi')).toBeInTheDocument();
    expect(screen.getByRole('dialog').textContent).toContain(NIK);

    expect(localStorage.length).toBe(0);
    expect(sessionStorage.length).toBe(0);
    // jsdom ships no IndexedDB; the e2e runs the same check in a real browser
    // (`e2e/log-auditor.spec.ts`), so here it is only asserted where the API exists.
    if (typeof indexedDB !== 'undefined') {
      expect(await indexedDB.databases()).toEqual([]);
    }

    unmount();
    expect(document.body.textContent).not.toContain(NIK);
  });

  it('drops an in-flight payload when the dialog closes, so the next opening asks again', async () => {
    // Regression (review round 1): the caller keeps the dialog mounted and only flips `isOpen`.
    // A late response for transaction A must not land in state — otherwise the next opening skips
    // the reason dialog and shows A's payload under B's heading (section 18.3, ADR-0003 3.6b).
    let resolveFirst: (view: typeof payload) => void = () => {};
    const first = new Promise<typeof payload>((resolve) => {
      resolveFirst = resolve;
    });
    const client = stubClient({
      bukaPayload: vi
        .fn()
        .mockReturnValueOnce(first)
        .mockResolvedValueOnce({ ...payload, kode_transaksi: 'TRX-2026-0099' }),
    });

    function Harness() {
      const [kode, setKode] = useState('TRX-2026-0042');
      return (
        <>
          <button type="button" onClick={() => setKode('TRX-2026-0099')}>
            pilih-trx-b
          </button>
          <PayloadDialog
            isOpen={kode !== ''}
            kodeTransaksi={kode}
            initialReason=""
            client={client}
            onClose={() => setKode('')}
          />
        </>
      );
    }

    renderWithI18n(<Harness />);

    const reasonDialog = await screen.findByRole('dialog');
    await userEvent.type(
      within(reasonDialog).getByLabelText('Alasan (minimal 10 karakter)'),
      'Cek aduan W-2026-07',
    );
    await userEvent.click(within(reasonDialog).getByRole('button', { name: 'Lihat payload' }));

    // Close while the request is still in flight (the payload dialog shows the loading state).
    await screen.findByText('Memuat…');
    // Both dialogs stay mounted, so scope to the close button that sits next to the loading line.
    const [closeButton] = screen.getAllByRole('button', { name: 'Tutup' });
    await userEvent.click(closeButton!);
    expect(vi.mocked(client.bukaPayload).mock.calls[0]![1]?.signal).toBeInstanceOf(AbortSignal);

    // The response arrives after the close: it must be ignored, not rendered.
    resolveFirst(payload);
    await waitFor(() => expect(vi.mocked(client.bukaPayload)).toHaveBeenCalledTimes(1));
    expect(screen.queryByText('Ringkasan transaksi')).not.toBeInTheDocument();

    // Reopen for another transaction: the reason dialog comes first and A's payload is nowhere.
    await userEvent.click(screen.getByRole('button', { name: 'pilih-trx-b' }));
    const reopened = await screen.findByRole('dialog');
    expect(within(reopened).getByLabelText('Alasan (minimal 10 karakter)')).toHaveValue('');
    expect(screen.queryByText('Ringkasan transaksi')).not.toBeInTheDocument();
    expect(screen.queryByText('TRX-2026-0042')).not.toBeInTheDocument();

    await userEvent.type(
      within(reopened).getByLabelText('Alasan (minimal 10 karakter)'),
      'Cek aduan W-2026-08',
    );
    await userEvent.click(within(reopened).getByRole('button', { name: 'Lihat payload' }));
    expect(await screen.findByText('Ringkasan transaksi')).toBeInTheDocument();
    expect(vi.mocked(client.bukaPayload).mock.calls[1]![0]).toEqual({
      kode_transaksi: 'TRX-2026-0099',
      alasan: 'Cek aduan W-2026-08',
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
    await userEvent.click(within(dialog).getByRole('button', { name: 'Lihat payload' }));

    expect(await screen.findByText('Payload sudah dihapus.')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /coba lagi/i })).not.toBeInTheDocument();
  });
});
