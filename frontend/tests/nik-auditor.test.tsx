import { useState } from 'react';
import { describe, expect, it, vi } from 'vitest';
import { screen, waitFor, within } from '@testing-library/react';
import { today } from '@internationalized/date';
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

// These tests used to drive input the way a person types, and that was the flake: `user.type` fires a
// real `setTimeout` before every keystroke and re-renders the whole screen once per character, so a
// 16-digit NIK plus a reason ran to dozens of timers and renders per search. Under CPU contention that
// crossed vitest's 5 s per-test budget and timed out. `delay: null` drops the inter-event timers, and
// `paste` (below) enters each field in one input event. The events and assertions are unchanged.
const user = userEvent.setup({ delay: null });

async function fillNikAndSearch(reason = 'Audit aduan W-2026-07') {
  await user.click(screen.getByLabelText('NIK (16 digit)'));
  await user.paste(NIK);
  await user.click(screen.getByRole('button', { name: 'Cari' }));

  const dialog = await screen.findByRole('dialog');
  const reasonField = within(dialog).getByLabelText('Alasan (minimal 10 karakter)');
  await user.click(reasonField);
  await user.paste(reason);
  await user.click(within(dialog).getByRole('button', { name: 'Cari' }));
}

describe('S12 pencarian log per NIK', () => {
  it('confirms the search in a snackbar only after it succeeded', async () => {
    const client = stubClient();
    renderWithI18n(<NikSearchScreen client={client} />);

    await fillNikAndSearch();

    // Section 7: the confirmation is a success signal, so it waits for the recorded request.
    expect(await screen.findByText('Pencarian dicatat')).toBeInTheDocument();
  });

  it('confirms the same search again when the auditor repeats it', async () => {
    const client = stubClient();
    renderWithI18n(<NikSearchScreen client={client} />);

    await fillNikAndSearch();
    expect(await screen.findByText('Pencarian dicatat')).toBeInTheDocument();

    // Same filters, but a second audited action: the second request is not swallowed as a
    // duplicate of the first one's key.
    await user.clear(screen.getByLabelText('NIK (16 digit)'));
    await fillNikAndSearch();
    expect(await screen.findAllByText('Pencarian dicatat')).toHaveLength(2);
    expect(client.cariLogNik).toHaveBeenCalledTimes(2);
  });

  it('does not claim a failed search was recorded', async () => {
    const client = stubClient({
      cariLogNik: vi.fn().mockRejectedValue(new ApiError(500, { status: 500 })),
    });
    renderWithI18n(<NikSearchScreen client={client} />);

    await fillNikAndSearch();

    expect(await screen.findByRole('alert')).toBeInTheDocument();
    // The write never landed, so the snackbar must stay silent.
    expect(screen.queryByText('Pencarian dicatat')).not.toBeInTheDocument();
  });

  it('refuses a NIK shorter than 16 digits before asking for a reason', async () => {
    const client = stubClient();
    renderWithI18n(<NikSearchScreen client={client} />);

    await user.type(screen.getByLabelText('NIK (16 digit)'), '31750');
    await user.click(screen.getByRole('button', { name: 'Cari' }));

    expect(await screen.findByText('Masukkan NIK 16 digit.')).toBeInTheDocument();
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(client.cariLogNik).not.toHaveBeenCalled();
  });

  it('strips non-digits so a pasted NIK with spaces still validates', async () => {
    renderWithI18n(<NikSearchScreen client={stubClient()} />);

    const field = screen.getByLabelText('NIK (16 digit)');
    await user.click(field);
    await user.paste('3175 0000 0000 0001');

    expect(field).toHaveValue(NIK);
  });

  it('requires a reason before the search leaves the browser', async () => {
    const client = stubClient();
    renderWithI18n(<NikSearchScreen client={client} />);

    await user.click(screen.getByLabelText('NIK (16 digit)'));
    await user.paste(NIK);
    await user.click(screen.getByRole('button', { name: 'Cari' }));

    const dialog = await screen.findByRole('dialog');
    const confirm = within(dialog).getByRole('button', { name: 'Cari' });
    // Section 8: submit stays enabled; pressing it reports the problem instead of doing nothing.
    expect(confirm).toBeEnabled();

    const reasonField = within(dialog).getByLabelText('Alasan (minimal 10 karakter)');
    await user.click(reasonField);
    await user.paste('pendek');
    await user.click(confirm);
    expect(
      await within(dialog).findByText('Alasan wajib diisi dengan 10–500 karakter.'),
    ).toBeInTheDocument();
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
    const day = today('Asia/Jakarta').toString();
    expect(body).toMatchObject({
      nik: NIK,
      dari: `${day}T00:00:00+07:00`,
      sampai: `${day}T23:59:59+07:00`,
      alasan: 'Audit aduan W-2026-07',
    });
    // The reason also rides a header so it never appears in a URL or an access log line.
    expect(options?.alasan).toBe('Audit aduan W-2026-07');
  });
});

describe('S12 layout from the Penpot frame', () => {
  it('shows the field help, orders the columns and lists Payload as an inline action', async () => {
    renderWithI18n(<NikSearchScreen client={stubClient()} />);

    expect(
      screen.getByText('Wajib 16 digit angka. Tidak tersimpan di riwayat, URL, atau filter.'),
    ).toBeInTheDocument();
    // No export before a search has produced results (frame S12c).
    expect(screen.queryByRole('button', { name: 'Ekspor hasil' })).not.toBeInTheDocument();

    await fillNikAndSearch();
    const table = within(await screen.findByRole('table'));
    expect(table.getAllByRole('columnheader').map((th) => th.textContent)).toEqual([
      'Waktu',
      'NIK',
      'Status akses',
      'Kode transaksi',
      'Client',
      'Aksi',
    ]);
    const row = within(table.getAllByRole('row')[1]!);
    expect(row.getByRole('button', { name: /TRX-2026-0042/ })).toHaveTextContent('Payload');
    expect(screen.getByRole('button', { name: 'Ekspor hasil' })).toBeEnabled();
    expect(screen.getByText('1–1 dari 1')).toBeInTheDocument();
    expect(
      screen.getByText(
        'Data di luar retensi online belum bisa dicari. Pencarian arsip tersedia di F1.',
      ),
    ).toBeInTheDocument();
  });

  it('uses one date range control beside the NIK field, as frame S12c does', () => {
    renderWithI18n(<NikSearchScreen client={stubClient()} />);

    const form = within(screen.getByLabelText('NIK (16 digit)').closest('form')!);
    const range = within(form.getByRole('group', { name: 'Rentang waktu' }));
    // RAC labels the trigger with the calendar label and the field label together.
    expect(range.getByRole('button', { name: /Pilih rentang tanggal/ })).toBeInTheDocument();
    expect(form.getAllByText('Rentang waktu')).toHaveLength(1);
    expect(form.queryByRole('textbox', { name: 'Dari' })).not.toBeInTheDocument();
    expect(form.queryByText('Pilih tanggal mulai')).not.toBeInTheDocument();
    expect(form.queryByText('Jam mulai')).not.toBeInTheDocument();
  });

  it('explains that a range alone is not enough next to the NIK error', async () => {
    renderWithI18n(<NikSearchScreen client={stubClient()} />);
    expect(
      screen.queryByText('Pencarian rentang waktu saja tidak tersedia; NIK wajib diisi.'),
    ).not.toBeInTheDocument();

    await user.click(screen.getByLabelText('NIK (16 digit)'));
    await user.paste('3175');
    await user.click(screen.getByRole('button', { name: 'Cari' }));

    expect(await screen.findByText('Masukkan NIK 16 digit.')).toBeInTheDocument();
    expect(
      screen.getByText('Pencarian rentang waktu saja tidak tersedia; NIK wajib diisi.'),
    ).toBeInTheDocument();
  });

  it('opens the payload with the frame copy for the reason step (S12b)', async () => {
    const bukaPayload = vi.fn().mockResolvedValue({
      request_id: '0b6f2a1e-0000-4000-8000-000000000000',
      kode_transaksi: 'TRX-2026-0042',
      status_akses: 'ok',
      requested_at: '2026-11-03T07:30:00.000Z',
      input: { nik: NIK },
      metadata: null,
      hasil: null,
    });
    renderWithI18n(<NikSearchScreen client={stubClient({ bukaPayload })} />);

    await fillNikAndSearch('Audit aduan W-2026-07');
    await user.click(await screen.findByRole('button', { name: /TRX-2026-0042/ }));

    const dialog = await screen.findByRole('dialog');
    const scoped = within(dialog);
    expect(scoped.getByText('Alasan membuka payload')).toBeInTheDocument();
    expect(scoped.getByLabelText('Alasan (minimal 10 karakter)')).toHaveValue(
      'Audit aduan W-2026-07',
    );
    expect(scoped.getByText('Terisi dari alasan pencarian. Boleh diubah.')).toBeInTheDocument();
    await user.click(scoped.getByRole('button', { name: 'Buka payload' }));
    await waitFor(() => expect(bukaPayload).toHaveBeenCalled());
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
    const reasonField = within(dialog).getByLabelText('Alasan membuka payload');
    await user.click(reasonField);
    await user.paste('Cek aduan W-2026-07');
    await user.click(within(dialog).getByRole('button', { name: 'Lihat payload' }));

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
    const reasonField = within(dialog).getByLabelText('Alasan membuka payload');
    await user.click(reasonField);
    await user.paste('Cek aduan W-2026-07');
    await user.click(within(dialog).getByRole('button', { name: 'Lihat payload' }));

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
    const firstReason = within(reasonDialog).getByLabelText('Alasan membuka payload');
    await user.click(firstReason);
    await user.paste('Cek aduan W-2026-07');
    await user.click(within(reasonDialog).getByRole('button', { name: 'Lihat payload' }));

    // Close while the request is still in flight (the payload dialog shows the loading state).
    await screen.findByText('Memuat…');
    // Both dialogs stay mounted, so scope to the close button that sits next to the loading line.
    const [closeButton] = screen.getAllByRole('button', { name: 'Tutup' });
    await user.click(closeButton!);
    expect(vi.mocked(client.bukaPayload).mock.calls[0]![1]?.signal).toBeInstanceOf(AbortSignal);

    // The response arrives after the close: it must be ignored, not rendered.
    resolveFirst(payload);
    await waitFor(() => expect(vi.mocked(client.bukaPayload)).toHaveBeenCalledTimes(1));
    expect(screen.queryByText('Ringkasan transaksi')).not.toBeInTheDocument();

    // Reopen for another transaction: the reason dialog comes first and A's payload is nowhere.
    await user.click(screen.getByRole('button', { name: 'pilih-trx-b' }));
    const reopened = await screen.findByRole('dialog');
    expect(within(reopened).getByLabelText('Alasan membuka payload')).toHaveValue('');
    expect(screen.queryByText('Ringkasan transaksi')).not.toBeInTheDocument();
    expect(screen.queryByText('TRX-2026-0042')).not.toBeInTheDocument();

    const secondReason = within(reopened).getByLabelText('Alasan membuka payload');
    await user.click(secondReason);
    await user.paste('Cek aduan W-2026-08');
    await user.click(within(reopened).getByRole('button', { name: 'Lihat payload' }));
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
    await user.click(within(dialog).getByRole('button', { name: 'Lihat payload' }));

    expect(await screen.findByText('Payload sudah dihapus.')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /coba lagi/i })).not.toBeInTheDocument();
  });

  it('turns pages with the response cursor and the same reason, without a new dialog', async () => {
    const cariLogNik = vi
      .fn()
      .mockResolvedValueOnce({ ...PAGE, kursor: 'cur-2' })
      .mockResolvedValueOnce({ ...PAGE, kursor: null });
    const client = stubClient({ cariLogNik });
    renderWithI18n(<NikSearchScreen client={client} />);

    await fillNikAndSearch('Audit aduan W-2026-07');
    await screen.findByText('TRX-2026-0042');
    expect(cariLogNik.mock.calls[0]![0].kursor).toBeUndefined();

    await user.click(screen.getByRole('button', { name: 'Halaman berikutnya' }));

    await waitFor(() => expect(cariLogNik).toHaveBeenCalledTimes(2));
    const second = cariLogNik.mock.calls[1]![0];
    expect(second.kursor).toBe('cur-2');
    expect(second.alasan).toBe('Audit aduan W-2026-07');
    expect(second.nik).toBe(NIK);
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    // The NIK never reaches the page title or any accessible label of the pager.
    expect(document.title).not.toContain(NIK);
    const pager = screen.getByRole('navigation');
    expect(pager.getAttribute('aria-label')).not.toContain(NIK);
    expect(pager.textContent).not.toContain(NIK);
    expect(window.location.href).not.toContain(NIK);
    expect(screen.getByRole('button', { name: 'Halaman berikutnya' })).toBeDisabled();
  });

  it('resets the cursor and asks for the reason again when the NIK changes', async () => {
    const cariLogNik = vi.fn().mockResolvedValue({ ...PAGE, kursor: 'cur-2' });
    renderWithI18n(<NikSearchScreen client={stubClient({ cariLogNik })} />);

    await fillNikAndSearch();
    await screen.findByText('TRX-2026-0042');
    await user.click(screen.getByRole('button', { name: 'Halaman berikutnya' }));
    await waitFor(() => expect(cariLogNik).toHaveBeenCalledTimes(2));

    await user.clear(screen.getByLabelText('NIK (16 digit)'));
    await user.click(screen.getByLabelText('NIK (16 digit)'));
    await user.paste('3175000000000002');
    await user.click(screen.getByRole('button', { name: 'Cari' }));
    // A new search needs a reason again.
    const dialog = await screen.findByRole('dialog');
    const reasonField = within(dialog).getByLabelText('Alasan (minimal 10 karakter)');
    await user.click(reasonField);
    await user.paste('Audit aduan W-2026-08');
    await user.click(within(dialog).getByRole('button', { name: 'Cari' }));

    await waitFor(() => expect(cariLogNik).toHaveBeenCalledTimes(3));
    const third = cariLogNik.mock.calls[2]![0];
    expect(third.kursor).toBeUndefined();
    expect(third.nik).toBe('3175000000000002');
  });
});
